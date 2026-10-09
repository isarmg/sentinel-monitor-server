use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RecordingQuery {
    camera_id: Uuid,
    profile: Option<String>,
    start: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
}

pub(super) fn validate_recording_range(
    start: Option<&DateTime<Utc>>,
    end: Option<&DateTime<Utc>>,
) -> Result<()> {
    if start.zip(end).is_some_and(|(start, end)| start > end) {
        return Err(AppError::Validation(
            "录像查询的开始时间不能晚于结束时间".into(),
        ));
    }
    Ok(())
}

pub(super) async fn list_recordings(
    user: CurrentUser,
    State(state): State<AppState>,
    Query(query): Query<RecordingQuery>,
) -> Result<Json<Vec<crate::mediamtx::RecordingSpan>>> {
    validate_recording_range(query.start.as_ref(), query.end.as_ref())?;
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
pub(super) struct PlayRecordingQuery {
    camera_id: Uuid,
    start: DateTime<Utc>,
    duration: f64,
    format: Option<String>,
}

pub(super) async fn play_recording(
    user: CurrentUser,
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PlayRecordingQuery>,
) -> Result<Response<Body>> {
    if !query.duration.is_finite()
        || query.duration <= 0.0
        || query.duration > crate::mediamtx::MAX_PLAYBACK_SECONDS
    {
        return Err(AppError::Validation(
            "单次回放时长必须大于0秒且不超过6小时".into(),
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
