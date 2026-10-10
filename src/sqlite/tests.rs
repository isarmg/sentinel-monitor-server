use super::paths::{sqlite_generation_paths, sqlite_sidecar};
use super::*;
use sqlx::{sqlite::SqliteConnectOptions, ConnectOptions, Connection, SqliteConnection};
use std::{fs, path::Path, time::Duration};

async fn fixture_connection(path: &Path) -> SqliteConnection {
    SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(false)
        .connect()
        .await
        .unwrap()
}
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
#[tokio::test]
async fn history_cpu_deadline_interrupts_sqlite_and_does_not_poison_the_pool() {
    let directory = private_tempdir();
    let pool = initialize_test_pool(&format!(
        "sqlite://{}",
        directory.path().join("budget.sqlite3").display()
    ))
    .await
    .unwrap();
    let mut connection = history_connection(&pool).await.unwrap();
    let began = std::time::Instant::now();
    let error = sqlx::query_scalar::<_, i64>("WITH RECURSIVE counter(n) AS (VALUES(0) UNION ALL SELECT n+1 FROM counter WHERE n<1000000000) SELECT sum(n) FROM counter")
        .fetch_one(&mut *connection).await.unwrap_err();
    assert!(matches!(
        history_error(error),
        crate::error::AppError::HistoryCapacity
    ));
    assert!(began.elapsed() < Duration::from_secs(5));
    connection.close().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT 42")
            .fetch_one(&pool)
            .await
            .unwrap(),
        42
    );
}
fn private_tempdir() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    directory
}
fn private_generation(path: &Path) {
    for file in sqlite_generation_paths(path) {
        if file.exists() {
            fs::set_permissions(file, fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
}

#[tokio::test]
async fn current_schema_fingerprint_matches_the_compiled_contract() {
    let temporary = private_tempdir();
    let database = temporary.path().join("current.sqlite3");
    initialize_current_database(&database).unwrap();
    validate_current_database(&database).unwrap();

    let mut connection = fixture_connection(&database).await;
    let metadata: (String, String, i64, String) = sqlx::query_as(
        "SELECT application, application_version, schema_revision, schema_sha256 FROM product_metadata WHERE singleton=1"
    ).fetch_one(&mut connection).await.unwrap();
    assert_eq!(
        metadata,
        (
            APPLICATION.to_string(),
            "xcos-db-v2".to_string(),
            CURRENT_SCHEMA_REVISION,
            CURRENT_SCHEMA_SHA256.to_string()
        )
    );
    connection.close().await.unwrap();
}

#[tokio::test]
async fn foreign_database_without_metadata_is_rejected_without_changing_bytes() {
    let temporary = private_tempdir();
    let database = temporary.path().join("foreign.sqlite3");
    let mut connection = fixture_connection(&database).await;
    sqlx::raw_sql(
        "CREATE TABLE unrelated_records(id TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO unrelated_records VALUES('record', 'foreign');",
    )
    .execute(&mut connection)
    .await
    .unwrap();
    connection.close().await.unwrap();

    assert_rejected_without_byte_changes(&database).await;
}

#[tokio::test]
async fn noncurrent_wal_generation_is_rejected_without_changing_bytes() {
    let temporary = private_tempdir();
    let database = temporary.path().join("noncurrent-wal.sqlite3");
    let mut connection = fixture_connection(&database).await;
    sqlx::raw_sql(
        "PRAGMA journal_mode=WAL;
             PRAGMA wal_autocheckpoint=0;
             CREATE TABLE users(id TEXT PRIMARY KEY, username TEXT NOT NULL);
             INSERT INTO users VALUES('unknown-user', 'unknown-admin');",
    )
    .execute(&mut connection)
    .await
    .unwrap();
    assert!(sqlite_sidecar(&database, "-wal").exists());

    // Keep the writer open so the committed schema exists only in the WAL
    // while the product performs its read-only current-schema rejection.
    assert_rejected_without_byte_changes(&database).await;
    connection.close().await.unwrap();
}

#[tokio::test]
async fn corrupt_global_lease_wal_generations_are_read_only_restart_rejections() {
    let cases = [
        ("missing-row", "DELETE FROM media_reconciler_leases;"),
        (
            "extra-row",
            "INSERT INTO media_reconciler_leases (singleton, updated_at)
             VALUES (2, '1970-01-01T00:00:00+00:00');",
        ),
        (
            "owner-without-expiry",
            "UPDATE media_reconciler_leases
             SET lease_owner = '00000000-0000-4000-8000-000000000001';",
        ),
        (
            "noncanonical-owner",
            "UPDATE media_reconciler_leases
             SET lease_owner = '00000000-0000-1000-8000-000000000001',
                 lease_expires_at = '2030-01-01T00:01:00+00:00',
                 updated_at = '2030-01-01T00:00:00+00:00';",
        ),
        (
            "invalid-time-relation",
            "UPDATE media_reconciler_leases
             SET lease_owner = '00000000-0000-4000-8000-000000000001',
                 lease_expires_at = '2030-01-01T00:00:00+00:00',
                 updated_at = '2030-01-01T00:00:00+00:00';",
        ),
        (
            "unknown-shape",
            "ALTER TABLE media_reconciler_leases ADD COLUMN unexpected TEXT;",
        ),
    ];

    for (name, mutation) in cases {
        let temporary = private_tempdir();
        let database = temporary.path().join(format!("{name}.sqlite3"));
        initialize_current_database(&database).unwrap();
        let mut connection = fixture_connection(&database).await;
        sqlx::raw_sql(
            "PRAGMA journal_mode=WAL;
                 PRAGMA wal_autocheckpoint=0;
                 PRAGMA ignore_check_constraints=ON;",
        )
        .execute(&mut connection)
        .await
        .unwrap();
        sqlx::raw_sql(mutation)
            .execute(&mut connection)
            .await
            .unwrap();
        assert!(sqlite_sidecar(&database, "-wal").exists());

        // The second row is possible only because this fixture deliberately
        // models an out-of-protocol writer bypassing SQLite CHECK constraints.
        if name == "extra-row" {
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media_reconciler_leases")
                .fetch_one(&mut connection)
                .await
                .unwrap();
            assert_eq!(count, 2);
        }
        assert_rejected_without_byte_changes(&database).await;
        connection.close().await.unwrap();
        assert_rejected_without_byte_changes(&database).await;
    }
}

#[tokio::test]
async fn current_schema_committed_only_in_wal_is_validated_without_changes() {
    let temporary = private_tempdir();
    let database = temporary.path().join("current-wal.sqlite3");
    let mut connection = fixture_connection(&database).await;
    sqlx::raw_sql("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::raw_sql(CURRENT_SCHEMA)
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("INSERT INTO media_reconciler_leases(singleton,updated_at) VALUES(1,'1970-01-01T00:00:00+00:00')")
        .execute(&mut connection).await.unwrap();
    let fingerprint = xcss::sqlite::schema_fingerprint(&mut connection)
        .await
        .unwrap();
    sqlx::query("INSERT INTO product_metadata(singleton,application,application_version,schema_revision,schema_sha256) VALUES(1,?,?,?,?)")
        .bind(APPLICATION).bind("xcos-db-v2").bind(CURRENT_SCHEMA_REVISION).bind(fingerprint)
        .execute(&mut connection).await.unwrap();
    assert!(sqlite_sidecar(&database, "-wal").exists());

    private_generation(&database);
    let before = generation_bytes(&database);
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "sqlite::tests::current_wal_validation_child",
            "--nocapture",
        ])
        .env("XCOS_WAL_VALIDATION_PATH", &database)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        generation_bytes(&database) == before,
        "current-schema validation changed SQLite generation bytes"
    );
    connection.close().await.unwrap();
}

#[tokio::test]
async fn nonexact_metadata_and_actual_schema_are_read_only_rejections() {
    for (name, statement) in [
        (
            "wrong-application",
            "UPDATE product_metadata SET application = 'another-product'",
        ),
        (
            "noncurrent-version",
            "UPDATE product_metadata SET application_version = 'noncurrent-version'",
        ),
        (
            "wrong-revision",
            "UPDATE product_metadata SET schema_revision = 9",
        ),
        (
            "negative-revision",
            "UPDATE product_metadata SET schema_revision = -1",
        ),
        (
            "wrong-fingerprint",
            "UPDATE product_metadata SET schema_sha256 = 'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'",
        ),
        (
            "wrong-metadata-storage-class",
            "UPDATE product_metadata SET schema_sha256 = x'00'",
        ),
        ("missing-metadata-row", "DELETE FROM product_metadata"),
        (
            "extra-metadata-row",
            "PRAGMA ignore_check_constraints=ON;
             INSERT INTO product_metadata (
                 singleton, application, application_version, schema_revision, schema_sha256
             ) VALUES (
                 2, 'xcos', '0.2.2', 1,
                 'f547ddc817d830d23b5305bb1f88b29898d6531568edd6eb194c2b629eb560c0'
             )",
        ),
        (
            "metadata-shape-drift",
            "ALTER TABLE product_metadata ADD COLUMN unexpected TEXT",
        ),
        ("schema-tamper", "CREATE TABLE unexpected_product_table(id INTEGER)"),
    ] {
        let temporary = private_tempdir();
        let database = temporary.path().join(format!("{name}.sqlite3"));
        initialize_current_database(&database).unwrap();
        let mut connection = fixture_connection(&database).await;
        sqlx::raw_sql("PRAGMA journal_mode=DELETE;").execute(&mut connection).await.unwrap();
        sqlx::raw_sql(statement).execute(&mut connection).await.unwrap();
        connection.close().await.unwrap();

        assert_rejected_without_byte_changes(&database).await;
    }
}

async fn assert_rejected_without_byte_changes(path: &Path) {
    private_generation(path);
    let before = generation_bytes(path);
    let url = format!("sqlite://{}", path.display());
    let error = open_pool(&url).await.unwrap_err();
    assert!(
        format!("{error:#}").contains("database")
            || format!("{error:#}").contains("product_metadata")
            || format!("{error:#}").contains("schema")
            || format!("{error:#}").contains("lease"),
        "rejection must identify the current-state boundary: {error:#}"
    );
    assert!(
        generation_bytes(path) == before,
        "current-schema rejection changed SQLite generation bytes"
    );
}

#[test]
#[ignore = "subprocess helper for live WAL generation validation"]
fn current_wal_validation_child() {
    let Some(path) = std::env::var_os("XCOS_WAL_VALIDATION_PATH") else {
        return;
    };
    validate_current_database(Path::new(&path)).unwrap();
}

#[tokio::test]
async fn normal_open_rejects_missing_database_without_creating_it() {
    let directory = private_tempdir();
    let path = directory.path().join("missing.sqlite3");
    let before = fs::read_dir(directory.path()).unwrap().count();
    assert!(open_pool(&format!("sqlite://{}", path.display()))
        .await
        .is_err());
    assert!(!path.exists());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), before);
}

fn generation_bytes(path: &Path) -> BTreeMap<String, Vec<u8>> {
    sqlite_generation_paths(path)
        .into_iter()
        .filter(|candidate| candidate.exists())
        .map(|candidate| {
            (
                candidate
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                fs::read(candidate).unwrap(),
            )
        })
        .collect()
}
