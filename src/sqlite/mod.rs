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
pub const CURRENT_SCHEMA_REVISION: i64 = 1;
pub const CURRENT_SCHEMA_SHA256: &str =
    "c648d0eb3dc04e3b32e774775072ba826c5a3f7b9945d306920f2f34f23223d0";
const CURRENT_SCHEMA: &str = include_str!("../../schema/generated/current_schema.sql");

pub(crate) fn current_schema_identity() -> anyhow::Result<SchemaIdentity> {
    Ok(SchemaIdentity::new(
        APPLICATION,
        "xcos-db-v1",
        u64::try_from(CURRENT_SCHEMA_REVISION).context("current schema revision is negative")?,
        CURRENT_SCHEMA_SHA256,
    )?)
}

#[cfg(test)]
mod tests;
