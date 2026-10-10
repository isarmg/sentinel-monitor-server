use super::*;

pub(super) const DELIVERED_COMMAND_RESULT_GRACE_SECONDS: i64 = 60;
pub(super) const CLIENT_CAMERA_UPSERT: &str =
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

pub(super) const CLIENT_LIST_SELECT: &str =
    "SELECT id, installation_id, name, client_version, authorization_code_enc, status, \
     last_seen_at, created_at, updated_at \
     FROM xcocs ORDER BY name COLLATE NOCASE, name, id";

pub(super) async fn list_clients(
    _user: CurrentUser,
    State(state): State<AppState>,
) -> Result<Response> {
    let rows = sqlx::query_as::<_, XcosClientRecord>(CLIENT_LIST_SELECT)
        .fetch_all(&state.pool)
        .await?;
    let views = rows
        .into_iter()
        .map(|record| client_view(&state, record))
        .collect::<Result<Vec<_>>>()?;
    Ok(([("cache-control", "no-store")], Json(views)).into_response())
}

pub(super) async fn create_client(
    user: CurrentUser,
    State(state): State<AppState>,
    xcss::server_cli::ContractJson(request): xcss::server_cli::ContractJson<
        CreateXcosClientRequest,
    >,
) -> Result<Response> {
    let name = validate_client_name(request.name.as_deref().unwrap_or("新实例"))?;
    let id = Uuid::new_v4();
    let code = random_authorization_code()?;
    let encoded = state
        .secrets
        .encrypt_client_authorization(&id.to_string(), &code)?;
    let now = Utc::now();
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query(
        "INSERT INTO xcocs (id, name, authorization_code_enc, authorization_code_hash, \
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
        Json(XcosClientView {
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

pub(super) async fn update_client_authorization(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    xcss::server_cli::ContractJson(request): xcss::server_cli::ContractJson<
        UpdateAuthorizationCodeRequest,
    >,
) -> Result<Response> {
    validate_authorization_code(&request.authorization_code)?;
    let encoded = state
        .secrets
        .encrypt_client_authorization(&id.to_string(), &request.authorization_code)?;
    let now = Utc::now();
    let mut transaction = state.pool.begin().await?;
    let changed = sqlx::query(
        "UPDATE xcocs SET authorization_code_enc = ?, authorization_code_hash = ?, \
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
    let cameras = sqlx::query_as::<_, CameraRecord>(sqlx::AssertSqlSafe(format!(
        "{CAMERA_SELECT} WHERE client_id = ? AND deleted_at IS NULL"
    )))
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
    state.reconcile_notify.notify_one();
    let record = sqlx::query_as::<_, XcosClientRecord>(
        "SELECT id, installation_id, name, client_version, authorization_code_enc, status, \
         last_seen_at, created_at, updated_at FROM xcocs WHERE id = ?",
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

pub(super) async fn update_client_name(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    xcss::server_cli::ContractJson(request): xcss::server_cli::ContractJson<
        UpdateXcosClientRequest,
    >,
) -> Result<Response> {
    let name = validate_client_name(&request.name)?;
    let now = Utc::now();
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let changed = sqlx::query(
        "UPDATE xcocs SET name = ?, updated_at = ? \
         WHERE id = ? AND revoked_at IS NULL",
    )
    .bind(&name)
    .bind(now)
    .bind(id)
    .execute(&mut *transaction)
    .await?;
    if changed.rows_affected() != 1 {
        return Err(AppError::NotFound("客户端实例不存在".into()));
    }
    write_audit_in(
        &mut transaction,
        Some(&user.id),
        "client.rename",
        "client",
        Some(id),
        json!({ "name": name }),
    )
    .await?;
    transaction.commit().await?;
    let record = sqlx::query_as::<_, XcosClientRecord>(
        "SELECT id, installation_id, name, client_version, authorization_code_enc, status, \
         last_seen_at, created_at, updated_at FROM xcocs WHERE id = ?",
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

pub(super) fn client_view(state: &AppState, record: XcosClientRecord) -> Result<XcosClientView> {
    let status = if record.status == "online"
        && record
            .last_seen_at
            .is_none_or(|last_seen| last_seen <= Utc::now() - chrono::Duration::seconds(30))
    {
        "offline".to_owned()
    } else {
        record.status
    };
    Ok(XcosClientView {
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

pub(super) async fn pair_client(
    State(state): State<AppState>,
    xcss::server_cli::ContractJson(request): xcss::server_cli::ContractJson<PairClientRequest>,
) -> Result<Response> {
    if request.protocol != CLIENT_PAIRING_PROTOCOL || request.product != "xcos" {
        return Err(AppError::UnsupportedClientProtocol);
    }
    if request.installation_id.is_nil()
        || validate_pairing_authorization_code(&request.authorization_code).is_err()
        || request.client_version.is_empty()
        || request.client_version.len() > 64
        || request.client_version.chars().any(char::is_control)
    {
        return Err(AppError::Validation("客户端配对输入无效".into()));
    }
    validate_client_name(&request.name)?;
    let code_hash = hash_secret(&request.authorization_code).to_vec();
    let access_token = xcss::admin_auth::random_token()
        .map_err(|_| AppError::Internal("client token generation failed".into()))?;
    let now = Utc::now();
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let (client_id, owner) = sqlx::query_as::<_, (Uuid, Option<String>)>(
        "SELECT id, created_by FROM xcocs WHERE authorization_code_hash = ? \
         AND token_hash IS NULL AND revoked_at IS NULL",
    )
    .bind(code_hash)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(AppError::Unauthorized)?;
    let changed = sqlx::query(
        "UPDATE xcocs SET installation_id = ?, client_version = ?, token_hash = ?, \
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

pub(super) async fn revoke_client(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode> {
    let now = Utc::now();
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let status: Option<String> = sqlx::query_scalar("SELECT status FROM xcocs WHERE id = ?")
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
        "UPDATE xcocs SET status = 'revoked', revoked_at = ?, updated_at = ? \
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
    let cameras = sqlx::query_as::<_, CameraRecord>(sqlx::AssertSqlSafe(format!(
        "{CAMERA_SELECT} WHERE client_id = ? AND deleted_at IS NULL"
    )))
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
    state.reconcile_notify.notify_one();
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn purge_revoked_client_in(
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
            "SELECT state, resolution_code, request_payload FROM _common_operations \
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
    sqlx::query("DELETE FROM xcocs WHERE id = ?")
        .bind(client_id)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

pub(super) async fn client_snapshot(
    State(state): State<AppState>,
    headers: HeaderMap,
    xcss::server_cli::ContractJson(request): xcss::server_cli::ContractJson<ClientSnapshotRequest>,
) -> Result<Response> {
    validate_client_snapshot_shape(&request)?;
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
        "SELECT id FROM xcocs WHERE token_hash = ? AND revoked_at IS NULL",
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
        "UPDATE xcocs SET status = 'online', last_seen_at = ?, updated_at = ? \
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

    apply_client_command_results_in(&mut transaction, client_id, &request.command_results, now)
        .await?;
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
        let existing = sqlx::query_as::<_, CameraRecord>(sqlx::AssertSqlSafe(format!(
            "{CAMERA_SELECT} WHERE id = ?"
        )))
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
            let current = sqlx::query_as::<_, CameraRecord>(sqlx::AssertSqlSafe(format!(
                "{CAMERA_SELECT} WHERE id = ?"
            )))
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

    remove_missing_client_cameras_in(&mut transaction, client_id, &ids, now).await?;
    let mut publish = Vec::new();
    for camera in request
        .cameras
        .iter()
        .filter(|camera| camera.enabled && camera.status == "online")
    {
        if !media_publish_ready_in(&mut transaction, camera.id, camera.has_sub_stream).await? {
            continue;
        }
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
         AND (delivered_at IS NULL OR delivered_at <= ?) ORDER BY created_at LIMIT ?",
    )
    .bind(client_id)
    .bind(now)
    .bind(retry_before)
    .bind(i64::try_from(request.command_capacity).expect("validated command capacity"))
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
    state.reconcile_notify.notify_one();
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

pub(super) async fn remove_missing_client_cameras_in(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    client_id: Uuid,
    ids: &HashSet<Uuid>,
    now: DateTime<Utc>,
) -> Result<()> {
    let existing = sqlx::query_as::<_, CameraRecord>(sqlx::AssertSqlSafe(format!(
        "{CAMERA_SELECT} WHERE client_id = ? AND deleted_at IS NULL"
    )))
    .bind(client_id)
    .fetch_all(&mut **transaction)
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
            .execute(&mut **transaction)
            .await?;
            sqlx::query(
                "UPDATE device_commands SET status = 'expired', finished_at = ? \
                 WHERE camera_id = ? AND client_id = ? AND status = 'pending'",
            )
            .bind(now)
            .bind(camera.id)
            .bind(client_id)
            .execute(&mut **transaction)
            .await?;
            reconciliation::queue_camera_change(
                transaction,
                &camera,
                false,
                &client_id.to_string(),
                "client_camera_removed",
            )
            .await?;
        }
    }
    Ok(())
}

pub(super) async fn media_publish_ready_in(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    camera_id: Uuid,
    requires_sub: bool,
) -> Result<bool> {
    let state = sqlx::query_as::<_, (bool, Option<String>, bool, bool)>(
        "SELECT d.desired_present, d.sub_path, \
         EXISTS(SELECT 1 FROM media_actual_paths a \
                WHERE a.path_name = d.main_path AND a.present = 1 \
                  AND a.applied_generation = d.generation), \
         EXISTS(SELECT 1 FROM media_actual_paths a \
                WHERE a.path_name = d.sub_path AND a.present = 1 \
                  AND a.applied_generation = d.generation) \
         FROM media_desired_states d WHERE d.camera_id = ?",
    )
    .bind(camera_id)
    .fetch_optional(&mut **transaction)
    .await?;
    Ok(
        state.is_some_and(|(desired, sub_path, main_ready, sub_ready)| {
            desired && main_ready && (!requires_sub || (sub_path.is_some() && sub_ready))
        }),
    )
}

pub(super) fn validate_client_snapshot_shape(request: &ClientSnapshotRequest) -> Result<()> {
    if request.protocol != CLIENT_PAIRING_PROTOCOL {
        return Err(AppError::UnsupportedClientProtocol);
    }
    if request.command_capacity > 100 {
        return Err(AppError::Validation(
            "客户端命令容量必须在0到100之间".into(),
        ));
    }
    if request.cameras.len() > 1 {
        return Err(AppError::Validation(
            "每个授权实例最多只能上报一台摄像机".into(),
        ));
    }
    if request.command_results.len() > 100 {
        return Err(AppError::Validation("命令结果数量超过限制".into()));
    }
    Ok(())
}

pub(super) async fn apply_client_command_results_in(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    client_id: Uuid,
    results: &[DeviceCommandResult],
    now: DateTime<Utc>,
) -> Result<()> {
    // expires_at limits how long an unclaimed physical action can be delivered.
    // Once delivered, allow time for the camera SOAP call and the next report.
    sqlx::query(
        "UPDATE device_commands SET status = 'expired', finished_at = ? \
         WHERE client_id = ? AND status = 'pending' AND ( \
           (delivered_at IS NULL AND expires_at <= ?) OR \
           (delivered_at IS NOT NULL AND delivered_at <= ?))",
    )
    .bind(now)
    .bind(client_id)
    .bind(now)
    .bind(now - chrono::Duration::seconds(DELIVERED_COMMAND_RESULT_GRACE_SECONDS))
    .execute(&mut **transaction)
    .await?;
    for result in results {
        sqlx::query(
            "UPDATE device_commands SET status = ?, error = ?, finished_at = ? \
             WHERE id = ? AND client_id = ? AND status = 'pending' \
             AND delivered_at IS NOT NULL",
        )
        .bind(match result.outcome {
            CommandOutcome::Succeeded => "succeeded",
            CommandOutcome::Failed => "failed",
            CommandOutcome::Unknown => "expired",
        })
        .bind(result.error_code.map(CommandErrorCode::as_str))
        .bind(now)
        .bind(result.id)
        .bind(client_id)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

pub(super) fn client_token_hash(headers: &HeaderMap) -> Result<Vec<u8>> {
    let header = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| value.len() == 43)
        .ok_or(AppError::Unauthorized)?;
    Ok(hash_secret(header).to_vec())
}

pub(super) fn validate_client_name(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 64 || value.chars().any(char::is_control) {
        return Err(AppError::Validation(
            "客户端名称须为 1–64 个字符，不能包含控制字符".into(),
        ));
    }
    Ok(value.to_owned())
}

pub(super) fn validate_client_camera(camera: &ClientCameraSnapshot) -> Result<()> {
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
    if camera.capabilities.sub_stream.is_supported() != camera.has_sub_stream
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
    if camera.enabled == (camera.status == "disabled")
        || profiles.contains("sub") != camera.has_sub_stream
        || (!profiles.is_empty() && !profiles.contains("main"))
    {
        return Err(AppError::Validation("客户端摄像头状态与码流不一致".into()));
    }
    Ok(())
}

pub(super) fn validate_command_result(result: &DeviceCommandResult) -> Result<()> {
    let valid = match result.outcome {
        CommandOutcome::Succeeded => result.error_code.is_none(),
        CommandOutcome::Unknown => result.error_code == Some(CommandErrorCode::OutcomeUnknown),
        CommandOutcome::Failed => result
            .error_code
            .is_some_and(|code| code != CommandErrorCode::OutcomeUnknown),
    };
    if result.id.is_nil() || !valid {
        return Err(AppError::Validation("客户端命令结果无效".into()));
    }
    Ok(())
}

pub(super) fn valid_adapter_kind(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
}

pub(super) fn valid_optional_device_text(value: Option<&str>, max: usize) -> bool {
    value.is_none_or(|value| {
        !value.is_empty() && value.chars().count() <= max && !value.chars().any(char::is_control)
    })
}

pub(super) fn client_publish_url(
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

pub(super) fn hash_secret(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}

pub(super) fn random_authorization_code() -> Result<String> {
    const ALPHABET: &[u8; 36] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut value = String::with_capacity(36);
    let mut bytes = [0_u8; 64];
    while value.len() < 36 {
        getrandom::fill(&mut bytes)
            .map_err(|_| AppError::Internal("secure randomness unavailable".into()))?;
        for byte in bytes {
            if byte < 252 {
                value.push(ALPHABET[usize::from(byte % 36)] as char);
                if value.len() == 36 {
                    break;
                }
            }
        }
    }
    Ok(value)
}

pub(super) fn validate_authorization_code(value: &str) -> Result<()> {
    if !is_current_authorization_code(value) {
        return Err(AppError::Validation(
            "密码必须是 36 个小写英文字母或数字".into(),
        ));
    }
    Ok(())
}

pub(super) fn validate_pairing_authorization_code(value: &str) -> Result<()> {
    if validate_authorization_code(value).is_ok() {
        Ok(())
    } else {
        Err(AppError::Validation("密码格式无效".into()))
    }
}
