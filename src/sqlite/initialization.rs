use super::connection::{initialize_current_database, open_initialization_pool};
use anyhow::Context;

pub(crate) async fn initialize_with_administrator(
    database: &std::path::Path,
    username: &str,
    password: &str,
) -> anyhow::Result<()> {
    let root = database
        .parent()
        .ok_or_else(|| anyhow::anyhow!("database parent is required"))?;
    let directory = xcss_fs_safety::PrivateDirectory::open_existing(root)?;
    anyhow::ensure!(
        !database.try_exists()?,
        "refusing to overwrite current database"
    );
    // One direct child is required by the shared no-clobber publication contract.
    // initialize_current_database reserves this unique name with create_new.
    let staging = root.join(format!(".xcos-init-{}.sqlite3", uuid::Uuid::new_v4()));
    initialize_current_database(&staging).context("initialize staging schema")?;
    let _staging_cleanup = tempfile::TempPath::try_from_path(staging.clone())?;
    let pool = open_initialization_pool(&format!("sqlite://{}", staging.display()))
        .await
        .context("open initialization pool")?;
    let result = async {
        let admin = xcss_admin_core::AdministratorService::new(
            xcss_admin_sqlite::SqliteAdministratorStore::new(pool.clone()),
        );
        admin
            .bootstrap_administrator(username, password, current_time_micros()?)
            .await
            .map_err(|error| anyhow::anyhow!(error))
            .context("bootstrap staging administrator")?;
        xcss_sqlite::checkpoint(&pool)
            .await
            .context("checkpoint staging pool")?;
        let mode: String = sqlx::query_scalar("PRAGMA journal_mode=DELETE")
            .fetch_one(&pool)
            .await
            .context("finalize staging journal")?;
        anyhow::ensure!(
            mode.eq_ignore_ascii_case("delete"),
            "staging journal mode did not change"
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    pool.close().await;
    result?;
    // Finalize on the one initialization connection before publishing one file.
    std::fs::File::open(&staging)?.sync_all()?;
    let source = xcss_fs_safety::RelativePath::new(staging.strip_prefix(root)?)?;
    let destination = xcss_fs_safety::RelativePath::new(
        database
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("database filename is required"))?,
    )?;
    xcss_fs_safety::NoClobberPublish::publish(&directory, &source, &destination)
        .context("publish completed current database")?;
    Ok(())
}

fn current_time_micros() -> anyhow::Result<u64> {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_micros(),
    )
    .map_err(|_| anyhow::anyhow!("current time exceeds administrator timestamp range"))
}

#[cfg(test)]
mod initialization_tests {
    #[tokio::test]
    async fn initialization_publishes_a_complete_authenticated_database_without_overwrite() {
        let directory = tempfile::tempdir().unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let database = directory.path().join("app.db");
        super::initialize_with_administrator(&database, "smoke-admin", "TemporaryPassphrase123")
            .await
            .unwrap();
        crate::sqlite::validate_current_database(&database).unwrap();
        let pool = crate::sqlite::open_pool(&format!("sqlite://{}", database.display()))
            .await
            .unwrap();
        let service = xcss_admin_core::AdministratorService::new(
            xcss_admin_sqlite::SqliteAdministratorStore::new(pool.clone()),
        );
        use xcss_admin_core::AdministratorStore as _;
        assert_eq!(service.store().administrator_count().await.unwrap(), 1);
        service.store().validate_all_administrators().await.unwrap();
        pool.close().await;
        let before = std::fs::read(&database).unwrap();
        assert!(super::initialize_with_administrator(
            &database,
            "other-admin",
            "AnotherPassphrase123"
        )
        .await
        .is_err());
        assert_eq!(std::fs::read(&database).unwrap(), before);
    }
}
