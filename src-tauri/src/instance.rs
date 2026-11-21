use std::{
    fs::{File, OpenOptions},
    path::Path,
};

pub struct InstanceGuard {
    _file: File,
}

impl InstanceGuard {
    pub fn acquire(workspace: &Path) -> Result<Option<Self>, String> {
        let directory = workspace.join("cache");
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("toolbox.instance.lock"))
            .map_err(|e| e.to_string())?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn permits_one_owner_and_releases_after_drop() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.work/host-tests/instance-lock");
        std::fs::create_dir_all(&dir).unwrap();
        let owner = InstanceGuard::acquire(&dir)
            .unwrap()
            .expect("first instance");
        assert!(InstanceGuard::acquire(&dir).unwrap().is_none());
        drop(owner);
        assert!(InstanceGuard::acquire(&dir).unwrap().is_some());
    }
}
