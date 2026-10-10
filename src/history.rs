//! Product admission budgets. Existing facts and outstanding receipts are never
//! deleted to make room; finishing accepted work uses its reserved headroom.
use crate::error::{AppError, Result};
use sqlx::{Sqlite, Transaction};

#[derive(Clone, Copy)]
pub(crate) struct Limits {
    pub rows: i64,
    pub owner_rows: i64,
    pub bytes: u64,
    pub owner_bytes: u64,
    pub free_floor: u64,
}

impl Limits {
    pub const CURRENT: Self = Self {
        rows: 1_000_000,
        owner_rows: 100_000,
        bytes: 8 * 1024 * 1024 * 1024,
        owner_bytes: 2 * 1024 * 1024 * 1024,
        free_floor: 1024 * 1024 * 1024,
    };
}

pub(crate) async fn reserve_in(
    tx: &mut Transaction<'_, Sqlite>,
    owner: Option<uuid::Uuid>,
    new_operation: bool,
) -> Result<()> {
    reserve_with(tx, owner, new_operation, Limits::CURRENT).await
}

pub(crate) async fn reserve_with(
    tx: &mut Transaction<'_, Sqlite>,
    owner: Option<uuid::Uuid>,
    new_operation: bool,
    limits: Limits,
) -> Result<()> {
    let (rows, own, outstanding, pending_commands): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM events)+(SELECT count(*) FROM audit_logs)+\
         (SELECT count(*) FROM _xcss_operations)+(SELECT count(*) FROM _xcss_operation_audit_outbox),\
         (SELECT count(*) FROM events WHERE camera_id=?)+\
         (SELECT count(*) FROM audit_logs WHERE entity_type='camera' AND entity_id=?)+\
         (SELECT count(*) FROM _xcss_operations WHERE target_key=?)+\
         (SELECT count(*) FROM _xcss_operation_audit_outbox a JOIN _xcss_operations o ON o.operation_id=a.operation_id WHERE o.target_key=?),\
         (SELECT count(*) FROM _xcss_operations WHERE state IN ('pending','running','unknown','dead_letter')),\
         (SELECT count(*) FROM device_commands WHERE status='pending')",
    )
    .bind(owner).bind(owner).bind(owner.map(|id| id.to_string())).bind(owner.map(|id| id.to_string())).fetch_one(&mut **tx).await?;
    // A terminal operation releases its future transition budget, but each
    // undelivered outbox fact still needs a separate audit row. Keep that
    // projection reserved until the audit insert and acknowledgement commit.
    let (pending_projections, own_pending_projections): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM _xcss_operation_audit_outbox WHERE delivered_at_micros IS NULL),\
         (SELECT count(*) FROM _xcss_operation_audit_outbox a JOIN _xcss_operations o ON o.operation_id=a.operation_id WHERE a.delivered_at_micros IS NULL AND o.target_key=?)",
    )
    .bind(owner.map(|id| id.to_string())).fetch_one(&mut **tx).await?;
    // Eight attempts, final/unknown reconciliation and their outbox projections
    // fit inside 64 future rows per outstanding operation. PTZ results update
    // their existing receipt; a new PTZ also writes its queued audit.
    let reserved_rows = outstanding
        .saturating_mul(64)
        .saturating_add(pending_projections);
    // Creating an operation also creates its initial outbox fact and reserves
    // that fact's projection, besides the 64 future transition/projection rows.
    let added_rows = if new_operation { 67 } else { 1 };
    let own_outstanding: i64 = sqlx::query_scalar("SELECT count(*) FROM _xcss_operations WHERE target_key=? AND state IN ('pending','running','unknown','dead_letter')")
        .bind(owner.map(|id| id.to_string())).fetch_one(&mut **tx).await?;
    if rows
        .saturating_add(reserved_rows)
        .saturating_add(added_rows)
        > limits.rows
        || (owner.is_some()
            && own
                .saturating_add(own_outstanding.saturating_mul(64))
                .saturating_add(own_pending_projections)
                .saturating_add(added_rows)
                > limits.owner_rows)
    {
        return Err(AppError::HistoryStorageCapacity);
    }
    if let Some(owner) = owner {
        let own_payload: i64 = sqlx::query_scalar(
            "SELECT coalesce((SELECT sum(length(CAST(message AS BLOB))+length(CAST(details AS BLOB))) FROM events WHERE camera_id=?),0)+\
             coalesce((SELECT sum(length(CAST(details AS BLOB))) FROM audit_logs WHERE entity_type='camera' AND entity_id=?),0)+\
             coalesce((SELECT sum(length(request_payload)+coalesce(length(result_payload),0)) FROM _xcss_operations WHERE target_key=?),0)+\
             coalesce((SELECT sum(length(CAST(a.payload_json AS BLOB))) FROM _xcss_operation_audit_outbox a JOIN _xcss_operations o ON o.operation_id=a.operation_id WHERE o.target_key=?),0)")
            .bind(owner).bind(owner).bind(owner.to_string()).bind(owner.to_string()).fetch_one(&mut **tx).await?;
        let own_reserve = (own_outstanding.max(0) as u64)
            .saturating_mul(1024 * 1024)
            .saturating_add((own_pending_projections.max(0) as u64).saturating_mul(4096))
            .saturating_add(if new_operation {
                1024 * 1024
            } else {
                128 * 1024
            });
        let charged = (own_payload.max(0) as u64)
            .saturating_add((own.max(0) as u64).saturating_mul(2048))
            .saturating_add(own_reserve)
            .saturating_mul(2);
        if charged > limits.owner_bytes {
            return Err(AppError::HistoryStorageCapacity);
        }
    }
    let pages: i64 = sqlx::query_scalar("PRAGMA page_count")
        .fetch_one(&mut **tx)
        .await?;
    let page_size: i64 = sqlx::query_scalar("PRAGMA page_size")
        .fetch_one(&mut **tx)
        .await?;
    let allocated = (pages.max(0) as u64).saturating_mul(page_size.max(0) as u64);
    let path: String =
        sqlx::query_scalar("SELECT file FROM pragma_database_list WHERE name='main'")
            .fetch_one(&mut **tx)
            .await?;
    let (wal_bytes, free) = if path.is_empty() {
        (0, u64::MAX)
    } else {
        tokio::task::spawn_blocking(move || -> Result<(u64, u64)> {
            let wal = std::path::PathBuf::from(format!("{path}-wal"));
            let wal_bytes = match std::fs::symlink_metadata(wal) {
                Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                    metadata.len()
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
                _ => return Err(AppError::Internal("数据库日志文件状态无效".into())),
            };
            let parent = std::path::Path::new(&path)
                .parent()
                .ok_or_else(|| AppError::Internal("数据库路径无效".into()))?;
            let stats = rustix::fs::statvfs(parent)
                .map_err(|_| AppError::Internal("无法检查数据库可用空间".into()))?;
            Ok((wal_bytes, stats.f_bavail.saturating_mul(stats.f_frsize)))
        })
        .await
        .map_err(|_| AppError::Internal("数据库容量检查失败".into()))??
    };
    // Reserve bounded result/audit updates for every outstanding operation and
    // pending PTZ. Charge both main and WAL copies; do not call quota exhaustion
    // a physical disk failure. A new operation reserves 1 MiB before acceptance.
    let reserve = (outstanding.max(0) as u64)
        .saturating_mul(1024 * 1024)
        .saturating_add((pending_commands.max(0) as u64).saturating_mul(4096))
        .saturating_add((pending_projections.max(0) as u64).saturating_mul(4096))
        .saturating_add(if new_operation {
            1024 * 1024
        } else {
            128 * 1024
        })
        .saturating_mul(2);
    if allocated.saturating_add(wal_bytes).saturating_add(reserve) > limits.bytes
        || free < limits.free_floor.saturating_add(reserve)
    {
        return Err(AppError::HistoryStorageCapacity);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn full_history_preserves_facts_and_admits_reserved_final_receipts() {
        let directory = tempfile::tempdir().unwrap();
        let pool = crate::sqlite::initialize_test_pool(&format!(
            "sqlite://{}",
            directory.path().join("history.sqlite3").display()
        ))
        .await
        .unwrap();
        let event = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO events(id,kind,severity,message,created_at) VALUES(?,'test','critical','preserve',?)")
            .bind(event).bind(chrono::Utc::now()).execute(&pool).await.unwrap();
        let small = Limits {
            rows: 1,
            owner_rows: 1,
            bytes: u64::MAX,
            owner_bytes: u64::MAX,
            free_floor: 0,
        };
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        assert!(matches!(
            reserve_with(&mut tx, None, false, small).await,
            Err(AppError::HistoryStorageCapacity)
        ));
        tx.rollback().await.unwrap();
        let retained: String = sqlx::query_scalar("SELECT message FROM events WHERE id=?")
            .bind(event)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(retained, "preserve");
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        assert!(matches!(
            reserve_with(
                &mut tx,
                None,
                false,
                Limits {
                    rows: 100,
                    bytes: 1,
                    ..small
                }
            )
            .await,
            Err(AppError::HistoryStorageCapacity)
        ));
        assert!(matches!(
            reserve_with(
                &mut tx,
                None,
                false,
                Limits {
                    rows: 100,
                    free_floor: u64::MAX,
                    ..small
                }
            )
            .await,
            Err(AppError::HistoryStorageCapacity)
        ));
        tx.rollback().await.unwrap();
        // Capacity prevents new writes; acknowledging an existing alert still
        // updates its retained row without allocating another history fact.
        sqlx::query("UPDATE events SET acknowledged_at=? WHERE id=?")
            .bind(chrono::Utc::now())
            .bind(event)
            .execute(&pool)
            .await
            .unwrap();
        let acknowledged: bool =
            sqlx::query_scalar("SELECT acknowledged_at IS NOT NULL FROM events WHERE id=?")
                .bind(event)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(acknowledged);
    }
    #[tokio::test]
    async fn uuid_owner_limits_and_unknown_operation_final_audit_remain_enforced() {
        use xcss::operations::{NewOperation, SqliteOperationStore, Transition};
        let directory = tempfile::tempdir().unwrap();
        let pool = crate::sqlite::initialize_test_pool(&format!(
            "sqlite://{}",
            directory.path().join("receipts.sqlite3").display()
        ))
        .await
        .unwrap();
        let owner = uuid::Uuid::new_v4();
        let limits = Limits {
            rows: 1000,
            owner_rows: 2,
            bytes: u64::MAX,
            owner_bytes: u64::MAX,
            free_floor: 0,
        };
        for _ in 0..2 {
            sqlx::query("INSERT INTO audit_logs(id,action,entity_type,entity_id,created_at) VALUES(?,'test','camera',?,?)")
                .bind(uuid::Uuid::new_v4()).bind(owner).bind(chrono::Utc::now()).execute(&pool).await.unwrap();
        }
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        assert!(matches!(
            reserve_with(&mut tx, Some(owner), false, limits).await,
            Err(AppError::HistoryStorageCapacity)
        ));
        assert!(matches!(
            reserve_with(
                &mut tx,
                Some(owner),
                false,
                Limits {
                    owner_rows: 1000,
                    owner_bytes: 1,
                    ..limits
                }
            )
            .await,
            Err(AppError::HistoryStorageCapacity)
        ));
        reserve_with(&mut tx, Some(uuid::Uuid::new_v4()), false, limits)
            .await
            .unwrap();
        tx.rollback().await.unwrap();
        let now = chrono::Utc::now().timestamp_micros();
        let id = uuid::Uuid::new_v4().to_string();
        let store = SqliteOperationStore::new(pool.clone());
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        let small = Limits {
            rows: 69,
            owner_rows: 1000,
            ..limits
        };
        reserve_with(&mut tx, Some(owner), true, small)
            .await
            .unwrap();
        SqliteOperationStore::enqueue_in(
            &mut tx,
            NewOperation {
                operation_id: id.clone(),
                namespace: crate::reconciliation::OPERATION_NAMESPACE.into(),
                target_key: owner.to_string(),
                action: "reconcile_camera".into(),
                idempotency_digest: [2; 32],
                request_fingerprint: [3; 32],
                request_payload: serde_json::to_vec(&serde_json::json!({
                    "camera_id": owner,
                    "generation": 1,
                    "reason": "test",
                    "requested_by": null
                }))
                .unwrap(),
                max_attempts: 8,
                not_before_micros: now,
                created_at_micros: now,
            },
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        assert!(matches!(
            reserve_with(&mut tx, Some(owner), false, small).await,
            Err(AppError::HistoryStorageCapacity)
        ));
        tx.rollback().await.unwrap();
        let claim = store
            .claim_next(
                crate::reconciliation::OPERATION_NAMESPACE,
                "owner",
                now,
                now + 1_000_000,
            )
            .await
            .unwrap()
            .unwrap();
        store
            .apply_transition_owned(
                &claim.operation,
                Transition::MarkIndeterminate {
                    code: "outcome_unknown".into(),
                },
                None,
                now + 1,
            )
            .await
            .unwrap();
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        SqliteOperationStore::resolve_in(
            &mut tx,
            &id,
            xcss::operations::Resolution::ConfirmedSucceeded,
            now + 2,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(store.pending_audit_count().await.unwrap(), 4);
        assert_eq!(
            store.get(&id).await.unwrap().unwrap().operation.state,
            xcss::operations::OperationState::Resolved
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM audit_logs")
                .fetch_one(&pool)
                .await
                .unwrap(),
            2
        );
        let terminal = Limits {
            rows: 11,
            owner_rows: 11,
            ..limits
        };
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        // Two retained audit rows, one operation and four outbox facts already
        // use seven rows. The four pending projections still own the remainder.
        assert!(matches!(
            reserve_with(&mut tx, Some(owner), false, terminal).await,
            Err(AppError::HistoryStorageCapacity)
        ));
        assert!(matches!(
            reserve_with(
                &mut tx,
                Some(owner),
                false,
                Limits {
                    rows: 1000,
                    ..terminal
                }
            )
            .await,
            Err(AppError::HistoryStorageCapacity)
        ));
        let charged_payload: i64 = sqlx::query_scalar(
            "SELECT (SELECT coalesce(sum(length(CAST(details AS BLOB))),0) FROM audit_logs WHERE entity_type='camera' AND entity_id=?)+\
             (SELECT length(request_payload)+coalesce(length(result_payload),0) FROM _xcss_operations WHERE operation_id=?)+\
             (SELECT coalesce(sum(length(CAST(payload_json AS BLOB))),0) FROM _xcss_operation_audit_outbox WHERE operation_id=?)",
        ).bind(owner).bind(&id).bind(&id).fetch_one(&mut *tx).await.unwrap();
        let without_projection_bytes = (charged_payload as u64 + 7 * 2048 + 128 * 1024) * 2;
        assert!(matches!(
            reserve_with(
                &mut tx,
                Some(owner),
                false,
                Limits {
                    rows: 1000,
                    owner_rows: 1000,
                    owner_bytes: without_projection_bytes,
                    ..terminal
                }
            )
            .await,
            Err(AppError::HistoryStorageCapacity)
        ));
        tx.rollback().await.unwrap();
        assert_eq!(
            crate::reconciliation::flush_operation_audit(&pool)
                .await
                .unwrap(),
            4
        );
        assert_eq!(store.pending_audit_count().await.unwrap(), 0);
        let retained: i64 = sqlx::query_scalar(
            "SELECT (SELECT count(*) FROM events)+(SELECT count(*) FROM audit_logs)+\
             (SELECT count(*) FROM _xcss_operations)+(SELECT count(*) FROM _xcss_operation_audit_outbox)",
        ).fetch_one(&pool).await.unwrap();
        assert_eq!(retained, terminal.rows);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM audit_logs WHERE action='test'")
                .fetch_one(&pool)
                .await
                .unwrap(),
            2
        );
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        assert!(matches!(
            reserve_with(
                &mut tx,
                Some(owner),
                false,
                Limits {
                    rows: 1000,
                    ..terminal
                }
            )
            .await,
            Err(AppError::HistoryStorageCapacity)
        ));
        let without_outbox_payload: i64 = sqlx::query_scalar(
            "SELECT (SELECT coalesce(sum(length(CAST(details AS BLOB))),0) FROM audit_logs WHERE entity_type='camera' AND entity_id=?)+\
             (SELECT length(request_payload)+coalesce(length(result_payload),0) FROM _xcss_operations WHERE operation_id=?)",
        ).bind(owner).bind(&id).fetch_one(&mut *tx).await.unwrap();
        let owner_bytes = (without_outbox_payload as u64 + retained as u64 * 2048 + 128 * 1024) * 2;
        assert!(matches!(
            reserve_with(
                &mut tx,
                Some(owner),
                false,
                Limits {
                    rows: 1000,
                    owner_rows: 1000,
                    owner_bytes,
                    ..terminal
                }
            )
            .await,
            Err(AppError::HistoryStorageCapacity)
        ));
        tx.rollback().await.unwrap();
        pool.close().await;
    }
}
