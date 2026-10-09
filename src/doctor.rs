use crate::{crypto::SecretBox, protocol::CONTRACT, runtime_lock::DatabaseMaintenanceLock, sqlite};
use anyhow::{ensure, Context};
use chrono::Utc;
use futures_util::TryStreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::Connection;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

#[derive(Clone, Debug)]
pub struct DoctorOptions {
    pub database_url: String,
    pub mediamtx_config: PathBuf,
    pub mediamtx_contract: PathBuf,
    pub mediamtx_binary: PathBuf,
    pub recordings_directory: PathBuf,
    pub credentials_key: [u8; 32],
    pub app_ready_url: String,
    pub mediamtx_ready_url: String,
    pub offline: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct DoctorReport {
    pub status: &'static str,
    pub database_read: bool,
    pub database_write: bool,
    pub credential_decryption: bool,
    pub recording_storage_read_write: bool,
    pub companion_contract: bool,
    pub application_ready: Option<bool>,
    pub mediamtx_ready: Option<bool>,
}

struct ParsedContract {
    version: String,
    platform: String,
    sha256: String,
}

pub async fn run(options: &DoctorOptions) -> anyhow::Result<DoctorReport> {
    let _maintenance = DatabaseMaintenanceLock::shared(&options.database_url)?;
    let database = sqlite::database_path(&options.database_url)?;
    let snapshot = sqlite::current_validation_snapshot(&database).await?;
    let checked = async {
        xcss_sqlite::integrity_check(snapshot.pool()).await?;
        xcss_sqlite::foreign_key_check(snapshot.pool()).await?;
        verify_credentials_on_snapshot(snapshot.pool(), &options.credentials_key).await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    snapshot.close().await;
    checked?;
    database_write_probe(&database).await?;
    verify_companion(
        &options.mediamtx_contract,
        &options.mediamtx_binary,
        &options.mediamtx_config,
        &options.recordings_directory,
    )
    .await?;
    recording_write_probe(&options.recordings_directory)?;

    let (application_ready, mediamtx_ready) = if options.offline {
        (None, None)
    } else {
        let application = live_probe(&options.app_ready_url, ReadinessKind::Application).await?;
        let mediamtx = live_probe(&options.mediamtx_ready_url, ReadinessKind::MediaMtx).await?;
        ensure!(application, "application readiness endpoint is unavailable");
        ensure!(mediamtx, "MediaMTX readiness endpoint is unavailable");
        verify_live_recordings(&options.mediamtx_ready_url, &options.recordings_directory).await?;
        (Some(true), Some(true))
    };

    Ok(DoctorReport {
        status: "ok",
        database_read: true,
        database_write: true,
        credential_decryption: true,
        recording_storage_read_write: true,
        companion_contract: true,
        application_ready,
        mediamtx_ready,
    })
}

#[derive(Clone, Copy)]
enum ReadinessKind {
    Application,
    MediaMtx,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplicationReadiness {
    ready: bool,
}

async fn live_probe(url: &str, kind: ReadinessKind) -> anyhow::Result<bool> {
    let parsed = loopback_url(url)?;
    let client = probe_client()?;
    let response = client.get(parsed).send().await?;
    if response.status() != reqwest::StatusCode::OK {
        return Ok(false);
    }
    if matches!(kind, ReadinessKind::MediaMtx) {
        return Ok(true);
    }
    let service_values = response
        .headers()
        .get_all(xcss_server_cli::SERVICE_IDENTITY_HEADER);
    if service_values.iter().count() != 1
        || !service_values
            .iter()
            .next()
            .is_some_and(|value| value == "xcos")
    {
        return Ok(false);
    }
    if !has_json_content_type(&response) {
        return Ok(false);
    }
    let bytes = xcss_secure_http::bounded_response(
        response,
        xcss_secure_http::ResponseBudget {
            max_header_bytes: 4096,
            max_body_bytes: 128,
        },
    )
    .await?;
    Ok(serde_json::from_slice::<ApplicationReadiness>(&bytes).is_ok_and(|value| value.ready))
}

fn loopback_url(url: &str) -> anyhow::Result<url::Url> {
    let parsed = url::Url::parse(url).context("parse readiness URL")?;
    ensure!(
        matches!(parsed.scheme(), "http" | "https"),
        "readiness URL must use HTTP or HTTPS"
    );
    ensure!(
        parsed.username().is_empty() && parsed.password().is_none(),
        "readiness URL must not contain credentials"
    );
    ensure!(
        parsed.query().is_none() && parsed.fragment().is_none(),
        "readiness URL must not contain a query or fragment"
    );
    let loopback = match parsed.host() {
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        None => false,
    };
    ensure!(loopback, "readiness URL must target loopback");
    Ok(parsed)
}

fn probe_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()?)
}

fn has_json_content_type(response: &reqwest::Response) -> bool {
    let content_types = response.headers().get_all(reqwest::header::CONTENT_TYPE);
    content_types.iter().count() == 1
        && content_types
            .iter()
            .next()
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
}

#[derive(serde::Deserialize)]
struct LivePathDefaults {
    #[serde(rename = "recordPath")]
    record_path: String,
}

/// Inspect the companion's effective configuration, including environment overrides.
/// The pinned MediaMTX API owns its other path-default fields.
async fn verify_live_recordings(url: &str, recordings_directory: &Path) -> anyhow::Result<()> {
    let directory = xcss_state_file::PrivateStateDirectory::open(recordings_directory)?;
    let mut endpoint = loopback_url(url)?;
    endpoint.set_path("/v3/config/pathdefaults/get");
    let response = probe_client()?.get(endpoint).send().await?;
    ensure!(
        response.status() == reqwest::StatusCode::OK && has_json_content_type(&response),
        "MediaMTX effective path configuration is unavailable"
    );
    let bytes = xcss_secure_http::bounded_response(
        response,
        xcss_secure_http::ResponseBudget {
            max_header_bytes: 4096,
            max_body_bytes: 16 * 1024,
        },
    )
    .await?;
    let effective = serde_json::from_slice::<LivePathDefaults>(&bytes)
        .map_err(|_| anyhow::anyhow!("MediaMTX effective path configuration is invalid"))?;
    verify_record_path(&effective.record_path, recordings_directory)?;
    directory.verify_identity()?;
    Ok(())
}

/// Authenticate encrypted instance credentials on the private validation copy.
pub(crate) async fn verify_credentials_on_snapshot(
    pool: &sqlx::SqlitePool,
    key: &[u8; 32],
) -> anyhow::Result<()> {
    let secret_box = SecretBox::new(key);
    let columns = sqlite::bounded_columns(&[("id", 36), ("authorization_code_enc", 1024)]);
    // SQLite permits both text UUIDs and SQLx's native 16-byte UUID blobs in
    // this column. Read bounded bytes so both storage forms authenticate.
    let sql = format!(
        "SELECT CAST(id AS BLOB),authorization_code_enc FROM \
         (SELECT {columns} FROM xcocs)"
    );
    // Both identifiers and length guards come from product constants; all
    // stored values remain query results, never executable SQL input.
    let mut rows = sqlx::query_as::<_, (Vec<u8>, Vec<u8>)>(sqlx::AssertSqlSafe(sql)).fetch(pool);
    while let Some((id, envelope)) = rows.try_next().await? {
        let id = if id.len() == 16 {
            Uuid::from_slice(&id)
        } else {
            Uuid::try_parse_ascii(&id)
        }
        .map_err(|_| anyhow::anyhow!("camera instance identity is invalid"))?;
        secret_box
            .decrypt_client_authorization(&id.to_string(), &envelope)
            .map_err(|_| {
                anyhow::anyhow!(
                    "credentials_key cannot authenticate current camera authorization envelopes"
                )
            })?;
    }
    Ok(())
}

async fn database_write_probe(path: &Path) -> anyhow::Result<()> {
    let directory = xcss_state_file::PrivateStateDirectory::open(
        path.parent().context("database parent is required")?,
    )?;
    let before = fs::symlink_metadata(path)?;
    let mut connection = sqlite::connect_existing(path, false).await?;
    let result=async {
        let mut transaction=connection.begin_with("BEGIN IMMEDIATE").await?;
        let probe=sqlx::query("INSERT INTO audit_logs(id,action,entity_type,details,created_at) VALUES(?,'doctor_probe','system','{}',?)")
            .bind(Uuid::new_v4().to_string()).bind(Utc::now().to_rfc3339())
            .execute(&mut *transaction).await;
        let rolled_back=transaction.rollback().await;
        probe.context("database write probe failed")?;
        rolled_back.context("roll back database write probe")?;
        Ok::<(),anyhow::Error>(())
    }.await;
    let closed = connection.close().await;
    result?;
    closed?;
    directory.verify_identity()?;
    #[cfg(unix)]
    {
        let after = fs::symlink_metadata(path)?;
        ensure!(
            (before.dev(), before.ino()) == (after.dev(), after.ino()),
            "database identity changed during the write probe"
        );
    }
    Ok(())
}

fn recording_write_probe(root: &Path) -> anyhow::Result<()> {
    let directory = xcss_state_file::PrivateStateDirectory::open(root)?;
    let path = root.join(format!(".xcos-doctor-{}", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options
        .mode(0o600)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    let mut file = options.open(&path)?;
    let result = (|| {
        file.write_all(b"xcos-storage-probe")?;
        file.sync_all()?;
        let content = read_limited(&path, 64)?;
        ensure!(
            content == b"xcos-storage-probe",
            "recording storage read/write probe failed"
        );
        Ok::<_, anyhow::Error>(())
    })();
    drop(file);
    let cleanup = fs::remove_file(&path).context("remove recording storage probe");
    sync_parent(&path)?;
    result?;
    cleanup?;
    directory.verify_identity()?;
    Ok(())
}

pub(crate) async fn verify_companion(
    contract_path: &Path,
    binary_path: &Path,
    config_path: &Path,
    recordings_directory: &Path,
) -> anyhow::Result<()> {
    require_secure_file(contract_path, "MediaMTX contract")?;
    require_secure_file(binary_path, "MediaMTX binary")?;
    require_secure_file(config_path, "MediaMTX config")?;
    let directory = xcss_state_file::PrivateStateDirectory::open(recordings_directory)?;

    let contract = parse_contract(contract_path)?;
    ensure!(
        contract.platform == "linux_amd64",
        "MediaMTX companion platform is unsupported"
    );
    ensure!(
        sha256_file(binary_path)? == contract.sha256,
        "MediaMTX binary hash does not match its contract"
    );
    let stdout = bounded_companion_version(binary_path).await?;
    let version = String::from_utf8(stdout).context("MediaMTX version is not UTF-8")?;
    ensure!(
        version.trim() == contract.version,
        "MediaMTX binary version does not match its contract"
    );
    verify_media_config(config_path, recordings_directory)?;
    directory.verify_identity()?;
    Ok(())
}

async fn bounded_companion_version(binary_path: &Path) -> anyhow::Result<Vec<u8>> {
    use tokio::io::{AsyncRead, AsyncReadExt};
    async fn read_version_pipe(reader: impl AsyncRead + Unpin) -> anyhow::Result<Vec<u8>> {
        let mut bytes = Vec::with_capacity(257);
        reader.take(257).read_to_end(&mut bytes).await?;
        ensure!(
            bytes.len() <= 256,
            "MediaMTX version output exceeds its limit"
        );
        Ok(bytes)
    }
    let mut child = tokio::process::Command::new(binary_path)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("execute MediaMTX version check")?;
    let stdout = child
        .stdout
        .take()
        .context("MediaMTX stdout is unavailable")?;
    let stderr = child
        .stderr
        .take()
        .context("MediaMTX stderr is unavailable")?;
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::try_join!(
            read_version_pipe(stdout),
            read_version_pipe(stderr),
            async { child.wait().await.map_err(anyhow::Error::from) }
        )
    })
    .await;
    match result {
        Ok(Ok((stdout, _, status))) if status.success() => Ok(stdout),
        failed => {
            // Reap on every failure; a caller cannot leave an unbounded version probe running.
            let _ = child.kill().await;
            let _ = child.wait().await;
            match failed {
                Err(_) => anyhow::bail!("MediaMTX version check timed out"),
                Ok(Err(error)) => Err(error),
                Ok(Ok(_)) => anyhow::bail!("MediaMTX version check failed"),
            }
        }
    }
}

fn parse_contract(path: &Path) -> anyhow::Result<ParsedContract> {
    let content = String::from_utf8(read_limited(path, 16 * 1024)?)
        .context("MediaMTX contract is not UTF-8")?;
    let mut values = BTreeMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .context("MediaMTX contract contains an invalid line")?;
        ensure!(
            matches!(key, "version" | "platform" | "sha256"),
            "MediaMTX contract contains an unknown field"
        );
        ensure!(
            values.insert(key, value.trim()).is_none(),
            "MediaMTX contract contains a duplicate field"
        );
    }
    ensure!(values.len() == 3, "MediaMTX contract is incomplete");
    let version = values["version"].to_string();
    let platform = values["platform"].to_string();
    let sha256 = values["sha256"].to_ascii_lowercase();
    ensure!(!version.is_empty(), "MediaMTX version is missing");
    ensure!(
        sha256.len() == 64
            && sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "MediaMTX SHA-256 must be lowercase hexadecimal"
    );
    Ok(ParsedContract {
        version,
        platform,
        sha256,
    })
}

fn verify_media_config(path: &Path, recordings_directory: &Path) -> anyhow::Result<()> {
    let content = String::from_utf8(read_limited(path, 1024 * 1024)?)
        .context("MediaMTX config is not UTF-8")?;
    // Only the exact immutable template in this verified executing release can
    // use the launcher's recordings_directory override. A copied or development
    // YAML retains the strict on-disk recordPath contract.
    let managed = if env!("XCOS_SOURCE_REVISION") == "unbound" {
        false
    } else {
        let root = crate::release::executing_release_root()?;
        managed_config_path(path, &root)
    };
    verify_media_config_content(&content, recordings_directory, managed)
}

const MANAGED_MEDIA_CONFIG: &str = include_str!("../config/mediamtx.yml");
const MANAGED_RECORD_PATH: &str = "/var/lib/isarmg/xcos/recordings/%path/%Y-%m-%d_%H-%M-%S-%f";

fn managed_config_path(path: &Path, root: &Path) -> bool {
    path.as_os_str() == root.join("config/mediamtx.yml").as_os_str()
}

fn verify_media_config_content(
    content: &str,
    recordings_directory: &Path,
    managed: bool,
) -> anyhow::Result<()> {
    let directory = xcss_state_file::PrivateStateDirectory::open(recordings_directory)?;
    if managed {
        ensure!(
            content == MANAGED_MEDIA_CONFIG,
            "managed MediaMTX template does not match the executing release"
        );
    }
    let auth_address = format!("http://127.0.0.1:8080{}", CONTRACT.media_auth_path);
    for (key, expected) in [
        ("authMethod", "http"),
        ("authHTTPAddress", auth_address.as_str()),
        ("apiAddress", "127.0.0.1:9997"),
        ("playbackAddress", "127.0.0.1:9996"),
        ("recordFormat", "fmp4"),
    ] {
        let values = config_values(content, key);
        ensure!(
            values.len() == 1 && values[0].trim_matches(['\'', '"']) == expected,
            "MediaMTX config has a missing, duplicate or invalid companion setting"
        );
    }
    let paths = config_values(content, "recordPath");
    ensure!(
        paths.len() == 1,
        "MediaMTX config must declare exactly one recordPath"
    );
    let configured = paths[0].trim_matches(['\'', '"']);
    if managed {
        ensure!(
            configured == MANAGED_RECORD_PATH,
            "managed MediaMTX template has an invalid inert recordPath"
        );
    } else {
        verify_record_path(configured, recordings_directory)?;
    }
    directory.verify_identity()?;
    Ok(())
}

fn verify_record_path(configured: &str, recordings_directory: &Path) -> anyhow::Result<()> {
    let (root, suffix) = configured
        .split_once("/%path/")
        .context("MediaMTX recordPath must contain one %path directory component")?;
    ensure!(
        !suffix.is_empty()
            && !suffix.contains("%path")
            && suffix
                .split('/')
                .all(|part| !matches!(part, "" | "." | "..")),
        "MediaMTX recordPath has an invalid path component"
    );
    ensure!(!root.is_empty(), "MediaMTX recordPath has an empty root");
    let root = PathBuf::from(root);
    ensure!(root.is_absolute(), "MediaMTX recordPath must be absolute");
    let source = xcss_state_file::PrivateStateDirectory::open(&root)?;
    let expected = xcss_state_file::PrivateStateDirectory::open(recordings_directory)?;
    ensure!(
        fs::canonicalize(&root)? == fs::canonicalize(recordings_directory)?,
        "MediaMTX recordPath points at a different recordings directory"
    );
    source.verify_identity()?;
    expected.verify_identity()?;
    Ok(())
}

fn config_values<'a>(content: &'a str, key: &str) -> Vec<&'a str> {
    let prefix = format!("{key}:");
    content
        .lines()
        .filter_map(|line| line.trim().strip_prefix(&prefix).map(str::trim))
        .collect()
}

fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut file = open_read_no_follow(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn read_limited(path: &Path, limit: u64) -> anyhow::Result<Vec<u8>> {
    require_secure_file(path, "input file")?;
    let before = fs::symlink_metadata(path)?;
    ensure!(before.len() <= limit, "input file exceeds its size limit");
    let mut output = Vec::with_capacity(before.len() as usize);
    open_read_no_follow(path)?.read_to_end(&mut output)?;
    let after = fs::symlink_metadata(path)?;
    ensure!(
        stable_metadata(&before, &after) && output.len() as u64 == after.len(),
        "input file changed while it was read"
    );
    Ok(output)
}

fn require_secure_file(path: &Path, description: &str) -> anyhow::Result<()> {
    reject_symlink_components(path)?;
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("{description} does not exist: {}", path.display()))?;
    ensure!(metadata.is_file(), "{description} must be a regular file");
    #[cfg(unix)]
    ensure!(
        metadata.nlink() == 1,
        "{description} must not have hard-link aliases"
    );
    Ok(())
}

fn reject_symlink_components(path: &Path) -> anyhow::Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::CurDir => current.push("."),
            Component::Normal(value) => current.push(value),
            Component::ParentDir => anyhow::bail!("operational path must not contain traversal"),
        }
        let metadata = fs::symlink_metadata(&current)
            .with_context(|| format!("operational path does not exist: {}", current.display()))?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "symbolic links are not accepted in operational paths"
        );
    }
    Ok(())
}

fn open_read_no_follow(path: &Path) -> anyhow::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC).bits() as i32);
    options
        .open(path)
        .with_context(|| format!("open {}", path.display()))
}

fn stable_metadata(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return false;
    }
    #[cfg(unix)]
    {
        before.dev() == after.dev()
            && before.ino() == after.ino()
            && before.mtime_nsec() == after.mtime_nsec()
    }
    #[cfg(not(unix))]
    true
}

fn sync_parent(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    File::open(path.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn credential_validation_accepts_native_uuid_storage_and_rejects_wrong_keys() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE xcocs(id TEXT PRIMARY KEY, authorization_code_enc BLOB NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let id = Uuid::new_v4();
        let key = [0x42; 32];
        let envelope = SecretBox::new(&key)
            .encrypt_client_authorization(&id.to_string(), &"a".repeat(36))
            .unwrap();
        // Match the pairing route: SQLx binds UUIDs as 16-byte SQLite blobs.
        sqlx::query("INSERT INTO xcocs VALUES (?, ?)")
            .bind(id)
            .bind(envelope)
            .execute(&pool)
            .await
            .unwrap();
        verify_credentials_on_snapshot(&pool, &key).await.unwrap();
        assert!(verify_credentials_on_snapshot(&pool, &[0x99; 32])
            .await
            .is_err());
        pool.close().await;
    }

    fn private_directory() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        }
        directory
    }

    #[test]
    fn managed_template_override_requires_exact_template_and_private_physical_storage() {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt;
        let temporary = private_directory();
        assert!(managed_config_path(
            &temporary.path().join("config/mediamtx.yml"),
            temporary.path()
        ));
        assert!(!managed_config_path(
            &temporary.path().join("config/./mediamtx.yml"),
            temporary.path()
        ));
        // The real caller derives `managed` only from the executing verified
        // full release and its fixed config path; this tests that branch's bytes.
        verify_media_config_content(MANAGED_MEDIA_CONFIG, temporary.path(), true).unwrap();
        for changed in [
            format!("{MANAGED_MEDIA_CONFIG}\n# changed\n"),
            format!("{MANAGED_MEDIA_CONFIG}\nrecordPath: {MANAGED_RECORD_PATH}\n"),
            MANAGED_MEDIA_CONFIG.replace("recordFormat: fmp4", "recordFormat: mpegts"),
            MANAGED_MEDIA_CONFIG.replace(MANAGED_RECORD_PATH, "/tmp/other/%path/%Y"),
        ] {
            assert!(verify_media_config_content(&changed, temporary.path(), true).is_err());
        }
        assert!(
            verify_media_config_content(MANAGED_MEDIA_CONFIG, temporary.path(), false).is_err()
        );
        let copied = temporary.path().join("mediamtx.yml");
        fs::write(&copied, MANAGED_MEDIA_CONFIG).unwrap();
        assert!(verify_media_config(&copied, temporary.path()).is_err());
        #[cfg(unix)]
        {
            let alias = temporary.path().join("alias");
            std::os::unix::fs::symlink(temporary.path(), &alias).unwrap();
            assert!(verify_media_config_content(MANAGED_MEDIA_CONFIG, &alias, true).is_err());
            fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o755)).unwrap();
            assert!(
                verify_media_config_content(MANAGED_MEDIA_CONFIG, temporary.path(), true).is_err()
            );
        }
    }

    #[test]
    fn external_yaml_record_path_cannot_be_overridden_or_escape_storage() {
        let temporary = private_directory();
        let path = format!("{}/%path/%Y-%m-%d", temporary.path().display());
        let content = MANAGED_MEDIA_CONFIG.replace(MANAGED_RECORD_PATH, &path);
        verify_media_config_content(&content, temporary.path(), false).unwrap();
        let other = private_directory();
        assert!(verify_media_config_content(&content, other.path(), false).is_err());
        for value in [
            format!("{}/%path/%path", temporary.path().display()),
            format!("{}/%path/../escape", temporary.path().display()),
            format!("{}/%pathsuffix/%Y", temporary.path().display()),
            "relative/%path/%Y".into(),
        ] {
            assert!(verify_record_path(&value, temporary.path()).is_err());
        }
        assert!(verify_media_config_content(
            &format!("{content}\nrecordPath: {path}\n"),
            temporary.path(),
            false,
        )
        .is_err());
    }

    #[tokio::test]
    async fn effective_companion_path_is_bounded_loopback_json_and_matches_storage() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let storage = private_directory();
        let other = private_directory();
        let valid = format!("{}/%path/%Y", storage.path().display());
        let wrong = format!("{}/%path/%Y", other.path().display());
        for (status, content_type, body, expected) in [
            (
                "200 OK",
                "application/json",
                serde_json::json!({"recordPath":valid,"recordFormat":"fmp4"}).to_string(),
                true,
            ),
            (
                "200 OK",
                "application/json",
                serde_json::json!({"recordPath":wrong}).to_string(),
                false,
            ),
            ("200 OK", "application/json", "{}".into(), false),
            (
                "200 OK",
                "application/json",
                format!(
                    "{{\"recordPath\":{},\"recordPath\":{}}}",
                    serde_json::to_string(&wrong).unwrap(),
                    serde_json::to_string(&valid).unwrap()
                ),
                false,
            ),
            (
                "200 OK",
                "text/html",
                serde_json::json!({"recordPath":valid}).to_string(),
                false,
            ),
            (
                "301 Moved Permanently",
                "application/json",
                serde_json::json!({"recordPath":valid}).to_string(),
                false,
            ),
            (
                "200 OK",
                "application/json",
                "x".repeat(16 * 1024 + 1),
                false,
            ),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0_u8; 4096];
                let length = stream.read(&mut request).await.unwrap();
                assert!(std::str::from_utf8(&request[..length])
                    .unwrap()
                    .starts_with("GET /v3/config/pathdefaults/get HTTP/1.1\r\n"));
                let response = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(response.as_bytes()).await;
            });
            assert_eq!(
                verify_live_recordings(&format!("http://{address}/v3/info"), storage.path())
                    .await
                    .is_ok(),
                expected
            );
            server.await.unwrap();
        }
        for url in [
            "http://example.com/v3/info",
            "http://user:secret@127.0.0.1/v3/info",
            "http://127.0.0.1/v3/info?secret=value",
        ] {
            assert!(verify_live_recordings(url, storage.path()).await.is_err());
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn companion_version_probe_bounds_both_pipes_and_reaps_timed_out_process() {
        use std::os::unix::fs::PermissionsExt;
        let temporary = tempfile::tempdir().unwrap();
        let binary = temporary.path().join("companion");
        fs::write(
            &binary,
            "#!/bin/sh\nprintf 'v1.20.0\\n'\nprintf warning >&2\n",
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            bounded_companion_version(&binary).await.unwrap(),
            b"v1.20.0\n"
        );
        for pipe in ["", " >&2"] {
            fs::write(
                &binary,
                format!(
                    "#!/bin/sh\nwhile :; do printf '{}'{pipe}; done\n",
                    "x".repeat(257)
                ),
            )
            .unwrap();
            let began = std::time::Instant::now();
            assert!(bounded_companion_version(&binary).await.is_err());
            assert!(began.elapsed() < Duration::from_secs(2));
        }
        let pid_file = temporary.path().join("pid");
        fs::write(
            &binary,
            format!(
                "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nexec sleep 30\n",
                pid_file.display()
            ),
        )
        .unwrap();
        let began = std::time::Instant::now();
        assert!(bounded_companion_version(&binary).await.is_err());
        assert!(began.elapsed() < Duration::from_secs(6));
        let pid = fs::read_to_string(pid_file).unwrap();
        assert!(!Path::new("/proc").join(pid).exists());
    }

    #[tokio::test]
    async fn offline_doctor_checks_current_database_storage_companion_and_credentials() {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().unwrap();
        let database = temporary.path().join("xcos.sqlite3");
        let database_url = format!("sqlite://{}", database.display());
        let pool = sqlite::initialize_test_pool(&database_url).await.unwrap();
        let key = [0x42; 32];
        let now = Utc::now().to_rfc3339();
        let now_micros = Utc::now().timestamp_micros();
        let user = Uuid::new_v4().to_string();
        let camera_id = Uuid::new_v4();
        let secret_box = SecretBox::new(&key);
        let authorization_code = "a".repeat(36);
        let encrypted = secret_box
            .encrypt_client_authorization(&camera_id.to_string(), &authorization_code)
            .unwrap();
        sqlx::query(
            "INSERT INTO _xcss_administrators(
             administrator_id,username,password_hash,created_at_micros,updated_at_micros)
             VALUES (?, 'doctor-admin', ?, ?, ?)",
        )
        .bind(user.to_string())
        .bind(xcss_admin_auth::hash_password("doctor-admin-password").unwrap())
        .bind(now_micros)
        .bind(now_micros)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO xcocs (id, name, authorization_code_enc, authorization_code_hash, \
             created_by, created_at, updated_at) VALUES (?, 'Doctor', ?, ?, ?, ?, ?)",
        )
        .bind(camera_id.to_string())
        .bind(encrypted)
        .bind(vec![0x22_u8; 32])
        .bind(&user)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO cameras (id, name, client_id, client_camera_id, adapter_kind, \
             created_by, created_at, updated_at) VALUES (?, 'Doctor', ?, ?, 'onvif', ?, ?, ?)",
        )
        .bind(camera_id.to_string())
        .bind(camera_id.to_string())
        .bind(camera_id.to_string())
        .bind(&user)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();
        // Drain the fixture's connection workers explicitly. A pool-return
        // task can release its permit before the final SQLite checkpoint,
        // while offline validation intentionally rejects an active writer.
        let mut connections = Vec::new();
        while connections.len() < pool.size() as usize {
            connections.push(pool.acquire().await.unwrap());
        }
        let closed = pool.close();
        for connection in connections {
            connection.close().await.unwrap();
        }
        closed.await;

        let recordings = temporary.path().join("recordings");
        fs::create_dir(&recordings).unwrap();
        #[cfg(unix)]
        fs::set_permissions(&recordings, fs::Permissions::from_mode(0o700)).unwrap();
        let binary = temporary.path().join("mediamtx");
        let companion_marker = temporary.path().join("companion-executed");
        fs::write(
            &binary,
            format!(
                "#!/bin/sh\nprintf touched > '{}'\nprintf 'v1.20.0\\n'\n",
                companion_marker.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let contract = temporary.path().join("mediamtx.lock");
        fs::write(
            &contract,
            format!(
                "version=v1.20.0\nplatform=linux_amd64\nsha256={}\n",
                sha256_file(&binary).unwrap()
            ),
        )
        .unwrap();
        let config = temporary.path().join("mediamtx.yml");
        fs::write(
            &config,
            format!(
                "authMethod: http\n\
                 authHTTPAddress: http://127.0.0.1:8080/internal/v1/media/auth\n\
                 apiAddress: 127.0.0.1:9997\n\
                 playbackAddress: 127.0.0.1:9996\n\
                 recordFormat: fmp4\n\
                 recordPath: {}/%path/%Y-%m-%d.mp4\n",
                recordings.display()
            ),
        )
        .unwrap();

        let options = DoctorOptions {
            database_url: database_url.clone(),
            mediamtx_config: config.clone(),
            mediamtx_contract: contract.clone(),
            mediamtx_binary: binary.clone(),
            recordings_directory: recordings.clone(),
            credentials_key: key,
            app_ready_url: "http://127.0.0.1:1/readyz".to_string(),
            mediamtx_ready_url: "http://127.0.0.1:1/v3/info".to_string(),
            offline: true,
        };
        let report = run(&options).await.unwrap();
        assert_eq!(report.status, "ok");
        assert!(companion_marker.exists());
        fs::remove_file(&companion_marker).unwrap();
        assert!(!fs::read_dir(&recordings).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".xcos-doctor-")));

        let wrong_key = DoctorOptions {
            credentials_key: [0x99; 32],
            ..options.clone()
        };
        let wrong_key_error = run(&wrong_key).await.unwrap_err().to_string();
        assert!(!wrong_key_error.contains(&authorization_code));
        assert!(!companion_marker.exists());

        let mut connection = sqlite::connect_existing(&database, false).await.unwrap();
        let mut tampered: Vec<u8> =
            sqlx::query_scalar("SELECT authorization_code_enc FROM xcocs WHERE id=?")
                .bind(camera_id.to_string())
                .fetch_one(&mut connection)
                .await
                .unwrap();
        *tampered.last_mut().unwrap() ^= 1;
        sqlx::query("UPDATE xcocs SET authorization_code_enc=? WHERE id=?")
            .bind(tampered)
            .bind(camera_id.to_string())
            .execute(&mut connection)
            .await
            .unwrap();
        sqlx::raw_sql("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&mut connection)
            .await
            .unwrap();
        connection.close().await.unwrap();
        let before = sha256_file(&database).unwrap();
        let tampered_error = run(&options).await.unwrap_err().to_string();
        assert_eq!(sha256_file(&database).unwrap(), before);
        assert!(!tampered_error.contains(&authorization_code));
        assert!(!companion_marker.exists());
        assert!(
            live_probe("http://example.com/readyz", ReadinessKind::Application)
                .await
                .is_err()
        );
    }
}
#[tokio::test]
async fn application_probe_requires_the_exact_bounded_current_readiness_contract() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for (status, content_type, service, body, expected) in [
        (
            "200 OK",
            "application/json",
            "xcos",
            "{\"ready\":true}",
            true,
        ),
        (
            "200 OK",
            "application/json",
            "different-service",
            "{\"ready\":true}",
            false,
        ),
        ("200 OK", "application/json", "", "{\"ready\":true}", false),
        (
            "200 OK",
            "application/json",
            "xcos",
            "{\"ready\":false}",
            false,
        ),
        (
            "200 OK",
            "application/json",
            "xcos",
            "{\"status\":\"ok\"}",
            false,
        ),
        (
            "200 OK",
            "application/json",
            "xcos",
            "{\"ready\":true,\"extra\":1}",
            false,
        ),
        (
            "200 OK",
            "application/json",
            "xcos",
            "{\"ready\":false,\"ready\":true}",
            false,
        ),
        ("200 OK", "text/html", "xcos", "{\"ready\":true}", false),
        (
            "503 Service Unavailable",
            "application/json",
            "xcos",
            "{\"ready\":true}",
            false,
        ),
        (
            "301 Moved Permanently",
            "application/json",
            "xcos",
            "{\"ready\":true}",
            false,
        ),
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).await;
            let response = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nx-xcss-service: {service}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = stream.write_all(response.as_bytes()).await;
        });
        assert_eq!(
            live_probe(
                &format!("http://{address}/readyz"),
                ReadinessKind::Application
            )
            .await
            .unwrap(),
            expected
        );
        server.await.unwrap();
    }
}
