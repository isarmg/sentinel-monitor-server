use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8080";

#[derive(Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub database_url: String,
    pub jwt_secret: Vec<u8>,
    pub credentials_key: [u8; 32],
    pub runtime_directory: PathBuf,
    pub development_mode: bool,
    pub media_token_ttl: Duration,
    pub mediamtx_api_url: String,
    pub mediamtx_playback_url: String,
    pub public_webrtc_base_url: String,
    pub public_hls_base_url: String,
    pub public_rtsp_publish_base_url: String,
    pub status_interval: Duration,
    pub reconcile_interval: Duration,
    pub request_timeout: Duration,
    pub static_dir: Option<PathBuf>,
}

/// The current JSON contract. Secrets never appear in a diagnostic report.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Immutable bundle authority; `current` is the one controlled deployment pointer.
    pub release_root: Option<PathBuf>,
    pub recordings_directory: Option<PathBuf>,
    pub mediamtx_config: Option<PathBuf>,
    pub mediamtx_contract: Option<PathBuf>,
    pub mediamtx_binary: Option<PathBuf>,
    pub bind_addr: SocketAddr,
    pub data_dir: Option<PathBuf>,
    pub database_url: Option<String>,
    pub jwt_secret: Option<String>,
    pub credentials_key: Option<String>,
    pub runtime_directory: Option<PathBuf>,
    pub app_env: String,
    pub media_token_ttl_secs: u64,
    pub mediamtx_api_url: String,
    pub mediamtx_playback_url: String,
    pub public_webrtc_base_url: String,
    pub public_hls_base_url: String,
    pub public_rtsp_publish_base_url: String,
    pub status_interval_secs: u64,
    pub reconcile_interval_secs: u64,
    pub request_timeout_secs: u64,
    pub static_dir: Option<PathBuf>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            release_root: None,
            recordings_directory: None,
            mediamtx_config: None,
            mediamtx_contract: None,
            mediamtx_binary: None,
            bind_addr: DEFAULT_BIND_ADDR.parse().expect("default address"),
            data_dir: None,
            database_url: None,
            jwt_secret: None,
            credentials_key: None,
            runtime_directory: None,
            app_env: "production".into(),
            media_token_ttl_secs: 120,
            mediamtx_api_url: "http://127.0.0.1:9997".into(),
            mediamtx_playback_url: "http://127.0.0.1:9996".into(),
            public_webrtc_base_url: "/media-webrtc".into(),
            public_hls_base_url: "/media-hls".into(),
            public_rtsp_publish_base_url: "rtsp://127.0.0.1:8554".into(),
            status_interval_secs: 10,
            reconcile_interval_secs: 60,
            request_timeout_secs: 20,
            static_dir: None,
        }
    }
}
#[derive(Clone, Default, clap::Args)]
pub struct Overrides {
    #[arg(long)]
    pub bind: Option<SocketAddr>,
    #[arg(long)]
    pub database_url: Option<String>,
    #[arg(long, num_args=0..=1, default_missing_value="true")]
    pub development_mode: Option<bool>,
}

pub fn load(
    file: Option<&Path>,
    data_dir: Option<&Path>,
    cli: &Overrides,
) -> anyhow::Result<xcss::config::Loaded<Settings>> {
    use xcss::config::{EnvMapping, EnvValueKind, Override};
    let mappings = [
        EnvMapping {
            variable: "XCOS_RELEASE_ROOT",
            path: "/release_root",
            kind: EnvValueKind::String,
        },
        EnvMapping {
            variable: "RECORDINGS_DIR",
            path: "/recordings_directory",
            kind: EnvValueKind::String,
        },
        EnvMapping {
            variable: "MEDIAMTX_CONFIG",
            path: "/mediamtx_config",
            kind: EnvValueKind::String,
        },
        EnvMapping {
            variable: "MEDIAMTX_CONTRACT",
            path: "/mediamtx_contract",
            kind: EnvValueKind::String,
        },
        EnvMapping {
            variable: "MEDIAMTX_BINARY",
            path: "/mediamtx_binary",
            kind: EnvValueKind::String,
        },
        EnvMapping {
            variable: "BIND_ADDR",
            path: "/bind_addr",
            kind: EnvValueKind::String,
        },
        EnvMapping {
            variable: "XCOS_DATA_DIR",
            path: "/data_dir",
            kind: EnvValueKind::String,
        },
        EnvMapping {
            variable: "DATABASE_URL",
            path: "/database_url",
            kind: EnvValueKind::String,
        },
        EnvMapping {
            variable: "APP_JWT_SECRET",
            path: "/jwt_secret",
            kind: EnvValueKind::String,
        },
        EnvMapping {
            variable: "CREDENTIALS_KEY",
            path: "/credentials_key",
            kind: EnvValueKind::String,
        },
        EnvMapping {
            variable: "XCOS_RUNTIME_DIR",
            path: "/runtime_directory",
            kind: EnvValueKind::String,
        },
        EnvMapping {
            variable: "APP_ENV",
            path: "/app_env",
            kind: EnvValueKind::String,
        },
        EnvMapping {
            variable: "MEDIA_TOKEN_TTL_SECS",
            path: "/media_token_ttl_secs",
            kind: EnvValueKind::UnsignedInteger,
        },
        EnvMapping {
            variable: "MEDIAMTX_API_URL",
            path: "/mediamtx_api_url",
            kind: EnvValueKind::String,
        },
        EnvMapping {
            variable: "MEDIAMTX_PLAYBACK_URL",
            path: "/mediamtx_playback_url",
            kind: EnvValueKind::String,
        },
        EnvMapping {
            variable: "PUBLIC_WEBRTC_BASE_URL",
            path: "/public_webrtc_base_url",
            kind: EnvValueKind::String,
        },
        EnvMapping {
            variable: "PUBLIC_HLS_BASE_URL",
            path: "/public_hls_base_url",
            kind: EnvValueKind::String,
        },
        EnvMapping {
            variable: "PUBLIC_RTSP_PUBLISH_BASE_URL",
            path: "/public_rtsp_publish_base_url",
            kind: EnvValueKind::String,
        },
        EnvMapping {
            variable: "STATUS_INTERVAL_SECS",
            path: "/status_interval_secs",
            kind: EnvValueKind::UnsignedInteger,
        },
        EnvMapping {
            variable: "RECONCILE_INTERVAL_SECS",
            path: "/reconcile_interval_secs",
            kind: EnvValueKind::UnsignedInteger,
        },
        EnvMapping {
            variable: "REQUEST_TIMEOUT_SECS",
            path: "/request_timeout_secs",
            kind: EnvValueKind::UnsignedInteger,
        },
        EnvMapping {
            variable: "XCSS_DEV_WEB_DIR",
            path: "/static_dir",
            kind: EnvValueKind::String,
        },
    ];
    let mut invalid_unicode = false;
    let environment =
        xcss::config::read_environment(&mappings, |name| match std::env::var(name) {
            Ok(value) => Some(value),
            Err(std::env::VarError::NotPresent) => None,
            Err(_) => {
                invalid_unicode = true;
                None
            }
        })?;
    anyhow::ensure!(
        !invalid_unicode,
        "mapped environment settings must be valid Unicode"
    );
    let bytes = file.map(xcss::config::read_private_file).transpose()?;
    let mut explicit = Vec::new();
    if let Some(value) = data_dir {
        explicit.push(Override::new("/data_dir", serde_json::json!(value)));
    }
    if let Some(value) = cli.bind {
        explicit.push(Override::new("/bind_addr", serde_json::json!(value)));
    }
    if let Some(value) = &cli.database_url {
        explicit.push(Override::new("/database_url", serde_json::json!(value)));
    }
    if let Some(value) = cli.development_mode {
        explicit.push(Override::new(
            "/app_env",
            serde_json::json!(if value { "development" } else { "production" }),
        ));
    }
    Ok(xcss::config::resolve_validated(
        &Settings::default(),
        bytes.as_deref(),
        &environment,
        &explicit,
        validate_intrinsic,
    )?)
}

fn validate_intrinsic(
    value: &Settings,
    source: xcss::config::ConfigSource,
) -> Result<(), xcss::config::ConfigError> {
    let invalid =
        |path| xcss::config::ConfigError::new(xcss::config::Reason::InvalidValue, path, source);
    for (field, path) in [
        ("/release_root", &value.release_root),
        ("/data_dir", &value.data_dir),
        ("/runtime_directory", &value.runtime_directory),
        ("/static_dir", &value.static_dir),
        ("/recordings_directory", &value.recordings_directory),
        ("/mediamtx_config", &value.mediamtx_config),
        ("/mediamtx_contract", &value.mediamtx_contract),
        ("/mediamtx_binary", &value.mediamtx_binary),
    ] {
        if path.as_ref().is_some_and(|path| {
            !path.is_absolute()
                || path.components().any(|c| {
                    matches!(
                        c,
                        std::path::Component::ParentDir | std::path::Component::CurDir
                    )
                })
        }) {
            return Err(invalid(field));
        }
    }
    if let Some(root) = &value.release_root {
        if !root.ends_with("opt/isarmg/xcos/current")
            && !root.ends_with(format!(
                "opt/isarmg/xcos/releases/{}",
                env!("CARGO_PKG_VERSION")
            ))
        {
            return Err(invalid("/release_root"));
        }
        if value.mediamtx_config.is_some()
            || value.mediamtx_contract.is_some()
            || value.mediamtx_binary.is_some()
        {
            return Err(invalid("/release_root"));
        }
    }
    if value
        .database_url
        .as_ref()
        .is_some_and(|url| crate::sqlite::database_path(url).is_err())
    {
        return Err(invalid("/database_url"));
    }
    if value.app_env == "development" && !value.bind_addr.ip().is_loopback() {
        return Err(invalid("/bind_addr"));
    }
    if !matches!(value.app_env.as_str(), "production" | "development") {
        return Err(invalid("/app_env"));
    }
    if value
        .jwt_secret
        .as_ref()
        .is_some_and(|secret| !(32..=4096).contains(&secret.len()))
    {
        return Err(invalid("/jwt_secret"));
    }
    if value
        .credentials_key
        .as_ref()
        .is_some_and(|secret| STANDARD.decode(secret).map_or(true, |v| v.len() != 32))
    {
        return Err(invalid("/credentials_key"));
    }
    for (field, number, min, max) in [
        ("/media_token_ttl_secs", value.media_token_ttl_secs, 30, 300),
        ("/status_interval_secs", value.status_interval_secs, 1, 3600),
        (
            "/reconcile_interval_secs",
            value.reconcile_interval_secs,
            1,
            3600,
        ),
        ("/request_timeout_secs", value.request_timeout_secs, 1, 300),
    ] {
        if !(min..=max).contains(&number) {
            return Err(invalid(field));
        }
    }
    for (field, text) in [
        ("/mediamtx_api_url", &value.mediamtx_api_url),
        ("/mediamtx_playback_url", &value.mediamtx_playback_url),
    ] {
        let url = url::Url::parse(text).map_err(|_| invalid(field))?;
        if url.scheme() != "http"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.host_str().is_some_and(|host| {
                host == "localhost"
                    || host
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            })
        {
            return Err(invalid(field));
        }
    }
    // Origin reachability and required secrets are checked on final effective
    // settings; syntax and secret/interval bounds are checked in every layer.
    if validate_public_rtsp_base(&value.public_rtsp_publish_base_url, true).is_err() {
        return Err(invalid("/public_rtsp_publish_base_url"));
    }
    Ok(())
}

impl Settings {
    pub fn companion_paths(&self) -> anyhow::Result<(PathBuf, PathBuf, PathBuf)> {
        if let Some(root) = &self.release_root {
            crate::release::verify_release(root)?;
            let root = crate::release::executing_release_root()?;
            return Ok((
                root.join("config/mediamtx.yml"),
                root.join("config/mediamtx.lock"),
                root.join("bin/mediamtx"),
            ));
        }
        Ok((
            self.mediamtx_config
                .clone()
                .ok_or_else(|| anyhow::anyhow!("MediaMTX config path is required"))?,
            self.mediamtx_contract
                .clone()
                .ok_or_else(|| anyhow::anyhow!("MediaMTX contract path is required"))?,
            self.mediamtx_binary
                .clone()
                .ok_or_else(|| anyhow::anyhow!("MediaMTX binary path is required"))?,
        ))
    }
    pub fn database(&self) -> anyhow::Result<PathBuf> {
        let path = match &self.database_url {
            Some(value) => crate::sqlite::database_path(value)?,
            None => self
                .data_dir
                .as_ref()
                .ok_or_else(|| {
                    crate::CliFailure(xcss::server_cli::ErrorEnvelope::with_code(
                        xcss::server_cli::ErrorCode::new("config.data_directory_required")
                            .expect("static code"),
                        "An explicit data directory or database is required.",
                    ))
                })?
                .join("app.sqlite3"),
        };
        if let Some(root) = &self.data_dir {
            anyhow::ensure!(
                path.parent() == Some(root.as_path()),
                "database must be a direct child of data-dir"
            );
        }
        Ok(path)
    }
    pub fn data_directory(&self) -> anyhow::Result<PathBuf> {
        Ok(self
            .database()?
            .parent()
            .ok_or_else(|| anyhow::anyhow!("database parent is required"))?
            .to_path_buf())
    }
    pub fn effective(&self) -> anyhow::Result<Config> {
        let development_mode = match self.app_env.as_str() {
            "production" => false,
            "development" => true,
            _ => anyhow::bail!("app_env must be production or development"),
        };
        validate_development_bind(self.bind_addr, development_mode).map_err(anyhow::Error::msg)?;
        let jwt_secret = self
            .jwt_secret
            .as_ref()
            .ok_or_else(|| {
                crate::CliFailure(xcss::server_cli::ErrorEnvelope::with_code(
                    xcss::server_cli::ErrorCode::new("config.jwt_secret_required")
                        .expect("static code"),
                    "A private jwt_secret is required.",
                ))
            })?
            .as_bytes()
            .to_vec();
        anyhow::ensure!(
            jwt_secret.len() >= 32 && jwt_secret.len() <= 4096,
            "jwt_secret must contain 32..=4096 bytes"
        );
        let credentials_key: [u8; 32] = STANDARD
            .decode(self.credentials_key.as_ref().ok_or_else(|| {
                crate::CliFailure(xcss::server_cli::ErrorEnvelope::with_code(
                    xcss::server_cli::ErrorCode::new("config.credentials_key_required")
                        .expect("static code"),
                    "A private credentials_key is required.",
                ))
            })?)
            .map_err(|_| anyhow::anyhow!("credentials_key must be valid base64"))?
            .try_into()
            .map_err(|_| anyhow::anyhow!("credentials_key must decode to 32 bytes"))?;
        let directory = self.data_directory()?;
        let runtime_directory = self
            .runtime_directory
            .clone()
            .unwrap_or_else(|| directory.join("runtime"));
        anyhow::ensure!(
            runtime_directory.is_absolute(),
            "runtime directory must be absolute"
        );
        if self.static_dir.is_some() && !development_mode {
            return Err(
                crate::CliFailure(xcss::server_cli::ErrorEnvelope::with_code(
                    xcss::server_cli::ErrorCode::new("invalid_development_override")
                        .expect("static code"),
                    "External Web overrides require explicit loopback development mode.",
                ))
                .into(),
            );
        }
        if let Some(path) = &self.static_dir {
            anyhow::ensure!(path.is_absolute(), "static_dir must be absolute");
        }
        let bounded = |value, min, max| -> anyhow::Result<Duration> {
            anyhow::ensure!(
                (min..=max).contains(&value),
                "configured interval exceeds its current bound"
            );
            Ok(Duration::from_secs(value))
        };
        Ok(Config {
            bind_addr: self.bind_addr,
            database_url: format!("sqlite://{}", self.database()?.display()),
            jwt_secret,
            credentials_key,
            runtime_directory,
            development_mode,
            media_token_ttl: bounded(self.media_token_ttl_secs, 30, 300)?,
            mediamtx_api_url: trim_slash(self.mediamtx_api_url.clone()),
            mediamtx_playback_url: trim_slash(self.mediamtx_playback_url.clone()),
            public_webrtc_base_url: trim_slash(self.public_webrtc_base_url.clone()),
            public_hls_base_url: trim_slash(self.public_hls_base_url.clone()),
            public_rtsp_publish_base_url: validate_public_rtsp_base(
                &self.public_rtsp_publish_base_url,
                development_mode,
            )
            .map_err(anyhow::Error::msg)?,
            status_interval: bounded(self.status_interval_secs, 1, 3600)?,
            reconcile_interval: bounded(self.reconcile_interval_secs, 1, 3600)?,
            request_timeout: bounded(self.request_timeout_secs, 1, 300)?,
            static_dir: self.static_dir.clone(),
        })
    }
}

fn validate_public_rtsp_base(value: &str, development_mode: bool) -> Result<String, String> {
    let value = trim_slash(value.to_owned());
    let parsed = url::Url::parse(&value)
        .map_err(|_| "PUBLIC_RTSP_PUBLISH_BASE_URL must be a valid RTSP URL".to_owned())?;
    if !matches!(parsed.scheme(), "rtsp" | "rtsps")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || !matches!(parsed.path(), "" | "/")
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(
            "PUBLIC_RTSP_PUBLISH_BASE_URL must be an rtsp(s) origin without query or fragment"
                .to_owned(),
        );
    }
    let local_only = parsed.host_str().is_none_or(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback() || address.is_unspecified())
    });
    if !development_mode && local_only {
        return Err(
            "PUBLIC_RTSP_PUBLISH_BASE_URL must be reachable by paired clients in production"
                .to_owned(),
        );
    }
    if !development_mode && parsed.scheme() != "rtsps" {
        return Err("PUBLIC_RTSP_PUBLISH_BASE_URL must use rtsps in production".to_owned());
    }
    Ok(value)
}

#[cfg(test)]
fn absolute_path(name: &str, value: String) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(format!("{name} must be an absolute path"))
    }
}

fn validate_development_bind(bind_addr: SocketAddr, development_mode: bool) -> Result<(), String> {
    if development_mode && !bind_addr.ip().is_loopback() {
        Err("BIND_ADDR must be loopback in development mode".into())
    } else {
        Ok(())
    }
}

/// Keep the bundled MediaMTX callback and local readiness probes reachable even
/// when the configured ingress uses a different interface or port. The common
/// runtime uses IPv6-only sockets, so IPv6 binds never cover this IPv4 endpoint.
pub(crate) fn listener_addresses(bind_addr: SocketAddr) -> Vec<SocketAddr> {
    let local: SocketAddr = DEFAULT_BIND_ADDR.parse().expect("default address");
    let covers_local = bind_addr == local
        || (bind_addr.is_ipv4()
            && bind_addr.ip().is_unspecified()
            && bind_addr.port() == local.port());
    if covers_local {
        vec![bind_addr]
    } else {
        vec![bind_addr, local]
    }
}

fn trim_slash(mut value: String) -> String {
    while value.ends_with('/') {
        value.pop();
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn development_mode_only_accepts_loopback_bindings() {
        assert!(validate_development_bind("127.0.0.1:8080".parse().unwrap(), true).is_ok());
        assert!(validate_development_bind("[::1]:8080".parse().unwrap(), true).is_ok());
        assert!(validate_development_bind("0.0.0.0:8080".parse().unwrap(), true).is_err());
        assert!(validate_development_bind("192.168.1.10:8080".parse().unwrap(), true).is_err());
        assert!(validate_development_bind("0.0.0.0:8080".parse().unwrap(), false).is_ok());
        assert!(validate_development_bind("192.168.1.10:8080".parse().unwrap(), false).is_ok());
        assert!(validate_development_bind("[fd00::10]:8080".parse().unwrap(), false).is_ok());
    }

    #[test]
    fn configured_ingress_preserves_the_local_companion_endpoint() {
        let local = "127.0.0.1:8080".parse::<SocketAddr>().unwrap();
        for address in ["127.0.0.1:8080", "0.0.0.0:8080"] {
            let address = address.parse::<SocketAddr>().unwrap();
            assert_eq!(listener_addresses(address), vec![address]);
        }
        for address in [
            "192.168.1.10:8080",
            "192.168.1.10:9080",
            "0.0.0.0:9080",
            "127.0.0.1:9080",
            "127.0.0.2:8080",
            "[::1]:8080",
            "[::]:8080",
            "[fd00::10]:9080",
        ] {
            let address = address.parse::<SocketAddr>().unwrap();
            assert_eq!(listener_addresses(address), vec![address, local]);
        }
    }

    #[test]
    fn production_configuration_accepts_explicit_remote_ingress() {
        for address in ["192.168.1.10:9080", "0.0.0.0:8080", "[fd00::10]:8080"] {
            let settings = Settings {
                bind_addr: address.parse().unwrap(),
                data_dir: Some("/var/lib/isarmg/xcos/db".into()),
                jwt_secret: Some("x".repeat(32)),
                credentials_key: Some(STANDARD.encode([7_u8; 32])),
                public_rtsp_publish_base_url: "rtsps://xcos.example.org:8322".into(),
                ..Settings::default()
            };
            let loaded = xcss::config::resolve_validated(
                &Settings::default(),
                Some(&serde_json::to_vec(&settings).unwrap()),
                &[],
                &[],
                validate_intrinsic,
            )
            .unwrap();
            let config = loaded.value.effective().unwrap();
            assert_eq!(config.bind_addr, settings.bind_addr);
            assert!(!config.development_mode);
            let development = Settings {
                app_env: "development".into(),
                ..settings
            };
            assert!(validate_intrinsic(&development, xcss::config::ConfigSource::File).is_err());
            assert!(development.effective().is_err());
        }
    }

    #[test]
    fn default_server_binding_is_loopback() {
        let address = DEFAULT_BIND_ADDR.parse::<SocketAddr>().unwrap();
        assert!(address.ip().is_loopback());
        assert_eq!(address.port(), 8080);
    }

    #[test]
    fn runtime_paths_are_absolute() {
        assert_eq!(
            absolute_path("XCSS_DEV_WEB_DIR", "/opt/xcos/web".into()).unwrap(),
            PathBuf::from("/opt/xcos/web")
        );
        assert_eq!(
            absolute_path("XCSS_DEV_WEB_DIR", "web/dist".into()).unwrap_err(),
            "XCSS_DEV_WEB_DIR must be an absolute path"
        );
    }

    #[test]
    fn production_rtsp_publish_origin_must_be_client_reachable() {
        assert!(validate_public_rtsp_base("rtsps://xcos.example:8322", false).is_ok());
        assert!(validate_public_rtsp_base("rtsps://192.168.1.20:8322", false).is_ok());
        assert!(validate_public_rtsp_base("rtsp://xcos.example:8554", false).is_err());
        assert!(validate_public_rtsp_base("rtsp://127.0.0.1:8554", false).is_err());
        assert!(validate_public_rtsp_base("rtsp://localhost:8554", false).is_err());
        assert!(validate_public_rtsp_base("rtsp://user:secret@xcos.example:8554", false).is_err());
        assert!(validate_public_rtsp_base("rtsp://xcos.example:8554/path", false).is_err());
        assert!(validate_public_rtsp_base("rtsp://127.0.0.1:8554", true).is_ok());
    }
}
