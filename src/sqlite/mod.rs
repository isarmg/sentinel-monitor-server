use anyhow::Context;
use xcss::schema_identity::SchemaIdentity;

mod connection;
mod initialization;
mod lease;
mod paths;

pub(crate) use connection::{
    bounded_columns, connect_existing, current_validation_snapshot, history_connection,
    history_error, open_pool, validate_current_database,
};
#[cfg(test)]
pub(crate) use connection::{initialize_current_database, initialize_test_pool};
pub(crate) use initialization::initialize_with_administrator;
pub(crate) use lease::{
    validate_global_lease_schema_sql, validate_global_lease_values, GlobalLeaseState,
};
pub(crate) use paths::database_path;

const APPLICATION: &str = "xcos";
pub const CURRENT_SCHEMA_REVISION: i64 = 2;
pub const CURRENT_SCHEMA_SHA256: &str =
    "4d20083821ff39d78792d0795b26206e851c2e6d0523109ee49cfc06666a1d4a";
const CURRENT_SCHEMA: &str = include_str!("../../schema/generated/current_schema.sql");

pub(crate) fn current_schema_identity() -> anyhow::Result<SchemaIdentity> {
    Ok(SchemaIdentity::new(
        APPLICATION,
        "xcos-db-v2",
        u64::try_from(CURRENT_SCHEMA_REVISION).context("current schema revision is negative")?,
        CURRENT_SCHEMA_SHA256,
    )?)
}

#[cfg(test)]
mod tests;
