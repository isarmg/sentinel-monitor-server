mod auth;
mod background;
mod camera_state;
mod config;
mod crypto;
mod doctor;
mod error;
mod history;
mod lifecycle;
mod mediamtx;
mod models;
mod protocol;
mod reconciliation;
mod release;
mod routes;
mod runtime_lock;
mod sqlite;
mod static_assets;

mod app;
mod cli;

pub use app::AppState;
pub(crate) use xcss_server_cli::CliError as CliFailure;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    cli::run().await
}
