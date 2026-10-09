use anyhow::{ensure, Context};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{
    fs::{self, File},
    path::{Component, Path, PathBuf},
};

pub(crate) fn database_path(database_url: &str) -> anyhow::Result<PathBuf> {
    let value = database_url
        .strip_prefix("sqlite://")
        .or_else(|| database_url.strip_prefix("sqlite:"))
        .context("DATABASE_URL must use the sqlite scheme")?;
    ensure!(!value.is_empty(), "SQLite database path must not be empty");
    ensure!(value != ":memory:", "in-memory SQLite is not supported");
    ensure!(
        !value.contains('?')
            && !value.contains('#')
            && !value.contains('%')
            && !value.contains('\0'),
        "DATABASE_URL must be a plain, unescaped SQLite file URL"
    );
    let path = PathBuf::from(value);
    ensure!(path.is_absolute(), "SQLite database path must be absolute");
    ensure!(
        path.file_name().is_some(),
        "SQLite database path must name a file"
    );
    ensure!(
        !path
            .components()
            .any(|component| matches!(component, Component::ParentDir)),
        "SQLite database path must not contain parent traversal"
    );
    Ok(path)
}

pub(super) fn require_secure_database_file(path: &Path) -> anyhow::Result<()> {
    require_real_parent(path)?;
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("SQLite database does not exist: {}", path.display()))?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "SQLite database must be a regular file without symbolic links"
    );
    #[cfg(unix)]
    ensure!(
        metadata.nlink() == 1,
        "SQLite database must not have hard-link aliases"
    );
    Ok(())
}

pub(super) fn require_real_parent(path: &Path) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .context("SQLite database must have a parent")?;
    let mut current = PathBuf::new();
    for component in parent.components() {
        match component {
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::CurDir => current.push("."),
            Component::Normal(value) => current.push(value),
            Component::ParentDir => anyhow::bail!("SQLite path must not contain parent traversal"),
        }
        let metadata = fs::symlink_metadata(&current)
            .with_context(|| format!("SQLite parent does not exist: {}", current.display()))?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "SQLite path must not traverse symbolic links or special files"
        );
    }
    Ok(())
}

pub(super) fn sqlite_generation_paths(path: &Path) -> [PathBuf; 4] {
    [
        path.to_path_buf(),
        sqlite_sidecar(path, "-wal"),
        sqlite_sidecar(path, "-shm"),
        sqlite_sidecar(path, "-journal"),
    ]
}

pub(super) fn sqlite_sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

pub(super) fn sync_parent(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    File::open(path.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()?;
    Ok(())
}
