use crate::{
    error::Result,
    models::{CameraRecord, EventRecord},
    reconciliation, AppState,
};
use chrono::Utc;
use serde_json::{json, Value};
use tokio::{
    sync::watch,
    time::{self, MissedTickBehavior},
};
use uuid::Uuid;

const CAMERA_SELECT: &str = "SELECT id, name, location, source_kind, client_id, \
    adapter_kind, manufacturer, model, firmware_version, serial_number, capabilities_json, \
    streams_json, health_message, device_status, has_sub_stream, \
    enabled, record_enabled, storage_mode, status, last_seen_at, created_at, updated_at \
    FROM cameras WHERE deleted_at IS NULL";

pub async fn operation_audit_loop(
    pool: sqlx::SqlitePool,
    mut shutdown: watch::Receiver<bool>,
) -> std::result::Result<(), String> {
    let mut interval = time::interval(std::time::Duration::from_secs(1));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = xcss_server_runtime::wait_for_shutdown(&mut shutdown) => return Ok(()),
            _ = interval.tick() => {
                reconciliation::flush_operation_audit(&pool).await
                    .map_err(|_| "operation audit delivery failed".to_owned())?;
            }
        }
    }
}

pub async fn reconcile_loop(
    state: AppState,
    mut shutdown: watch::Receiver<bool>,
) -> std::result::Result<(), String> {
    let mut interval = time::interval(state.config.reconcile_interval);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { return Ok(()); }
            }
            _ = interval.tick() => {}
            _ = state.reconcile_notify.notified() => {}
        }
        if let Err(error) = reconciliation::reconcile_available(&state).await {
            tracing::warn!(error = %error, "camera reconciliation cycle failed");
        }
    }
}

pub async fn status_loop(
    state: AppState,
    mut shutdown: watch::Receiver<bool>,
) -> std::result::Result<(), String> {
    tokio::select! {
        changed = shutdown.changed() => {
            if changed.is_err() || *shutdown.borrow() { return Ok(()); }
        }
        _ = time::sleep(std::time::Duration::from_secs(2)) => {}
    }
    let mut interval = time::interval(state.config.status_interval);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { return Ok(()); }
            }
            _ = interval.tick() => {
                if let Err(error) = refresh_statuses(&state).await {
                    tracing::warn!(%error, "camera status refresh failed");
                }
            }
        }
    }
}

pub fn camera_path(id: Uuid, profile: &str) -> String {
    format!("cam_{}_{}", id.simple(), profile)
}

async fn emit_event_in(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    camera_id: Option<Uuid>,
    kind: &str,
    severity: &str,
    message: &str,
    details: Value,
    limits: crate::history::Limits,
) -> Result<EventRecord> {
    if message.len() > 8192
        || serde_json::to_vec(&details)
            .map_err(|_| crate::error::AppError::Internal("事件编码失败".into()))?
            .len()
            > 65536
    {
        return Err(crate::error::AppError::Validation(
            "事件超过大小限制".into(),
        ));
    }
    crate::history::reserve_with(transaction, camera_id, false, limits).await?;
    let now = chrono::Utc::now();
    let event = sqlx::query_as::<_, EventRecord>(
        "INSERT INTO events (id, camera_id, kind, severity, message, details, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?) \
         RETURNING id, camera_id, kind, severity, message, details, acknowledged_at, acknowledged_by, created_at",
    )
    .bind(Uuid::new_v4())
    .bind(camera_id)
    .bind(kind)
    .bind(severity)
    .bind(message)
    .bind(details)
    .bind(now)
    .fetch_one(&mut **transaction)
    .await?;
    Ok(event)
}

async fn refresh_statuses(state: &AppState) -> Result<()> {
    reconciliation::validate_stored_camera_credentials(state).await?;
    let observation_started_at = Utc::now();
    let paths = state.media.paths().await?;
    let cameras = sqlx::query_as::<_, CameraRecord>(CAMERA_SELECT)
        .fetch_all(&state.pool)
        .await?;
    for camera in cameras {
        let snapshot = paths.get(&camera_path(camera.id, "main"));
        let new_status = if !camera.enabled {
            "disabled"
        } else if snapshot.map(|path| path.ready).unwrap_or(false) {
            "online"
        } else {
            "offline"
        };
        if new_status == camera.status {
            if new_status == "online" {
                sqlx::query(
                    "UPDATE cameras SET last_seen_at = datetime('now') WHERE id = ? \
                    AND enabled = ? AND deleted_at IS NULL AND updated_at <= ?",
                )
                .bind(camera.id)
                .bind(camera.enabled)
                .bind(observation_started_at)
                .execute(&state.pool)
                .await?;
            }
            continue;
        }

        match persist_status_observation(
            &state.pool,
            &camera,
            new_status,
            snapshot,
            observation_started_at,
            crate::history::Limits::CURRENT,
        )
        .await
        {
            Ok(Some(event)) => {
                let _ = state.events.send(event);
            }
            Ok(None) => {}
            Err(crate::error::AppError::HistoryStorageCapacity) => {
                // Only this observation rolled back. A full camera must not
                // repeatedly starve status updates for the following cameras.
                tracing::warn!(camera_id=%camera.id, "camera observation rejected by history capacity");
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

async fn persist_status_observation(
    pool: &sqlx::SqlitePool,
    camera: &CameraRecord,
    new_status: &str,
    snapshot: Option<&crate::mediamtx::PathSnapshot>,
    observation_started_at: chrono::DateTime<Utc>,
    limits: crate::history::Limits,
) -> Result<Option<EventRecord>> {
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    let updated = sqlx::query(
            "UPDATE cameras SET status = ?, last_seen_at = CASE WHEN ? = 'online' THEN datetime('now') ELSE last_seen_at END, updated_at = datetime('now') \
             WHERE id = ? AND enabled = ? AND deleted_at IS NULL AND updated_at <= ?",
        )
        .bind(new_status)
        .bind(new_status)
        .bind(camera.id)
        .bind(camera.enabled)
        .bind(observation_started_at)
        .execute(&mut *transaction)
        .await?;

    if updated.rows_affected() != 1 {
        return Ok(None);
    }

    if camera.status != "pending" && new_status != "disabled" {
        let (severity, message) = if new_status == "online" {
            ("info", format!("{} 已恢复在线", camera.name))
        } else {
            ("warning", format!("{} 已离线", camera.name))
        };
        let event_result = emit_event_in(
            &mut transaction,
            Some(camera.id),
            &format!("camera.{new_status}"),
            severity,
            &message,
            json!({
                "previous_status": camera.status,
                "readers": snapshot.map(|path| path.readers).unwrap_or(0),
                "tracks": snapshot.map(|path| path.tracks).unwrap_or(0)
            }),
            limits,
        )
        .await;
        let event = match event_result {
            Ok(event) => event,
            Err(error) => {
                transaction.rollback().await?;
                return Err(error);
            }
        };
        transaction.commit().await?;
        return Ok(Some(event));
    } else {
        transaction.commit().await?;
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn full_camera_rolls_back_its_observation_while_another_camera_updates() {
        let directory = tempfile::tempdir().unwrap();
        let pool = crate::sqlite::initialize_test_pool(&format!(
            "sqlite://{}",
            directory.path().join("observations.sqlite3").display()
        ))
        .await
        .unwrap();
        let mut cameras = Vec::new();
        let now = Utc::now();
        for marker in [1u8, 2] {
            let id = Uuid::new_v4();
            sqlx::query("INSERT INTO xcocs(id,name,authorization_code_enc,authorization_code_hash,created_at,updated_at) VALUES(?,'test',?,?,?,?)")
                .bind(id).bind(vec![0u8;64]).bind(vec![marker;32]).bind(now).bind(now).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO cameras(id,name,client_id,client_camera_id,adapter_kind,status,created_at,updated_at) VALUES(?,'test',?,?,'rtsp','online',?,?)")
                .bind(id).bind(id).bind(id.to_string()).bind(now).bind(now).execute(&pool).await.unwrap();
            cameras.push(
                sqlx::query_as::<_, CameraRecord>(CAMERA_SELECT)
                    .fetch_all(&pool)
                    .await
                    .unwrap()
                    .into_iter()
                    .find(|camera| camera.id == id)
                    .unwrap(),
            );
        }
        sqlx::query("INSERT INTO events(id,camera_id,kind,severity,message,created_at) VALUES(?,?,'test','warning','retain',?)")
            .bind(Uuid::new_v4()).bind(cameras[0].id).bind(now).execute(&pool).await.unwrap();
        let limits = crate::history::Limits {
            rows: 100,
            owner_rows: 1,
            bytes: u64::MAX,
            owner_bytes: u64::MAX,
            free_floor: 0,
        };
        let mut rejected = 0;
        for camera in &cameras {
            match persist_status_observation(&pool, camera, "offline", None, Utc::now(), limits)
                .await
            {
                Err(crate::error::AppError::HistoryStorageCapacity) => rejected += 1,
                result => assert!(result.unwrap().is_some()),
            }
        }
        assert_eq!(rejected, 1);
        let status: String = sqlx::query_scalar("SELECT status FROM cameras WHERE id=?")
            .bind(cameras[0].id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            status, "online",
            "capacity rejection published the status without its alert"
        );
        let status: String = sqlx::query_scalar("SELECT status FROM cameras WHERE id=?")
            .bind(cameras[1].id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(status, "offline");
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM events")
                .fetch_one(&pool)
                .await
                .unwrap(),
            2
        );
    }
}
