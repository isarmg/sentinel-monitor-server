use anyhow::{ensure, Context};
use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::SqliteConnection;
use uuid::{Uuid, Version};

// Current lease rows retain their exact SQL storage classes and field order.
type GlobalLeaseRow = (
    String,
    i64,
    String,
    Option<String>,
    String,
    Option<String>,
    String,
    String,
);

const GLOBAL_LEASE_SQL: &str = "CREATE TABLE media_reconciler_leases (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
    lease_owner TEXT,
    lease_expires_at TEXT,
    updated_at TEXT NOT NULL,
    CHECK ((lease_owner IS NULL) = (lease_expires_at IS NULL)),
    CHECK (julianday(updated_at) IS NOT NULL),
    CHECK (
        lease_owner IS NULL OR (
            length(lease_owner) = 36
            AND lease_owner = lower(lease_owner)
            AND substr(lease_owner, 9, 1) = '-'
            AND substr(lease_owner, 14, 1) = '-'
            AND substr(lease_owner, 15, 1) = '4'
            AND substr(lease_owner, 19, 1) = '-'
            AND substr(lease_owner, 20, 1) GLOB '[89ab]'
            AND substr(lease_owner, 24, 1) = '-'
            AND lease_owner NOT GLOB '*[^0-9a-f-]*'
            AND length(replace(lease_owner, '-', '')) = 32
        )
    ),
    CHECK (
        lease_expires_at IS NULL OR (
            julianday(lease_expires_at) IS NOT NULL
            AND julianday(lease_expires_at) > julianday(updated_at)
        )
    )
)";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GlobalLeaseState {
    pub owner: Option<String>,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

pub(super) async fn validate_global_lease_table(
    connection: &mut SqliteConnection,
) -> anyhow::Result<GlobalLeaseState> {
    let sql: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_schema WHERE type='table' AND name='media_reconciler_leases'",
    )
    .fetch_one(&mut *connection)
    .await
    .context("database has no current media_reconciler_leases table")?;
    validate_global_lease_schema_sql(&sql)?;
    let actual: Vec<(String, String, i64, i64)> = sqlx::query_as(r#"SELECT name,type,"notnull",pk FROM pragma_table_info('media_reconciler_leases') ORDER BY cid"#)
        .fetch_all(&mut *connection).await?;
    let expected = vec![
        ("singleton".to_string(), "INTEGER".to_string(), 1, 1),
        ("lease_owner".to_string(), "TEXT".to_string(), 0, 0),
        ("lease_expires_at".to_string(), "TEXT".to_string(), 0, 0),
        ("updated_at".to_string(), "TEXT".to_string(), 1, 0),
    ];
    ensure!(
        actual == expected,
        "media_reconciler_leases columns do not match the current contract"
    );
    // Two rows suffice to reject an extra singleton; guard scalar decoding too.
    let rows: Vec<GlobalLeaseRow> = sqlx::query_as(
        "SELECT typeof(singleton), singleton, typeof(lease_owner), lease_owner,
                typeof(lease_expires_at), lease_expires_at, typeof(updated_at), updated_at
         FROM media_reconciler_leases ORDER BY singleton LIMIT 2",
    )
    .fetch_all(&mut *connection)
    .await?;
    ensure!(
        rows.len() == 1,
        "media_reconciler_leases must contain exactly one current row"
    );
    let (
        singleton_storage,
        singleton,
        owner_storage,
        owner,
        expiry_storage,
        expiry,
        updated_storage,
        updated,
    ) = rows.into_iter().next().expect("one lease row was required");
    validate_global_lease_values(
        &singleton_storage,
        singleton,
        &owner_storage,
        owner.as_deref(),
        &expiry_storage,
        expiry.as_deref(),
        &updated_storage,
        &updated,
    )
}

pub(crate) fn validate_global_lease_schema_sql(sql: &str) -> anyhow::Result<()> {
    ensure!(
        normalize_sql(sql) == normalize_sql(GLOBAL_LEASE_SQL),
        "media_reconciler_leases table does not match the current contract"
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn validate_global_lease_values(
    singleton_storage: &str,
    singleton: i64,
    owner_storage: &str,
    owner: Option<&str>,
    expiry_storage: &str,
    expiry: Option<&str>,
    updated_storage: &str,
    updated: &str,
) -> anyhow::Result<GlobalLeaseState> {
    ensure!(
        singleton_storage == "integer" && singleton == 1,
        "global lease singleton is not exactly current"
    );
    ensure!(
        updated_storage == "text",
        "global lease updated_at storage is not exactly current"
    );
    let updated_at = parse_canonical_utc(updated, "global lease updated_at")?;

    let (owner, lease_expires_at) = match (owner, expiry) {
        (None, None) => {
            ensure!(
                owner_storage == "null" && expiry_storage == "null",
                "free global lease storage is not exactly current"
            );
            (None, None)
        }
        (Some(owner), Some(expiry)) => {
            ensure!(
                owner_storage == "text" && expiry_storage == "text",
                "owned global lease storage is not exactly current"
            );
            let owner_id = Uuid::parse_str(owner).context("global lease owner is not a UUID")?;
            ensure!(
                owner_id.hyphenated().to_string() == owner
                    && owner_id.get_version() == Some(Version::Random),
                "global lease owner is not a canonical lowercase UUIDv4"
            );
            let lease_expires_at = parse_canonical_utc(expiry, "global lease lease_expires_at")?;
            ensure!(
                lease_expires_at > updated_at,
                "global lease expiry must be later than updated_at"
            );
            (Some(owner.to_string()), Some(lease_expires_at))
        }
        _ => anyhow::bail!("global lease owner and expiry must be present or absent together"),
    };

    Ok(GlobalLeaseState {
        owner,
        lease_expires_at,
        updated_at,
    })
}

fn parse_canonical_utc(value: &str, label: &str) -> anyhow::Result<DateTime<Utc>> {
    let parsed =
        DateTime::parse_from_rfc3339(value).with_context(|| format!("{label} is not RFC 3339"))?;
    ensure!(parsed.offset().local_minus_utc() == 0, "{label} is not UTC");
    let parsed = parsed.with_timezone(&Utc);
    ensure!(
        parsed.to_rfc3339_opts(SecondsFormat::AutoSi, false) == value,
        "{label} is not canonical"
    );
    Ok(parsed)
}

fn normalize_sql(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}
