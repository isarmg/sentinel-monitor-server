use crate::{
    background, camera_state, config::Config, crypto::SecretBox, doctor, lifecycle,
    mediamtx::MediaMtxClient, models::EventRecord, reconciliation, routes, runtime_lock, sqlite,
    static_assets, CliFailure,
};
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::{broadcast, Notify};

#[derive(Clone)]
pub struct AppState {
    pub web_directory: Option<Arc<xcss::web_assets::DirectoryAssets>>,
    pub config: Arc<Config>,
    pub pool: SqlitePool,
    pub secrets: SecretBox,
    pub http: reqwest::Client,
    pub media: MediaMtxClient,
    pub events: broadcast::Sender<EventRecord>,
    pub reconcile_notify: Arc<Notify>,
    pub administrator:
        Arc<xcss::admin_core::AdministratorService<xcss::admin_sqlite::SqliteAdministratorStore>>,
    pub administrator_origin: xcss::admin_auth::AdministratorOriginMode,
    pub scope: xcss::server_runtime::WorkScope,
}

pub(crate) async fn serve(
    config: Config,
    release_root: Option<&std::path::Path>,
    log_layer: &xcss::log::XcssStructuredLayer,
) -> anyhow::Result<()> {
    let config = Arc::new(config);
    if release_root.is_some() {
        anyhow::ensure!(
            config.static_dir.is_none(),
            "formal releases cannot override embedded Web assets"
        );
    }
    let web_directory = config
        .static_dir
        .as_ref()
        .map(xcss::web_assets::DirectoryAssets::new)
        .transpose()?
        .map(Arc::new);
    static_assets::embedded_contract_sha256()?;
    // Reject missing/unknown data before creating operational files or listening.
    let database = sqlite::database_path(&config.database_url)?;
    sqlite::validate_current_database(&database)?;
    let application_lock = Arc::new(runtime_lock::ApplicationLock::acquire(
        &config.database_url,
        &config.runtime_directory,
    )?);
    if config.development_mode {
        tracing::warn!(
            address = %config.bind_addr,
            "development mode uses loopback-only cookies without Secure"
        );
    }
    let snapshot = sqlite::current_validation_snapshot(&database).await?;
    let validation = async {
        xcss::sqlite::integrity_check(snapshot.pool()).await?;
        xcss::sqlite::foreign_key_check(snapshot.pool()).await?;
        let store = xcss::admin_sqlite::SqliteAdministratorStore::new(snapshot.pool().clone());
        use xcss::admin_core::AdministratorStore as _;
        anyhow::ensure!(
            store.administrator_count().await? > 0,
            "explicit initialization is required"
        );
        store.validate_all_administrators().await?;
        doctor::verify_credentials_on_snapshot(snapshot.pool(), &config.credentials_key).await?;
        camera_state::validate_current_camera_data(snapshot.pool()).await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    snapshot.close().await;
    validation?;
    let pool = sqlite::open_pool(&config.database_url).await?;
    // Retain the owner through every startup failure until all connections close.
    let result = async {
        application_lock.validate_open_database()?;

        let http = reqwest::Client::builder()
            .timeout(config.request_timeout)
            .user_agent(concat!("xcos/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let stream_http = reqwest::Client::builder()
            .connect_timeout(config.request_timeout)
            .read_timeout(config.request_timeout)
            .user_agent(concat!("xcos/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let media = MediaMtxClient::new(
            http.clone(),
            stream_http,
            config.mediamtx_api_url.clone(),
            config.mediamtx_playback_url.clone(),
        );
        let (events, _) = broadcast::channel(256);
        let reconcile_notify = Arc::new(Notify::new());
        let administrator = Arc::new(xcss::admin_core::AdministratorService::new(
            xcss::admin_sqlite::SqliteAdministratorStore::new(pool.clone()),
        ));
        use xcss::admin_core::AdministratorStore as _;
        if administrator.store().administrator_count().await? == 0 {
            anyhow::bail!(
                "current administrator state is missing; explicit init is required for new data"
            );
        }
        administrator.store().validate_all_administrators().await?;
        camera_state::validate_current_camera_data(&pool).await?;
        let administrator_origin = if config.development_mode {
            xcss::admin_auth::AdministratorOriginMode::LoopbackDevelopmentHttp
        } else {
            xcss::admin_auth::AdministratorOriginMode::ProductionHttps
        };
        let scope = xcss::server_runtime::WorkScope::new();
        let state = AppState {
            scope,
            web_directory,
            secrets: SecretBox::new(&config.credentials_key),
            config: config.clone(),
            pool: pool.clone(),
            http,
            media,
            events,
            reconcile_notify,
            administrator,
            administrator_origin,
        };

        reconciliation::validate_stored_camera_credentials(&state).await?;
        let data_dir = sqlite::database_path(&state.config.database_url)?
            .parent()
            .expect("validated database parent")
            .to_path_buf();
        xcss::server_cli::validate_runtime_log_directory(&data_dir).map_err(CliFailure)?;
        let logs = data_dir.join("logs");
        log_layer.set_rotating_file(xcss::log::RotatingLogFile::open(
            logs,
            "server",
            xcss::log::LogRetention::default(),
        )?)?;
        tracing::info!(event = "common.config.loaded");
        let recovered = reconciliation::recover_interrupted_operations(&state.pool).await?;
        if recovered > 0 {
            tracing::warn!(
                recovered_operations = recovered,
                "expired media operation leases were marked unknown for safe reconciliation"
            );
        }
        let health_pool = state.pool.clone();
        let health_media = state.media.clone();
        let reconcile_state = state.clone();
        let status_state = state.clone();
        let audit_pool = state.pool.clone();
        let audit_delivery_pool = state.pool.clone();
        let operations_pool = state.pool.clone();
        let runtime =
            xcss::server_runtime::ServerRuntime::builder(xcss::server_runtime::ProductDescriptor {
                id: "xcos".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                xcss_revision: env!("XCSS_REVISION").into(),
                profile: "server-control-plane".into(),
                capabilities: vec![
                    "embedded-web".into(),
                    "admin-persistent".into(),
                    "server-runtime".into(),
                    "server-health".into(),
                    "durable-operations".into(),
                    "secure-http".into(),
                    "secret-envelope".into(),
                ],
            })
            .with_schema_identity(sqlite::current_schema_identity()?)
            .register_metric(
                xcss::server_runtime::DiagnosticMetric::AuditBacklog,
                move || {
                    let store = xcss::operations::SqliteOperationStore::new(audit_pool.clone());
                    async move { store.pending_audit_count().await.ok() }
                },
            )
            .register_metric(
                xcss::server_runtime::DiagnosticMetric::OperationBacklog,
                move || {
                    let store =
                        xcss::operations::SqliteOperationStore::new(operations_pool.clone());
                    async move { store.active_operation_count().await.ok() }
                },
            )
            .register_health_check(
                "mediamtx",
                xcss::server_runtime::health_check(move || {
                    let media = health_media.clone();
                    async move { media.health().await }
                }),
            )
            .register_health_check(
                "database",
                xcss::server_runtime::health_check(move || {
                    let pool = health_pool.clone();
                    async move {
                        sqlx::query_scalar::<_, i64>("SELECT 1")
                            .fetch_one(&pool)
                            .await
                            .is_ok_and(|value| value == 1)
                    }
                }),
            )
            .register_background_task(
                "media-reconciliation",
                xcss::server_runtime::TaskCriticality::Critical,
                move |shutdown| background::reconcile_loop(reconcile_state, shutdown),
            )
            .register_background_task(
                "operation-audit",
                xcss::server_runtime::TaskCriticality::Degrading,
                move |shutdown| background::operation_audit_loop(audit_delivery_pool, shutdown),
            )
            .register_background_task(
                "camera-status",
                xcss::server_runtime::TaskCriticality::Degrading,
                move |shutdown| background::status_loop(status_state, shutdown),
            )
            .build()
            .await?;
        let signals = xcss::server_runtime::ProcessSignals::install()?;
        let listeners = xcss::server_runtime::BoundListeners::bind([config.bind_addr])?;
        tracing::info!(event = "common.runtime.started");
        let mut transport = xcss::server_runtime::HttpServer::new(listeners, signals);
        transport.participant = Some(Arc::new(lifecycle::Lifecycle {
            scope: state.scope.clone(),
            pool: state.pool.clone(),
            lock: std::sync::Mutex::new(Some(application_lock.clone())),
        }));
        let runtime_handle = runtime.handle();
        tracing::info!(address = %config.bind_addr, "xcos monitor started");
        if let Err(error) = runtime
            .serve(transport, routes::router(state, runtime_handle)?)
            .await
        {
            if matches!(error, xcss::server_runtime::Error::ShutdownIncomplete(_)) {
                eprintln!("{error}");
                std::process::exit(1);
            }
            return Err(error.into());
        }
        tracing::info!(event = "common.runtime.stopped");
        Ok::<_, anyhow::Error>(())
    }
    .await;
    pool.close().await;
    result
}
