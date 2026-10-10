//! Product state ownership composed from the common instance/maintenance protocol.
use anyhow::{ensure, Context};
use std::os::unix::fs::MetadataExt;
use std::{
    fs,
    path::{Path, PathBuf},
};
use xcss::state_file::{FileIdentity, InstanceLock, MaintenanceLock, PrivateStateDirectory};

pub struct ApplicationLock {
    _instance: InstanceLock,
    _runtime: Option<InstanceLock>,
    database: PathBuf,
    database_identity: Option<(u64, u64)>,
    runtime: PrivateStateDirectory,
    pid_identity: FileIdentity,
}
pub struct DatabaseMaintenanceLock {
    _lock: MaintenanceLock,
}

impl ApplicationLock {
    pub fn acquire(database_url: &str, runtime_directory: &Path) -> anyhow::Result<Self> {
        let database = crate::sqlite::database_path(database_url)?;
        let root = database.parent().context("database parent is required")?;
        let directory = PrivateStateDirectory::open(root)?;
        let instance = directory.try_instance_lock()?;
        let database_identity = match fs::symlink_metadata(&database) {
            Ok(_) => {
                let descriptor = directory.open_existing(
                    database
                        .file_name()
                        .context("database filename is required")?,
                )?;
                let identity = descriptor.identity();
                Some((identity.device(), identity.inode()))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let runtime = PrivateStateDirectory::open(runtime_directory)?;
        let runtime_lock = if runtime_directory != root {
            Some(runtime.try_instance_lock()?)
        } else {
            None
        };
        let private = xcss::fs_safety::PrivateDirectory::open_existing(runtime_directory)?;
        xcss::fs_safety::AtomicFile::replace(
            &private,
            &xcss::fs_safety::RelativePath::new("app.pid")?,
            format!("{}\n", std::process::id()).as_bytes(),
        )?;
        let pid_identity = runtime.open_existing("app.pid")?.identity();
        Ok(Self {
            _instance: instance,
            _runtime: runtime_lock,
            database,
            database_identity,
            runtime,
            pid_identity,
        })
    }
    pub fn validate_open_database(&self) -> anyhow::Result<()> {
        let metadata = fs::symlink_metadata(&self.database)?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink() && metadata.nlink() == 1,
            "database must remain a regular single-link file"
        );
        if let Some(identity) = self.database_identity {
            ensure!(
                identity == (metadata.dev(), metadata.ino()),
                "database identity changed during startup"
            );
        }
        Ok(())
    }
}
impl DatabaseMaintenanceLock {
    pub fn shared(database_url: &str) -> anyhow::Result<Self> {
        let database = crate::sqlite::database_path(database_url)?;
        let root =
            PrivateStateDirectory::open(database.parent().context("database parent is required")?)?;
        root.verify_no_pending_maintenance()?;
        Ok(Self {
            _lock: root.try_shared_maintenance_lock()?,
        })
    }
    #[cfg(test)]
    fn exclusive(database_url: &str) -> anyhow::Result<Self> {
        let database = crate::sqlite::database_path(database_url)?;
        let root =
            PrivateStateDirectory::open(database.parent().context("database parent is required")?)?;
        Ok(Self {
            _lock: root.try_maintenance_lock()?,
        })
    }
}
impl Drop for ApplicationLock {
    fn drop(&mut self) {
        if self.runtime.verify_identity().is_err() {
            return;
        }
        if let Ok(file) = self.runtime.open_existing("app.pid") {
            if file.identity() == self.pid_identity {
                let _ = fs::remove_file(self.runtime.path().join("app.pid"));
                if let Ok(directory) =
                    xcss::fs_safety::PrivateDirectory::open_existing(self.runtime.path())
                {
                    let _ = directory.sync();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};
    fn state() -> (tempfile::TempDir, PathBuf, PathBuf, String) {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = root.path().join("runtime");
        fs::create_dir(&runtime).unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        let database = root.path().join("app.sqlite3");
        crate::sqlite::initialize_current_database(&database).unwrap();
        let url = format!("sqlite://{}", database.display());
        (root, database, runtime, url)
    }
    #[test]
    fn different_runtime_directories_still_exclude_the_same_state() {
        let (root, _database, runtime, url) = state();
        let other = root.path().join("other-runtime");
        fs::create_dir(&other).unwrap();
        fs::set_permissions(&other, fs::Permissions::from_mode(0o700)).unwrap();
        let first = ApplicationLock::acquire(&url, &runtime).unwrap();
        first.validate_open_database().unwrap();
        assert!(ApplicationLock::acquire(&url, &other).is_err());
        assert!(DatabaseMaintenanceLock::exclusive(&url).is_err());
        DatabaseMaintenanceLock::shared(&url).unwrap();
        drop(first);
        assert!(!runtime.join("app.pid").exists());
        let maintenance = DatabaseMaintenanceLock::exclusive(&url).unwrap();
        assert!(ApplicationLock::acquire(&url, &runtime).is_err());
        drop(maintenance);
        ApplicationLock::acquire(&url, &runtime).unwrap();
    }
    #[test]
    fn aliased_database_and_pending_maintenance_are_rejected() {
        let (root, database, runtime, url) = state();
        let alias = root.path().join("alias");
        fs::hard_link(&database, &alias).unwrap();
        assert!(ApplicationLock::acquire(&url, &runtime).is_err());
        fs::remove_file(alias).unwrap();
        let link = root.path().join("link.sqlite3");
        symlink(&database, &link).unwrap();
        assert!(
            ApplicationLock::acquire(&format!("sqlite://{}", link.display()), &runtime).is_err()
        );
        fs::write(
            root.path().join(xcss::state_file::MAINTENANCE_PENDING_FILE),
            b"{}",
        )
        .unwrap();
        assert!(ApplicationLock::acquire(&url, &runtime).is_err());
    }
    #[test]
    fn replacing_pid_is_not_deleted_by_the_previous_owner() {
        let (_root, _database, runtime, url) = state();
        let owner = ApplicationLock::acquire(&url, &runtime).unwrap();
        let private = xcss::fs_safety::PrivateDirectory::open_existing(&runtime).unwrap();
        xcss::fs_safety::AtomicFile::replace(
            &private,
            &xcss::fs_safety::RelativePath::new("app.pid").unwrap(),
            b"123456\n",
        )
        .unwrap();
        drop(owner);
        assert_eq!(fs::read(runtime.join("app.pid")).unwrap(), b"123456\n");
    }
}
