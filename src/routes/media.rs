use super::*;

pub(super) async fn media_operation(
    _user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<reconciliation::MediaOperationView>> {
    Ok(Json(reconciliation::get_operation(&state.pool, &id).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MediaOperationQuery {
    date: Option<String>,
    start_date: Option<String>,
    end_date: Option<String>,
    cursor: Option<String>,
}

pub(super) async fn list_media_operations(
    _user: CurrentUser,
    State(state): State<AppState>,
    Query(query): Query<MediaOperationQuery>,
) -> Result<Response> {
    let (start, end, scope) = server_log_range(
        query.date.as_deref(),
        query.start_date.as_deref(),
        query.end_date.as_deref(),
    )?;
    let cursor = cursor_id(query.cursor.as_deref(), &scope, "operations")?;
    history_response(state.scope.clone(), async move {
        let operations = reconciliation::list_operations(
            &state.pool,
            start.timestamp_micros(),
            end.timestamp_micros(),
            cursor,
            LOG_PAGE_SIZE + 1,
        )
        .await?;
        Ok(log_page(
            operations
                .into_iter()
                .map(|operation| {
                    let created_at = operation.created_at;
                    ServerDated::new(operation, created_at)
                })
                .collect::<Vec<_>>(),
            &scope,
            "operations",
            |row| Uuid::parse_str(&row.record.id).expect("stored operation identity is validated"),
        ))
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResolveMediaOperation {
    resolution: xcss::operations::Resolution,
}

pub(super) async fn resolve_media_operation(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    xcss::server_cli::ContractJson(request): xcss::server_cli::ContractJson<ResolveMediaOperation>,
) -> Result<Json<reconciliation::MediaOperationView>> {
    let operation = reconciliation::get_operation(&state.pool, &id).await?;
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    xcss::operations::SqliteOperationStore::resolve_in(
        &mut transaction,
        &id,
        request.resolution,
        Utc::now().timestamp_micros(),
    )
    .await
    .map_err(|error| match error {
        xcss::operations::Error::InvalidTransition { .. }
        | xcss::operations::Error::ConcurrentModification => {
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

pub(super) async fn system_status(
    _user: CurrentUser,
    State(state): State<AppState>,
) -> Result<Json<Value>> {
    reconciliation::validate_stored_camera_credentials(&state).await?;
    let recording: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM cameras \
         WHERE deleted_at IS NULL AND record_enabled AND enabled",
    )
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(json!({
        "service": "xcos",
        "version": env!("CARGO_PKG_VERSION"),
        "database": "ok",
        "media_service": if state.media.health().await { "ok" } else { "unavailable" },
        "cameras": { "recording_configured": recording },
        "server_time": Utc::now()
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MediaAuthRequest {
    user: String,
    password: String,
    token: String,
    ip: String,
    pub(super) action: String,
    path: String,
    protocol: String,
    pub(super) id: Option<String>,
    query: String,
    #[serde(rename = "userAgent")]
    user_agent: String,
}

pub(super) async fn media_auth(
    State(state): State<AppState>,
    xcss::server_cli::ContractJson(request): xcss::server_cli::ContractJson<MediaAuthRequest>,
) -> StatusCode {
    if [
        request.user.as_str(),
        request.password.as_str(),
        request.token.as_str(),
        request.ip.as_str(),
        request.action.as_str(),
        request.path.as_str(),
        request.protocol.as_str(),
        request.id.as_deref().unwrap_or(""),
        request.query.as_str(),
        request.user_agent.as_str(),
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
        return if media_request_permitted(&state.pool, &claims, &request.action).await {
            StatusCode::OK
        } else {
            StatusCode::FORBIDDEN
        };
    }
    StatusCode::UNAUTHORIZED
}

pub(super) async fn media_request_permitted(
    pool: &sqlx::SqlitePool,
    claims: &MediaClaims,
    action: &str,
) -> bool {
    let row = sqlx::query_as::<_, (Uuid, bool, bool, String, Option<Vec<u8>>)>(
        "SELECT sc.id, c.enabled, c.has_sub_stream, c.storage_mode, sc.token_hash \
         FROM cameras c JOIN xcocs sc ON sc.id = c.client_id \
         WHERE c.id = ? AND c.deleted_at IS NULL AND sc.revoked_at IS NULL",
    )
    .bind(claims.camera_id)
    .fetch_optional(pool)
    .await;
    let Some((client_id, enabled, has_sub_stream, storage_mode, token_hash)) = row.ok().flatten()
    else {
        return false;
    };
    let main_path = camera_path(claims.camera_id, "main");
    let sub_path = camera_path(claims.camera_id, "sub");
    if claims.path != main_path && !(has_sub_stream && claims.path == sub_path) {
        return false;
    }
    match action {
        "read" => enabled,
        "playback" => storage_mode == "server",
        "publish" => {
            enabled
                && claims.sub == client_id.to_string()
                && token_hash.is_some_and(|hash| {
                    claims.credential_binding.as_deref()
                        == Some(crate::auth::publish_binding(&hash).as_str())
                })
        }
        _ => false,
    }
}
