use base64::Engine;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Mutex,
};

pub struct IconService {
    workspace: PathBuf,
    cache: Mutex<BTreeMap<String, Option<String>>>,
    attempted: Mutex<BTreeSet<String>>,
}
#[derive(Clone)]
pub struct RemoteIcon {
    pub id: String,
    pub url: String,
    pub key: String,
}
impl IconService {
    pub fn new(workspace: &Path) -> Self {
        Self {
            workspace: workspace.to_path_buf(),
            cache: Mutex::new(BTreeMap::new()),
            attempted: Mutex::new(BTreeSet::new()),
        }
    }
    pub fn enrich_tool(&self, tool: &mut Value, revision: u64) {
        let id = tool["id"].as_str().unwrap_or_default();
        if crate::cli::update_id(id.to_owned()).is_err() {
            return;
        }
        let key = format!(
            "{id}|{revision}|{}|{}|{}",
            tool["installedVersion"], tool["icon"], tool["iconSource"]
        );
        if let Some(value) = self.cache.lock().unwrap().get(&key).cloned() {
            tool["iconUrl"] = serde_json::json!(value);
            return;
        }
        let root = self.workspace.join("tools").join(id);
        if crate::self_update::ensure_managed(&self.workspace, &root).is_err() {
            tool["iconUrl"] = Value::Null;
            return;
        }
        let mut sources = vec![];
        let cache_root = self.workspace.join("cache/tool-icons").join(id);
        let remote_url = tool["icon"].as_str().filter(|url| {
            reqwest::Url::parse(url).is_ok_and(|url| ["http", "https"].contains(&url.scheme()))
        });
        if let Some(url) = remote_url {
            let key = hex::encode(Sha256::digest(url.as_bytes()));
            sources.push(cache_root.join(format!("remote-{}.png", &key[..24])));
            let legacy_stamp = format!(
                "{}\n{url}",
                tool["latestVersion"].as_str().unwrap_or_default()
            );
            sources.push(cache_root.join(format!(
                "{}.png",
                &hex::encode(Sha256::digest(legacy_stamp.as_bytes()))[..24]
            )));
        }
        for field in ["icon", "iconSource"] {
            if let Some(path) = tool[field]
                .as_str()
                .and_then(|value| relative(&root, value))
            {
                sources.push(path);
            }
        }
        if let Some(path) = relative(&root, "tool.json") {
            if let Ok(raw) = std::fs::read(path).and_then(|bytes| {
                serde_json::from_slice::<Value>(&bytes).map_err(std::io::Error::other)
            }) {
                for field in ["icon", "iconSource"] {
                    if let Some(path) = raw[field].as_str().and_then(|value| relative(&root, value))
                    {
                        sources.push(path);
                    }
                }
            }
        }
        for name in [
            "icon.png",
            "icon.ico",
            "bin/app/icon.ico",
            "bin/app/icon.png",
            "assets/icon.ico",
            "assets/icon.png",
        ] {
            if let Some(path) = relative(&root, name) {
                sources.push(path);
            }
        }
        for folder in ["bin/app", "assets", "bin/app/assets"] {
            let Some(directory) = relative(&root, folder) else {
                continue;
            };
            if let Some(path) = unique_extension(&directory, "ico") {
                sources.push(path);
            }
        }
        if let Some(path) = tool["entry"]
            .as_str()
            .and_then(|value| relative(&root, value))
        {
            sources.push(path);
        }
        if let Some(launcher) = relative(&root, "bin/run.cmd") {
            if let Ok(text) = std::fs::read_to_string(launcher) {
                for line in text.lines() {
                    let lower = line.to_lowercase();
                    if lower.trim_start().starts_with("start ") {
                        if let Some(start) = lower.find("%appdir%") {
                            let tail =
                                line[start + "%appdir%".len()..].trim_start_matches(['\\', '/']);
                            if let Some(end) = tail.find('"') {
                                if tail[..end].to_ascii_lowercase().ends_with(".exe") {
                                    if let Some(path) =
                                        relative(&root, &format!("bin/app/{}", &tail[..end]))
                                    {
                                        sources.push(path);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        for folder in ["", "bin/app"] {
            if let Some(path) = unique_extension(&root.join(folder), "exe") {
                sources.push(path);
            }
        }
        if remote_url.is_none() {
            if let Ok(entries) = std::fs::read_dir(&cache_root) {
                let mut cached: Vec<_> = entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| {
                        path.extension()
                            .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
                    })
                    .collect();
                cached.sort();
                sources.extend(cached);
            }
        }
        let result = sources.into_iter().find_map(|path| {
            if crate::self_update::ensure_managed(&self.workspace, &path).is_err()
                || !path.is_file()
            {
                return None;
            }
            let bytes = if path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
            {
                let metadata = std::fs::metadata(&path).ok()?;
                let stamp = format!(
                    "{}|{}|{:?}",
                    path.display(),
                    metadata.len(),
                    metadata.modified().ok()
                );
                let name = format!(
                    "local-{}.png",
                    &hex::encode(Sha256::digest(stamp.as_bytes()))[..24]
                );
                let cached = cache_root.join(name);
                if crate::self_update::ensure_managed(&self.workspace, &cached).is_err() {
                    return None;
                }
                if let Ok(bytes) = std::fs::read(&cached) {
                    bytes
                } else {
                    let bytes = exe_icon(&path)?;
                    if std::fs::create_dir_all(&cache_root).is_ok() {
                        let _ = std::fs::write(&cached, &bytes);
                    }
                    bytes
                }
            } else {
                read_image(&path)?
            };
            image_uri(&bytes)
        });
        self.cache.lock().unwrap().insert(key, result.clone());
        tool["iconUrl"] = serde_json::json!(result);
    }
    pub fn remote_jobs(&self, catalog: &Value, settings: &Value) -> Vec<RemoteIcon> {
        let mut jobs = vec![];
        let Some(tools) = catalog["tools"].as_array() else {
            return jobs;
        };
        for tool in tools {
            if tool["iconUrl"].is_string() {
                continue;
            }
            let Some(url) = tool["icon"].as_str() else {
                continue;
            };
            if !reqwest::Url::parse(url).is_ok_and(|url| {
                ["http", "https"].contains(&url.scheme()) && url.host_str().is_some()
            }) {
                continue;
            }
            let Some(id) = tool["id"].as_str() else {
                continue;
            };
            if crate::cli::update_id(id.to_owned()).is_err() {
                continue;
            }
            let stamp = format!(
                "{id}|{url}|{}|{}",
                catalog["revision"], settings["revision"]
            );
            if !self.attempted.lock().unwrap().insert(stamp) {
                continue;
            }
            jobs.push(RemoteIcon {
                id: id.into(),
                url: url.into(),
                key: hex::encode(Sha256::digest(url.as_bytes()))[..24].to_owned(),
            });
        }
        jobs
    }
    pub fn store_remote(&self, job: &RemoteIcon, bytes: &[u8]) -> Result<(), String> {
        if image_uri(bytes).is_none() {
            return Err("下载内容不是受支持的图像。".into());
        }
        let directory = self.workspace.join("cache/tool-icons").join(&job.id);
        crate::self_update::ensure_managed(&self.workspace, &directory)?;
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let target = directory.join(format!("remote-{}.png", job.key));
        let temporary = target.with_extension("part");
        std::fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(temporary, target).map_err(|e| e.to_string())?;
        self.cache
            .lock()
            .unwrap()
            .retain(|key, _| !key.starts_with(&format!("{}|", job.id)));
        Ok(())
    }
}

fn relative(root: &Path, value: &str) -> Option<PathBuf> {
    let path = Path::new(value);
    if value.is_empty()
        || value.chars().any(|c| ":<>\"|?*\0".contains(c))
        || !path
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
    {
        return None;
    }
    let path = root.join(path);
    crate::self_update::ensure_managed(root, &path).ok()?;
    Some(path)
}
fn unique_extension(directory: &Path, extension: &str) -> Option<PathBuf> {
    let mut entries = std::fs::read_dir(directory)
        .ok()?
        .flatten()
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|actual| actual.eq_ignore_ascii_case(extension))
        });
    let first = entries.next()?.path();
    if entries.next().is_some() {
        None
    } else {
        Some(first)
    }
}
fn read_image(path: &Path) -> Option<Vec<u8>> {
    if std::fs::metadata(path).ok()?.len() > 2 * 1024 * 1024 {
        return None;
    }
    std::fs::read(path).ok()
}
fn image_uri(bytes: &[u8]) -> Option<String> {
    if bytes.len() > 2 * 1024 * 1024 {
        return None;
    }
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if bytes.starts_with(&[0, 0, 1, 0]) {
        "x-icon"
    } else if bytes.starts_with(&[255, 216, 255]) {
        "jpeg"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "webp"
    } else {
        return None;
    };
    Some(format!(
        "data:image/{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

#[cfg(windows)]
fn exe_icon(path: &Path) -> Option<Vec<u8>> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Graphics::Gdi::{
            DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO,
            BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        },
        UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, PrivateExtractIconsW, ICONINFO},
    };
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let mut icon = std::ptr::null_mut();
        let count = PrivateExtractIconsW(
            wide.as_ptr(),
            0,
            256,
            256,
            &mut icon,
            std::ptr::null_mut(),
            1,
            0,
        );
        if count == 0 || count == u32::MAX || icon.is_null() {
            return None;
        }
        let mut info = ICONINFO::default();
        if GetIconInfo(icon, &mut info) == 0 {
            DestroyIcon(icon);
            return None;
        }
        let result = (|| {
            if info.hbmColor.is_null() {
                return None;
            }
            let mut bitmap = BITMAP::default();
            if GetObjectW(
                info.hbmColor,
                std::mem::size_of::<BITMAP>() as i32,
                (&mut bitmap as *mut BITMAP).cast(),
            ) == 0
            {
                return None;
            }
            let width = bitmap.bmWidth.unsigned_abs();
            let height = bitmap.bmHeight.unsigned_abs();
            if width == 0 || height == 0 || width > 1024 || height > 1024 {
                return None;
            }
            let dc = GetDC(std::ptr::null_mut());
            if dc.is_null() {
                return None;
            }
            let mut details = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width as i32,
                    biHeight: -(height as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut pixels = vec![0u8; (width * height * 4) as usize];
            let count = GetDIBits(
                dc,
                info.hbmColor,
                0,
                height,
                pixels.as_mut_ptr().cast(),
                &mut details,
                DIB_RGB_COLORS,
            );
            if count == 0 {
                ReleaseDC(std::ptr::null_mut(), dc);
                return None;
            }
            if pixels.chunks_exact(4).all(|pixel| pixel[3] == 0) {
                let mut mask = vec![0u8; pixels.len()];
                let mask_ok = !info.hbmMask.is_null()
                    && GetDIBits(
                        dc,
                        info.hbmMask,
                        0,
                        height,
                        mask.as_mut_ptr().cast(),
                        &mut details,
                        DIB_RGB_COLORS,
                    ) != 0;
                for (index, pixel) in pixels.chunks_exact_mut(4).enumerate() {
                    pixel[3] = if mask_ok { 255 - mask[index * 4] } else { 255 };
                }
            }
            ReleaseDC(std::ptr::null_mut(), dc);
            for pixel in pixels.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
            let mut output = vec![];
            {
                let mut encoder = png::Encoder::new(&mut output, width, height);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder
                    .write_header()
                    .ok()?
                    .write_image_data(&pixels)
                    .ok()?;
            }
            Some(output)
        })();
        if !info.hbmColor.is_null() {
            DeleteObject(info.hbmColor);
        }
        if !info.hbmMask.is_null() {
            DeleteObject(info.hbmMask);
        }
        DestroyIcon(icon);
        result
    }
}
#[cfg(not(windows))]
fn exe_icon(_path: &Path) -> Option<Vec<u8>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn area() -> PathBuf {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../.work/host-tests")
            .join(format!("icons-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(path.join("tools/tool-one/assets")).unwrap();
        path
    }
    #[test]
    fn returns_the_same_package_image_for_bootstrap_and_tool_details() {
        let root = area();
        std::fs::write(
            root.join("tools/tool-one/assets/icon.png"),
            b"\x89PNG\r\n\x1a\nfixture",
        )
        .unwrap();
        let icons = IconService::new(&root);
        let mut bootstrap =
            json!({"id":"tool-one","installedVersion":"1.0.0","iconSource":"assets/icon.png"});
        let mut detail = bootstrap.clone();
        icons.enrich_tool(&mut bootstrap, 1);
        icons.enrich_tool(&mut detail, 1);
        assert!(bootstrap["iconUrl"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
        assert_eq!(bootstrap["iconUrl"], detail["iconUrl"]);
    }
    #[test]
    fn never_reads_an_icon_outside_the_tool_directory() {
        let root = area();
        std::fs::write(root.join("outside.png"), b"\x89PNG\r\n\x1a\nsecret").unwrap();
        let icons = IconService::new(&root);
        let mut tool = json!({"id":"tool-one","iconSource":"../../outside.png"});
        icons.enrich_tool(&mut tool, 1);
        assert!(tool["iconUrl"].is_null());
    }
    #[cfg(windows)]
    #[test]
    fn extracts_a_png_from_windows_icon_resources_without_executing_the_file() {
        let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap();
        let bytes = exe_icon(&root.join("System32/shell32.dll")).expect("system icon resource");
        let reader = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        assert!(reader.info().width > 0);
        assert!(reader.info().height > 0);
    }
}
