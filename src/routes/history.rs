use super::*;

pub(super) const LOG_PAGE_SIZE: usize = 100;
pub(super) const LOG_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

pub(super) struct HistoryJsonWriter(pub(super) Vec<u8>);
impl std::io::Write for HistoryJsonWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > LOG_RESPONSE_BYTES.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other(
                "history response byte budget reached",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(super) fn event_columns() -> String {
    crate::sqlite::bounded_columns(&[
        ("id", 36),
        ("camera_id", 36),
        ("kind", 128),
        ("severity", 16),
        ("message", 8192),
        ("details", 65536),
        ("acknowledged_at", 128),
        ("acknowledged_by", 128),
        ("created_at", 128),
    ])
}
pub(super) fn audit_columns() -> String {
    crate::sqlite::bounded_columns(&[
        ("id", 36),
        ("user_id", 128),
        ("action", 128),
        ("entity_type", 64),
        ("entity_id", 36),
        ("details", 65536),
        ("created_at", 128),
    ])
}

pub(super) fn history_slots() -> Arc<tokio::sync::Semaphore> {
    static SLOTS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    SLOTS
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(4)))
        .clone()
}

pub(super) async fn history_response<T, F>(
    scope: xcss::server_runtime::WorkScope,
    query: F,
) -> Result<Response>
where
    T: Serialize + Send + 'static,
    F: Future<Output = Result<T>> + Send + 'static,
{
    let permit = history_slots()
        .try_acquire_owned()
        .map_err(|_| AppError::HistoryCapacity)?;
    // A cancelled or timed-out caller cannot release admission while SQLite
    // still runs. The tracked task owns the permit, then the response body.
    let task = scope
        .work_tasks
        .try_spawn(async move {
            let result = query.await?;
            let mut writer = HistoryJsonWriter(Vec::with_capacity(16 * 1024));
            serde_json::to_writer(&mut writer, &result).map_err(|_| {
                AppError::Internal("log page cannot be encoded within the response budget".into())
            })?;
            let bytes = writer.0;
            let body = stream! {
                let _permit = permit;
                for chunk in bytes.chunks(16 * 1024) {
                    yield Ok::<_, Infallible>(axum::body::Bytes::copy_from_slice(chunk));
                }
            };
            Ok((
                [(CONTENT_TYPE, "application/json")],
                Body::from_stream(body),
            )
                .into_response())
        })
        .map_err(|_| AppError::HistoryCapacity)?;
    match tokio::time::timeout(Duration::from_secs(5), task).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(AppError::Internal("log query task failed".into())),
        Err(_) => Err(AppError::HistoryCapacity),
    }
}

#[derive(Serialize)]
pub(super) struct LogPage<T: Serialize> {
    pub(super) format: &'static str,
    pub(super) items: Vec<T>,
    pub(super) next_cursor: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LogCursor {
    pub(super) date: String,
    pub(super) kind: String,
    pub(super) id: Uuid,
}

pub(super) fn cursor_id(cursor: Option<&str>, date: &str, kind: &str) -> Result<Option<Uuid>> {
    let Some(cursor) = cursor else {
        return Ok(None);
    };
    let invalid = || AppError::Validation("日志页游标无效，请重新加载第一页".into());
    if cursor.len() > 512 {
        return Err(invalid());
    }
    let decoded = URL_SAFE_NO_PAD.decode(cursor).map_err(|_| invalid())?;
    let value: LogCursor = serde_json::from_slice(&decoded).map_err(|_| invalid())?;
    if value.date != date || value.kind != kind {
        return Err(invalid());
    }
    Ok(Some(value.id))
}

pub(super) fn log_page<T: Serialize>(
    mut items: Vec<T>,
    date: &str,
    kind: &str,
    id: impl Fn(&T) -> Uuid,
) -> LogPage<T> {
    let more = items.len() > LOG_PAGE_SIZE;
    items.truncate(LOG_PAGE_SIZE);
    let next_cursor = more.then(|| {
        let last = items.last().expect("a full page precedes another page");
        URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&LogCursor {
                date: date.to_owned(),
                kind: kind.to_owned(),
                id: id(last),
            })
            .expect("fixed log cursor serializes"),
        )
    });
    LogPage {
        format: "xcos-history-page-v1",
        items,
        next_cursor,
    }
}

pub(super) async fn dated_cursor(
    connection: &mut sqlx::SqliteConnection,
    table: &str,
    id: Option<Uuid>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Option<(String, Uuid)>> {
    let Some(id) = id else { return Ok(None) };
    let sql = match table {
        "events" => "SELECT CASE WHEN length(CAST(created_at AS BLOB)) <= 128 THEN created_at END FROM events WHERE id = ? AND created_at >= ? AND created_at < ?",
        "audit" => "SELECT CASE WHEN length(CAST(created_at AS BLOB)) <= 128 THEN created_at END FROM audit_logs WHERE id = ? AND created_at >= ? AND created_at < ?",
        _ => unreachable!("only static log tables are accepted"),
    };
    let timestamp: Option<Option<String>> = sqlx::query_scalar(sql)
        .bind(id)
        .bind(start)
        .bind(end)
        .fetch_optional(connection)
        .await
        .map_err(crate::sqlite::history_error)?;
    let timestamp = timestamp
        .ok_or_else(|| AppError::Validation("日志页游标已失效，请重新加载第一页".into()))?
        .ok_or_else(|| AppError::Internal("log-record timestamp exceeds the read budget".into()))?;
    Ok(Some((timestamp, id)))
}

#[derive(Serialize)]
pub(super) struct ServerDated<T: Serialize> {
    #[serde(flatten)]
    pub(super) record: T,
    pub(super) server_created_at: String,
}

impl<T: Serialize> ServerDated<T> {
    pub(super) fn new(record: T, created_at: DateTime<Utc>) -> Self {
        Self {
            record,
            server_created_at: created_at
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S %:z")
                .to_string(),
        }
    }
}

pub(super) fn local_day_start(date: NaiveDate) -> Result<DateTime<Local>> {
    // A local midnight can be skipped or repeated when the server clock changes.
    // Pick the first existing instant in that calendar day.
    for minute in 0..24 * 60 {
        let local = date
            .and_hms_opt(minute / 60, minute % 60, 0)
            .ok_or_else(|| AppError::Validation("无效的服务器日期".into()))?;
        if let Some(start) = Local.from_local_datetime(&local).earliest() {
            return Ok(start);
        }
    }
    Err(AppError::Validation("该日期在服务器时区中不存在".into()))
}

pub(super) fn server_day_bounds(value: &str) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    if value.len() != 10
        || !value.bytes().enumerate().all(|(index, byte)| {
            if index == 4 || index == 7 {
                byte == b'-'
            } else {
                byte.is_ascii_digit()
            }
        })
    {
        return Err(AppError::Validation("日期必须为 YYYY-MM-DD".into()));
    }
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| AppError::Validation("日期必须为 YYYY-MM-DD".into()))?;
    if date.format("%Y-%m-%d").to_string() != value || value.starts_with("0000-") {
        return Err(AppError::Validation("日期必须为 YYYY-MM-DD".into()));
    }
    let next = date
        .succ_opt()
        .ok_or_else(|| AppError::Validation("日期超出有效范围".into()))?;
    Ok((
        local_day_start(date)?.with_timezone(&Utc),
        local_day_start(next)?.with_timezone(&Utc),
    ))
}

pub(super) fn server_log_range(
    date: Option<&str>,
    start_date: Option<&str>,
    end_date: Option<&str>,
) -> Result<(DateTime<Utc>, DateTime<Utc>, String)> {
    let (start_date, end_date) = match (date, start_date, end_date) {
        (Some(date), None, None) => (date, date),
        (None, Some(start), Some(end)) => (start, end),
        _ => return Err(AppError::Validation("请选择单日或完整的日期范围".into())),
    };
    let (start, _) = server_day_bounds(start_date)?;
    let (_, end) = server_day_bounds(end_date)?;
    if start_date > end_date {
        return Err(AppError::Validation("结束日期不能早于开始日期".into()));
    }
    let scope = if start_date == end_date {
        start_date.to_owned()
    } else {
        format!("{start_date}/{end_date}")
    };
    Ok((start, end, scope))
}

pub(super) async fn log_calendar(_user: CurrentUser) -> Json<Value> {
    Json(json!({ "today": Local::now().date_naive().format("%Y-%m-%d").to_string() }))
}
pub(super) async fn list_events(
    _user: CurrentUser,
    State(state): State<AppState>,
    Query(query): Query<EventQuery>,
) -> Result<Response> {
    history_response(state.scope.clone(), async move {
        let limit = query.limit.unwrap_or(100);
        if !(1..=100).contains(&limit) {
            return Err(AppError::Validation(
                "事件页大小必须在 1 到 100 之间".into(),
            ));
        }
        let mut connection = crate::sqlite::history_connection(&state.pool).await?;
        let mut builder = sqlx::QueryBuilder::<sqlx::Sqlite>::new(format!(
            "SELECT {} FROM events WHERE 1 = 1",
            event_columns()
        ));
        if let Some(camera_id) = query.camera_id {
            builder.push(" AND camera_id = ").push_bind(camera_id);
        }
        builder
            .push(" ORDER BY created_at DESC LIMIT ")
            .push_bind(limit);
        let result = builder
            .build_query_as::<EventRecord>()
            .fetch_all(&mut *connection)
            .await
            .map_err(crate::sqlite::history_error);
        connection.close().await?;
        result
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EventLogQuery {
    date: Option<String>,
    start_date: Option<String>,
    end_date: Option<String>,
    cursor: Option<String>,
}

pub(super) async fn list_event_logs(
    _user: CurrentUser,
    State(state): State<AppState>,
    Query(query): Query<EventLogQuery>,
) -> Result<Response> {
    let (start, end, scope) = server_log_range(
        query.date.as_deref(),
        query.start_date.as_deref(),
        query.end_date.as_deref(),
    )?;
    let cursor = cursor_id(query.cursor.as_deref(), &scope, "events")?;
    history_response(state.scope.clone(), async move {
        let events = event_logs_for_day(&state.pool, start, end, cursor).await?;
        Ok(log_page(
            events
                .into_iter()
                .map(|event| {
                    let created_at = event.created_at;
                    ServerDated::new(event, created_at)
                })
                .collect::<Vec<_>>(),
            &scope,
            "events",
            |row| row.record.id,
        ))
    })
    .await
}

pub(super) async fn event_logs_for_day(
    pool: &sqlx::SqlitePool,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    cursor: Option<Uuid>,
) -> Result<Vec<EventRecord>> {
    let mut connection = crate::sqlite::history_connection(pool).await?;
    let boundary = dated_cursor(&mut connection, "events", cursor, start, end).await?;
    let mut builder = sqlx::QueryBuilder::<sqlx::Sqlite>::new(format!(
        "SELECT {} FROM events WHERE created_at >= ",
        event_columns()
    ));
    builder
        .push_bind(start)
        .push(" AND created_at < ")
        .push_bind(end);
    if let Some((timestamp, id)) = boundary {
        builder
            .push(" AND (created_at < ")
            .push_bind(timestamp.clone())
            .push(" OR (created_at = ")
            .push_bind(timestamp)
            .push(" AND id < ")
            .push_bind(id)
            .push("))");
    }
    builder
        .push(" ORDER BY created_at DESC, id DESC LIMIT ")
        .push_bind((LOG_PAGE_SIZE + 1) as i64);
    let result = builder
        .build_query_as::<EventRecord>()
        .fetch_all(&mut *connection)
        .await
        .map_err(crate::sqlite::history_error);
    connection.close().await?;
    result
}

pub(super) async fn ack_event(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode> {
    acknowledge_event_in(&state.pool, id, Some(&user.id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn acknowledge_event_in(
    pool: &sqlx::SqlitePool,
    id: Uuid,
    user_id: Option<&str>,
) -> Result<()> {
    let result = sqlx::query(
        "UPDATE events SET acknowledged_at = ?, acknowledged_by = ? \
         WHERE id = ? AND acknowledged_at IS NULL",
    )
    .bind(Utc::now())
    .bind(user_id)
    .bind(id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0
        && !sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM events WHERE id = ?)")
            .bind(id)
            .fetch_one(pool)
            .await?
    {
        return Err(AppError::NotFound("事件不存在".into()));
    }
    Ok(())
}

pub(super) async fn event_stream(
    _user: CurrentUser,
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = std::result::Result<Event, Infallible>>> {
    let mut receiver = state.events.subscribe();
    let output = stream! {
        loop {
            let received=tokio::select! {_ = state.scope.shutdown.cancelled()=>break,received=receiver.recv()=>received};
            match received {
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
pub(super) struct AuditQuery {
    date: Option<String>,
    start_date: Option<String>,
    end_date: Option<String>,
    cursor: Option<String>,
}

pub(super) async fn list_audit(
    _user: CurrentUser,
    State(state): State<AppState>,
    Query(query): Query<AuditQuery>,
) -> Result<Response> {
    let (start, end, scope) = server_log_range(
        query.date.as_deref(),
        query.start_date.as_deref(),
        query.end_date.as_deref(),
    )?;
    let cursor = cursor_id(query.cursor.as_deref(), &scope, "audit")?;
    history_response(state.scope.clone(), async move {
        let rows = audit_logs_for_day(&state.pool, start, end, cursor).await?;
        Ok(log_page(
            rows.into_iter()
                .map(|row| {
                    let created_at = row.created_at;
                    ServerDated::new(row, created_at)
                })
                .collect::<Vec<_>>(),
            &scope,
            "audit",
            |row| row.record.id,
        ))
    })
    .await
}

pub(super) async fn audit_logs_for_day(
    pool: &sqlx::SqlitePool,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    cursor: Option<Uuid>,
) -> Result<Vec<AuditRecord>> {
    let mut connection = crate::sqlite::history_connection(pool).await?;
    let boundary = dated_cursor(&mut connection, "audit", cursor, start, end).await?;
    let mut builder = sqlx::QueryBuilder::<sqlx::Sqlite>::new(format!(
        "SELECT {} FROM audit_logs WHERE created_at >= ",
        audit_columns()
    ));
    builder
        .push_bind(start)
        .push(" AND created_at < ")
        .push_bind(end);
    if let Some((timestamp, id)) = boundary {
        builder
            .push(" AND (created_at < ")
            .push_bind(timestamp.clone())
            .push(" OR (created_at = ")
            .push_bind(timestamp)
            .push(" AND id < ")
            .push_bind(id)
            .push("))");
    }
    builder
        .push(" ORDER BY created_at DESC, id DESC LIMIT ")
        .push_bind((LOG_PAGE_SIZE + 1) as i64);
    let result = builder
        .build_query_as::<AuditRecord>()
        .fetch_all(&mut *connection)
        .await
        .map_err(crate::sqlite::history_error);
    connection.close().await?;
    result
}
