use crate::{app, camera_state, config, doctor, release, sqlite, static_assets, CliFailure};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "xcos",
    version,
    long_version = concat!(env!("CARGO_PKG_VERSION"), " target=", env!("XCOS_BUILD_TARGET"), " source=", env!("XCOS_SOURCE_REVISION")),
    about = "Xcos camera monitoring control plane"
)]
struct Cli {
    #[arg(long, global = true, env = "XCOS_CONFIG", hide_env_values = true)]
    config: Option<PathBuf>,
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize a new state with an administrator password read from stdin.
    Init {
        #[arg(long, default_value = "admin")]
        username: String,
        #[command(flatten)]
        settings: config::Overrides,
    },
    /// Run the current initialized data and embedded Web resources.
    Run {
        #[arg(long)]
        release_root: Option<PathBuf>,
        #[command(flatten)]
        settings: config::Overrides,
    },
    /// Perform read-only current configuration and data checks.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Query actual readiness with exact service identity verification.
    Status {
        #[command(flatten)]
        settings: config::Overrides,
    },
    /// Check database read/write, credential, storage, readiness and companion contracts.
    Doctor(DoctorArgs),
    /// Print the static asset contract embedded in this build.
    #[command(hide = true)]
    StaticContract,
    /// Print the complete Foundation-generated Web inventory.
    #[command(hide = true)]
    WebAssets,
    /// Print the complete identity compiled into this binary.
    ReleaseIdentity,
    StateContract,
    /// Verify a complete immutable release with its own binary.
    VerifyRelease {
        release_root: PathBuf,
    },
    /// Print the canonical release-manifest identity header.
    #[command(hide = true)]
    ReleaseManifestHeader,
}

#[derive(Subcommand)]
enum ConfigCommand {
    Validate {
        #[command(flatten)]
        settings: config::Overrides,
    },
}

#[derive(Args)]
struct DoctorArgs {
    #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
    database_url: String,
    #[command(flatten)]
    media: MediaPaths,
    /// Skip live loopback HTTP probes; storage and companion checks still run.
    #[arg(long)]
    offline: bool,
    #[arg(
        long,
        env = "XCOS_READY_URL",
        default_value = "http://127.0.0.1:8080/readyz"
    )]
    app_ready_url: String,
    #[arg(
        long,
        env = "MEDIAMTX_READY_URL",
        default_value = "http://127.0.0.1:9997/v3/info"
    )]
    mediamtx_ready_url: String,
}

#[derive(Args)]
struct MediaPaths {
    #[arg(long, env = "MEDIAMTX_CONFIG")]
    mediamtx_config: PathBuf,
    #[arg(long, env = "MEDIAMTX_CONTRACT")]
    mediamtx_contract: PathBuf,
    #[arg(long, env = "MEDIAMTX_BINARY")]
    mediamtx_binary: PathBuf,
    #[arg(long, env = "RECORDINGS_DIR")]
    recordings_dir: PathBuf,
}

pub(crate) async fn run() -> std::process::ExitCode {
    let arguments = std::env::args_os().collect::<Vec<_>>();
    let json = arguments.iter().any(|argument| argument == "--json");
    let mut cli = match Cli::try_parse_from(arguments) {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            let _ = error.print();
            return std::process::ExitCode::SUCCESS;
        }
        Err(_) => {
            return xcss_server_cli::report_error(
                &xcss_server_cli::ErrorEnvelope::new(
                    xcss_server_cli::HttpStatus::BadRequest,
                    "Command arguments do not satisfy the current contract.",
                ),
                json,
                2,
            );
        }
    };
    if let Some(path) = cli.config.take() {
        let current = if path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "unsafe configuration path",
            ))
        } else {
            std::path::absolute(path)
        };
        match current {
            Ok(path) => cli.config = Some(path),
            Err(_) => {
                return xcss_server_cli::report_error(
                    &xcss_config::ConfigError::new(
                        xcss_config::Reason::InvalidValue,
                        "/config",
                        xcss_config::ConfigSource::CommandLine,
                    )
                    .envelope(),
                    json,
                    2,
                )
            }
        }
    }
    use tracing_subscriber::prelude::*;
    let log_layer =
        xcss_log::FoundationStructuredLayer::new("xcos").expect("static service identity");
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=info".into()),
        )
        .with(log_layer.clone())
        .init();
    match execute(cli, &log_layer).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            let envelope = if let Some(error) = error.downcast_ref::<xcss_config::ConfigError>() {
                error.envelope()
            } else if let Some(error) = error.downcast_ref::<xcss_state_file::Error>() {
                xcss_server_cli::state_error(error)
            } else if let Some(error) = error.downcast_ref::<xcss_server_cli::SnapshotError>() {
                xcss_server_cli::snapshot_error(error)
            } else if let Some(auth_error) = error.downcast_ref::<xcss_admin_auth::Error>() {
                let path = match auth_error {
                    xcss_admin_auth::Error::InvalidAdministratorUsername => Some("/username"),
                    xcss_admin_auth::Error::InvalidPassword => Some("/password"),
                    xcss_admin_auth::Error::InvalidPasswordHash => Some("/password_hash"),
                    _ => None,
                };
                if let Some(path) = path {
                    xcss_server_cli::ErrorEnvelope::with_code(
                        xcss_server_cli::ErrorCode::new("auth.invalid_request")
                            .expect("static code"),
                        "Administrator credentials do not satisfy the current contract.",
                    )
                    .with_detail("reason", "INVALID_VALUE")
                    .with_detail("path", path)
                } else {
                    xcss_server_cli::ErrorEnvelope::with_code(
                        xcss_server_cli::ErrorCode::new("auth.operation_failed")
                            .expect("static code"),
                        "The administrator operation could not be completed.",
                    )
                }
            } else if let Some(error) = error.downcast_ref::<CliFailure>() {
                error.0.clone()
            } else if error
                .downcast_ref::<xcss_schema_identity::Error>()
                .is_some()
                || matches!(
                    error.downcast_ref::<xcss_sqlite::Error>(),
                    Some(
                        xcss_sqlite::Error::SchemaBudgetExceeded
                            | xcss_sqlite::Error::ProductMetadataTableMissing
                            | xcss_sqlite::Error::ProductMetadataStorageClass { .. }
                            | xcss_sqlite::Error::SchemaIdentity(_)
                    )
                )
            {
                xcss_server_cli::ErrorEnvelope::with_code(
                    xcss_server_cli::ErrorCode::new("contract_violation").expect("static code"),
                    "The actual database structure does not satisfy the current schema contract.",
                )
            } else {
                tracing::error!(%error,"Xcos command failed");
                xcss_server_cli::ErrorEnvelope::with_code(
                    xcss_server_cli::ErrorCode::new("xcos.command_failed").expect("static code"),
                    "The command failed. Check the service diagnostic log.",
                )
            };
            xcss_server_cli::report_error(&envelope, json, 1)
        }
    }
}

async fn execute(cli: Cli, log_layer: &xcss_log::FoundationStructuredLayer) -> anyhow::Result<()> {
    match cli.command {
        Command::Init { username, settings } => {
            let loaded = config::load(cli.config.as_deref(), cli.data_dir.as_deref(), &settings)?;
            let settings = loaded.value;
            let config = settings.effective()?;
            let root = settings.data_directory()?;
            let username = xcss_admin_auth::normalize_administrator_username(&username)?;
            use std::io::Read;
            let mut password = String::new();
            std::io::stdin().take(4097).read_to_string(&mut password)?;

            let password = password.trim_end_matches(['\r', '\n']);
            xcss_admin_auth::validate_password(password)?;
            xcss_server_cli::create_empty_private_directory(&root).map_err(CliFailure)?;
            let state_directory = xcss_state_file::PrivateStateDirectory::open(&root)?;
            state_directory.verify_no_pending_maintenance()?;
            let _maintenance = state_directory.try_maintenance_lock()?;
            state_directory.verify_no_pending_maintenance()?;
            xcss_state_file::PrivateStateDirectory::create(&config.runtime_directory)?;
            xcss_server_cli::create_runtime_log_directory(&root).map_err(CliFailure)?;
            let recordings = settings
                .recordings_directory
                .clone()
                .unwrap_or_else(|| root.join("recordings"));
            xcss_state_file::PrivateStateDirectory::create(&recordings)?;
            sqlite::initialize_with_administrator(&settings.database()?, &username, password)
                .await?;
            tracing::info!(event = "common.initialization.completed");
            println!(
                "{}",
                if cli.json {
                    serde_json::json!({"status":"initialized","schema_identity":sqlite::current_schema_identity()?}).to_string()
                } else {
                    "current data initialized".into()
                }
            );
            Ok(())
        }
        Command::Run {
            release_root,
            settings,
        } => {
            let loaded = config::load(cli.config.as_deref(), cli.data_dir.as_deref(), &settings)?;
            let release_root = release_root.or_else(|| loaded.value.release_root.clone());
            if let Some(root) = &release_root {
                release::verify_release(root)?;
            } else {
                release::ensure_unbound_run()?;
            }
            app::serve(
                loaded.value.effective()?,
                release_root.as_deref(),
                log_layer,
            )
            .await
        }
        Command::Config {
            command: ConfigCommand::Validate { settings },
        } => {
            let loaded = config::load(cli.config.as_deref(), cli.data_dir.as_deref(), &settings)?;
            let config = loaded.value.effective()?;
            let root = loaded.value.data_directory()?;
            xcss_state_file::PrivateStateDirectory::open(&root)?;
            xcss_server_cli::validate_runtime_log_directory(&root).map_err(CliFailure)?;
            xcss_state_file::PrivateStateDirectory::open(&config.runtime_directory)?;
            let database = loaded.value.database()?;
            let recordings = loaded
                .value
                .recordings_directory
                .clone()
                .unwrap_or_else(|| root.join("recordings"));
            xcss_state_file::PrivateStateDirectory::open(&recordings)?;
            let (media_config, media_contract, media_binary) = loaded.value.companion_paths()?;
            doctor::verify_companion(&media_contract, &media_binary, &media_config, &recordings)
                .await?;
            let snapshot = sqlite::current_validation_snapshot(&database).await?;
            let validation = async {
                xcss_sqlite::integrity_check(snapshot.pool()).await?;
                xcss_sqlite::foreign_key_check(snapshot.pool()).await?;
                let store =
                    xcss_admin_sqlite::SqliteAdministratorStore::new(snapshot.pool().clone());
                store.validate_all_administrators().await?;
                doctor::verify_credentials_on_snapshot(snapshot.pool(), &config.credentials_key)
                    .await?;
                camera_state::validate_current_camera_data(snapshot.pool()).await?;
                Ok::<_, anyhow::Error>(())
            }
            .await;
            snapshot.close().await;
            validation?;
            let mut state_paths = vec![root, config.runtime_directory, recordings];
            if loaded.value.release_root.is_none() {
                state_paths.extend([media_config, media_contract]);
            }
            if let Some(path) = cli.config {
                state_paths.push(path);
            }
            let report = serde_json::json!({"status":"valid","schema_identity":sqlite::current_schema_identity()?,"state_paths":state_paths,"sources":loaded.sources});
            if cli.json {
                println!("{report}");
            } else {
                println!("configuration and current data are valid");
            }
            Ok(())
        }
        Command::Status { settings } => {
            let loaded = config::load(cli.config.as_deref(), cli.data_dir.as_deref(), &settings)?;
            let report = xcss_server_cli::query_status(loaded.value.bind_addr, "xcos")
                .await
                .map_err(CliFailure)?;
            xcss_server_cli::print_report(&report, cli.json)?;
            if !report.ready {
                std::process::exit(1);
            }
            Ok(())
        }
        Command::Doctor(args) => {
            let key = required_credentials_key()?;
            let report = doctor::run(&doctor::DoctorOptions {
                database_url: args.database_url,
                mediamtx_config: args.media.mediamtx_config,
                mediamtx_contract: args.media.mediamtx_contract,
                mediamtx_binary: args.media.mediamtx_binary,
                recordings_directory: args.media.recordings_dir,
                credentials_key: key,
                app_ready_url: args.app_ready_url,
                mediamtx_ready_url: args.mediamtx_ready_url,
                offline: args.offline,
            })
            .await?;
            println!("{}", serde_json::to_string(&report)?);
            Ok(())
        }
        Command::WebAssets => {
            static_assets::embedded_contract_sha256()?;
            print!("{}", static_assets::MANIFEST);
            Ok(())
        }
        Command::StaticContract => {
            println!("{}", static_assets::embedded_contract_sha256()?);
            Ok(())
        }
        Command::ReleaseIdentity => {
            println!("{}", serde_json::to_string(&release::standard_identity()?)?);
            Ok(())
        }
        Command::StateContract => {
            println!("{}", String::from_utf8(release::state_contract_bytes()?)?);
            Ok(())
        }
        Command::VerifyRelease { release_root } => {
            println!(
                "{}",
                serde_json::to_string(&release::verify_release(&release_root)?)?
            );
            Ok(())
        }
        Command::ReleaseManifestHeader => {
            print!("{}", release::manifest_header()?);
            Ok(())
        }
    }
}

fn required_credentials_key() -> anyhow::Result<[u8; 32]> {
    let encoded = std::env::var("CREDENTIALS_KEY").map_err(|_| {
        anyhow::anyhow!("CREDENTIALS_KEY is required and must remain in secret management")
    })?;
    let decoded = STANDARD
        .decode(encoded)
        .map_err(|_| anyhow::anyhow!("CREDENTIALS_KEY must be valid base64"))?;
    decoded
        .try_into()
        .map_err(|_| anyhow::anyhow!("CREDENTIALS_KEY must decode to exactly 32 bytes"))
}
