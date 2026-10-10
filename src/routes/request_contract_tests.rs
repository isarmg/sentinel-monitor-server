use super::*;

#[test]
fn response_encoding_stops_before_allocating_past_its_byte_budget() {
    let mut writer = HistoryJsonWriter(Vec::new());
    assert!(serde_json::to_writer(&mut writer, &"x".repeat(LOG_RESPONSE_BYTES + 1)).is_err());
    assert!(writer.0.len() <= LOG_RESPONSE_BYTES);
}

#[tokio::test]
async fn malformed_large_history_scalar_is_rejected_without_truncation_or_repair() {
    let directory = tempfile::tempdir().unwrap();
    let pool = crate::sqlite::initialize_test_pool(&format!(
        "sqlite://{}",
        directory.path().join("large.sqlite3").display()
    ))
    .await
    .unwrap();
    let id = Uuid::new_v4();
    let start = DateTime::parse_from_rfc3339("2026-09-23T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    sqlx::query("INSERT INTO events(id,kind,severity,message,details,created_at) VALUES(?,?,'warning','preserve','{}',?)")
        .bind(id).bind("x".repeat(129)).bind(start).execute(&pool).await.unwrap();
    assert!(
        event_logs_for_day(&pool, start, start + chrono::Duration::days(1), None)
            .await
            .is_err()
    );
    let kind: String = sqlx::query_scalar("SELECT kind FROM events WHERE id=?")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        kind.len(),
        129,
        "history read repaired or truncated the original"
    );
    sqlx::query("UPDATE events SET kind='test' WHERE id=?")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        event_logs_for_day(&pool, start, start + chrono::Duration::days(1), None)
            .await
            .unwrap()
            .len(),
        1
    );
    sqlx::query("UPDATE events SET created_at=? WHERE id=?")
        .bind(format!("2026-09-23T00:00:00Z{}", "x".repeat(109)))
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let mut connection = crate::sqlite::history_connection(&pool).await.unwrap();
    assert!(matches!(
        dated_cursor(
            &mut connection,
            "events",
            Some(id),
            start,
            start + chrono::Duration::days(1)
        )
        .await,
        Err(AppError::Internal(_))
    ));
    connection.close().await.unwrap();
    let length: i64 = sqlx::query_scalar("SELECT length(created_at) FROM events WHERE id=?")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        length, 129,
        "cursor read modified malformed source timestamp"
    );
}

#[tokio::test]
async fn history_admission_is_held_until_body_drop_and_cancelled_query_completion() {
    let scope = xcss::server_runtime::WorkScope::new();
    let mut bodies = Vec::new();
    for _ in 0..4 {
        bodies.push(
            history_response(scope.clone(), async { Ok(json!({"items": []})) })
                .await
                .unwrap(),
        );
    }
    assert!(matches!(
        history_response(scope.clone(), async { Ok(json!({})) }).await,
        Err(AppError::HistoryCapacity)
    ));
    bodies.pop();
    let replacement = history_response(scope.clone(), async { Ok(json!({})) })
        .await
        .unwrap();
    drop(replacement);
    drop(bodies);
    assert_eq!(history_slots().available_permits(), 4);
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
    let caller_scope = scope.clone();
    let caller = tokio::spawn(async move {
        history_response(caller_scope, async move {
            started_tx.send(()).unwrap();
            finish_rx.await.unwrap();
            Ok(json!({}))
        })
        .await
    });
    started_rx.await.unwrap();
    caller.abort();
    let _ = caller.await;
    assert_eq!(
        history_slots().available_permits(),
        3,
        "caller cancellation released a still-running query"
    );
    finish_tx.send(()).unwrap();
    scope.work_tasks.close();
    tokio::time::timeout(Duration::from_secs(1), scope.work_tasks.wait())
        .await
        .unwrap();
    assert_eq!(history_slots().available_permits(), 4);
}

#[test]
fn server_day_uses_each_local_midnight_across_clock_changes() {
    for value in ["2026-03-08", "2026-11-01"] {
        let (start, end) = server_day_bounds(value).unwrap();
        let selected = NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap();
        assert_eq!(start.with_timezone(&Local).date_naive(), selected);
        assert_eq!(
            end.with_timezone(&Local).date_naive(),
            selected.succ_opt().unwrap()
        );
        assert!(end > start);
    }
    if std::env::var("TZ").as_deref() == Ok("America/New_York") {
        let (spring_start, spring_end) = server_day_bounds("2026-03-08").unwrap();
        let (fall_start, fall_end) = server_day_bounds("2026-11-01").unwrap();
        assert_eq!((spring_end - spring_start).num_hours(), 23);
        assert_eq!((fall_end - fall_start).num_hours(), 25);
    }
}

#[tokio::test]
async fn server_date_logs_include_all_rows_with_half_open_utc_bounds() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("daily-logs.sqlite3");
    let pool = crate::sqlite::initialize_test_pool(&format!("sqlite://{}", database.display()))
        .await
        .unwrap();
    let (start, end, _) = server_log_range(None, Some("2026-09-23"), Some("2026-09-24")).unwrap();
    let samples = [
        (start - chrono::Duration::microseconds(1), true),
        (start, true),
        (start, false),
        (end - chrono::Duration::microseconds(1), true),
        (end - chrono::Duration::microseconds(1), false),
        (end, true),
        (end, false),
    ];
    for (index, (instant, zulu)) in samples.into_iter().enumerate() {
        let timestamp = instant.to_rfc3339_opts(chrono::SecondsFormat::Micros, zulu);
        let acknowledged_at = (index == 2).then_some(&timestamp);
        sqlx::query(
            "INSERT INTO events (id, kind, severity, message, details, acknowledged_at, created_at) \
             VALUES (?, 'test', 'info', 'test', '{}', ?, ?)",
        )
        .bind(Uuid::new_v4())
        .bind(acknowledged_at)
        .bind(&timestamp)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO audit_logs (id, action, entity_type, details, created_at) \
             VALUES (?, 'test', 'test', '{}', ?)",
        )
        .bind(Uuid::new_v4())
        .bind(&timestamp)
        .execute(&pool)
        .await
        .unwrap();
    }
    assert_eq!(
        event_logs_for_day(&pool, start, end, None)
            .await
            .unwrap()
            .len(),
        4
    );
    assert_eq!(
        audit_logs_for_day(&pool, start, end, None)
            .await
            .unwrap()
            .len(),
        4
    );
    for value in [
        "2026-02-30",
        "2026-9-23",
        "2026-09-23T00:00:00",
        "",
        "-0001-01-01",
        "+10000-01-01",
    ] {
        assert!(server_day_bounds(value).is_err());
    }
}

#[tokio::test]
async fn server_date_media_operations_return_every_row_across_bounded_pages() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("daily-operations.sqlite3");
    let pool = crate::sqlite::initialize_test_pool(&format!("sqlite://{}", database.display()))
        .await
        .unwrap();
    let (start, end) = server_day_bounds("2026-09-23").unwrap();
    let camera_id = Uuid::new_v4();
    let request_payload = serde_json::to_vec(&serde_json::json!({
        "camera_id": camera_id,
        "generation": 1,
        "reason": "test",
        "requested_by": null
    }))
    .unwrap();
    for index in 0_u16..122 {
        let created_at_micros = match index {
            120 => start.timestamp_micros() - 1,
            121 => end.timestamp_micros(),
            _ => start.timestamp_micros() + i64::from(index),
        };
        let mut digest = [0_u8; 32];
        digest[..2].copy_from_slice(&index.to_le_bytes());
        sqlx::query(
            "INSERT INTO _xcss_operations (operation_id, namespace, target_key, action, \
             idempotency_digest, request_fingerprint, request_payload, state, attempt, \
             max_attempts, not_before_micros, created_at_micros, updated_at_micros) \
             VALUES (?, ?, ?, 'reconcile_camera', ?, ?, ?, 'succeeded', 1, 3, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(reconciliation::OPERATION_NAMESPACE)
        .bind(camera_id.to_string())
        .bind(digest.to_vec())
        .bind(vec![1_u8; 32])
        .bind(&request_payload)
        .bind(created_at_micros)
        .bind(created_at_micros)
        .bind(created_at_micros)
        .execute(&pool)
        .await
        .unwrap();
    }
    let mut cursor = None;
    let mut operations = Vec::new();
    loop {
        let rows = reconciliation::list_operations(
            &pool,
            start.timestamp_micros(),
            end.timestamp_micros(),
            cursor,
            LOG_PAGE_SIZE + 1,
        )
        .await
        .unwrap();
        assert!(rows.len() <= LOG_PAGE_SIZE + 1);
        let page = log_page(rows, "2026-09-23", "operations", |row| {
            Uuid::parse_str(&row.id).unwrap()
        });
        assert!(page.items.len() <= LOG_PAGE_SIZE);
        operations.extend(page.items);
        cursor = cursor_id(page.next_cursor.as_deref(), "2026-09-23", "operations").unwrap();
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(operations.len(), 120);
    assert_eq!(
        operations
            .iter()
            .map(|row| &row.id)
            .collect::<HashSet<_>>()
            .len(),
        120
    );
    assert_eq!(
        operations[0].created_at.timestamp_micros(),
        start.timestamp_micros() + 119
    );
}

#[tokio::test]
async fn history_pages_preserve_equal_timestamp_rows_and_reject_invalid_cursors() {
    let directory = tempfile::tempdir().unwrap();
    let pool = crate::sqlite::initialize_test_pool(&format!(
        "sqlite://{}",
        directory.path().join("pages.sqlite3").display()
    ))
    .await
    .unwrap();
    let (start, end) = server_day_bounds("2026-09-23").unwrap();
    for index in 1..=237_u128 {
        let id = Uuid::from_u128(index);
        let timestamp = start.to_rfc3339_opts(chrono::SecondsFormat::Micros, index % 2 == 0);
        sqlx::query("INSERT INTO events (id,kind,severity,message,details,created_at) VALUES (?, 'test','info','test','{}',?)")
            .bind(id).bind(&timestamp).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO audit_logs (id,action,entity_type,details,created_at) VALUES (?, 'test','test','{}',?)")
            .bind(id).bind(&timestamp).execute(&pool).await.unwrap();
    }
    let mut events = HashSet::new();
    let mut audit = HashSet::new();
    let mut cursor = None;
    let mut pages = 0;
    loop {
        let rows = event_logs_for_day(&pool, start, end, cursor).await.unwrap();
        let page = log_page(rows, "2026-09-23", "events", |row| row.id);
        assert!(page.items.len() <= LOG_PAGE_SIZE);
        for row in page.items {
            assert!(events.insert(row.id));
        }
        pages += 1;
        if let Some(ref next) = page.next_cursor {
            assert!(cursor_id(Some(next), "2026-09-24", "events").is_err());
            assert!(cursor_id(Some(next), "2026-09-23", "audit").is_err());
        }
        cursor = cursor_id(page.next_cursor.as_deref(), "2026-09-23", "events").unwrap();
        if cursor.is_none() {
            break;
        }
    }
    cursor = None;
    loop {
        let rows = audit_logs_for_day(&pool, start, end, cursor).await.unwrap();
        let page = log_page(rows, "2026-09-23", "audit", |row| row.id);
        for row in page.items {
            assert!(audit.insert(row.id));
        }
        cursor = cursor_id(page.next_cursor.as_deref(), "2026-09-23", "audit").unwrap();
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(pages, 3);
    assert_eq!(events.len(), 237);
    assert_eq!(audit, events);
    assert!(cursor_id(Some("malformed"), "2026-09-23", "events").is_err());
    assert!(cursor_id(Some(&"A".repeat(513)), "2026-09-23", "events").is_err());
    assert!(event_logs_for_day(&pool, start, end, Some(Uuid::new_v4()))
        .await
        .is_err());
}

#[test]
fn generated_authorization_codes_have_the_shared_format() {
    for _ in 0..64 {
        let value = random_authorization_code().unwrap();
        assert_eq!(value.len(), 36);
        assert!(value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase()));
        validate_authorization_code(&value).unwrap();
    }
}

#[test]
fn camera_snapshot_rejects_contradictory_status_and_stream_metadata() {
    let camera = json!({
        "id": Uuid::new_v4(), "name": "front", "location": "entrance", "adapter_kind": "rtsp",
        "identity": { "manufacturer": null, "model": null, "firmware_version": null, "serial_number": null },
        "capabilities": { "video": "supported", "main_stream": "supported", "sub_stream": "unsupported",
            "local_recording": "supported", "server_recording": "supported", "ptz": "unsupported", "events": "unsupported",
            "audio_input": "unsupported", "audio_output": "unsupported" },
        "streams": [{ "profile": "main", "video_codec": "h264", "audio_codec": null,
            "width": 1920, "height": 1080, "frame_rate": 25.0 }],
        "has_sub_stream": false, "enabled": true, "storage_mode": "server",
        "status": "online", "health_message": null
    });
    let decoded = |value| serde_json::from_value::<ClientCameraSnapshot>(value).unwrap();
    validate_client_camera(&decoded(camera.clone())).unwrap();

    let mut disabled = camera.clone();
    disabled["enabled"] = json!(false);
    assert!(validate_client_camera(&decoded(disabled)).is_err());

    let mut stale_sub = camera;
    stale_sub["status"] = json!("error");
    stale_sub["health_message"] = json!("sub stream failed");
    stale_sub["has_sub_stream"] = json!(true);
    stale_sub["capabilities"]["sub_stream"] = json!("supported");
    assert!(validate_client_camera(&decoded(stale_sub)).is_err());
}

#[tokio::test]
async fn acknowledging_an_event_twice_preserves_its_original_record() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("event-ack.sqlite3");
    let pool = crate::sqlite::initialize_test_pool(&format!("sqlite://{}", database.display()))
        .await
        .unwrap();
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO events (id, kind, severity, message, details, created_at) \
         VALUES (?, 'test', 'info', 'test', '{}', ?)",
    )
    .bind(id)
    .bind(Utc::now())
    .execute(&pool)
    .await
    .unwrap();

    acknowledge_event_in(&pool, id, None).await.unwrap();
    let first: DateTime<Utc> =
        sqlx::query_scalar("SELECT acknowledged_at FROM events WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    acknowledge_event_in(&pool, id, None).await.unwrap();
    let second: DateTime<Utc> =
        sqlx::query_scalar("SELECT acknowledged_at FROM events WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(first, second);
    assert!(matches!(
        acknowledge_event_in(&pool, Uuid::new_v4(), None).await,
        Err(AppError::NotFound(_))
    ));
}

async fn insert_test_client(pool: &sqlx::SqlitePool, id: Uuid, name: &str) {
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO xcocs (id, name, authorization_code_enc, \
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

fn test_state(pool: sqlx::SqlitePool) -> AppState {
    let config = Arc::new(crate::config::Config {
        bind_addr: "127.0.0.1:8080".parse().unwrap(),
        database_url: "sqlite://unused".into(),
        jwt_secret: vec![9_u8; 32],
        credentials_key: [7_u8; 32],
        runtime_directory: "/tmp/xcos-test".into(),
        development_mode: true,
        media_token_ttl: Duration::from_secs(120),
        mediamtx_api_url: "http://127.0.0.1:1".into(),
        mediamtx_playback_url: "http://127.0.0.1:1".into(),
        public_webrtc_base_url: "/media-webrtc".into(),
        public_hls_base_url: "/media-hls".into(),
        public_rtsp_publish_base_url: "rtsp://127.0.0.1:8554".into(),
        status_interval: Duration::from_secs(10),
        reconcile_interval: Duration::from_secs(60),
        request_timeout: Duration::from_secs(2),
        static_dir: None,
    });
    let http = reqwest::Client::new();
    let (events, _) = tokio::sync::broadcast::channel(4);
    AppState {
        scope: xcss::server_runtime::WorkScope::new(),
        web_directory: None,
        config,
        pool: pool.clone(),
        secrets: crate::crypto::SecretBox::new(&[7_u8; 32]),
        http: http.clone(),
        media: crate::mediamtx::MediaMtxClient::new(
            http.clone(),
            http,
            "http://127.0.0.1:1".into(),
            "http://127.0.0.1:1".into(),
        ),
        events,
        reconcile_notify: Arc::new(tokio::sync::Notify::new()),
        administrator: Arc::new(xcss::admin_core::AdministratorService::new(
            xcss::admin_sqlite::SqliteAdministratorStore::new(pool),
        )),
        administrator_origin: xcss::admin_auth::AdministratorOriginMode::LoopbackDevelopmentHttp,
    }
}

#[tokio::test]
async fn client_list_returns_every_instance_in_case_insensitive_name_order() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("client-list.sqlite3");
    let pool = crate::sqlite::initialize_test_pool(&format!("sqlite://{}", database.display()))
        .await
        .unwrap();
    for name in ["zulu", "Bravo", "alpha"] {
        insert_test_client(&pool, Uuid::new_v4(), name).await;
    }
    let names = sqlx::query_as::<_, XcosClientRecord>(CLIENT_LIST_SELECT)
        .fetch_all(&pool)
        .await
        .unwrap()
        .into_iter()
        .map(|client| client.name)
        .collect::<Vec<_>>();
    assert_eq!(names, ["alpha", "Bravo", "zulu"]);
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
async fn empty_snapshot_removes_camera_and_expires_its_commands() {
    let empty = ClientSnapshotRequest {
        protocol: CLIENT_PAIRING_PROTOCOL.to_owned(),
        command_capacity: 100,
        cameras: Vec::new(),
        command_results: Vec::new(),
    };
    validate_client_snapshot_shape(&empty).unwrap();

    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("empty-snapshot.sqlite3");
    let pool = crate::sqlite::initialize_test_pool(&format!("sqlite://{}", database.display()))
        .await
        .unwrap();
    let client_id = Uuid::new_v4();
    insert_test_client(&pool, client_id, "empty-snapshot").await;
    upsert_test_camera(&pool, client_id, client_id, "old-camera").await;
    let now = Utc::now();
    let command_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO device_commands (id, camera_id, client_id, kind, payload, created_at, expires_at) \
         VALUES (?, ?, ?, 'ptz', '{}', ?, ?)",
    )
    .bind(command_id)
    .bind(client_id)
    .bind(client_id)
    .bind(now)
    .bind(now + chrono::Duration::seconds(30))
    .execute(&pool)
    .await
    .unwrap();

    let mut transaction = pool.begin().await.unwrap();
    remove_missing_client_cameras_in(&mut transaction, client_id, &HashSet::new(), now)
        .await
        .unwrap();
    transaction.commit().await.unwrap();
    let (deleted_at, enabled): (Option<DateTime<Utc>>, bool) =
        sqlx::query_as("SELECT deleted_at, enabled FROM cameras WHERE id = ?")
            .bind(client_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(deleted_at.is_some());
    assert!(!enabled);
    let desired_present: bool =
        sqlx::query_scalar("SELECT desired_present FROM media_desired_states WHERE camera_id = ?")
            .bind(client_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!desired_present);
    let command_status: String =
        sqlx::query_scalar("SELECT status FROM device_commands WHERE id = ?")
            .bind(command_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(command_status, "expired");
}

#[tokio::test]
async fn http_empty_snapshot_revokes_existing_media_access() {
    use tower::ServiceExt as _;

    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("http-empty-snapshot.sqlite3");
    let pool = crate::sqlite::initialize_test_pool(&format!("sqlite://{}", database.display()))
        .await
        .unwrap();
    let client_id = Uuid::new_v4();
    insert_test_client(&pool, client_id, "http-owner").await;
    upsert_test_camera(&pool, client_id, client_id, "old-camera").await;
    let client_token = "A".repeat(43);
    sqlx::query("UPDATE xcocs SET token_hash = ? WHERE id = ?")
        .bind(hash_secret(&client_token).to_vec())
        .bind(client_id)
        .execute(&pool)
        .await
        .unwrap();
    let state = test_state(pool.clone());
    let (media_token, _) = issue_media_token(
        "administrator",
        client_id,
        camera_path(client_id, "main"),
        vec!["read".into()],
        None,
        &state.config,
    )
    .unwrap();
    let app = Router::new()
        .route("/snapshot", put(client_snapshot))
        .route("/pair", post(pair_client))
        .route("/media-auth", post(media_auth))
        .with_state(state);
    let unsupported = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/pair")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "protocol": "old-edge", "product": "xcos",
                        "installation_id": Uuid::new_v4(), "name": "old",
                        "client_version": "0.1", "authorization_code": "a".repeat(36)
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unsupported.status(), StatusCode::BAD_REQUEST);
    let unsupported_body = axum::body::to_bytes(unsupported.into_body(), 1_048_576)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&unsupported_body).unwrap()["code"],
        "contract_violation"
    );
    let media_body = json!({
        "user": "", "password": "", "token": media_token, "ip": "127.0.0.1",
        "action": "read", "path": camera_path(client_id, "main"),
        "protocol": "webrtc", "id": "connection", "query": "", "userAgent": ""
    });
    let media_request = || {
        axum::http::Request::builder()
            .method("POST")
            .uri("/media-auth")
            .header("content-type", "application/json")
            .body(Body::from(media_body.to_string()))
            .unwrap()
    };
    assert_eq!(
        app.clone().oneshot(media_request()).await.unwrap().status(),
        StatusCode::OK
    );

    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("PUT")
                .uri("/snapshot")
                .header("authorization", format!("Bearer {client_token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "protocol": CLIENT_PAIRING_PROTOCOL, "command_capacity": 100,
                        "cameras": [], "command_results": []
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1_048_576)
        .await
        .unwrap();
    let snapshot: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(snapshot["publish"], json!([]));
    assert_eq!(snapshot["commands"], json!([]));
    let deleted_at: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT deleted_at FROM cameras WHERE id = ?")
            .bind(client_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(deleted_at.is_some());
    assert_eq!(
        app.clone().oneshot(media_request()).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    sqlx::query("UPDATE cameras SET deleted_at = NULL, enabled = 1 WHERE id = ?")
        .bind(client_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE xcocs SET revoked_at = ? WHERE id = ?")
        .bind(Utc::now())
        .bind(client_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        app.oneshot(media_request()).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn first_snapshot_waits_for_media_path_before_returning_publish_grant() {
    use tower::ServiceExt as _;

    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("first-snapshot.sqlite3");
    let pool = crate::sqlite::initialize_test_pool(&format!("sqlite://{}", database.display()))
        .await
        .unwrap();
    let client_id = Uuid::new_v4();
    insert_test_client(&pool, client_id, "new-camera").await;
    let client_token = "B".repeat(43);
    sqlx::query("UPDATE xcocs SET token_hash = ? WHERE id = ?")
        .bind(hash_secret(&client_token).to_vec())
        .bind(client_id)
        .execute(&pool)
        .await
        .unwrap();
    let app = Router::new()
        .route("/snapshot", put(client_snapshot))
        .with_state(test_state(pool.clone()));
    let request_body = json!({
        "protocol": CLIENT_PAIRING_PROTOCOL, "command_capacity": 100,
        "cameras": [{
            "id": client_id, "name": "new-camera", "location": "door",
            "adapter_kind": "rtsp",
            "identity": { "manufacturer": null, "model": null,
                "firmware_version": null, "serial_number": null },
            "capabilities": { "video": "supported", "main_stream": "supported", "sub_stream": "unsupported",
                "local_recording": "supported", "server_recording": "supported", "ptz": "unsupported",
                "events": "unsupported", "audio_input": "unsupported", "audio_output": "unsupported" },
            "streams": [{ "profile": "main", "video_codec": "h264",
                "audio_codec": null, "width": 1920, "height": 1080,
                "frame_rate": 25.0 }],
            "has_sub_stream": false, "enabled": true,
            "storage_mode": "server", "status": "online", "health_message": null
        }],
        "command_results": []
    });
    let request = || {
        axum::http::Request::builder()
            .method("PUT")
            .uri("/snapshot")
            .header("authorization", format!("Bearer {client_token}"))
            .header("content-type", "application/json")
            .body(Body::from(request_body.to_string()))
            .unwrap()
    };
    let response = app.clone().oneshot(request()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let first: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 1_048_576)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(first["publish"], json!([]));

    let generation: i64 =
        sqlx::query_scalar("SELECT generation FROM media_desired_states WHERE camera_id = ?")
            .bind(client_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO media_actual_paths (path_name, camera_id, profile, present, ready, \
         publisher_active, recording_active, applied_generation, observed_at) \
         VALUES (?, ?, 'main', 1, 0, 0, 0, ?, ?)",
    )
    .bind(camera_path(client_id, "main"))
    .bind(client_id)
    .bind(generation)
    .bind(Utc::now())
    .execute(&pool)
    .await
    .unwrap();
    let response = app.oneshot(request()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let second: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 1_048_576)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(second["publish"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn command_admission_preserves_stop_capacity_and_unknown_receipts() {
    let root = tempfile::tempdir().unwrap();
    let pool = crate::sqlite::initialize_test_pool(&format!(
        "sqlite://{}",
        root.path().join("app.sqlite3").display()
    ))
    .await
    .unwrap();
    let client_id = Uuid::new_v4();
    insert_test_client(&pool, client_id, "budget-owner").await;
    upsert_test_camera(&pool, client_id, client_id, "camera").await;
    let now = Utc::now();
    let mut command = Uuid::nil();
    for _ in 0..127 {
        command = Uuid::new_v4();
        sqlx::query("INSERT INTO device_commands(id,camera_id,client_id,kind,payload,created_at,delivered_at,expires_at) VALUES(?,?,?,'ptz','{}',?,?,?)")
            .bind(command).bind(client_id).bind(client_id).bind(now).bind(now).bind(now+chrono::Duration::seconds(10)).execute(&pool).await.unwrap();
    }
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    assert!(matches!(
        reserve_device_command(&mut tx, client_id, false).await,
        Err(AppError::CommandCapacity)
    ));
    reserve_device_command(&mut tx, client_id, true)
        .await
        .unwrap();
    let result = DeviceCommandResult {
        id: command,
        outcome: CommandOutcome::Unknown,
        error_code: Some(CommandErrorCode::OutcomeUnknown),
    };
    validate_command_result(&result).unwrap();
    apply_client_command_results_in(&mut tx, client_id, &[result], now)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let row: (String, Option<String>) =
        sqlx::query_as("SELECT status,error FROM device_commands WHERE id=?")
            .bind(command)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row, ("expired".into(), Some("outcome_unknown".into())));
    let bad = DeviceCommandResult {
        id: command,
        outcome: CommandOutcome::Failed,
        error_code: Some(CommandErrorCode::OutcomeUnknown),
    };
    assert!(validate_command_result(&bad).is_err());
    // The obsolete result shape is rejected even with a current UUID.
    assert!(serde_json::from_value::<DeviceCommandResult>(
        json!({"id":command,"status":"failed","error":"secret"})
    )
    .is_err());
    pool.close().await;
}

#[tokio::test]
async fn delivered_ptz_result_can_arrive_after_its_delivery_deadline() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("command-result.sqlite3");
    let pool = crate::sqlite::initialize_test_pool(&format!("sqlite://{}", database.display()))
        .await
        .unwrap();
    let client_id = Uuid::new_v4();
    insert_test_client(&pool, client_id, "command-owner").await;
    upsert_test_camera(&pool, client_id, client_id, "camera").await;
    let now = Utc::now();
    let delivered = Uuid::new_v4();
    let never_delivered = Uuid::new_v4();
    for (id, delivered_at) in [
        (delivered, Some(now - chrono::Duration::seconds(5))),
        (never_delivered, None),
    ] {
        sqlx::query(
            "INSERT INTO device_commands (id, camera_id, client_id, kind, payload, \
             created_at, delivered_at, expires_at) VALUES (?, ?, ?, 'ptz', '{}', ?, ?, ?)",
        )
        .bind(id)
        .bind(client_id)
        .bind(client_id)
        .bind(now - chrono::Duration::seconds(10))
        .bind(delivered_at)
        .bind(now - chrono::Duration::seconds(1))
        .execute(&pool)
        .await
        .unwrap();
    }
    let results = [DeviceCommandResult {
        id: delivered,
        outcome: CommandOutcome::Succeeded,
        error_code: None,
    }];
    let mut transaction = pool.begin().await.unwrap();
    apply_client_command_results_in(&mut transaction, client_id, &results, now)
        .await
        .unwrap();
    transaction.commit().await.unwrap();
    let rows = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT id, status FROM device_commands WHERE client_id = ?",
    )
    .bind(client_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(rows.contains(&(delivered, "succeeded".into())));
    assert!(rows.contains(&(never_delivered, "expired".into())));
}

#[tokio::test]
async fn media_callback_rechecks_camera_and_client_authorization() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("media-auth.sqlite3");
    let pool = crate::sqlite::initialize_test_pool(&format!("sqlite://{}", database.display()))
        .await
        .unwrap();
    let client_id = Uuid::new_v4();
    insert_test_client(&pool, client_id, "media-owner").await;
    assert_eq!(
        upsert_test_camera(&pool, client_id, client_id, "camera").await,
        1
    );
    let hash = vec![3_u8; 32];
    sqlx::query("UPDATE xcocs SET token_hash = ? WHERE id = ?")
        .bind(&hash)
        .bind(client_id)
        .execute(&pool)
        .await
        .unwrap();
    let mut claims = MediaClaims {
        protocol: CONTRACT.media_jwt_protocol.clone(),
        iss: CONTRACT.media_jwt_issuer.clone(),
        aud: CONTRACT.media_jwt_audience.clone(),
        kind: CONTRACT.media_jwt_kind.clone(),
        sub: client_id.to_string(),
        camera_id: client_id,
        path: camera_path(client_id, "main"),
        actions: vec!["publish".into()],
        credential_binding: Some(crate::auth::publish_binding(&hash)),
        jti: Uuid::new_v4(),
        iat: 1,
        nbf: 1,
        exp: 2,
    };
    assert!(media_request_permitted(&pool, &claims, "publish").await);
    assert!(media_request_permitted(&pool, &claims, "read").await);
    assert!(media_request_permitted(&pool, &claims, "playback").await);
    claims.path = camera_path(Uuid::new_v4(), "main");
    assert!(!media_request_permitted(&pool, &claims, "read").await);
    claims.path = camera_path(client_id, "main");

    sqlx::query("UPDATE cameras SET enabled = 0 WHERE id = ?")
        .bind(client_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(!media_request_permitted(&pool, &claims, "read").await);
    assert!(!media_request_permitted(&pool, &claims, "publish").await);
    assert!(media_request_permitted(&pool, &claims, "playback").await);

    sqlx::query("UPDATE cameras SET deleted_at = ? WHERE id = ?")
        .bind(Utc::now())
        .bind(client_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(!media_request_permitted(&pool, &claims, "playback").await);
    sqlx::query("UPDATE cameras SET deleted_at = NULL, enabled = 1 WHERE id = ?")
        .bind(client_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE xcocs SET revoked_at = ? WHERE id = ?")
        .bind(Utc::now())
        .bind(client_id)
        .execute(&pool)
        .await
        .unwrap();
    for action in ["read", "playback", "publish"] {
        assert!(!media_request_permitted(&pool, &claims, action).await);
    }
}

#[test]
fn media_callback_accepts_null_playback_connection_id() {
    let callback: MediaAuthRequest = serde_json::from_value(json!({
        "user": "", "password": "", "token": "signed-token", "ip": "127.0.0.1",
        "action": "playback", "path": "cam_test_main", "protocol": "playback",
        "id": null, "query": "", "userAgent": ""
    }))
    .unwrap();
    assert_eq!(callback.action, "playback");
    assert_eq!(callback.id, None);
}

#[test]
fn recording_search_rejects_reversed_time_ranges() {
    let now = Utc::now();
    let earlier = now - chrono::Duration::seconds(1);
    assert!(validate_recording_range(Some(&earlier), Some(&now)).is_ok());
    assert!(validate_recording_range(Some(&now), Some(&now)).is_ok());
    assert!(validate_recording_range(None, Some(&now)).is_ok());
    assert!(matches!(
        validate_recording_range(Some(&now), Some(&earlier)),
        Err(AppError::Validation(_))
    ));
}

#[tokio::test]
async fn client_camera_snapshot_upsert_is_exact_and_preserves_soft_deleted_ownership() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("snapshot.sqlite3");
    let pool = crate::sqlite::initialize_test_pool(&format!("sqlite://{}", database.display()))
        .await
        .unwrap();
    let owner = Uuid::new_v4();
    let other = Uuid::new_v4();
    let camera = Uuid::new_v4();
    insert_test_client(&pool, owner, "owner").await;
    insert_test_client(&pool, other, "other").await;
    let installation = Uuid::new_v4();
    sqlx::query("UPDATE xcocs SET installation_id = ? WHERE id IN (?, ?)")
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
    let preserved: (String, Uuid, bool) =
        sqlx::query_as("SELECT name, client_id, deleted_at IS NOT NULL FROM cameras WHERE id = ?")
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
    let pool = crate::sqlite::initialize_test_pool(&format!("sqlite://{}", database.display()))
        .await
        .unwrap();
    let client = Uuid::new_v4();
    insert_test_client(&pool, client, "delete-owner").await;
    assert_eq!(
        upsert_test_camera(&pool, client, client, "delete-camera").await,
        1
    );
    sqlx::query("UPDATE xcocs SET status = 'revoked', revoked_at = ? WHERE id = ?")
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
        "INSERT INTO _xcss_operations (operation_id, namespace, target_key, action, \
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
    let client_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM xcocs WHERE id = ?")
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
    rejects_unknown::<AuditQuery>(json!({ "date": "2026-09-23", "unknown": true }));
    rejects_unknown::<EventLogQuery>(json!({ "date": "2026-09-23", "unknown": true }));
    rejects_unknown::<EventLogQuery>(json!({ "date": "2026-09-23", "unacknowledged": true }));
    rejects_unknown::<EventQuery>(json!({ "unacknowledged": true }));
    rejects_unknown::<MediaOperationQuery>(json!({ "date": "2026-09-23", "unknown": true }));
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
    assert!(server_log_range(None, None, None).is_err());
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

#[test]
fn log_ranges_validate_dates_and_bind_both_endpoints_to_cursors() {
    for (date, start, end) in [
        (None, None, None),
        (None, Some("2022-02-01"), None),
        (Some("2022-02-01"), Some("2022-02-01"), Some("2023-02-02")),
        (None, Some("2023-02-02"), Some("2022-02-01")),
        (None, Some("2022-02-29"), Some("2023-02-02")),
        (None, Some("0000-01-01"), Some("2023-02-02")),
        (None, Some("2022-2-1"), Some("2023-02-02")),
    ] {
        assert!(server_log_range(date, start, end).is_err());
    }
    let (start, end, scope) =
        server_log_range(None, Some("2022-02-01"), Some("2023-02-02")).unwrap();
    assert_eq!(
        start.with_timezone(&Local).format("%Y-%m-%d").to_string(),
        "2022-02-01"
    );
    assert_eq!(
        end.with_timezone(&Local).format("%Y-%m-%d").to_string(),
        "2023-02-03"
    );
    let page = log_page(
        (0..=LOG_PAGE_SIZE).map(|_| Uuid::new_v4()).collect(),
        &scope,
        "events",
        |id| *id,
    );
    let cursor = page.next_cursor.unwrap();
    assert!(cursor_id(Some(&cursor), &scope, "events").is_ok());
    assert!(cursor_id(Some(&cursor), "2022-02-01/2023-02-03", "events").is_err());
    assert!(cursor_id(Some(&cursor), &scope, "audit").is_err());
}
