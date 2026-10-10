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
        match persist_status_observation(
            &pool,
            camera,
            "offline",
            None,
            Utc::now(),
            Utc::now() + chrono::Duration::seconds(60),
            limits,
        )
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
    let observations = sqlx::query_as::<_, CameraRecord>(CAMERA_SELECT)
        .fetch_all(&pool)
        .await
        .unwrap();
    let rejected = observations
        .iter()
        .find(|row| row.id == cameras[0].id)
        .unwrap();
    assert_eq!(
        (rejected.last_observed_at, rejected.observation_expires_at),
        (None, None),
        "rolled-back status cannot publish a fresh observation"
    );
    let accepted = observations
        .iter()
        .find(|row| row.id == cameras[1].id)
        .unwrap();
    assert!(accepted.last_observed_at.is_some() && accepted.observation_expires_at.is_some());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        2
    );
}

use std::{
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
    time::Duration,
};
fn test_state(pool: sqlx::SqlitePool, endpoint: &str) -> AppState {
    let config = Arc::new(crate::config::Config {
        bind_addr: "127.0.0.1:8080".parse().unwrap(),
        database_url: "sqlite://unused".into(),
        jwt_secret: vec![9_u8; 32],
        credentials_key: [7_u8; 32],
        runtime_directory: "/tmp/xcos-test".into(),
        development_mode: true,
        media_token_ttl: Duration::from_secs(120),
        mediamtx_api_url: endpoint.into(),
        mediamtx_playback_url: endpoint.into(),
        public_webrtc_base_url: "/media-webrtc".into(),
        public_hls_base_url: "/media-hls".into(),
        public_rtsp_publish_base_url: "rtsp://127.0.0.1:8554".into(),
        status_interval: Duration::from_secs(10),
        reconcile_interval: Duration::from_secs(60),
        request_timeout: Duration::from_secs(2),
        static_dir: None,
    });
    let http = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(1))
        .build()
        .unwrap();
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
            endpoint.into(),
            endpoint.into(),
        ),
        events,
        reconcile_notify: Arc::new(tokio::sync::Notify::new()),
        administrator: Arc::new(xcss::admin_core::AdministratorService::new(
            xcss::admin_sqlite::SqliteAdministratorStore::new(pool),
        )),
        administrator_origin: xcss::admin_auth::AdministratorOriginMode::LoopbackDevelopmentHttp,
    }
}

async fn insert_camera(state: &AppState, id: Uuid, seen: chrono::DateTime<Utc>) {
    use sha2::Digest;
    let code = id.simple().to_string() + "abcd";
    let encrypted = state
        .secrets
        .encrypt_client_authorization(&id.to_string(), &code)
        .unwrap();
    sqlx::query("INSERT INTO xcocs(id,name,authorization_code_enc,authorization_code_hash,status,last_seen_at,created_at,updated_at) VALUES(?,'status fixture',?,?,'online',?,?,?)")
        .bind(id).bind(encrypted).bind(sha2::Sha256::digest(code.as_bytes()).to_vec())
        .bind(seen).bind(seen).bind(seen).execute(&state.pool).await.unwrap();
    let capabilities =
        serde_json::to_string(&crate::models::DeviceCapabilities::default()).unwrap();
    sqlx::query("INSERT INTO cameras(id,name,client_id,client_camera_id,adapter_kind,capabilities_json,status,device_status,last_seen_at,created_at,updated_at) VALUES(?,'status fixture',?,?,'rtsp',?,'online','online',?,?,?)")
        .bind(id).bind(id).bind(id.to_string()).bind(capabilities)
        .bind(seen).bind(seen).bind(seen).execute(&state.pool).await.unwrap();
}

async fn camera_view(pool: &sqlx::SqlitePool, id: Uuid) -> crate::models::CameraView {
    let record = sqlx::query_as::<_, CameraRecord>(sqlx::AssertSqlSafe(format!(
        "{CAMERA_SELECT} AND id = ?"
    )))
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap();
    crate::models::CameraView::from_record(&record).unwrap()
}

#[tokio::test]
async fn failed_inventory_preserves_known_state_and_marks_observation_stale_or_unknown() {
    use crate::models::ObservationStatus;
    let id = Uuid::new_v4();
    let never_observed = Uuid::new_v4();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let mode = Arc::new(AtomicU8::new(0));
    let server_mode = mode.clone();
    let app = axum::Router::new()
        .route("/v3/info", axum::routing::get(|| async { axum::Json(json!({"version":"v1.20.0"})) }))
        .route("/v3/paths/list", axum::routing::get(move || {
            let mode = server_mode.clone();
            async move {
                use axum::http::StatusCode;
                let (status, body) = match mode.load(Ordering::SeqCst) {
                    0 => (StatusCode::SERVICE_UNAVAILABLE, json!({"error":"fixture inventory unavailable"})),
                    1 => (StatusCode::OK, json!({"itemCount":0,"pageCount":1,"items":[]})),
                    2 => (StatusCode::OK, json!({"itemCount":1,"pageCount":1,"items":[{"name":camera_path(id,"main"),"ready":true,"readers":[],"tracks":[]}]})),
                    _ => (StatusCode::OK, json!({"items":"invalid inventory"})),
                };
                (status, axum::Json(body))
            }
        }));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let directory = tempfile::tempdir().unwrap();
    let pool = crate::sqlite::initialize_test_pool(&format!(
        "sqlite://{}",
        directory.path().join("status.sqlite3").display()
    ))
    .await
    .unwrap();
    let state = test_state(pool.clone(), &endpoint);
    let old = Utc::now() - chrono::Duration::days(1);
    insert_camera(&state, id, old).await;
    insert_camera(&state, never_observed, old).await;
    sqlx::query("UPDATE cameras SET last_observed_at = ?, observation_expires_at = ? WHERE id = ?")
        .bind(old)
        .bind(old + chrono::Duration::seconds(30))
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        camera_view(&pool, id).await.observation_status,
        ObservationStatus::Stale
    );
    assert!(state.media.health().await);
    for _ in 0..3 {
        assert!(refresh_statuses(&state).await.is_err());
    }
    let view = camera_view(&pool, id).await;
    assert_eq!(view.status, "online");
    assert_eq!(view.device_status, "online");
    assert_eq!(view.last_seen_at, Some(old));
    assert_eq!(view.last_observed_at, Some(old));
    assert_eq!(view.observation_status, ObservationStatus::Stale);
    assert_eq!(view.observation_expires_at, None);
    let unknown = camera_view(&pool, never_observed).await;
    assert_eq!(unknown.observation_status, ObservationStatus::Unknown);
    assert_eq!(unknown.last_observed_at, None);
    assert_eq!(unknown.status, "online");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );

    // A successful empty inventory is actual evidence of missing media paths.
    mode.store(1, Ordering::SeqCst);
    refresh_statuses(&state).await.unwrap();
    let offline = camera_view(&pool, id).await;
    assert_eq!(offline.status, "offline");
    assert_eq!(offline.device_status, "online");
    assert_eq!(offline.observation_status, ObservationStatus::Fresh);
    assert!(offline.last_observed_at.unwrap() > old);
    assert_eq!(
        offline.last_seen_at,
        Some(old),
        "an offline observation must not become last-online time"
    );
    let first_observation = offline.last_observed_at;
    refresh_statuses(&state).await.unwrap();
    let unchanged = camera_view(&pool, id).await;
    assert_eq!(unchanged.status, "offline");
    assert!(
        unchanged.last_observed_at > first_observation,
        "unchanged offline status still has a fresh successful observation"
    );

    mode.store(2, Ordering::SeqCst);
    refresh_statuses(&state).await.unwrap();
    let online = camera_view(&pool, id).await;
    assert_eq!(online.status, "online");
    assert_eq!(online.observation_status, ObservationStatus::Fresh);
    assert_eq!(online.last_seen_at, online.last_observed_at);
    let last_success = online.last_observed_at;
    mode.store(3, Ordering::SeqCst);
    assert!(refresh_statuses(&state).await.is_err());
    let stale = camera_view(&pool, id).await;
    assert_eq!(stale.status, "online");
    assert_eq!(stale.last_observed_at, last_success);
    assert_eq!(stale.observation_status, ObservationStatus::Stale);
    let serialized = serde_json::to_value(stale).unwrap();
    assert_eq!(serialized["observation_status"], "stale");
    assert_eq!(serialized["status"], "online");
    assert!(!serialized["last_observed_at"].is_null());

    // Restart invalidation also retains the success timestamp and known state.
    mode.store(2, Ordering::SeqCst);
    refresh_statuses(&state).await.unwrap();
    let before_restart = camera_view(&pool, id).await;
    invalidate_status_observations(&pool).await.unwrap();
    let after_restart = camera_view(&pool, id).await;
    assert_eq!(after_restart.observation_status, ObservationStatus::Stale);
    assert_eq!(
        after_restart.last_observed_at,
        before_restart.last_observed_at
    );
    assert_eq!(after_restart.status, "online");
    drop(state);
    pool.close().await;
    let reopened = xcss::sqlite::open_existing(
        directory.path().join("status.sqlite3"),
        xcss::sqlite::PoolOptions::new(2),
    )
    .await
    .unwrap();
    let persisted = camera_view(&reopened, id).await;
    assert_eq!(persisted.observation_status, ObservationStatus::Stale);
    assert_eq!(persisted.last_observed_at, after_restart.last_observed_at);
    assert_eq!(persisted.status, "online");
    reopened.close().await;
    server.abort();
    let _ = server.await;
}
