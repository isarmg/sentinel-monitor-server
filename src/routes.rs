use crate::{
    auth::{decode_media_token, issue_media_token, CurrentUser},
    background::camera_path,
    error::{AppError, Result},
    models::*,
    protocol::CONTRACT,
    reconciliation, AppState,
};
use async_stream::stream;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{
        header::{
            ACCEPT_RANGES, AUTHORIZATION, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE,
            CONTENT_TYPE, RANGE,
        },
        HeaderMap, StatusCode,
    },
    response::{sse::Event, IntoResponse, Response, Sse},
    routing::{get, post, put},
    Json, Router,
};
use chrono::{DateTime, Utc};
use futures_util::Stream;
use rand::RngCore;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::{convert::Infallible, sync::Arc, time::Duration};
use tower_http::{compression::CompressionLayer, services::ServeDir, trace::TraceLayer};
use url::Url;
use uuid::Uuid;

const CAMERA_SELECT: &str = "SELECT id, name, location, source_kind, client_id, adapter_kind, manufacturer, model, firmware_version, serial_number, capabilities_json, streams_json, health_message, device_status, has_sub_stream, enabled, record_enabled, storage_mode, status, last_seen_at, created_at, updated_at FROM cameras";
const CLIENT_CAMERA_UPSERT: &str =
    "INSERT INTO cameras (id, name, location, source_kind, client_id, client_camera_id, \
     adapter_kind, manufacturer, model, firmware_version, serial_number, capabilities_json, \
     streams_json, health_message, device_status, has_sub_stream, enabled, record_enabled, storage_mode, \
     status, last_seen_at, created_at, updated_at) \
     VALUES (?1, ?2, ?3, 'client', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, 'pending', ?19, ?20, ?21) \
     ON CONFLICT(id) DO UPDATE SET name = excluded.name, location = excluded.location, \
     adapter_kind = excluded.adapter_kind, manufacturer = excluded.manufacturer, \
     model = excluded.model, firmware_version = excluded.firmware_version, \
     serial_number = excluded.serial_number, capabilities_json = excluded.capabilities_json, \
     streams_json = excluded.streams_json, health_message = excluded.health_message, \
     device_status = excluded.device_status, \
     has_sub_stream = excluded.has_sub_stream, enabled = excluded.enabled, \
     record_enabled = excluded.record_enabled, storage_mode = excluded.storage_mode, \
     status = CASE WHEN excluded.enabled THEN cameras.status ELSE 'disabled' END, \
     last_seen_at = excluded.last_seen_at, updated_at = excluded.updated_at, deleted_at = NULL \
     WHERE cameras.source_kind = 'client' AND cameras.client_id = excluded.client_id";

pub fn router(state: AppState, runtime: sarmg_server_runtime::RuntimeHandle) -> Result<Router> {
    let static_dir = state.config.static_dir.clone();
    let platform = sarmg_server_runtime::platform_router(
        runtime,
        "sentinel-monitor",
        state.administrator_origin,
        Arc::clone(&state.administrator),
    )
    .map_err(|error| AppError::Internal(error.to_string()))?;
    let api = Router::new()
        .route("/cameras", get(list_cameras))
        .route("/media/operations", get(list_media_operations))
        .route("/media/operations/{id}", get(media_operation))
        .route(
            "/media/operations/{id}/resolve",
            post(resolve_media_operation),
        )
        .route("/cameras/{id}/stream-ticket", get(stream_ticket))
        .route("/cameras/{id}/ptz", post(ptz))
        .route("/clients", get(list_clients).post(create_client))
        .route(
            "/clients/{id}/authorization",
            put(update_client_authorization),
        )
        .route("/clients/{id}", axum::routing::delete(revoke_client))
        .route("/client/pair", post(pair_client))
        .route("/client/snapshot", put(client_snapshot))
        .route("/recordings", get(list_recordings))
        .route("/recordings/play", get(play_recording))
        .route("/events", get(list_events))
        .route("/events/stream", get(event_stream))
        .route("/events/{id}/ack", post(ack_event))
        .route("/audit", get(list_audit))
        .route("/system/status", get(system_status));

    let product = Router::new()
        .route(&CONTRACT.media_auth_path, post(media_auth))
        .nest(&CONTRACT.api_prefix, api)
        .fallback_service(ServeDir::new(static_dir).append_index_html_on_directories(true))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state);
    Ok(Router::new().merge(platform).merge(product))
}

async fn list_cameras(
    _user: CurrentUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<CameraView>>> {
    let cameras = sqlx::query_as::<_, CameraRecord>(&format!(
        "{CAMERA_SELECT} WHERE deleted_at IS NULL ORDER BY name"
    ))
    .fetch_all(&state.pool)
    .await?;
    let views = cameras.iter().map(CameraView::from_record).collect();
    Ok(Json(views))
}

async fn media_operation(
    _user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<reconciliation::MediaOperationView>> {
    Ok(Json(reconciliation::get_operation(&state.pool, &id).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MediaOperationQuery {
    limit: Option<u32>,
    offset: Option<u32>,
}

async fn list_media_operations(
    _user: CurrentUser,
    State(state): State<AppState>,
    Query(query): Query<MediaOperationQuery>,
) -> Result<Json<Vec<reconciliation::MediaOperationView>>> {
    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    let offset = query.offset.unwrap_or(0).min(10_000);
    Ok(Json(
        reconciliation::list_operations(&state.pool, limit, offset).await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveMediaOperation {
    resolution: sarmg_operations::Resolution,
}

async fn resolve_media_operation(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ResolveMediaOperation>,
) -> Result<Json<reconciliation::MediaOperationView>> {
    let operation = reconciliation::get_operation(&state.pool, &id).await?;
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    sarmg_operations::SqliteOperationStore::resolve_in(
        &mut transaction,
        &id,
        request.resolution,
        Utc::now().timestamp_micros(),
    )
    .await
    .map_err(|error| match error {
        sarmg_operations::Error::InvalidTransition { .. }
        | sarmg_operations::Error::ConcurrentModification => {
            AppError::Conflict("媒体操作当前状态不允许此人工处理".into())
        }
        _ => AppError::Internal("媒体操作人工处理写入失败".into()),
    })?;
    write_audit_in(
        &mut transaction,
        Some(&user.id),
        "media.operation.resolve",
        "camera",
        Some(operation.camera_id),
        json!({ "operation_id": id, "resolution": request.resolution }),
    )
    .await?;
    transaction.commit().await?;
    Ok(Json(reconciliation::get_operation(&state.pool, &id).await?))
}

async fn list_clients(_user: CurrentUser, State(state): State<AppState>) -> Result<Response> {
    let rows = sqlx::query_as::<_, SentinelClientRecord>(
        "SELECT id, installation_id, name, client_version, authorization_code_enc, status, \
         last_seen_at, created_at, updated_at \
         FROM sentinel_clients ORDER BY name, created_at",
    )
    .fetch_all(&state.pool)
    .await?;
    let views = rows
        .into_iter()
        .map(|record| client_view(&state, record))
        .collect::<Result<Vec<_>>>()?;
    Ok(([("cache-control", "no-store")], Json(views)).into_response())
}

async fn create_client(
    user: CurrentUser,
    State(state): State<AppState>,
    Json(request): Json<CreateSentinelClientRequest>,
) -> Result<Response> {
    let name = validate_client_name(&request.name)?;
    let id = Uuid::new_v4();
    let code = random_authorization_code();
    let encoded = state
        .secrets
        .encrypt_client_authorization(&id.to_string(), &code)?;
    let now = Utc::now();
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query(
        "INSERT INTO sentinel_clients (id, name, authorization_code_enc, authorization_code_hash, \
         created_by, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(&name)
    .bind(encoded)
    .bind(hash_secret(&code).to_vec())
    .bind(&user.id)
    .bind(now)
    .bind(now)
    .execute(&mut *transaction)
    .await?;
    write_audit_in(
        &mut transaction,
        Some(&user.id),
        "client.create",
        "client",
        Some(id),
        json!({ "name": name }),
    )
    .await?;
    transaction.commit().await?;
    Ok((
        StatusCode::CREATED,
        [("cache-control", "no-store")],
        Json(SentinelClientView {
            id,
            installation_id: None,
            name,
            client_version: None,
            authorization_code: code,
            status: "pending".into(),
            last_seen_at: None,
            created_at: now,
            updated_at: now,
        }),
    )
        .into_response())
}

async fn update_client_authorization(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateAuthorizationCodeRequest>,
) -> Result<Response> {
    validate_authorization_code(&request.authorization_code)?;
    let encoded = state
        .secrets
        .encrypt_client_authorization(&id.to_string(), &request.authorization_code)?;
    let now = Utc::now();
    let mut transaction = state.pool.begin().await?;
    let changed = sqlx::query(
        "UPDATE sentinel_clients SET authorization_code_enc = ?, authorization_code_hash = ?, \
         installation_id = NULL, client_version = NULL, token_hash = NULL, status = 'pending', \
         last_seen_at = NULL, updated_at = ? WHERE id = ? AND revoked_at IS NULL",
    )
    .bind(encoded)
    .bind(hash_secret(&request.authorization_code).to_vec())
    .bind(now)
    .bind(id)
    .execute(&mut *transaction)
    .await?;
    if changed.rows_affected() != 1 {
        return Err(AppError::NotFound("客户端实例不存在".into()));
    }
    sqlx::query(
        "UPDATE device_commands SET status = 'expired', finished_at = ? \
         WHERE client_id = ? AND status = 'pending'",
    )
    .bind(now)
    .bind(id)
    .execute(&mut *transaction)
    .await?;
    let cameras = sqlx::query_as::<_, CameraRecord>(&format!(
        "{CAMERA_SELECT} WHERE client_id = ? AND deleted_at IS NULL"
    ))
    .bind(id)
    .fetch_all(&mut *transaction)
    .await?;
    for camera in cameras {
        sqlx::query(
            "UPDATE cameras SET enabled = 0, status = 'disabled', updated_at = ? WHERE id = ?",
        )
        .bind(now)
        .bind(camera.id)
        .execute(&mut *transaction)
        .await?;
        reconciliation::queue_camera_change(
            &mut transaction,
            &camera,
            false,
            &user.id,
            "client_authorization_rotated",
        )
        .await?;
    }
    write_audit_in(
        &mut transaction,
        Some(&user.id),
        "client.authorization.rotate",
        "client",
        Some(id),
        json!({}),
    )
    .await?;
    transaction.commit().await?;
    let record = sqlx::query_as::<_, SentinelClientRecord>(
        "SELECT id, installation_id, name, client_version, authorization_code_enc, status, \
         last_seen_at, created_at, updated_at FROM sentinel_clients WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await?;
    Ok((
        [("cache-control", "no-store")],
        Json(client_view(&state, record)?),
    )
        .into_response())
}

fn client_view(state: &AppState, record: SentinelClientRecord) -> Result<SentinelClientView> {
    let status = if record.status == "online"
        && record
            .last_seen_at
            .is_none_or(|last_seen| last_seen <= Utc::now() - chrono::Duration::seconds(30))
    {
        "offline".to_owned()
    } else {
        record.status
    };
    Ok(SentinelClientView {
        id: record.id,
        installation_id: record.installation_id,
        name: record.name,
        client_version: record.client_version,
        authorization_code: state
            .secrets
            .decrypt_client_authorization(&record.id.to_string(), &record.authorization_code_enc)?,
        status,
        last_seen_at: record.last_seen_at,
        created_at: record.created_at,
        updated_at: record.updated_at,
    })
}

async fn pair_client(
    State(state): State<AppState>,
    Json(request): Json<PairClientRequest>,
) -> Result<Response> {
    if request.protocol != CLIENT_PAIRING_PROTOCOL || request.product != "sentinel-monitor" {
        return Err(AppError::Validation("客户端配对协议或产品不受支持".into()));
    }
    if request.installation_id.is_nil()
        || validate_authorization_code(&request.authorization_code).is_err()
        || request.client_version.is_empty()
        || request.client_version.len() > 64
        || request.client_version.chars().any(char::is_control)
    {
        return Err(AppError::Validation("客户端配对输入无效".into()));
    }
    validate_client_name(&request.name)?;
    let code_hash = hash_secret(&request.authorization_code).to_vec();
    let access_token = sarmg_admin_auth::random_token()
        .map_err(|_| AppError::Internal("client token generation failed".into()))?;
    let now = Utc::now();
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let (client_id, owner) = sqlx::query_as::<_, (Uuid, Option<String>)>(
        "SELECT id, created_by FROM sentinel_clients WHERE authorization_code_hash = ? \
         AND token_hash IS NULL AND revoked_at IS NULL",
    )
    .bind(code_hash)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(AppError::Unauthorized)?;
    let changed = sqlx::query(
        "UPDATE sentinel_clients SET installation_id = ?, client_version = ?, token_hash = ?, \
         status = 'online', last_seen_at = ?, updated_at = ? WHERE id = ? AND token_hash IS NULL",
    )
    .bind(request.installation_id)
    .bind(&request.client_version)
    .bind(hash_secret(&access_token).to_vec())
    .bind(now)
    .bind(now)
    .bind(client_id)
    .execute(&mut *transaction)
    .await?;
    if changed.rows_affected() != 1 {
        return Err(AppError::Conflict("客户端实例已经配对".into()));
    }
    write_audit_in(
        &mut transaction,
        owner.as_deref(),
        "client.pair",
        "client",
        Some(client_id),
        json!({ "installation_id": request.installation_id }),
    )
    .await?;
    transaction.commit().await?;
    Ok((
        StatusCode::CREATED,
        [("cache-control", "no-store")],
        Json(PairClientResponse {
            protocol: CLIENT_PAIRING_PROTOCOL,
            client_id,
            access_token,
        }),
    )
        .into_response())
}

async fn revoke_client(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode> {
    let now = Utc::now();
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM sentinel_clients WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *transaction)
            .await?;
    let Some(status) = status else {
        return Err(AppError::NotFound("客户端不存在".into()));
    };
    if status == "revoked" {
        purge_revoked_client_in(&mut transaction, id).await?;
        write_audit_in(
            &mut transaction,
            Some(&user.id),
            "client.delete",
            "client",
            Some(id),
            json!({}),
        )
        .await?;
        transaction.commit().await?;
        return Ok(StatusCode::NO_CONTENT);
    }
    let changed = sqlx::query(
        "UPDATE sentinel_clients SET status = 'revoked', revoked_at = ?, updated_at = ? \
         WHERE id = ? AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(now)
    .bind(id)
    .execute(&mut *transaction)
    .await?;
    if changed.rows_affected() == 0 {
        return Err(AppError::NotFound("客户端不存在".into()));
    }
    sqlx::query(
        "UPDATE device_commands SET status = 'expired', finished_at = ? \
         WHERE client_id = ? AND status = 'pending'",
    )
    .bind(now)
    .bind(id)
    .execute(&mut *transaction)
    .await?;
    let cameras = sqlx::query_as::<_, CameraRecord>(&format!(
        "{CAMERA_SELECT} WHERE client_id = ? AND deleted_at IS NULL"
    ))
    .bind(id)
    .fetch_all(&mut *transaction)
    .await?;
    for camera in cameras {
        sqlx::query(
            "UPDATE cameras SET enabled = 0, status = 'disabled', deleted_at = ?, updated_at = ? WHERE id = ?",
        )
        .bind(now)
        .bind(now)
        .bind(camera.id)
        .execute(&mut *transaction)
        .await?;
        reconciliation::queue_camera_change(
            &mut transaction,
            &camera,
            false,
            &user.id,
            "client_revoked",
        )
        .await?;
    }
    write_audit_in(
        &mut transaction,
        Some(&user.id),
        "client.revoke",
        "client",
        Some(id),
        json!({}),
    )
    .await?;
    transaction.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn purge_revoked_client_in(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    client_id: Uuid,
) -> Result<()> {
    let camera_ids = sqlx::query_scalar::<_, Uuid>("SELECT id FROM cameras WHERE client_id = ?")
        .bind(client_id)
        .fetch_all(&mut **transaction)
        .await?;
    for camera_id in &camera_ids {
        let desired = sqlx::query_as::<_, (i64, bool)>(
            "SELECT generation, desired_present FROM media_desired_states WHERE camera_id = ?",
        )
        .bind(camera_id)
        .fetch_optional(&mut **transaction)
        .await?;
        let Some((generation, false)) = desired else {
            return Err(AppError::Conflict(
                "摄像机媒体删除期望态不完整，不能永久删除".into(),
            ));
        };
        let operation = sqlx::query_as::<_, (String, Option<String>, Vec<u8>)>(
            "SELECT state, resolution_code, request_payload FROM _sarmg_operations \
             WHERE namespace = ? AND target_key = ? ORDER BY created_at_micros DESC LIMIT 1",
        )
        .bind(reconciliation::OPERATION_NAMESPACE)
        .bind(camera_id.hyphenated().to_string())
        .fetch_optional(&mut **transaction)
        .await?;
        let removal_confirmed = operation.is_some_and(|(state, resolution, payload)| {
            reconciliation::removal_operation_matches(&payload, *camera_id, generation)
                && (state == "succeeded"
                    || (state == "resolved"
                        && resolution.as_deref() == Some("confirmed_succeeded")))
        });
        if !removal_confirmed {
            return Err(AppError::Conflict(
                "摄像机媒体清理尚未确认完成，请稍后重试删除".into(),
            ));
        }
        let still_present: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM media_actual_paths WHERE camera_id = ? AND present = 1)",
        )
        .bind(camera_id)
        .fetch_one(&mut **transaction)
        .await?;
        if still_present {
            return Err(AppError::Conflict(
                "摄像机媒体路径仍然存在，请稍后重试删除".into(),
            ));
        }
    }
    for camera_id in &camera_ids {
        sqlx::query("DELETE FROM media_actual_paths WHERE camera_id = ?")
            .bind(camera_id)
            .execute(&mut **transaction)
            .await?;
        sqlx::query("DELETE FROM media_desired_states WHERE camera_id = ?")
            .bind(camera_id)
            .execute(&mut **transaction)
            .await?;
    }
    sqlx::query("DELETE FROM cameras WHERE client_id = ?")
        .bind(client_id)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("DELETE FROM sentinel_clients WHERE id = ?")
        .bind(client_id)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn client_snapshot(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ClientSnapshotRequest>,
) -> Result<Response> {
    if request.protocol != CLIENT_PAIRING_PROTOCOL
        || request.cameras.len() != 1
        || request.command_results.len() > 100
    {
        return Err(AppError::Validation(
            "每个授权实例必须且只能上报一台摄像机".into(),
        ));
    }
    let token_hash = client_token_hash(&headers)?;
    let mut ids = HashSet::with_capacity(request.cameras.len());
    for camera in &request.cameras {
        validate_client_camera(camera)?;
        if !ids.insert(camera.id) {
            return Err(AppError::Validation("客户端快照包含重复摄像头".into()));
        }
    }
    for result in &request.command_results {
        validate_command_result(result)?;
    }

    let now = Utc::now();
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let client_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM sentinel_clients WHERE token_hash = ? AND revoked_at IS NULL",
    )
    .bind(token_hash.clone())
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(AppError::Unauthorized)?;
    if request.cameras.iter().any(|camera| camera.id != client_id) {
        return Err(AppError::Validation(
            "摄像机身份必须与授权实例身份一致".into(),
        ));
    }
    let authenticated = sqlx::query(
        "UPDATE sentinel_clients SET status = 'online', last_seen_at = ?, updated_at = ? \
         WHERE id = ? AND token_hash = ? AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(now)
    .bind(client_id)
    .bind(token_hash.clone())
    .execute(&mut *transaction)
    .await?;
    if authenticated.rows_affected() != 1 {
        return Err(AppError::Unauthorized);
    }

    sqlx::query(
        "UPDATE device_commands SET status = 'expired', finished_at = ? \
         WHERE client_id = ? AND status = 'pending' AND expires_at <= ?",
    )
    .bind(now)
    .bind(client_id)
    .bind(now)
    .execute(&mut *transaction)
    .await?;
    for result in &request.command_results {
        sqlx::query(
            "UPDATE device_commands SET status = ?, error = ?, finished_at = ? \
             WHERE id = ? AND client_id = ? AND status = 'pending' AND expires_at > ?",
        )
        .bind(&result.status)
        .bind(result.error.as_deref())
        .bind(now)
        .bind(result.id)
        .bind(client_id)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
    }
    sqlx::query(
        "DELETE FROM device_commands WHERE client_id = ? AND status != 'pending' \
         AND finished_at < ?",
    )
    .bind(client_id)
    .bind(now - chrono::Duration::days(1))
    .execute(&mut *transaction)
    .await?;

    for camera in &request.cameras {
        // Ownership is immutable even while a camera is soft-deleted. Excluding
        // tombstones here would let another client revive and mutate an ID that
        // it does not own through the UPSERT below.
        let existing = sqlx::query_as::<_, CameraRecord>(&format!("{CAMERA_SELECT} WHERE id = ?"))
            .bind(camera.id)
            .fetch_optional(&mut *transaction)
            .await?;
        if existing
            .as_ref()
            .is_some_and(|value| value.client_id != Some(client_id))
        {
            return Err(AppError::Conflict("摄像头身份已由另一个来源使用".into()));
        }
        let record_enabled = camera.storage_mode == "server";
        let capabilities_json = serde_json::to_string(&camera.capabilities)
            .map_err(|_| AppError::Validation("客户端能力模型无效".into()))?;
        let streams_json = serde_json::to_string(&camera.streams)
            .map_err(|_| AppError::Validation("客户端码流模型无效".into()))?;
        let media_changed = existing.as_ref().is_none_or(|value| {
            value.has_sub_stream != camera.has_sub_stream
                || value.enabled != camera.enabled
                || value.storage_mode != camera.storage_mode
        });
        let written = sqlx::query(CLIENT_CAMERA_UPSERT)
            .bind(camera.id)
            .bind(camera.name.trim())
            .bind(camera.location.trim())
            .bind(client_id)
            .bind(camera.id)
            .bind(&camera.adapter_kind)
            .bind(camera.identity.manufacturer.as_deref())
            .bind(camera.identity.model.as_deref())
            .bind(camera.identity.firmware_version.as_deref())
            .bind(camera.identity.serial_number.as_deref())
            .bind(capabilities_json)
            .bind(streams_json)
            .bind(camera.health_message.as_deref())
            .bind(&camera.status)
            .bind(camera.has_sub_stream)
            .bind(camera.enabled)
            .bind(record_enabled)
            .bind(&camera.storage_mode)
            .bind(now)
            .bind(now)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
        if written.rows_affected() != 1 {
            return Err(AppError::Conflict("摄像头身份已由另一个来源使用".into()));
        }
        if media_changed {
            let current =
                sqlx::query_as::<_, CameraRecord>(&format!("{CAMERA_SELECT} WHERE id = ?"))
                    .bind(camera.id)
                    .fetch_one(&mut *transaction)
                    .await?;
            reconciliation::queue_camera_change(
                &mut transaction,
                &current,
                current.enabled,
                &client_id.to_string(),
                "client_camera_snapshot",
            )
            .await?;
        }
    }

    let existing = sqlx::query_as::<_, CameraRecord>(&format!(
        "{CAMERA_SELECT} WHERE client_id = ? AND deleted_at IS NULL"
    ))
    .bind(client_id)
    .fetch_all(&mut *transaction)
    .await?;
    for camera in existing {
        if !ids.contains(&camera.id) {
            sqlx::query(
                "UPDATE cameras SET deleted_at = ?, enabled = 0, status = 'disabled', updated_at = ? \
                 WHERE id = ?",
            )
            .bind(now)
            .bind(now)
            .bind(camera.id)
            .execute(&mut *transaction)
            .await?;
            reconciliation::queue_camera_change(
                &mut transaction,
                &camera,
                false,
                &client_id.to_string(),
                "client_camera_removed",
            )
            .await?;
        }
    }
    let mut publish = Vec::new();
    for camera in request
        .cameras
        .iter()
        .filter(|camera| camera.enabled && camera.status == "online")
    {
        publish.push(CameraPublishGrant {
            camera_id: camera.id,
            profile: "main".into(),
            publish_url: client_publish_url(&state, client_id, camera.id, "main", &token_hash)?,
        });
        if camera.has_sub_stream {
            publish.push(CameraPublishGrant {
                camera_id: camera.id,
                profile: "sub".into(),
                publish_url: client_publish_url(&state, client_id, camera.id, "sub", &token_hash)?,
            });
        }
    }
    let retry_before = now - chrono::Duration::seconds(5);
    let command_rows = sqlx::query_as::<_, (Uuid, Uuid, String, String, DateTime<Utc>)>(
        "SELECT id, camera_id, kind, payload, expires_at FROM device_commands \
         WHERE client_id = ? AND status = 'pending' AND expires_at > ? \
         AND (delivered_at IS NULL OR delivered_at <= ?) ORDER BY created_at LIMIT 100",
    )
    .bind(client_id)
    .bind(now)
    .bind(retry_before)
    .fetch_all(&mut *transaction)
    .await?;
    let mut commands = Vec::with_capacity(command_rows.len());
    for (id, camera_id, kind, payload, expires_at) in command_rows {
        commands.push(DeviceCommand {
            id,
            camera_id,
            kind,
            payload: serde_json::from_str(&payload)
                .map_err(|_| AppError::Internal("stored device command is invalid".into()))?,
            expires_at,
        });
        sqlx::query("UPDATE device_commands SET delivered_at = ? WHERE id = ?")
            .bind(now)
            .bind(id)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    Ok((
        [("cache-control", "no-store")],
        Json(ClientSnapshotResponse {
            protocol: CLIENT_PAIRING_PROTOCOL,
            accepted_at: now,
            publish,
            commands,
        }),
    )
        .into_response())
}

fn client_token_hash(headers: &HeaderMap) -> Result<Vec<u8>> {
    let header = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| value.len() == 43)
        .ok_or(AppError::Unauthorized)?;
    Ok(hash_secret(header).to_vec())
}

fn validate_client_name(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 64 || value.chars().any(char::is_control) {
        return Err(AppError::Validation(
            "客户端名称须为 1–64 个字符，不能包含控制字符".into(),
        ));
    }
    Ok(value.to_owned())
}

fn validate_client_camera(camera: &ClientCameraSnapshot) -> Result<()> {
    validate_client_name(&camera.name)?;
    if camera.id.is_nil()
        || camera.location.chars().count() > 128
        || camera.location.chars().any(char::is_control)
        || !valid_adapter_kind(&camera.adapter_kind)
        || !valid_optional_device_text(camera.identity.manufacturer.as_deref(), 128)
        || !valid_optional_device_text(camera.identity.model.as_deref(), 128)
        || !valid_optional_device_text(camera.identity.firmware_version.as_deref(), 128)
        || !valid_optional_device_text(camera.identity.serial_number.as_deref(), 256)
        || !valid_optional_device_text(camera.health_message.as_deref(), 512)
        || !matches!(camera.storage_mode.as_str(), "client" | "server")
        || !matches!(
            camera.status.as_str(),
            "pending" | "online" | "offline" | "disabled" | "error"
        )
    {
        return Err(AppError::Validation("客户端摄像头快照无效".into()));
    }
    if !camera.capabilities.video
        || !camera.capabilities.main_stream
        || camera.capabilities.sub_stream != camera.has_sub_stream
        || !camera.capabilities.local_recording
        || !camera.capabilities.server_recording
        || camera.streams.len() > 2
    {
        return Err(AppError::Validation("客户端摄像头能力无效".into()));
    }
    let mut profiles = HashSet::new();
    for stream in &camera.streams {
        if !matches!(stream.profile.as_str(), "main" | "sub")
            || !profiles.insert(stream.profile.as_str())
            || !valid_optional_device_text(stream.video_codec.as_deref(), 64)
            || !valid_optional_device_text(stream.audio_codec.as_deref(), 64)
            || stream
                .width
                .is_some_and(|value| value == 0 || value > 32768)
            || stream
                .height
                .is_some_and(|value| value == 0 || value > 32768)
            || stream
                .frame_rate
                .is_some_and(|value| !value.is_finite() || value <= 0.0 || value > 240.0)
        {
            return Err(AppError::Validation("客户端摄像头码流无效".into()));
        }
    }
    if camera.status == "online"
        && (!profiles.contains("main") || profiles.contains("sub") != camera.has_sub_stream)
    {
        return Err(AppError::Validation("在线摄像头码流能力不完整".into()));
    }
    Ok(())
}

fn validate_command_result(result: &DeviceCommandResult) -> Result<()> {
    if result.id.is_nil()
        || !matches!(result.status.as_str(), "succeeded" | "failed")
        || !valid_optional_device_text(result.error.as_deref(), 512)
        || (result.status == "succeeded" && result.error.is_some())
        || (result.status == "failed" && result.error.is_none())
    {
        return Err(AppError::Validation("客户端命令结果无效".into()));
    }
    Ok(())
}

fn valid_adapter_kind(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
}

fn valid_optional_device_text(value: Option<&str>, max: usize) -> bool {
    value.is_none_or(|value| {
        !value.is_empty() && value.chars().count() <= max && !value.chars().any(char::is_control)
    })
}

fn client_publish_url(
    state: &AppState,
    client_id: Uuid,
    camera_id: Uuid,
    profile: &str,
    token_hash: &[u8],
) -> Result<String> {
    let mut url = Url::parse(&format!(
        "{}/{}",
        state.config.public_rtsp_publish_base_url,
        camera_path(camera_id, profile)
    ))
    .map_err(|_| AppError::Internal("public RTSP publish URL is invalid".into()))?;
    let (token, _) = issue_media_token(
        &client_id.to_string(),
        camera_id,
        camera_path(camera_id, profile),
        vec!["publish".into()],
        Some(crate::auth::publish_binding(token_hash)),
        &state.config,
    )?;
    url.query_pairs_mut().append_pair("jwt", &token);
    Ok(url.into())
}

fn hash_secret(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}

fn random_authorization_code() -> String {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn validate_authorization_code(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(AppError::Validation(
            "授权码必须是 64 个小写十六进制字符".into(),
        ));
    }
    Ok(())
}

async fn stream_ticket(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<StreamTicketQuery>,
) -> Result<Json<StreamTicket>> {
    let camera = load_camera(&state, id).await?;
    if !camera.enabled {
        return Err(AppError::Conflict("摄像头已停用".into()));
    }
    let profile = query.profile.as_deref().unwrap_or("main");
    if profile != "main" && profile != "sub" {
        return Err(AppError::Validation("profile只能是main或sub".into()));
    }
    if profile == "sub" && !camera.has_sub_stream {
        return Err(AppError::Validation("该摄像头没有配置子码流".into()));
    }
    let path = camera_path(id, profile);
    let (token, expires_at) = issue_media_token(
        &user.id,
        id,
        path.clone(),
        vec!["read".into()],
        None,
        &state.config,
    )?;
    Ok(Json(StreamTicket {
        profile: profile.to_string(),
        whep_url: format!("{}/{}/whep", state.config.public_webrtc_base_url, path),
        hls_url: format!("{}/{}/index.m3u8", state.config.public_hls_base_url, path),
        token,
        expires_at,
    }))
}

async fn ptz(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<PtzRequest>,
) -> Result<StatusCode> {
    if request.action != "move" && request.action != "stop" {
        return Err(AppError::Validation("PTZ action只能是move或stop".into()));
    }
    let values = (
        request.pan.unwrap_or(0.0),
        request.tilt.unwrap_or(0.0),
        request.zoom.unwrap_or(0.0),
    );
    if [values.0, values.1, values.2]
        .iter()
        .any(|value| !(-1.0..=1.0).contains(value))
    {
        return Err(AppError::Validation("PTZ速度必须在-1到1之间".into()));
    }
    let camera = load_camera(&state, id).await?;
    let capabilities: DeviceCapabilities = serde_json::from_str(&camera.capabilities_json)
        .map_err(|_| AppError::Internal("stored camera capabilities are invalid".into()))?;
    if !capabilities.ptz {
        return Err(AppError::Validation("摄像头不支持PTZ".into()));
    }
    let client_id = camera
        .client_id
        .ok_or_else(|| AppError::Internal("client camera has no owner".into()))?;
    let command_id = Uuid::new_v4();
    let now = Utc::now();
    let client_last_seen = sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
        "SELECT last_seen_at FROM sentinel_clients \
         WHERE id = ? AND token_hash IS NOT NULL AND revoked_at IS NULL",
    )
    .bind(client_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::Conflict("客户端授权当前无效".into()))?;
    if request.action == "move" {
        let freshness = i64::try_from(
            state
                .config
                .status_interval
                .as_secs()
                .saturating_mul(3)
                .max(30),
        )
        .unwrap_or(i64::MAX);
        if !camera.enabled
            || camera.device_status != "online"
            || client_last_seen
                .is_none_or(|seen| seen <= now - chrono::Duration::seconds(freshness))
        {
            return Err(AppError::Conflict("摄像头或客户端当前不在线".into()));
        }
    }
    let expires_at = now + chrono::Duration::seconds(10);
    sqlx::query(
        "INSERT INTO device_commands \
         (id, camera_id, client_id, kind, payload, created_at, expires_at) \
         VALUES (?, ?, ?, 'ptz', ?, ?, ?)",
    )
    .bind(command_id)
    .bind(camera.id)
    .bind(client_id)
    .bind(
        serde_json::to_string(&json!({
            "action": request.action,
            "pan": values.0,
            "tilt": values.1,
            "zoom": values.2,
        }))
        .map_err(|_| AppError::Internal("PTZ command serialization failed".into()))?,
    )
    .bind(now)
    .bind(expires_at)
    .execute(&state.pool)
    .await?;
    write_audit(
        &state,
        Some(&user.id),
        "camera.ptz.queued",
        "camera",
        Some(id),
        json!({ "command_id": command_id, "action": request.action, "pan": values.0, "tilt": values.1, "zoom": values.2 }),
    )
    .await;
    Ok(StatusCode::ACCEPTED)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordingQuery {
    camera_id: Uuid,
    profile: Option<String>,
    start: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
}

async fn list_recordings(
    user: CurrentUser,
    State(state): State<AppState>,
    Query(query): Query<RecordingQuery>,
) -> Result<Json<Vec<crate::mediamtx::RecordingSpan>>> {
    let camera = load_camera(&state, query.camera_id).await?;
    if camera.storage_mode == "client" {
        return Err(AppError::Conflict(
            "该摄像头的录像保存在客户端本地，服务器仅提供实时画面".into(),
        ));
    }
    let profile = query.profile.as_deref().unwrap_or("main");
    if profile == "sub" && !camera.has_sub_stream {
        return Err(AppError::Validation("该摄像头没有配置子码流".into()));
    }
    if profile != "main" && profile != "sub" {
        return Err(AppError::Validation("profile只能是main或sub".into()));
    }
    let path = camera_path(camera.id, profile);
    let (token, _) = issue_media_token(
        &user.id,
        camera.id,
        path.clone(),
        vec!["playback".into()],
        None,
        &state.config,
    )?;
    Ok(Json(
        state
            .media
            .recordings(&path, query.start, query.end, &token)
            .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlayRecordingQuery {
    camera_id: Uuid,
    start: DateTime<Utc>,
    duration: f64,
    format: Option<String>,
}

async fn play_recording(
    user: CurrentUser,
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PlayRecordingQuery>,
) -> Result<Response<Body>> {
    if !(0.1..=21_600.0).contains(&query.duration) {
        return Err(AppError::Validation(
            "单次回放时长必须在0.1秒到6小时之间".into(),
        ));
    }
    let format = query.format.as_deref().unwrap_or("mp4");
    if format != "mp4" && format != "fmp4" {
        return Err(AppError::Validation("回放格式只能是mp4或fmp4".into()));
    }
    let camera = load_camera(&state, query.camera_id).await?;
    if camera.storage_mode == "client" {
        return Err(AppError::Conflict(
            "该摄像头的录像保存在客户端本地，服务器仅提供实时画面".into(),
        ));
    }
    let path = camera_path(camera.id, "main");
    let (token, _) = issue_media_token(
        &user.id,
        camera.id,
        path.clone(),
        vec!["playback".into()],
        None,
        &state.config,
    )?;
    let range = headers.get(RANGE).and_then(|value| value.to_str().ok());
    let response = state
        .media
        .recording_stream(&path, query.start, query.duration, format, &token, range)
        .await?;
    if !response.status().is_success()
        && response.status() != reqwest::StatusCode::RANGE_NOT_SATISFIABLE
    {
        return Err(AppError::Upstream(format!(
            "recording playback returned {}",
            response.status()
        )));
    }
    let status = response.status();
    let upstream_headers = response.headers().clone();
    let mut builder = Response::builder().status(status);
    for name in [
        CONTENT_TYPE,
        CONTENT_LENGTH,
        CONTENT_RANGE,
        ACCEPT_RANGES,
        CONTENT_DISPOSITION,
    ] {
        if let Some(value) = upstream_headers.get(&name) {
            builder = builder.header(name, value);
        }
    }
    builder
        .body(Body::from_stream(response.bytes_stream()))
        .map_err(|error| AppError::Internal(format!("playback response failed: {error}")))
}

async fn list_events(
    _user: CurrentUser,
    State(state): State<AppState>,
    Query(query): Query<EventQuery>,
) -> Result<Json<Vec<EventRecord>>> {
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let mut builder = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
        "SELECT id, camera_id, kind, severity, message, details, acknowledged_at, acknowledged_by, created_at FROM events WHERE 1 = 1",
    );
    if let Some(camera_id) = query.camera_id {
        builder.push(" AND camera_id = ").push_bind(camera_id);
    }
    if query.unacknowledged.unwrap_or(false) {
        builder.push(" AND acknowledged_at IS NULL");
    }
    builder
        .push(" ORDER BY created_at DESC LIMIT ")
        .push_bind(limit);
    let events = builder
        .build_query_as::<EventRecord>()
        .fetch_all(&state.pool)
        .await?;
    Ok(Json(events))
}

async fn ack_event(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode> {
    let acknowledged_at = Utc::now();
    let result =
        sqlx::query("UPDATE events SET acknowledged_at = ?, acknowledged_by = ? WHERE id = ?")
            .bind(acknowledged_at)
            .bind(user.id.to_string())
            .bind(id)
            .execute(&state.pool)
            .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("事件不存在".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn event_stream(
    _user: CurrentUser,
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = std::result::Result<Event, Infallible>>> {
    let mut receiver = state.events.subscribe();
    let output = stream! {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    if let Ok(payload) = Event::default().event("system-event").json_data(event) {
                        yield Ok(payload);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    // Broadcast is only a notification channel; SQLite remains
                    // authoritative. A lagged client must perform a full query
                    // before subscribing again, never silently skip facts.
                    yield Ok(Event::default()
                        .event("resync-required")
                        .data(format!("{{\"skipped\":{skipped}}}")));
                    break;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(output).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuditQuery {
    limit: Option<i64>,
}

async fn list_audit(
    _user: CurrentUser,
    State(state): State<AppState>,
    Query(query): Query<AuditQuery>,
) -> Result<Json<Vec<AuditRecord>>> {
    let rows = sqlx::query_as::<_, AuditRecord>(
        "SELECT id, user_id, action, entity_type, entity_id, details, created_at FROM audit_logs ORDER BY created_at DESC LIMIT ?",
    )
    .bind(query.limit.unwrap_or(100).clamp(1, 500))
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows))
}

async fn system_status(_user: CurrentUser, State(state): State<AppState>) -> Result<Json<Value>> {
    reconciliation::validate_stored_camera_credentials(&state).await?;
    let recording: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM cameras \
         WHERE deleted_at IS NULL AND record_enabled AND enabled",
    )
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(json!({
        "service": "sentinel-monitor",
        "version": env!("CARGO_PKG_VERSION"),
        "database": "ok",
        "media_service": if state.media.health().await { "ok" } else { "unavailable" },
        "cameras": { "recording_configured": recording },
        "server_time": Utc::now()
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MediaAuthRequest {
    user: String,
    password: String,
    token: String,
    ip: String,
    action: String,
    path: String,
    protocol: String,
    id: String,
    query: String,
    #[serde(rename = "userAgent")]
    user_agent: String,
}

async fn media_auth(
    State(state): State<AppState>,
    Json(request): Json<MediaAuthRequest>,
) -> StatusCode {
    if [
        &request.user,
        &request.password,
        &request.token,
        &request.ip,
        &request.action,
        &request.path,
        &request.protocol,
        &request.id,
        &request.query,
        &request.user_agent,
    ]
    .iter()
    .any(|value| value.len() > 4_096)
    {
        return StatusCode::BAD_REQUEST;
    }
    if let Ok(claims) = decode_media_token(&request.token, &state.config) {
        if claims.path != request.path
            || !claims
                .actions
                .iter()
                .any(|action| action == &request.action)
        {
            return StatusCode::FORBIDDEN;
        }
        if request.action == "publish" {
            let current = sqlx::query_scalar::<_, Vec<u8>>(
                "SELECT sc.token_hash FROM cameras c \
                 JOIN sentinel_clients sc ON sc.id = c.client_id \
                 WHERE c.id = ? AND c.deleted_at IS NULL AND c.enabled = 1 \
                 AND sc.revoked_at IS NULL AND sc.token_hash IS NOT NULL",
            )
            .bind(claims.camera_id)
            .fetch_optional(&state.pool)
            .await;
            let valid = current.ok().flatten().is_some_and(|token_hash| {
                claims.credential_binding.as_deref()
                    == Some(crate::auth::publish_binding(&token_hash).as_str())
            });
            if !valid {
                return StatusCode::FORBIDDEN;
            }
        }
        return StatusCode::OK;
    }
    StatusCode::UNAUTHORIZED
}

async fn load_camera(state: &AppState, id: Uuid) -> Result<CameraRecord> {
    sqlx::query_as::<_, CameraRecord>(&format!(
        "{CAMERA_SELECT} WHERE id = ? AND deleted_at IS NULL"
    ))
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("摄像头不存在".into()))
}

async fn write_audit(
    state: &AppState,
    user_id: Option<&str>,
    action: &str,
    entity_type: &str,
    entity_id: Option<Uuid>,
    details: Value,
) {
    let now = Utc::now();
    if let Err(error) = sqlx::query(
        "INSERT INTO audit_logs (id, user_id, action, entity_type, entity_id, details, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(action)
    .bind(entity_type)
    .bind(entity_id)
    .bind(details)
    .bind(now)
    .execute(&state.pool)
    .await
    {
        tracing::warn!(%error, "audit log write failed");
    }
}

async fn write_audit_in(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    user_id: Option<&str>,
    action: &str,
    entity_type: &str,
    entity_id: Option<Uuid>,
    details: Value,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO audit_logs (id, user_id, action, entity_type, entity_id, details, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(action)
    .bind(entity_type)
    .bind(entity_id)
    .bind(details)
    .bind(Utc::now())
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

#[cfg(test)]
mod request_contract_tests {
    use super::*;

    async fn insert_test_client(pool: &sqlx::SqlitePool, id: Uuid, name: &str) {
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO sentinel_clients (id, name, authorization_code_enc, \
             authorization_code_hash, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(vec![7_u8; 64])
        .bind(hash_secret(name).to_vec())
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn upsert_test_camera(
        pool: &sqlx::SqlitePool,
        client_id: Uuid,
        camera_id: Uuid,
        name: &str,
    ) -> u64 {
        let now = Utc::now();
        sqlx::query(CLIENT_CAMERA_UPSERT)
            .bind(camera_id)
            .bind(name)
            .bind("entrance")
            .bind(client_id)
            .bind(camera_id)
            .bind("rtsp")
            .bind("vendor")
            .bind("model")
            .bind("firmware")
            .bind("serial")
            .bind("{}")
            .bind("[]")
            .bind(Option::<&str>::None)
            .bind("online")
            .bind(false)
            .bind(true)
            .bind(true)
            .bind("server")
            .bind(now)
            .bind(now)
            .bind(now)
            .execute(pool)
            .await
            .unwrap()
            .rows_affected()
    }

    #[tokio::test]
    async fn client_camera_snapshot_upsert_is_exact_and_preserves_soft_deleted_ownership() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("snapshot.sqlite3");
        let pool = crate::sqlite::open_pool(&format!("sqlite://{}", database.display()))
            .await
            .unwrap();
        let owner = Uuid::new_v4();
        let other = Uuid::new_v4();
        let camera = Uuid::new_v4();
        insert_test_client(&pool, owner, "owner").await;
        insert_test_client(&pool, other, "other").await;
        let installation = Uuid::new_v4();
        sqlx::query("UPDATE sentinel_clients SET installation_id = ? WHERE id IN (?, ?)")
            .bind(installation)
            .bind(owner)
            .bind(other)
            .execute(&pool)
            .await
            .expect("one physical client installation may hold multiple camera authorizations");

        assert_eq!(upsert_test_camera(&pool, owner, camera, "first").await, 1);
        assert_eq!(upsert_test_camera(&pool, owner, camera, "updated").await, 1);
        let second_camera = Uuid::new_v4();
        let second_for_same_authorization = sqlx::query(CLIENT_CAMERA_UPSERT)
            .bind(second_camera)
            .bind("second")
            .bind("entrance")
            .bind(owner)
            .bind(second_camera)
            .bind("rtsp")
            .bind("vendor")
            .bind("model")
            .bind("firmware")
            .bind("serial-2")
            .bind("{}")
            .bind("[]")
            .bind(Option::<&str>::None)
            .bind("online")
            .bind(false)
            .bind(true)
            .bind(true)
            .bind("server")
            .bind(Utc::now())
            .bind(Utc::now())
            .bind(Utc::now())
            .execute(&pool)
            .await;
        assert!(second_for_same_authorization.is_err());
        let direct_camera = sqlx::query(
            "INSERT INTO cameras (id, name, source_kind, adapter_kind, created_at, updated_at) \
             VALUES (?, 'legacy-direct', 'direct', 'server_direct', ?, ?)",
        )
        .bind(Uuid::new_v4())
        .bind(Utc::now())
        .bind(Utc::now())
        .execute(&pool)
        .await;
        assert!(direct_camera.is_err());
        let stored: (String, Uuid, String, String, bool) = sqlx::query_as(
            "SELECT name, client_id, storage_mode, device_status, record_enabled \
             FROM cameras WHERE id = ?",
        )
        .bind(camera)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            stored,
            (
                "updated".into(),
                owner,
                "server".into(),
                "online".into(),
                true
            )
        );

        sqlx::query("UPDATE cameras SET deleted_at = ?, name = 'tombstone' WHERE id = ?")
            .bind(Utc::now())
            .bind(camera)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(upsert_test_camera(&pool, other, camera, "stolen").await, 0);
        let preserved: (String, Uuid, bool) = sqlx::query_as(
            "SELECT name, client_id, deleted_at IS NOT NULL FROM cameras WHERE id = ?",
        )
        .bind(camera)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(preserved, ("tombstone".into(), owner, true));
        pool.close().await;
    }

    #[tokio::test]
    async fn revoked_paired_instance_is_deleted_only_after_media_removal_succeeds() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("delete.sqlite3");
        let pool = crate::sqlite::open_pool(&format!("sqlite://{}", database.display()))
            .await
            .unwrap();
        let client = Uuid::new_v4();
        insert_test_client(&pool, client, "delete-owner").await;
        assert_eq!(
            upsert_test_camera(&pool, client, client, "delete-camera").await,
            1
        );
        sqlx::query("UPDATE sentinel_clients SET status = 'revoked', revoked_at = ? WHERE id = ?")
            .bind(Utc::now())
            .bind(client)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO media_desired_states (camera_id, generation, desired_present, \
             main_path, record_enabled, updated_at) VALUES (?, 1, 0, 'delete-main', 0, ?)",
        )
        .bind(client)
        .bind(Utc::now())
        .execute(&pool)
        .await
        .unwrap();

        let mut blocked = pool.begin().await.unwrap();
        assert!(matches!(
            purge_revoked_client_in(&mut blocked, client).await,
            Err(AppError::Conflict(_))
        ));
        blocked.rollback().await.unwrap();

        let now = Utc::now().timestamp_micros();
        sqlx::query(
            "INSERT INTO _sarmg_operations (operation_id, namespace, target_key, action, \
             idempotency_digest, request_fingerprint, request_payload, state, attempt, \
             max_attempts, not_before_micros, created_at_micros, updated_at_micros) \
             VALUES (?, ?, ?, 'reconcile_camera', ?, ?, ?, 'succeeded', 1, 8, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(reconciliation::OPERATION_NAMESPACE)
        .bind(client.hyphenated().to_string())
        .bind(vec![1_u8; 32])
        .bind(vec![2_u8; 32])
        .bind(
            serde_json::to_vec(&json!({
                "camera_id": client,
                "generation": 1,
                "reason": "client_revoked",
                "requested_by": null
            }))
            .unwrap(),
        )
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();

        let mut transaction = pool.begin().await.unwrap();
        purge_revoked_client_in(&mut transaction, client)
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        let client_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sentinel_clients WHERE id = ?")
                .bind(client)
                .fetch_one(&pool)
                .await
                .unwrap();
        let camera_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cameras WHERE id = ?")
            .bind(client)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!((client_count, camera_count), (0, 0));
        pool.close().await;
    }

    use serde::de::DeserializeOwned;

    fn rejects_unknown<T: DeserializeOwned>(value: Value) {
        assert!(serde_json::from_value::<T>(value).is_err());
    }

    #[test]
    fn route_local_request_dtos_reject_unknown_fields() {
        let camera_id = Uuid::new_v4();
        rejects_unknown::<RecordingQuery>(json!({
            "camera_id": camera_id,
            "unknown": true
        }));
        rejects_unknown::<PlayRecordingQuery>(json!({
            "camera_id": camera_id,
            "start": Utc::now(),
            "duration": 1.0,
            "unknown": true
        }));
        rejects_unknown::<AuditQuery>(json!({ "unknown": true }));
        rejects_unknown::<ResolveMediaOperation>(
            json!({ "resolution": "confirmed_succeeded", "retry": true }),
        );
        rejects_unknown::<ResolveMediaOperation>(json!({ "resolution": "retry" }));
        rejects_unknown::<MediaAuthRequest>(json!({
            "user": "",
            "password": "",
            "token": "token",
            "ip": "127.0.0.1",
            "action": "read",
            "path": "camera/main",
            "protocol": "webrtc",
            "id": Uuid::new_v4(),
            "query": "",
            "userAgent": "test",
            "unknown": true
        }));
    }

    #[test]
    fn route_local_required_fields_cannot_be_omitted() {
        assert!(serde_json::from_value::<ResolveMediaOperation>(json!({})).is_err());
        assert!(serde_json::from_value::<RecordingQuery>(json!({})).is_err());
        assert!(serde_json::from_value::<PlayRecordingQuery>(json!({})).is_err());
        assert!(serde_json::from_value::<MediaAuthRequest>(json!({
            "user": "",
            "password": "",
            "token": "token",
            "ip": "127.0.0.1",
            "action": "read",
            "path": "camera/main",
            "protocol": "webrtc",
            "id": Uuid::new_v4(),
            "query": ""
        }))
        .is_err());
    }
}
