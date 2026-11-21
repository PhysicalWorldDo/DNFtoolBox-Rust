use crate::{CoreError, CoreResult};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

fn reserved(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or("").to_ascii_lowercase();
    ["con", "prn", "aux", "nul"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("com") || stem.starts_with("lpt"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}
pub(crate) fn validate_id(id: &str) -> CoreResult<()> {
    if id.is_empty()
        || id.len() > 64
        || !id.as_bytes()[0].is_ascii_lowercase() && !id.as_bytes()[0].is_ascii_digit()
        || !id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        || reserved(id)
    {
        return Err(CoreError::invalid(format!("无效工具 ID：{id}")));
    }
    Ok(())
}
pub fn validate_entry(entry: &str) -> CoreResult<()> {
    if entry.is_empty()
        || entry.contains('\\')
        || entry.split('/').any(|p| {
            p.is_empty()
                || p == "."
                || p == ".."
                || p.ends_with([' ', '.'])
                || reserved(p)
                || p.chars().any(|c| c < ' ' || "<>:\"|?*".contains(c))
        })
    {
        return Err(CoreError::invalid(format!("不安全的包路径：{entry}")));
    }
    Ok(())
}
pub(crate) fn reject_link(path: &Path) -> CoreResult<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            #[cfg(windows)]
            let reparse = {
                use std::os::windows::fs::MetadataExt;
                meta.file_attributes() & 0x400 != 0
            };
            #[cfg(not(windows))]
            let reparse = false;
            if meta.file_type().is_symlink() || reparse {
                return Err(CoreError::invalid(format!(
                    "受管路径包含链接：{}",
                    path.display()
                )));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(())
}
pub(crate) fn managed(root: &Path, relative: &str) -> CoreResult<PathBuf> {
    validate_entry(relative)?;
    for parent in root.ancestors() {
        reject_link(parent)?;
    }
    let mut target = root.to_path_buf();
    for component in relative.split('/') {
        target.push(component);
        reject_link(&target)?;
    }
    Ok(target)
}
pub(crate) fn plain_tree(root: &Path) -> CoreResult<()> {
    reject_link(root)?;
    if root.is_dir() {
        for entry in fs::read_dir(root)? {
            plain_tree(&entry?.path())?;
        }
    }
    Ok(())
}
pub(crate) fn atomic_json(path: &Path, value: &impl serde::Serialize) -> CoreResult<()> {
    atomic_write(path, &serde_json::to_vec_pretty(value)?)
}
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> CoreResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| CoreError::invalid("文件缺少父目录"))?;
    for ancestor in parent.ancestors() {
        reject_link(ancestor)?;
    }
    fs::create_dir_all(parent)?;
    reject_link(path)?;
    let temp = parent.join(format!(".write-{}.tmp", uuid::Uuid::new_v4().simple()));
    let result = (|| -> CoreResult<()> {
        let mut writer = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        writer.write_all(bytes)?;
        writer.sync_all()?;
        replace_file(&temp, path)?;
        Ok(())
    })();
    if temp.exists() {
        let _ = fs::remove_file(temp);
    }
    result
}
pub(crate) fn replace_file(source: &Path, target: &Path) -> CoreResult<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        };
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe {
            MoveFileExW(
                source.as_ptr(),
                target.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        fs::rename(source, target)?;
        Ok(())
    }
}
pub(crate) fn publish_new_file(source: &Path, target: &Path) -> CoreResult<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        // Creating the second name is atomic and refuses an existing destination.
        fs::hard_link(source, target)?;
        fs::remove_file(source)?;
        Ok(())
    }
}
pub(crate) fn read_json(path: &Path) -> CoreResult<serde_json::Value> {
    reject_link(path)?;
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(
        bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes),
    )?)
}
pub(crate) fn hash_file(path: &Path) -> CoreResult<String> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let mut input = fs::File::open(path)?;
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex::encode(hasher.finalize()))
}
pub(crate) fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit())
}
pub(crate) fn relative(path: &Path, root: &Path) -> CoreResult<String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| CoreError::invalid("路径越出受管目录"))?;
    let mut parts = vec![];
    for c in relative.components() {
        if let Component::Normal(part) = c {
            parts.push(part.to_string_lossy().into_owned());
        } else {
            return Err(CoreError::invalid("非相对受管路径"));
        }
    }
    let result = parts.join("/");
    validate_entry(&result)?;
    Ok(result)
}
pub(crate) fn remove_path(path: &Path) -> CoreResult<()> {
    plain_tree(path)?;
    if path.is_dir() {
        fs::remove_dir_all(path)?;
    } else if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}
