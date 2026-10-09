use anyhow::Context;
use xcss_schema_identity::SchemaIdentity;

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
    "89d3e59dab120939725a7e3b052cf3cf5887f051c024e0325c3e9494043909e3";
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
