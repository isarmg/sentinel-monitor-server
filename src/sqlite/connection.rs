use super::{
    current_schema_identity, database_path,
    lease::validate_global_lease_table,
    paths::{
        require_real_parent, require_secure_database_file, sqlite_generation_paths, sync_parent,
    },
    APPLICATION, CURRENT_SCHEMA, CURRENT_SCHEMA_REVISION, CURRENT_SCHEMA_SHA256,
};
use anyhow::Context;
use chrono::Utc;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    ConnectOptions, Connection, SqliteConnection, SqlitePool,
};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
    str::FromStr,
    time::Duration,
};

const MAX_CONNECTIONS: u32 = 10;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const CONNECTION_LIMITS: xcss::sqlite::ConnectionLimits =
    xcss::sqlite::ConnectionLimits::new(1024 * 1024)
        .with_max_sql_bytes(256 * 1024)
        .with_max_vm_operations(100_000);

async fn apply_connection_limits(connection: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    xcss::sqlite::apply_connection_limits(connection, CONNECTION_LIMITS)
        .await
        .map(|_| ())
        .map_err(|error| sqlx::Error::Configuration(Box::new(error)))
}

pub async fn open_pool(database_url: &str) -> anyhow::Result<SqlitePool> {
    open_pool_with_limit(database_url, MAX_CONNECTIONS).await
}

/// Column names and limits are fixed by the product, never supplied by input.
/// Guard every selected value before SQLx materializes it; a corrupt large
/// scalar rejects its row through the mandatory identity instead of truncation.
pub(crate) fn bounded_columns(columns: &[(&str, usize)]) -> String {
    let condition = columns
        .iter()
        .map(|(name, limit)| format!("length(CAST(coalesce({name},'') AS BLOB))<={limit}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    columns
        .iter()
        .map(|(name, _)| format!("CASE WHEN {condition} THEN {name} END AS {name}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// History uses four dedicated connections at most; six pool slots remain for
/// commands and status. Never return a connection carrying a progress callback
/// to the shared pool, including when its caller is cancelled.
pub(crate) async fn history_connection(
    pool: &SqlitePool,
) -> crate::error::Result<sqlx::pool::PoolConnection<sqlx::Sqlite>> {
    let mut connection = tokio::time::timeout(Duration::from_secs(2), pool.acquire())
        .await
        .map_err(|_| crate::error::AppError::HistoryCapacity)??;
    connection.close_on_drop();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    connection
        .lock_handle()
        .await?
        .set_progress_handler(1000, move || std::time::Instant::now() < deadline);
    Ok(connection)
}

pub(crate) fn history_error(error: sqlx::Error) -> crate::error::AppError {
    if error
        .as_database_error()
        .and_then(|error| error.code())
        .as_deref()
        == Some("9")
    {
        crate::error::AppError::HistoryCapacity
    } else {
        error.into()
    }
}

pub(crate) async fn open_initialization_pool(database_url: &str) -> anyhow::Result<SqlitePool> {
    open_pool_with_limit(database_url, 1).await
}

async fn open_pool_with_limit(database_url: &str, connections: u32) -> anyhow::Result<SqlitePool> {
    prepare_current_database(database_url)?;
    let options = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(false)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(BUSY_TIMEOUT)
        .synchronous(SqliteSynchronous::Full);

    let pool = SqlitePoolOptions::new()
        .max_connections(connections)
        .after_connect(|connection, _| Box::pin(apply_connection_limits(connection)))
        .connect_with(options)
        .await?;
    if let Err(error) =
        xcss::sqlite::require_pool_current_schema(&pool, &current_schema_identity()?).await
    {
        pool.close().await;
        return Err(error.into());
    }
    Ok(pool)
}

fn prepare_current_database(database_url: &str) -> anyhow::Result<()> {
    let path = database_path(database_url)?;
    require_real_parent(&path)?;
    validate_current_database(&path)
}

pub(crate) fn validate_current_database(path: &Path) -> anyhow::Result<()> {
    validated_snapshot(path)?;
    Ok(())
}

fn validated_snapshot(path: &Path) -> anyhow::Result<xcss::server_cli::ValidationSnapshot> {
    require_secure_database_file(path)?;
    // The same main/WAL/journal byte budget bounds accepted persistent state
    // and its validation copy; a default 4 GiB snapshot would reject otherwise
    // admissible state under this product's 8 GiB storage threshold.
    let snapshot = xcss::server_cli::ValidationSnapshot::capture_with_limits(
        path,
        xcss::server_cli::SnapshotLimits {
            max_total_bytes: crate::history::Limits::CURRENT.bytes,
            ..Default::default()
        },
    )?;
    xcss::sqlite::block_on_sqlite_connection(async {
        // Recovery is allowed only on this held private copy, never the source.
        let mut connection = SqliteConnectOptions::new()
            .filename(snapshot.database_path())
            .create_if_missing(false)
            .busy_timeout(BUSY_TIMEOUT)
            .pragma("query_only", "ON")
            .pragma("trusted_schema", "OFF")
            .connect()
            .await
            .context("open private SQLite validation snapshot")?;
        let result = async {
            apply_connection_limits(&mut connection).await?;
            validate_current_connection(&mut connection).await
        }
        .await;
        let closed = connection.close().await;
        result?;
        closed?;
        Ok::<(), anyhow::Error>(())
    })?;
    Ok(snapshot)
}

pub(crate) async fn current_validation_snapshot(
    path: &Path,
) -> anyhow::Result<xcss::server_cli::ValidationSnapshotPool> {
    Ok(validated_snapshot(path)?
        .into_pool_with_connection_limits(CONNECTION_LIMITS)
        .await?)
}

/// The caller holds the product's private state directory and maintenance role.
/// SQLx does not offer SQLITE_OPEN_NOFOLLOW; the path and held directory checks
/// protect this source, while untrusted current-state reads use a private copy.
pub(crate) async fn connect_existing(
    path: &Path,
    read_only: bool,
) -> anyhow::Result<SqliteConnection> {
    require_secure_database_file(path)?;
    let mut connection = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .read_only(read_only)
        .foreign_keys(true)
        .busy_timeout(BUSY_TIMEOUT)
        .pragma("trusted_schema", "OFF")
        .connect()
        .await?;
    if let Err(error) = apply_connection_limits(&mut connection).await {
        let _ = connection.close().await;
        return Err(error.into());
    }
    Ok(connection)
}

pub(crate) fn initialize_current_database(path: &Path) -> anyhow::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let reserved = options
        .open(path)
        .with_context(|| format!("create current SQLite database {}", path.display()))?;
    reserved.sync_all()?;
    drop(reserved);
    let result = xcss::sqlite::block_on_sqlite_connection(async {
        let mut connection = connect_existing(path, false).await?;
        let result=async {
            let mut transaction=connection.begin_with("BEGIN IMMEDIATE").await?;
            let initialized=async {
                sqlx::raw_sql(CURRENT_SCHEMA).execute(&mut *transaction).await?;
                sqlx::query("INSERT INTO _common_platform_metadata(singleton,platform_generation,platform_schema_revision,profile,created_at_micros) VALUES(1,1,1,'server-control-plane',?)")
                    .bind(Utc::now().timestamp_micros()).execute(&mut *transaction).await?;
                sqlx::query("INSERT INTO media_reconciler_leases(singleton,updated_at) VALUES(1,'1970-01-01T00:00:00+00:00')")
                    .execute(&mut *transaction).await?;
                let actual=xcss::sqlite::schema_fingerprint(&mut *transaction).await?;
                current_schema_identity()?.verify_fingerprint(&actual)?;
                sqlx::query("INSERT INTO product_metadata(singleton,application,application_version,schema_revision,schema_sha256) VALUES(1,?,?,?,?)")
                    .bind(APPLICATION).bind("xcos-db-v2").bind(CURRENT_SCHEMA_REVISION).bind(CURRENT_SCHEMA_SHA256)
                    .execute(&mut *transaction).await?;
                validate_current_connection(&mut transaction).await
            }.await;
            match initialized {
                Ok(()) => transaction.commit().await?,
                Err(error) => { let _=transaction.rollback().await; return Err(error); }
            }
            Ok::<(),anyhow::Error>(())
        }.await;
        let closed = connection.close().await;
        result?;
        closed?;
        File::open(path)?.sync_all()?;
        sync_parent(path)?;
        Ok::<(), anyhow::Error>(())
    });
    if result.is_err() {
        for candidate in sqlite_generation_paths(path) {
            let _ = fs::remove_file(candidate);
        }
        let _ = sync_parent(path);
    }
    result
}

async fn validate_current_connection(connection: &mut SqliteConnection) -> anyhow::Result<()> {
    match xcss::sqlite::require_current_schema(connection, &current_schema_identity()?).await {
        Ok(_) => {}
        // Preserve the CLI's typed contract rejection across the shared adapter.
        Err(xcss::sqlite::Error::SchemaIdentity(error)) => return Err(error.into()),
        Err(error) => return Err(error.into()),
    }
    validate_global_lease_table(connection).await?;
    Ok(())
}

#[cfg(test)]
pub(crate) async fn initialize_test_pool(database_url: &str) -> anyhow::Result<SqlitePool> {
    use std::os::unix::fs::PermissionsExt;
    let path = database_path(database_url)?;
    fs::set_permissions(
        path.parent().context("test data directory")?,
        fs::Permissions::from_mode(0o700),
    )?;
    initialize_current_database(&path)?;
    open_pool(database_url).await
}
