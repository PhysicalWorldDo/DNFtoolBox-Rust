use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const EXE_NAME: &str = "PhysicalWorldToolbox.exe";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    Prepared,
    WaitingForExit,
    Installing,
    AwaitingReady,
    Committed,
    RolledBack,
    RecoveryRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transaction {
    pub id: String,
    pub version: String,
    pub nonce: String,
    pub old_sha256: String,
    pub new_sha256: String,
    pub stage: Stage,
    pub message: Option<String>,
    #[serde(skip)]
    pub workspace: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthAck {
    pub transaction_id: String,
    pub nonce: String,
    pub version: String,
    pub session_id: String,
    pub pid: u32,
    pub exe_sha256: String,
}

pub fn validate_health(transaction: &Transaction, ack: &HealthAck, child_pid: u32) -> bool {
    transaction.stage == Stage::AwaitingReady
        && ack.transaction_id == transaction.id
        && ack.nonce == transaction.nonce
        && ack.version.trim_start_matches('v') == transaction.version.trim_start_matches('v')
        && !ack.session_id.is_empty()
        && ack.pid == child_pid
        && ack.exe_sha256 == transaction.new_sha256
}

impl Transaction {
    pub fn prepare(
        workspace: &Path,
        id: &str,
        version: &str,
        archive: &Path,
        sha: &str,
    ) -> Result<Self, String> {
        crate::cli::update_id(id.to_string())?;
        let workspace = workspace.canonicalize().map_err(|e| e.to_string())?;
        ensure_managed(&workspace, &workspace.join("cache/self-update"))?;
        let directory = workspace.join("cache/self-update").join(id);
        if directory.exists() {
            return Err("更新事务已经存在，请重新检查任务状态。".into());
        }
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let candidate = extract_package(archive, &directory, sha)?;
        let transaction = Self {
            id: id.into(),
            version: version.into(),
            nonce: uuid::Uuid::new_v4().to_string(),
            old_sha256: file_hash(&workspace.join(EXE_NAME))?,
            new_sha256: file_hash(&candidate)?,
            stage: Stage::Prepared,
            message: None,
            workspace,
        };
        transaction.save()?;
        Ok(transaction)
    }
    pub fn directory(&self) -> PathBuf {
        self.workspace.join("cache/self-update").join(&self.id)
    }
    pub fn target(&self) -> PathBuf {
        self.workspace.join(EXE_NAME)
    }
    pub fn load(workspace: &Path, id: &str) -> Result<Self, String> {
        crate::cli::update_id(id.to_owned())?;
        let workspace = workspace.canonicalize().map_err(|e| e.to_string())?;
        let directory = workspace.join("cache/self-update").join(id);
        ensure_managed(&workspace, &directory)?;
        for file in [
            "receipt.json",
            "receipt.json.tmp",
            "candidate.exe",
            "backup.exe",
            "helper.exe",
            "ready.json",
            "helper.accepted.json",
            "restore.exe",
            "update.zip",
        ] {
            ensure_managed(&workspace, &directory.join(file))?;
        }
        let mut result: Self = serde_json::from_slice(
            &fs::read(directory.join("receipt.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if result.id != id {
            return Err("更新事务身份不匹配。".into());
        }
        result.workspace = workspace;
        Ok(result)
    }
    pub fn save(&self) -> Result<(), String> {
        atomic_json(&self.directory().join("receipt.json"), self)
    }
    pub fn install(&mut self) -> Result<(), String> {
        if file_hash(&self.target())? != self.old_sha256
            || file_hash(&self.directory().join("candidate.exe"))? != self.new_sha256
        {
            return Err("更新候选或旧主程序已发生变化，请重新下载。".into());
        }
        let backup = self.directory().join("backup.exe");
        fs::copy(self.target(), &backup).map_err(|e| e.to_string())?;
        fs::OpenOptions::new()
            .write(true)
            .open(&backup)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        if file_hash(&backup)? != self.old_sha256 {
            return Err("旧主程序备份校验失败。".into());
        }
        self.stage = Stage::Installing;
        self.save()?;
        replace_with_retry(
            &self.directory().join("candidate.exe"),
            &self.target(),
            &self.new_sha256,
        )?;
        self.stage = Stage::AwaitingReady;
        self.save()
    }
    pub fn rollback(&mut self, reason: &str) -> Result<(), String> {
        if self.stage == Stage::Committed {
            return Ok(());
        }
        let target_hash = file_hash(&self.target()).ok();
        if target_hash.as_deref() != Some(&self.old_sha256) {
            let backup = self.directory().join("backup.exe");
            if file_hash(&backup).ok().as_deref() != Some(&self.old_sha256)
                || target_hash.as_deref().is_some_and(|h| h != self.new_sha256)
            {
                self.stage = Stage::RecoveryRequired;
                self.message = Some(format!(
                    "无法自动恢复主程序，旧版备份保留在 {}。{reason}",
                    backup.display()
                ));
                self.save()?;
                return Err(self.message.clone().unwrap());
            }
            let restore = self.directory().join("restore.exe");
            fs::copy(&backup, &restore).map_err(|e| e.to_string())?;
            if self.target().exists() {
                if let Err(error) = replace_with_retry(&restore, &self.target(), &self.old_sha256) {
                    self.stage = Stage::RecoveryRequired;
                    self.message = Some(format!(
                        "恢复主程序时文件仍被占用；备份保留在 {}。{error}",
                        backup.display()
                    ));
                    self.save()?;
                    return Err(self.message.clone().unwrap());
                }
            } else {
                fs::rename(&restore, self.target()).map_err(|e| e.to_string())?;
            }
        }
        self.stage = Stage::RolledBack;
        self.message = Some(reason.into());
        self.save()
    }
}

fn replace_with_retry(candidate: &Path, target: &Path, expected_hash: &str) -> Result<(), String> {
    let mut last_error = String::new();
    for _ in 0..30 {
        match crate::native::replace_file(candidate, target) {
            Ok(()) => return Ok(()),
            Err(error) => {
                if file_hash(target).ok().as_deref() == Some(expected_hash) {
                    return Ok(());
                }
                last_error = error;
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
    Err(format!(
        "FILE_IN_USE: 主程序替换失败，备份已保留。{last_error}"
    ))
}

pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let temporary = path.with_extension("json.tmp");
    let mut file = File::create(&temporary).map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    fs::rename(temporary, path).map_err(|e| e.to_string())
}

pub fn ensure_managed(workspace: &Path, path: &Path) -> Result<(), String> {
    if !path.starts_with(workspace) {
        return Err("路径不在工具箱工作区内。".into());
    }
    for component in path
        .strip_prefix(workspace)
        .map_err(|e| e.to_string())?
        .components()
        .scan(workspace.to_path_buf(), |current, component| {
            current.push(component);
            Some(current.clone())
        })
    {
        if let Ok(metadata) = fs::symlink_metadata(&component) {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err("受管更新目录不能是链接或重解析点。".into());
                }
            }
            if metadata.file_type().is_symlink() {
                return Err("受管更新目录不能是链接。".into());
            }
        }
    }
    Ok(())
}

pub fn acknowledge_ready(
    workspace: &Path,
    id: &str,
    version: &str,
    session_id: &str,
) -> Result<(), String> {
    let tx = Transaction::load(workspace, id)?;
    let exe = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if exe != tx.target().canonicalize().map_err(|e| e.to_string())? {
        return Err("试运行不是工作区中的主程序。".into());
    }
    let ack = HealthAck {
        transaction_id: tx.id.clone(),
        nonce: tx.nonce.clone(),
        version: version.into(),
        session_id: session_id.into(),
        pid: std::process::id(),
        exe_sha256: file_hash(&exe)?,
    };
    if !validate_health(&tx, &ack, std::process::id()) {
        return Err("新版的版本或文件身份与更新事务不匹配。".into());
    }
    atomic_json(&tx.directory().join("ready.json"), &ack)
}

pub fn upgrade_lock(workspace: &Path) -> Result<Option<File>, String> {
    ensure_managed(workspace, &workspace.join("cache/self-update"))?;
    fs::create_dir_all(workspace.join("cache/self-update")).map_err(|e| e.to_string())?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(workspace.join("cache/self-update/upgrade.lock"))
        .map_err(|e| e.to_string())?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

pub fn pending_recovery(workspace: &Path) -> Result<Option<Transaction>, String> {
    let root = workspace.join("cache/self-update");
    ensure_managed(workspace, &root)?;
    if !root.is_dir() {
        return Ok(None);
    }
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        let tx = match Transaction::load(workspace, &id) {
            Ok(tx) => tx,
            Err(_) => continue,
        };
        if matches!(
            tx.stage,
            Stage::WaitingForExit
                | Stage::Installing
                | Stage::AwaitingReady
                | Stage::RecoveryRequired
        ) {
            return Ok(Some(tx));
        }
        // A helper cannot remove its own mapped image on Windows. The next normal launch does so.
        if matches!(tx.stage, Stage::Committed | Stage::RolledBack) {
            for name in [
                "helper.exe",
                "backup.exe",
                "candidate.exe",
                "restore.exe",
                "update.zip",
            ] {
                let _ = fs::remove_file(tx.directory().join(name));
            }
        }
    }
    Ok(None)
}

/// Copy the running executable, then wait until that independent helper owns the handoff.
pub fn handoff(transaction: &Transaction) -> Result<(), String> {
    use std::{
        process::Command,
        time::{Duration, Instant},
    };
    let helper = transaction.directory().join("helper.exe");
    fs::copy(std::env::current_exe().map_err(|e| e.to_string())?, &helper)
        .map_err(|e| e.to_string())?;
    let accepted = transaction.directory().join("helper.accepted.json");
    let _ = fs::remove_file(&accepted);
    let mut child = crate::native::hidden(
        Command::new(helper)
            .arg("--workspace")
            .arg(&transaction.workspace)
            .arg("--apply-update")
            .arg(&transaction.id)
            .arg("--parent-pid")
            .arg(std::process::id().to_string()),
    )
    .current_dir(&transaction.workspace)
    .spawn()
    .map_err(|e| e.to_string())?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if let Ok(bytes) = fs::read(&accepted) {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if value["nonce"] == transaction.nonce
                    && value["pid"].as_u64() == Some(child.id() as u64)
                {
                    return Ok(());
                }
            }
        }
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("独立更新进程未完成交接，请查看更新日志。".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // This child has not received permission to replace the live image; terminate only our child.
    let _ = child.kill();
    let _ = child.wait();
    Err("独立更新进程交接超时，当前工具箱继续运行。".into())
}

#[cfg(windows)]
pub fn run_helper(workspace: &Path, id: &str, parent_pid: u32) -> Result<(), String> {
    use std::{
        process::Command,
        time::{Duration, Instant},
    };
    let mut tx = Transaction::load(workspace, id)?;
    let current = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if current
        != tx
            .directory()
            .join("helper.exe")
            .canonicalize()
            .map_err(|e| e.to_string())?
    {
        return Err("更新模式只允许从受管 helper 副本运行。".into());
    }
    let helper_hash = file_hash(&current)?;
    if helper_hash != tx.old_sha256 && helper_hash != tx.new_sha256 {
        return Err("更新 helper 文件身份错误。".into());
    }
    let upgrade = upgrade_lock(workspace)?.ok_or("另一个本体更新进程正在运行。")?;
    let parent = crate::native::ParentProcess::open(parent_pid)?;
    let recovery = tx.stage != Stage::Prepared;
    tx.stage = Stage::WaitingForExit;
    tx.save()?;
    atomic_json(
        &tx.directory().join("helper.accepted.json"),
        &serde_json::json!({"nonce":tx.nonce,"pid":std::process::id()}),
    )?;
    parent.wait(120_000)?;
    let instance = crate::instance::InstanceGuard::acquire(workspace)?
        .ok_or("旧工具箱仍持有工作区，未替换主程序。")?;
    if recovery {
        tx.rollback("恢复上次未完成的工具箱更新。")?;
        drop(instance);
        drop(upgrade);
        crate::native::hidden(Command::new(tx.target()).arg("--workspace").arg(workspace))
            .current_dir(workspace)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    if let Err(error) = tx.install() {
        tx.rollback(&error)?;
        drop(instance);
        drop(upgrade);
        crate::native::hidden(Command::new(tx.target()).arg("--workspace").arg(workspace))
            .current_dir(workspace)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Err(error);
    }
    drop(instance);
    let mut trial = match crate::native::hidden(
        Command::new(tx.target())
            .arg("--workspace")
            .arg(workspace)
            .arg("--update-trial")
            .arg(id),
    )
    .current_dir(workspace)
    .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            tx.rollback(&format!("无法启动新版：{error}"))?;
            drop(upgrade);
            let _ =
                crate::native::hidden(Command::new(tx.target()).arg("--workspace").arg(workspace))
                    .current_dir(workspace)
                    .spawn();
            return Err(error.to_string());
        }
    };
    let start = Instant::now();
    let mut healthy = false;
    while start.elapsed() < Duration::from_secs(45) {
        if let Ok(bytes) = fs::read(tx.directory().join("ready.json")) {
            if let Ok(ack) = serde_json::from_slice::<HealthAck>(&bytes) {
                if validate_health(&tx, &ack, trial.id())
                    && file_hash(&tx.target())? == tx.new_sha256
                {
                    healthy = true;
                    break;
                }
            }
        }
        if trial.try_wait().map_err(|e| e.to_string())?.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if healthy {
        tx.stage = Stage::Committed;
        tx.message = Some("新版初始化及 WebView 首屏已确认。".into());
        tx.save()?;
        let _ = fs::remove_file(tx.directory().join("backup.exe"));
        let _ = fs::remove_file(tx.directory().join("update.zip"));
        Ok(())
    } else {
        let _ = trial.kill();
        trial.wait().map_err(|e| e.to_string())?;
        tx.rollback("新版未在 45 秒内确认首屏就绪，已恢复旧版本。")?;
        drop(upgrade);
        crate::native::hidden(Command::new(tx.target()).arg("--workspace").arg(workspace))
            .current_dir(workspace)
            .spawn()
            .map_err(|e| e.to_string())?;
        Err("新版首屏未就绪，已恢复旧版本。".into())
    }
}

#[cfg(not(windows))]
pub fn run_helper(_workspace: &Path, _id: &str, _parent_pid: u32) -> Result<(), String> {
    Err("本体更新只支持 Windows。".into())
}

pub fn file_hash(path: &Path) -> Result<String, String> {
    let mut input = File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let count = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hex::encode(hash.finalize()))
}

pub fn extract_package(
    archive: &Path,
    directory: &Path,
    expected_sha256: &str,
) -> Result<PathBuf, String> {
    if expected_sha256.len() != 64
        || !expected_sha256.bytes().all(|c| c.is_ascii_hexdigit())
        || !file_hash(archive)?.eq_ignore_ascii_case(expected_sha256)
    {
        return Err("HASH_MISMATCH: 工具箱更新包校验失败，请重新下载。".into());
    }
    let mut zip = zip::ZipArchive::new(File::open(archive).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if zip.len() != 1 {
        return Err("工具箱更新包必须只包含根目录的 PhysicalWorldToolbox.exe。".into());
    }
    let mut entry = zip.by_index(0).map_err(|e| e.to_string())?;
    if entry.name() != "PhysicalWorldToolbox.exe"
        || entry.is_dir()
        || entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
    {
        return Err("工具箱更新包目录结构不正确。".into());
    }
    fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let candidate = directory.join("candidate.exe");
    let mut output = File::create(&candidate).map_err(|e| e.to_string())?;
    std::io::copy(&mut entry, &mut output).map_err(|e| e.to_string())?;
    output.sync_all().map_err(|e| e.to_string())?;
    let mut magic = [0; 2];
    File::open(&candidate)
        .and_then(|mut f| f.read_exact(&mut magic))
        .map_err(|e| e.to_string())?;
    if magic != *b"MZ" {
        return Err("工具箱更新包中没有有效的 Windows EXE。".into());
    }
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{fs, io::Write};

    fn area(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../.work/host-tests")
            .join(format!("{name}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn archive(dir: &Path, entries: &[(&str, &[u8])]) -> (PathBuf, String) {
        let path = dir.join("update.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        for (name, contents) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(contents).unwrap();
        }
        zip.finish().unwrap();
        let hash = hex::encode(Sha256::digest(fs::read(&path).unwrap()));
        (path, hash)
    }

    #[test]
    fn accepts_only_a_verified_single_root_executable() {
        let dir = area("single-exe");
        let (path, hash) = archive(&dir, &[("PhysicalWorldToolbox.exe", b"MZ-test-candidate")]);
        let candidate = extract_package(&path, &dir.join("stage"), &hash).unwrap();
        assert_eq!(fs::read(candidate).unwrap(), b"MZ-test-candidate");
    }

    #[test]
    fn rejects_bad_hash_before_creating_candidate() {
        let dir = area("bad-hash");
        let (path, _) = archive(&dir, &[("PhysicalWorldToolbox.exe", b"MZ-test")]);
        assert!(extract_package(&path, &dir.join("stage"), &"0".repeat(64)).is_err());
        assert!(!dir.join("stage/PhysicalWorldToolbox.exe").exists());
    }

    #[test]
    fn rejects_extra_or_nested_files() {
        for entries in [
            vec![
                ("PhysicalWorldToolbox.exe", b"MZ-test".as_slice()),
                ("extra.txt", b"bad".as_slice()),
            ],
            vec![("folder/PhysicalWorldToolbox.exe", b"MZ-test".as_slice())],
            vec![("../PhysicalWorldToolbox.exe", b"MZ-test".as_slice())],
        ] {
            let dir = area("invalid-layout");
            let (path, hash) = archive(&dir, &entries);
            assert!(extract_package(&path, &dir.join("stage"), &hash).is_err());
            assert!(!dir.join("stage/PhysicalWorldToolbox.exe").exists());
        }
    }

    #[test]
    fn restores_the_original_after_an_unhealthy_restart_and_repeats_safely() {
        let dir = area("rollback");
        fs::write(dir.join(EXE_NAME), b"MZ-original").unwrap();
        let (path, hash) = archive(&dir, &[(EXE_NAME, b"MZ-next")]);
        let mut tx = Transaction::prepare(&dir, "update-test", "0.5.0", &path, &hash).unwrap();
        tx.install().unwrap();
        assert_eq!(fs::read(dir.join(EXE_NAME)).unwrap(), b"MZ-next");
        tx.rollback("readiness timed out").unwrap();
        tx.rollback("second recovery").unwrap();
        assert_eq!(fs::read(dir.join(EXE_NAME)).unwrap(), b"MZ-original");
        assert_eq!(tx.stage, Stage::RolledBack);
    }

    #[test]
    fn preserves_an_unrelated_replacement_and_the_backup() {
        let dir = area("foreign-file");
        fs::write(dir.join(EXE_NAME), b"MZ-original").unwrap();
        let (path, hash) = archive(&dir, &[(EXE_NAME, b"MZ-next")]);
        let mut tx = Transaction::prepare(&dir, "update-test", "0.5.0", &path, &hash).unwrap();
        tx.install().unwrap();
        fs::write(dir.join(EXE_NAME), b"MZ-foreign").unwrap();
        assert!(tx.rollback("interrupted").is_err());
        assert_eq!(fs::read(dir.join(EXE_NAME)).unwrap(), b"MZ-foreign");
        assert_eq!(
            fs::read(dir.join("cache/self-update/update-test/backup.exe")).unwrap(),
            b"MZ-original"
        );
    }

    #[test]
    fn accepts_readiness_only_from_this_trial_process_and_version() {
        let tx = Transaction {
            id: "update-1".into(),
            version: "0.5.0".into(),
            nonce: "random-handoff".into(),
            old_sha256: "old".into(),
            new_sha256: "new".into(),
            stage: Stage::AwaitingReady,
            message: None,
            workspace: PathBuf::new(),
        };
        let mut ack = HealthAck {
            transaction_id: tx.id.clone(),
            nonce: tx.nonce.clone(),
            version: tx.version.clone(),
            session_id: "session-1".into(),
            pid: 42,
            exe_sha256: tx.new_sha256.clone(),
        };
        assert!(validate_health(&tx, &ack, 42));
        assert!(!validate_health(&tx, &ack, 43));
        ack.version = "0.4.0".into();
        assert!(!validate_health(&tx, &ack, 42));
        ack.version = tx.version.clone();
        ack.nonce = "other-handoff".into();
        assert!(!validate_health(&tx, &ack, 42));
    }

    #[cfg(windows)]
    #[test]
    fn waits_for_a_short_lived_file_lock_when_restoring_the_original() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = area("rollback-lock");
        fs::write(dir.join(EXE_NAME), b"MZ-original").unwrap();
        let (path, hash) = archive(&dir, &[(EXE_NAME, b"MZ-next")]);
        let mut tx = Transaction::prepare(&dir, "update-test", "0.5.0", &path, &hash).unwrap();
        tx.install().unwrap();
        let file = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(dir.join(EXE_NAME))
            .unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            drop(file);
        });
        let result = tx.rollback("readiness timed out");
        release.join().unwrap();
        result.unwrap();
        assert_eq!(fs::read(dir.join(EXE_NAME)).unwrap(), b"MZ-original");
    }
}
