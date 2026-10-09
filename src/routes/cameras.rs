use super::*;

pub(super) async fn list_cameras(
    _user: CurrentUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<CameraView>>> {
    let cameras = sqlx::query_as::<_, CameraRecord>(sqlx::AssertSqlSafe(format!(
        "{CAMERA_SELECT} WHERE deleted_at IS NULL ORDER BY name"
    )))
    .fetch_all(&state.pool)
    .await?;
    let views = cameras
        .iter()
        .map(|camera| {
            CameraView::from_record(camera)
                .map_err(|_| AppError::Internal("stored camera metadata is invalid".into()))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Json(views))
}

pub(super) async fn stream_ticket(
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

pub(super) async fn reserve_device_command(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    client_id: Uuid,
    stop: bool,
) -> Result<()> {
    // Receipts remain for the documented day; at the limit admission fails
    // explicitly rather than silently dropping commands or terminal results.
    sqlx::query("DELETE FROM device_commands WHERE status != 'pending' AND finished_at < ?")
        .bind(Utc::now() - chrono::Duration::days(1))
        .execute(&mut **transaction)
        .await?;
    let (total,own,pending,own_pending):(i64,i64,i64,i64)=sqlx::query_as(
        "SELECT count(*),coalesce(sum(client_id=?),0),coalesce(sum(status='pending'),0),coalesce(sum(client_id=? AND status='pending'),0) FROM device_commands")
        .bind(client_id).bind(client_id).fetch_one(&mut **transaction).await?;
    let limit = if stop { 128 } else { 127 };
    if total >= 8192 || own >= 512 || pending >= 4096 || own_pending >= limit {
        return Err(AppError::CommandCapacity);
    }
    Ok(())
}

pub(super) async fn device_command_status(
    _user: CurrentUser,
    State(state): State<AppState>,
    Path((id, command_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>> {
    load_camera(&state, id).await?;
    type ReceiptRow = (String, Option<String>, Option<DateTime<Utc>>, DateTime<Utc>);
    let row:Option<ReceiptRow>=sqlx::query_as("SELECT status,error,delivered_at,expires_at FROM device_commands WHERE camera_id=? AND id=?")
        .bind(id).bind(command_id).fetch_optional(&state.pool).await?;
    let (status, error, delivered, expires) =
        row.ok_or_else(|| AppError::NotFound("命令不存在或已超过保留期限".into()))?;
    let state = match status.as_str() {
        "pending" if expires <= Utc::now() && delivered.is_none() => "expired",
        "pending"
            if delivered.is_some_and(|at| {
                at <= Utc::now() - chrono::Duration::seconds(DELIVERED_COMMAND_RESULT_GRACE_SECONDS)
            }) =>
        {
            "unconfirmed"
        }
        "pending" if delivered.is_some() => "awaiting_result",
        "pending" => "queued",
        "succeeded" => "succeeded",
        "failed" => "failed",
        "expired" if delivered.is_none() => "expired",
        "expired" => "unconfirmed",
        _ => {
            return Err(AppError::Internal(
                "stored command status is invalid".into(),
            ))
        }
    };
    Ok(Json(
        json!({"command_id":command_id,"state":state,"error_code":error.filter(|code|matches!(code.as_str(),"invalid_command"|"unsupported_capability"|"expired_before_execution"|"device_unavailable"|"device_rejected"|"outcome_unknown"))}),
    ))
}

pub(super) async fn ptz(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    xcss_server_cli::ContractJson(request): xcss_server_cli::ContractJson<PtzRequest>,
) -> Result<(StatusCode, Json<Value>)> {
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
    if !capabilities.ptz.is_supported() {
        let message = if capabilities.ptz == CapabilitySupport::Unknown {
            "摄像头PTZ能力尚未确认"
        } else {
            "摄像头已确认不支持PTZ"
        };
        return Err(AppError::Validation(message.into()));
    }
    let client_id = camera
        .client_id
        .ok_or_else(|| AppError::Internal("client camera has no owner".into()))?;
    let command_id = Uuid::new_v4();
    let now = Utc::now();
    let client_last_seen = sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
        "SELECT last_seen_at FROM xcocs \
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
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    reserve_device_command(&mut transaction, client_id, request.action == "stop").await?;
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
    .execute(&mut *transaction)
    .await?;
    write_audit_in(
        &mut transaction,
        Some(&user.id),
        "camera.ptz.queued",
        "camera",
        Some(id),
        json!({ "command_id": command_id, "action": request.action, "pan": values.0, "tilt": values.1, "zoom": values.2 }),
    )
    .await?;
    transaction.commit().await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"command_id":command_id,"state":"queued"})),
    ))
}
