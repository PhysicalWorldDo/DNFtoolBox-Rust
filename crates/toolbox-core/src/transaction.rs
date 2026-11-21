use crate::{
    catalog::load_installed,
    model::{Installed, Manifest, ToolVersion},
    network::{check_cancel, ensure_space},
    package::PackageManifest,
    paths::{
        atomic_json, hash_file, managed, plain_tree, publish_new_file, read_json, relative,
        remove_path, replace_file, validate_id,
    },
    CoreError, CoreResult,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Mutation {
    relative: String,
    action: String,
    old_hash: Option<String>,
    new_hash: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Journal {
    schema_version: u32,
    operation_id: String,
    tool_id: String,
    kind: String,
    phase: String,
    had_target: bool,
    old_installed: Option<Installed>,
    target_installed: Option<Installed>,
    mutations: Vec<Mutation>,
}
fn journal_path(root: &Path) -> PathBuf {
    root.join("transaction.json")
}
fn save_journal(root: &Path, journal: &Journal) -> CoreResult<()> {
    atomic_json(&journal_path(root), journal)
}
fn save_record(workspace: &Path, id: &str, record: Option<&Installed>) -> CoreResult<()> {
    let (_, mut raw) = load_installed(workspace)?;
    if let Some(record) = record {
        raw["tools"][id] = serde_json::to_value(record)?;
    } else {
        raw["tools"].as_object_mut().unwrap().remove(id);
    }
    atomic_json(&managed(workspace, "config/installed.json")?, &raw)
}
fn inventory(root: &Path) -> CoreResult<(BTreeMap<String, String>, BTreeSet<String>)> {
    plain_tree(root)?;
    let mut files = BTreeMap::new();
    let mut directories = BTreeSet::new();
    fn visit(
        root: &Path,
        at: &Path,
        files: &mut BTreeMap<String, String>,
        dirs: &mut BTreeSet<String>,
    ) -> CoreResult<()> {
        if !at.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(at)? {
            let path = entry?.path();
            let name = relative(&path, root)?;
            if path.is_dir() {
                dirs.insert(name);
                visit(root, &path, files, dirs)?;
            } else if path.is_file() {
                files.insert(name, hash_file(&path)?);
            } else {
                return Err(CoreError::invalid("工具目录包含非普通文件"));
            }
        }
        Ok(())
    }
    if root.is_dir() {
        visit(root, root, &mut files, &mut directories)?;
    } else if root.exists() {
        return Err(CoreError::invalid("工具目标必须为目录"));
    }
    Ok((files, directories))
}
fn copy_tree(
    source: &Path,
    destination: &Path,
    preserve: bool,
    cancel: &AtomicBool,
) -> CoreResult<()> {
    check_cancel(cancel)?;
    if source.is_dir() {
        if preserve && destination.exists() && !destination.is_dir() {
            return Ok(());
        }
        fs::create_dir_all(destination)?;
        for child in fs::read_dir(source)? {
            let child = child?;
            let target = case_child(destination, &child.file_name().to_string_lossy())?;
            copy_tree(&child.path(), &target, preserve, cancel)?;
        }
    } else if !preserve || !destination.exists() {
        fs::create_dir_all(destination.parent().unwrap())?;
        fs::copy(source, destination)?;
    }
    Ok(())
}
fn case_child(parent: &Path, name: &str) -> CoreResult<PathBuf> {
    let matches: Vec<_> = fs::read_dir(parent)?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().to_lowercase() == name.to_lowercase())
        .collect();
    if matches.len() > 1 {
        return Err(CoreError::invalid("工具目录含大小写歧义文件"));
    }
    Ok(matches
        .first()
        .map(|e| e.path())
        .unwrap_or_else(|| parent.join(name)))
}
pub(crate) fn prepare(
    workspace: &Path,
    operation_id: &str,
    manifest: &Manifest,
    version: &ToolVersion,
    archive_path: &Path,
    cancel: &AtomicBool,
) -> CoreResult<PathBuf> {
    let root = managed(workspace, &format!("cache/operations/{operation_id}"))?;
    let staging = managed(&root, "extracted")?;
    let candidate = managed(&root, "candidate")?;
    fs::create_dir_all(&staging)?;
    let mut zip = zip::ZipArchive::new(fs::File::open(archive_path)?)?;
    let mut names = BTreeSet::new();
    let mut paths = Vec::with_capacity(zip.len());
    let mut size = 0_u64;
    for index in 0..zip.len() {
        let member = zip.by_index(index)?;
        // Some Windows ZIP producers store backslashes in member names.
        let name = member
            .name()
            .trim_end_matches(['/', '\\'])
            .replace('\\', "/");
        crate::validate_entry(&name)?;
        if !names.insert(name.to_lowercase()) {
            return Err(CoreError::invalid("ZIP 含重复或大小写冲突路径"));
        }
        if member.unix_mode().is_some_and(|mode| {
            matches!(
                mode & 0o170000,
                0o120000 | 0o060000 | 0o020000 | 0o010000 | 0o140000
            )
        }) {
            return Err(CoreError::invalid("ZIP 不允许链接或设备条目"));
        }
        size = size
            .checked_add(member.size())
            .ok_or_else(|| CoreError::invalid("ZIP 大小溢出"))?;
        paths.push(name);
    }
    let target = managed(workspace, &format!("tools/{}", manifest.id))?;
    let old_size = tree_size(&target)?;
    ensure_space(
        workspace,
        size.saturating_mul(3)
            .saturating_add(old_size)
            .saturating_add(16 * 1024 * 1024),
    )?;
    for (index, name) in paths.iter().enumerate() {
        check_cancel(cancel)?;
        let mut member = zip.by_index(index)?;
        let path = managed(&staging, name)?;
        if member.is_dir() {
            fs::create_dir_all(path)?;
            continue;
        }
        fs::create_dir_all(path.parent().unwrap())?;
        let mut output = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)?;
        let mut buffer = vec![0; 256 * 1024];
        let mut written = 0_u64;
        loop {
            check_cancel(cancel)?;
            let count = member.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            written += count as u64;
            if written > member.size() {
                return Err(CoreError::invalid("ZIP 条目解压大小不符"));
            }
            output.write_all(&buffer[..count])?;
        }
        if written != member.size() {
            return Err(CoreError::invalid("ZIP 条目被截断"));
        }
        output.sync_all()?;
    }
    validate_package(&staging, manifest, version)?;
    plain_tree(&target)?;
    if target.exists() {
        copy_tree(&target, &candidate, false, cancel)?;
    } else {
        fs::create_dir_all(&candidate)?;
    }
    for entry in fs::read_dir(&staging)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let destination = case_child(&candidate, &name)?;
        if ["config", "data"].contains(&name.to_lowercase().as_str()) {
            copy_tree(&entry.path(), &destination, true, cancel)?;
        } else {
            remove_path(&destination)?;
            copy_tree(&entry.path(), &destination, false, cancel)?;
        }
    }
    validate_package(&candidate, manifest, version)?;
    Ok(candidate)
}
fn validate_package(root: &Path, manifest: &Manifest, version: &ToolVersion) -> CoreResult<()> {
    (|| {
        let package = PackageManifest::from_value(read_json(&managed(root, "tool.json")?)?)?;
        package.validate_request(manifest, version)?;
        if !managed(root, &manifest.entry)?.is_file() {
            return Err(CoreError::new(
                "MISSING_ENTRY",
                format!("安装包缺少登记入口 {:?}", manifest.entry),
            ));
        }
        Ok(())
    })()
    .map_err(|mut error: CoreError| {
        error.message = format!(
            "工具 {:?}（请求 version={:?}, channel={:?}）：{}",
            manifest.id, version.version, version.channel, error.message
        );
        error
    })
}
fn tree_size(root: &Path) -> CoreResult<u64> {
    plain_tree(root)?;
    if root.is_file() {
        return Ok(fs::metadata(root)?.len());
    }
    let mut sum = 0_u64;
    if root.is_dir() {
        for child in fs::read_dir(root)? {
            sum = sum.saturating_add(tree_size(&child?.path())?);
        }
    }
    Ok(sum)
}
pub(crate) fn install(
    workspace: &Path,
    operation_id: &str,
    candidate: &Path,
    record: Installed,
) -> CoreResult<()> {
    commit_install(workspace, operation_id, candidate, record, || Ok(()))
}
fn commit_install(
    workspace: &Path,
    operation_id: &str,
    candidate: &Path,
    record: Installed,
    before_state: impl FnOnce() -> CoreResult<()>,
) -> CoreResult<()> {
    commit_install_with_copy(
        workspace,
        operation_id,
        candidate,
        record,
        before_state,
        |source, destination| {
            fs::copy(source, destination)?;
            Ok(())
        },
    )
}
fn commit_install_with_copy(
    workspace: &Path,
    operation_id: &str,
    candidate: &Path,
    record: Installed,
    before_state: impl FnOnce() -> CoreResult<()>,
    mut copy_file: impl FnMut(&Path, &Path) -> CoreResult<()>,
) -> CoreResult<()> {
    let target = managed(workspace, &format!("tools/{}", record.id))?;
    let root = managed(workspace, &format!("cache/operations/{operation_id}"))?;
    let backup = managed(&root, "backup")?;
    let prepared = managed(&root, "prepared")?;
    let old_installed = load_installed(workspace)?.0.remove(&record.id);
    let (old_files, old_dirs) = inventory(&target)?;
    let (new_files, new_dirs) = inventory(candidate)?;
    let mut journal = Journal {
        schema_version: 1,
        operation_id: operation_id.into(),
        tool_id: record.id.clone(),
        kind: "install".into(),
        phase: "preparing".into(),
        had_target: target.exists(),
        old_installed,
        target_installed: Some(record.clone()),
        mutations: vec![],
    };
    fs::create_dir_all(&backup)?;
    save_journal(&root, &journal)?;
    let result = (|| -> CoreResult<()> {
        // Finish every potentially partial copy in the private transaction directory.
        // Until all prepared files are synced and verified, the installed tree is untouched.
        for (name, hash) in &new_files {
            if old_files.get(name) == Some(hash) {
                continue;
            }
            let staged = managed(&prepared, name)?;
            fs::create_dir_all(staged.parent().unwrap())?;
            copy_file(&managed(candidate, name)?, &staged)?;
            fs::OpenOptions::new()
                .write(true)
                .open(&staged)?
                .sync_all()?;
            if hash_file(&staged)? != *hash {
                return Err(CoreError::new(
                    "HASH_MISMATCH",
                    "准备安装文件的内容不完整，旧工具未改动",
                ));
            }
        }
        journal.phase = "committing".into();
        save_journal(&root, &journal)?;
        fs::create_dir_all(&target)?;
        for (name, hash) in &old_files {
            if new_files.get(name) == Some(hash) {
                continue;
            }
            let source = managed(&target, name)?;
            if !source.is_file() || hash_file(&source)? != *hash {
                return Err(CoreError::new(
                    "FILE_IN_USE",
                    "工具文件在准备期间被修改，请关闭工具后重试",
                ));
            }
            journal.mutations.push(Mutation {
                relative: name.clone(),
                action: "file".into(),
                old_hash: Some(hash.clone()),
                new_hash: new_files.get(name).cloned(),
            });
            save_journal(&root, &journal)?;
            let saved = managed(&backup, name)?;
            fs::create_dir_all(saved.parent().unwrap())?;
            replace_file(&source, &saved)?;
        }
        // Preserve stable installed directory handles. Only remove directories for a type conflict.
        let mut removed: Vec<_> = old_dirs
            .iter()
            .filter(|directory| {
                new_files.keys().any(|file| {
                    directory.as_str() == file || directory.starts_with(&format!("{file}/"))
                })
            })
            .cloned()
            .collect();
        removed.sort_by_key(|p| std::cmp::Reverse(p.matches('/').count()));
        for name in removed {
            journal.mutations.push(Mutation {
                relative: name.clone(),
                action: "removedDirectory".into(),
                old_hash: None,
                new_hash: None,
            });
            save_journal(&root, &journal)?;
            fs::remove_dir(managed(&target, &name)?)?;
        }
        let mut dirs: Vec<_> = new_dirs.iter().collect();
        dirs.sort_by_key(|p| p.matches('/').count());
        for name in dirs {
            let destination = managed(&target, name)?;
            if !destination.exists() {
                journal.mutations.push(Mutation {
                    relative: name.clone(),
                    action: "createdDirectory".into(),
                    old_hash: None,
                    new_hash: None,
                });
                save_journal(&root, &journal)?;
                fs::create_dir(&destination)?;
            }
        }
        for (name, hash) in &new_files {
            if old_files.get(name) == Some(hash) {
                continue;
            }
            if !old_files.contains_key(name) {
                journal.mutations.push(Mutation {
                    relative: name.clone(),
                    action: "file".into(),
                    old_hash: None,
                    new_hash: Some(hash.clone()),
                });
                save_journal(&root, &journal)?;
            }
            let destination = managed(&target, name)?;
            fs::create_dir_all(destination.parent().unwrap())?;
            publish_new_file(&managed(&prepared, name)?, &destination)?;
        }
        before_state()?;
        save_record(workspace, &record.id, Some(&record))?;
        journal.phase = "committed".into();
        save_journal(&root, &journal)?;
        Ok(())
    })();
    if let Err(error) = result {
        if let Err(rollback) = rollback(workspace, &root, &journal) {
            return Err(CoreError::new(
                "RECOVERY_REQUIRED",
                format!(
                    "{}；回滚未完成：{}。备份位于 {}",
                    error.message,
                    rollback.message,
                    backup.display()
                ),
            ));
        }
        return Err(error);
    }
    cleanup(&root);
    let mut obsolete: Vec<_> = old_dirs.difference(&new_dirs).collect();
    obsolete.sort_by_key(|p| std::cmp::Reverse(p.matches('/').count()));
    for name in obsolete {
        let _ = fs::remove_dir(target.join(name));
    }
    Ok(())
}
pub(crate) fn uninstall(workspace: &Path, operation_id: &str, id: &str) -> CoreResult<()> {
    let target = managed(workspace, &format!("tools/{id}"))?;
    plain_tree(&target)?;
    if !target.is_dir() {
        return Err(CoreError::new(
            "RECOVERY_REQUIRED",
            "工具目录与安装记录不一致，请打开目录检查",
        ));
    }
    let root = managed(workspace, &format!("cache/operations/{operation_id}"))?;
    let trash = managed(&root, "removed")?;
    let old_installed = load_installed(workspace)?.0.remove(id);
    let mut journal = Journal {
        schema_version: 1,
        operation_id: operation_id.into(),
        tool_id: id.into(),
        kind: "uninstall".into(),
        phase: "committing".into(),
        had_target: true,
        old_installed,
        target_installed: None,
        mutations: vec![],
    };
    save_journal(&root, &journal)?;
    let result = (|| -> CoreResult<()> {
        fs::rename(&target, &trash)?;
        save_record(workspace, id, None)?;
        journal.phase = "committed".into();
        save_journal(&root, &journal)?;
        Ok(())
    })();
    if let Err(error) = result {
        if let Err(rollback) = rollback(workspace, &root, &journal) {
            return Err(CoreError::new(
                "RECOVERY_REQUIRED",
                format!("{}；卸载回滚失败：{}", error.message, rollback.message),
            ));
        }
        return Err(error);
    }
    cleanup(&root);
    Ok(())
}
fn rollback(workspace: &Path, root: &Path, journal: &Journal) -> CoreResult<()> {
    let target = managed(workspace, &format!("tools/{}", journal.tool_id))?;
    if journal.kind == "uninstall" {
        let trash = managed(root, "removed")?;
        if trash.exists() {
            if target.exists() {
                return Err(CoreError::new(
                    "RECOVERY_REQUIRED",
                    "卸载恢复目标已存在，保留回收目录",
                ));
            }
            fs::rename(trash, &target)?;
        } else if !target.exists() {
            return Err(CoreError::new(
                "RECOVERY_REQUIRED",
                "卸载恢复的工具目录和备份均不存在",
            ));
        }
    } else {
        let backup = managed(root, "backup")?;
        for mutation in journal.mutations.iter().rev() {
            let destination = managed(&target, &mutation.relative)?;
            match mutation.action.as_str() {
                "file" => {
                    let saved = managed(&backup, &mutation.relative)?;
                    if let Some(old_hash) = &mutation.old_hash {
                        if saved.exists() {
                            if hash_file(&saved)? != *old_hash {
                                return Err(CoreError::new(
                                    "RECOVERY_REQUIRED",
                                    "备份文件身份不一致",
                                ));
                            }
                            if destination.exists() {
                                if !destination.is_file()
                                    || mutation.new_hash.as_ref().is_none_or(|new| {
                                        hash_file(&destination).ok().as_ref() != Some(new)
                                    })
                                {
                                    return Err(CoreError::new(
                                        "RECOVERY_REQUIRED",
                                        "待恢复文件被外部修改，保留备份",
                                    ));
                                }
                                fs::remove_file(&destination)?;
                            }
                            fs::create_dir_all(destination.parent().unwrap())?;
                            replace_file(&saved, &destination)?;
                        } else if !destination.is_file() || hash_file(&destination)? != *old_hash {
                            return Err(CoreError::new("RECOVERY_REQUIRED", "恢复所需旧文件缺失"));
                        }
                    } else if destination.exists() {
                        if !destination.is_file()
                            || mutation.new_hash.as_ref().is_none_or(|new| {
                                hash_file(&destination).ok().as_ref() != Some(new)
                            })
                        {
                            return Err(CoreError::new(
                                "RECOVERY_REQUIRED",
                                "新文件被外部修改，无法自动删除",
                            ));
                        }
                        fs::remove_file(&destination)?;
                    }
                }
                "createdDirectory" => {
                    if destination.is_dir() {
                        fs::remove_dir(&destination)?;
                    }
                }
                "removedDirectory" => {
                    fs::create_dir_all(destination)?;
                }
                _ => return Err(CoreError::new("RECOVERY_REQUIRED", "未知事务动作")),
            }
        }
        if !journal.had_target && target.exists() {
            fs::remove_dir(&target)?;
        }
    }
    save_record(workspace, &journal.tool_id, journal.old_installed.as_ref())?;
    let mut rolled_back = journal.clone();
    rolled_back.phase = "rolledBack".into();
    save_journal(root, &rolled_back)?;
    cleanup(root);
    Ok(())
}
fn cleanup(root: &Path) {
    for name in ["backup", "candidate", "extracted", "removed", "prepared"] {
        if let Ok(path) = managed(root, name) {
            let _ = remove_path(&path);
        }
    }
}
pub(crate) fn recover(workspace: &Path) -> BTreeMap<String, String> {
    let mut errors = BTreeMap::new();
    let Ok(operations) = managed(workspace, "cache/operations") else {
        return errors;
    };
    let Ok(entries) = fs::read_dir(&operations) else {
        return errors;
    };
    let lock_result = (|| -> CoreResult<fs::File> {
        let path = managed(&operations, "writer.lock")?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .map_err(|_| CoreError::new("BUSY", "此工作区正由其他进程执行文件事务；恢复已暂停"))?;
        Ok(lock)
    })();
    let _writer = match lock_result {
        Ok(lock) => lock,
        Err(error) => {
            errors.insert("*".into(), error.message);
            return errors;
        }
    };
    for entry in entries.flatten() {
        let root = entry.path();
        let path = journal_path(&root);
        if !path.exists() {
            continue;
        }
        let result = (|| -> CoreResult<()> {
            plain_tree(&root)?;
            let journal: Journal = serde_json::from_value(read_json(&path)?)?;
            validate_id(&journal.tool_id)?;
            validate_id(&journal.operation_id)?;
            if journal.schema_version != 1
                || root.file_name().unwrap_or_default()
                    != std::ffi::OsStr::new(&journal.operation_id)
            {
                return Err(CoreError::new("RECOVERY_REQUIRED", "事务身份或版本不一致"));
            }
            if ["committed", "rolledBack"].contains(&journal.phase.as_str()) {
                cleanup(&root);
                Ok(())
            } else {
                rollback(workspace, &root, &journal)
            }
        })();
        if let Err(error) = result {
            let id = read_json(&path)
                .ok()
                .and_then(|v| v["toolId"].as_str().map(str::to_string))
                .unwrap_or_else(|| "unknown".into());
            errors.insert(id, error.message);
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn registry_manifest() -> Manifest {
        serde_json::from_value(json!({
            "schemaVersion": 1,
            "id": "dnf_avatar_studio",
            "name": "薛定谔的化裁间",
            "entry": "bin/run.cmd",
            "latest": {"stable": "2.2.0"},
            "versions": [{"version": "2.2.0", "channel": "stable"}]
        }))
        .unwrap()
    }

    fn short_package() -> Value {
        json!({
            "schemaVersion": 1,
            "id": "dnf_avatar_studio",
            "name": "薛定谔的化裁间",
            "category": "图像工具",
            "description": "时装预览、染色和换装",
            "version": "2.2.0",
            "entry": "bin/run.cmd",
            "needAdmin": false
        })
    }

    fn validate_package_json(raw: Value, channel: &str) -> CoreResult<()> {
        let test_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test-work");
        fs::create_dir_all(&test_root).unwrap();
        let dir = tempfile::tempdir_in(test_root).unwrap();
        fs::create_dir_all(dir.path().join("bin")).unwrap();
        fs::write(dir.path().join("bin/run.cmd"), "@exit /b 0").unwrap();
        atomic_json(&dir.path().join("tool.json"), &raw).unwrap();
        let manifest = registry_manifest();
        let mut version = manifest.versions[0].clone();
        version.channel = channel.into();
        validate_package(dir.path(), &manifest, &version)
    }

    #[test]
    fn short_package_descriptor_accepts_exact_version_and_defaults_to_stable_channel() {
        validate_package_json(short_package(), "stable").unwrap();
    }

    #[test]
    fn short_package_descriptor_accepts_its_explicit_channel() {
        let mut package = short_package();
        package["channel"] = json!("beta");
        validate_package_json(package, "beta").unwrap();
    }

    #[test]
    fn legacy_package_manifest_remains_accepted() {
        validate_package_json(serde_json::to_value(registry_manifest()).unwrap(), "stable")
            .unwrap();
    }

    #[test]
    fn versionless_package_metadata_can_be_read_but_cannot_be_installed() {
        let mut package = short_package();
        package.as_object_mut().unwrap().remove("version");
        let metadata = PackageManifest::from_value(package.clone()).unwrap();
        assert!(metadata.installed_version().is_none());
        let error = validate_package_json(package, "stable").unwrap_err();
        assert_eq!(error.code, "INVALID_ARGUMENT");
        assert!(
            error
                .message
                .contains("请求 version=\"2.2.0\", channel=\"stable\""),
            "{}",
            error.message
        );
        assert!(
            error
                .message
                .contains("缺少顶层 version 或非空 versions 声明"),
            "{}",
            error.message
        );
    }

    #[test]
    fn package_mismatch_errors_identify_the_tool_request_and_actual_field() {
        for (field, value, expected) in [
            (
                "id",
                "other_tool",
                "Registry.id=\"dnf_avatar_studio\"，tool.json.id=\"other_tool\"",
            ),
            (
                "entry",
                "bin/old.cmd",
                "Registry.entry=\"bin/run.cmd\"，tool.json.entry=\"bin/old.cmd\"",
            ),
            (
                "version",
                "2.0.4",
                "tool.json 声明=[(\"2.0.4\", \"stable\")]",
            ),
            ("channel", "beta", "tool.json 声明=[(\"2.2.0\", \"beta\")]"),
        ] {
            let mut package = short_package();
            package[field] = json!(value);
            let error = validate_package_json(package, "stable").unwrap_err();
            assert_eq!(error.code, "INVALID_ARGUMENT");
            assert!(
                error.message.starts_with(
                    "工具 \"dnf_avatar_studio\"（请求 version=\"2.2.0\", channel=\"stable\"）："
                ),
                "{}",
                error.message
            );
            assert!(error.message.contains(expected), "{}", error.message);
        }
    }

    #[test]
    fn legacy_package_version_mismatch_reports_every_declared_version_and_channel() {
        let mut package = serde_json::to_value(registry_manifest()).unwrap();
        package["latest"] = json!({"stable": "2.0.4", "beta": "2.2.0"});
        package["versions"] = json!([
            {"version": "2.0.4", "channel": "stable"},
            {"version": "2.2.0", "channel": "beta"}
        ]);
        let error = validate_package_json(package, "stable").unwrap_err();
        assert_eq!(error.code, "INVALID_ARGUMENT");
        assert!(
            error
                .message
                .contains("请求 version=\"2.2.0\", channel=\"stable\""),
            "{}",
            error.message
        );
        assert!(
            error
                .message
                .contains("tool.json 声明=[(\"2.0.4\", \"stable\"), (\"2.2.0\", \"beta\")]"),
            "{}",
            error.message
        );
    }

    #[test]
    fn package_rejects_contradictory_top_level_and_legacy_versions() {
        for (version, versions) in [
            ("2.0.4", json!([{"version": "2.2.0", "channel": "stable"}])),
            ("2.2.0", json!([{"version": "2.0.4", "channel": "stable"}])),
            ("2.2.0", json!([{"version": "2.2.0", "channel": "beta"}])),
        ] {
            let mut package = short_package();
            package["version"] = json!(version);
            package["versions"] = versions;
            let error = validate_package_json(package, "stable").unwrap_err();
            assert!(error.message.contains("顶层 version="), "{}", error.message);
            assert!(error.message.contains("versions"), "{}", error.message);
        }

        let mut package = short_package();
        package["versions"] = json!([
            {"version": "2.0.4", "channel": "stable"},
            {"version": "2.2.0", "channel": "stable"}
        ]);
        package["latest"] = json!({"stable": "2.0.4"});
        let error = validate_package_json(package, "stable").unwrap_err();
        assert!(
            error.message.contains("latest.stable=\"2.0.4\""),
            "{}",
            error.message
        );
    }

    #[test]
    fn failed_state_commit_restores_files_and_recovery_is_repeatable() {
        let test_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test-work");
        fs::create_dir_all(&test_root).unwrap();
        let dir = tempfile::tempdir_in(test_root).unwrap();
        let workspace = dir.path();
        fs::create_dir_all(workspace.join("tools/test/bin")).unwrap();
        fs::write(workspace.join("tools/test/bin/run.cmd"), "old").unwrap();
        fs::create_dir_all(workspace.join("cache/operations/op-test/candidate/bin")).unwrap();
        fs::write(
            workspace.join("cache/operations/op-test/candidate/bin/run.cmd"),
            "new",
        )
        .unwrap();
        let record = Installed {
            id: "test".into(),
            name: "测试".into(),
            version: "1.0.0".into(),
            channel: "stable".into(),
            entry: "bin/run.cmd".into(),
            installed_at: String::new(),
            updated_at: String::new(),
            extra: BTreeMap::new(),
        };
        let candidate = workspace.join("cache/operations/op-test/candidate");
        let result = commit_install(workspace, "op-test", &candidate, record, || {
            Err(CoreError::new("IO", "injected state write failure"))
        });
        assert!(result.is_err());
        assert_eq!(
            fs::read_to_string(workspace.join("tools/test/bin/run.cmd")).unwrap(),
            "old"
        );
        assert!(recover(workspace).is_empty());
        assert!(recover(workspace).is_empty());
        assert!(load_installed(workspace).unwrap().0.is_empty());
    }

    fn interrupted_workspace(phase: &str) -> tempfile::TempDir {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test-work");
        fs::create_dir_all(&root).unwrap();
        let dir = tempfile::tempdir_in(root).unwrap();
        let workspace = dir.path();
        let old = Installed {
            id: "test".into(),
            name: "测试".into(),
            version: "1.0.0".into(),
            channel: "stable".into(),
            entry: "bin/run.cmd".into(),
            installed_at: "original".into(),
            updated_at: "old".into(),
            extra: BTreeMap::new(),
        };
        let mut new = old.clone();
        new.version = "1.1.0".into();
        let operation = workspace.join("cache/operations/op-test");
        fs::create_dir_all(operation.join("backup/bin")).unwrap();
        fs::write(operation.join("backup/bin/run.cmd"), "old").unwrap();
        fs::create_dir_all(workspace.join("tools/test/bin")).unwrap();
        fs::write(workspace.join("tools/test/bin/run.cmd"), "new").unwrap();
        save_record(workspace, "test", Some(&new)).unwrap();
        let journal = Journal {
            schema_version: 1,
            operation_id: "op-test".into(),
            tool_id: "test".into(),
            kind: "install".into(),
            phase: phase.into(),
            had_target: true,
            old_installed: Some(old),
            target_installed: Some(new),
            mutations: vec![Mutation {
                relative: "bin/run.cmd".into(),
                action: "file".into(),
                old_hash: Some(hash_file(&operation.join("backup/bin/run.cmd")).unwrap()),
                new_hash: Some(hash_file(&workspace.join("tools/test/bin/run.cmd")).unwrap()),
            }],
        };
        save_journal(&operation, &journal).unwrap();
        dir
    }
    #[test]
    fn recovery_respects_the_workspace_writer_lock_and_then_restores_interrupted_commit() {
        let dir = interrupted_workspace("committing");
        let workspace = dir.path();
        let path = workspace.join("cache/operations/writer.lock");
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        fs2::FileExt::try_lock_exclusive(&lock).unwrap();
        assert!(!recover(workspace).is_empty());
        assert_eq!(
            fs::read_to_string(workspace.join("tools/test/bin/run.cmd")).unwrap(),
            "new"
        );
        drop(lock);
        assert!(recover(workspace).is_empty());
        assert!(recover(workspace).is_empty());
        assert_eq!(
            fs::read_to_string(workspace.join("tools/test/bin/run.cmd")).unwrap(),
            "old"
        );
        assert_eq!(
            load_installed(workspace).unwrap().0["test"].version,
            "1.0.0"
        );
    }
    #[test]
    fn committed_transaction_is_reported_succeeded_after_reopening_service() {
        let dir = interrupted_workspace("committed");
        let operation = dir.path().join("cache/operations/op-test");
        atomic_json(&operation.join("operation.json"),&serde_json::json!({"id":"op-test","kind":"update","toolId":"test","version":"1.1.0","state":"running","stage":"commit","doneBytes":0,"totalBytes":null,"canCancel":false,"error":null,"children":[],"result":null,"createdAt":"2026-11-21T00:00:00Z","updatedAt":"2026-11-21T00:00:00Z","request":{"kind":"update","toolId":"test"}})).unwrap();
        let service = crate::Service::open(dir.path(), "0.4.0").unwrap();
        assert_eq!(service.bootstrap()["operations"][0]["state"], "succeeded");
        assert_eq!(
            fs::read_to_string(dir.path().join("tools/test/bin/run.cmd")).unwrap(),
            "new"
        );
        assert!(!operation.join("backup").exists());
    }

    fn update_workspace() -> (tempfile::TempDir, Installed, PathBuf) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test-work");
        fs::create_dir_all(&root).unwrap();
        let dir = tempfile::tempdir_in(root).unwrap();
        let old = Installed {
            id: "test".into(),
            name: "测试".into(),
            version: "1.0.0".into(),
            channel: "stable".into(),
            entry: "bin/run.cmd".into(),
            installed_at: "original".into(),
            updated_at: "old".into(),
            extra: BTreeMap::new(),
        };
        fs::create_dir_all(dir.path().join("tools/test/bin")).unwrap();
        fs::write(dir.path().join("tools/test/bin/run.cmd"), "old").unwrap();
        save_record(dir.path(), "test", Some(&old)).unwrap();
        let candidate = dir.path().join("cache/operations/op-test/candidate");
        fs::create_dir_all(candidate.join("bin")).unwrap();
        fs::write(candidate.join("bin/run.cmd"), "complete new contents").unwrap();
        let mut new = old;
        new.version = "1.1.0".into();
        (dir, new, candidate)
    }
    #[test]
    fn partial_preparation_failure_keeps_the_old_tool_and_recovers_twice() {
        let (dir, record, candidate) = update_workspace();
        let workspace = dir.path();
        let result = commit_install_with_copy(
            workspace,
            "op-test",
            &candidate,
            record,
            || Ok(()),
            |_, destination| {
                fs::write(destination, "part")?;
                Err(CoreError::new(
                    "IO",
                    "injected disk full during preparation",
                ))
            },
        );
        assert_eq!(result.unwrap_err().code, "IO");
        assert_eq!(
            fs::read_to_string(workspace.join("tools/test/bin/run.cmd")).unwrap(),
            "old"
        );
        assert!(recover(workspace).is_empty());
        assert!(recover(workspace).is_empty());
        assert_eq!(
            load_installed(workspace).unwrap().0["test"].version,
            "1.0.0"
        );
    }
    #[test]
    fn interrupted_partial_preparation_never_exposes_partial_target_and_recovers_twice() {
        let (dir, record, candidate) = update_workspace();
        let workspace = dir.path();
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            commit_install_with_copy(
                workspace,
                "op-test",
                &candidate,
                record,
                || Ok(()),
                |_, destination| {
                    fs::write(destination, "part").unwrap();
                    panic!("injected process interruption during preparation");
                },
            )
        }));
        assert!(interrupted.is_err());
        assert_eq!(
            fs::read_to_string(workspace.join("tools/test/bin/run.cmd")).unwrap(),
            "old"
        );
        assert!(recover(workspace).is_empty());
        assert!(recover(workspace).is_empty());
        assert_eq!(
            load_installed(workspace).unwrap().0["test"].version,
            "1.0.0"
        );
        assert!(!workspace.join("cache/operations/op-test/prepared").exists());
    }
    #[test]
    fn external_target_edit_during_preparation_is_preserved() {
        let (dir, record, candidate) = update_workspace();
        let workspace = dir.path();
        let result = commit_install_with_copy(
            workspace,
            "op-test",
            &candidate,
            record,
            || Ok(()),
            |source, destination| {
                fs::copy(source, destination)?;
                fs::write(workspace.join("tools/test/bin/run.cmd"), "external edit")?;
                Ok(())
            },
        );
        assert_eq!(result.unwrap_err().code, "FILE_IN_USE");
        assert_eq!(
            fs::read_to_string(workspace.join("tools/test/bin/run.cmd")).unwrap(),
            "external edit"
        );
        assert!(recover(workspace).is_empty());
        assert!(recover(workspace).is_empty());
        assert_eq!(
            load_installed(workspace).unwrap().0["test"].version,
            "1.0.0"
        );
    }
    #[test]
    fn recovery_retains_backup_when_target_was_really_modified_externally() {
        let dir = interrupted_workspace("committing");
        let target = dir.path().join("tools/test/bin/run.cmd");
        fs::write(&target, "external edit").unwrap();
        assert!(!recover(dir.path()).is_empty());
        assert!(!recover(dir.path()).is_empty());
        assert_eq!(fs::read_to_string(target).unwrap(), "external edit");
        assert_eq!(
            fs::read_to_string(
                dir.path()
                    .join("cache/operations/op-test/backup/bin/run.cmd")
            )
            .unwrap(),
            "old"
        );
    }
}
