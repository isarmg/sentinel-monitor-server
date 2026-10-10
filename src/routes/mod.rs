use crate::{
    auth::{decode_media_token, issue_media_token, CurrentUser, MediaClaims},
    background::camera_path,
    crypto::is_current_authorization_code,
    error::{AppError, Result},
    models::*,
    protocol::CONTRACT,
    reconciliation, AppState,
};
use async_stream::stream;
use axum::{
    body::Body,
    extract::State,
    http::{
        header::{
            ACCEPT_RANGES, AUTHORIZATION, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE,
            CONTENT_TYPE, RANGE,
        },
        HeaderMap, StatusCode,
    },
    response::{sse::Event, IntoResponse, Response, Sse},
    routing::{get, patch, post, put},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::{
    convert::Infallible,
    future::Future,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tower_http::{compression::CompressionLayer, trace::TraceLayer};
use url::Url;
use uuid::Uuid;
use xcss::server_cli::{ContractPath as Path, ContractQuery as Query};

mod cameras;
mod clients;
mod history;
mod media;
mod recordings;

use cameras::*;
use clients::*;
use history::*;
use media::*;
use recordings::*;

#[cfg(test)]
mod request_contract_tests;

const CAMERA_SELECT: &str = "SELECT id, name, location, source_kind, client_id, adapter_kind, manufacturer, model, firmware_version, serial_number, capabilities_json, streams_json, health_message, device_status, has_sub_stream, enabled, record_enabled, storage_mode, status, last_seen_at, created_at, updated_at FROM cameras";
pub fn router(state: AppState, runtime: xcss::server_runtime::RuntimeHandle) -> Result<Router> {
    let platform = xcss::server_runtime::platform_router(
        runtime,
        "xcos",
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
        .route(
            "/cameras/{id}/commands/{command_id}",
            get(device_command_status),
        )
        .route("/clients", get(list_clients).post(create_client))
        .route(
            "/clients/{id}/authorization",
            put(update_client_authorization),
        )
        .route(
            "/clients/{id}",
            patch(update_client_name).delete(revoke_client),
        )
        .route("/client/pair", post(pair_client))
        .route("/client/snapshot", put(client_snapshot))
        .route("/recordings", get(list_recordings))
        .route("/recordings/play", get(play_recording))
        .route("/events", get(list_events))
        .route("/events/logs", get(list_event_logs))
        .route("/events/stream", get(event_stream))
        .route("/events/{id}/ack", post(ack_event))
        .route("/audit", get(list_audit))
        .route("/logs/calendar", get(log_calendar))
        .route("/system/status", get(system_status));

    let scope = state.scope.clone();
    let product = Router::new()
        .route(&CONTRACT.media_auth_path, post(media_auth))
        .nest(&CONTRACT.api_prefix, api)
        .fallback(crate::static_assets::serve)
        .layer(CompressionLayer::new())
        .with_state(state);
    Ok(Router::new()
        .merge(platform)
        .merge(product)
        .method_not_allowed_fallback(|| async {
            (
                StatusCode::METHOD_NOT_ALLOWED,
                Json(xcss::error::ErrorEnvelope::with_code(
                    xcss::error::ErrorCode::new("method_not_allowed").expect("static code"),
                    "The request method is not supported by this route.",
                )),
            )
        })
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &axum::extract::Request| {
                    let request_id = request
                        .extensions()
                        .get::<xcss::contracts::RequestId>()
                        .map(|value| value.as_str())
                        .unwrap_or("");
                    tracing::info_span!("http.request", request_id)
                })
                .on_request(())
                .on_response(
                    |response: &Response, duration: Duration, _: &tracing::Span| {
                        tracing::info!(
                            event = "xcos.http.completed",
                            component = "http",
                            status = u64::from(response.status().as_u16()),
                            duration_ms = duration.as_millis().min(u128::from(u64::MAX)) as u64
                        );
                    },
                )
                .on_body_chunk(())
                .on_eos(())
                .on_failure(()),
        )
        .layer(axum::middleware::from_fn_with_state(scope, admit_request))
        .layer(axum::middleware::from_fn_with_state(
            "xcos".to_string(),
            xcss::server_cli::service_identity_middleware,
        ))
        .layer(axum::middleware::from_fn(
            xcss::server_cli::request_context_middleware,
        )))
}

async fn admit_request(
    State(scope): State<xcss::server_runtime::WorkScope>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let Some(_admitted) = scope.enter_request().await else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(xcss::error::ErrorEnvelope::new(
                xcss::error::HttpStatus::ServiceUnavailable,
                "The service is stopping.",
            )),
        )
            .into_response();
    };
    next.run(request).await
}

async fn load_camera(state: &AppState, id: Uuid) -> Result<CameraRecord> {
    sqlx::query_as::<_, CameraRecord>(sqlx::AssertSqlSafe(format!(
        "{CAMERA_SELECT} WHERE id = ? AND deleted_at IS NULL"
    )))
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("摄像头不存在".into()))
}

async fn write_audit_in(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    user_id: Option<&str>,
    action: &str,
    entity_type: &str,
    entity_id: Option<Uuid>,
    details: Value,
) -> Result<()> {
    // Final human resolution uses space reserved when its operation was accepted.
    if action != "media.operation.resolve" {
        crate::history::reserve_in(transaction, entity_id, false).await?;
    }
    if serde_json::to_vec(&details)
        .map_err(|_| AppError::Internal("audit record encoding failed".into()))?
        .len()
        > 65536
    {
        return Err(AppError::Validation("审计记录超过大小限制".into()));
    }
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
