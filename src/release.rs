use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Component, Path, PathBuf},
};

use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

use crate::crypto::{credential_contract_sha256, CREDENTIAL_ENVELOPE_REVISION};

const MANIFEST_FORMAT: &str = "xcos-release-v1";
const PRODUCT: &str = "xcos";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const TARGET: &str = xcss::server_target::SERVER_TARGET_TRIPLE;
const SERVER_BINARY: &str = "bin/xcos";
const MANIFEST_NAME: &str = "RELEASE-MANIFEST";
const MAX_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_RELEASE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;
pub(crate) const PRODUCTION_RELEASE_ROOT: &str =
    concat!("/opt/isarmg/xcos/releases/", env!("CARGO_PKG_VERSION"));
const RELOCATABLE_RELEASE_SUFFIX: &str =
    concat!("opt/isarmg/xcos/releases/", env!("CARGO_PKG_VERSION"));
// config/ 是仓库内受审配置的唯一位置；发布包仍按运行时契约写入 config/。
const MEDIAMTX_LOCK: &[u8] = include_bytes!("../config/mediamtx.lock");
const MEDIAMTX_CONFIG: &[u8] = include_bytes!("../config/mediamtx.yml");

pub(crate) fn state_contract_bytes() -> Result<Vec<u8>> {
    use xcss::contracts::{
        CompanionContract, StateContract, StateResource, StateResourceKind, StateSchemaIdentity,
    };
    let media = mediamtx_identity()?;
    let contract = StateContract {
        contract_version: xcss::contracts::STATE_CONTRACT_VERSION,
        application: PRODUCT.into(),
        application_version: VERSION.into(),
        source_revision: env!("XCOS_SOURCE_REVISION").into(),
        schema: Some(StateSchemaIdentity {
            revision: crate::sqlite::CURRENT_SCHEMA_REVISION as u64,
            sha256: crate::sqlite::CURRENT_SCHEMA_SHA256.into(),
        }),
        maintenance_locks: vec![
            ".state-instance.lock".into(),
            ".state-maintenance.lock".into(),
        ],
        resources: vec![
            StateResource {
                name: "state".into(),
                kind: StateResourceKind::DataTree,
                required: true,
            },
            StateResource {
                name: "recordings".into(),
                kind: StateResourceKind::Recordings,
                required: true,
            },
            StateResource {
                name: "config".into(),
                kind: StateResourceKind::Configuration,
                required: true,
            },
        ],
        external_requirements: vec![],
        companion_contracts: vec![CompanionContract {
            name: "mediamtx".into(),
            version: media.version,
            platform: media.platform,
            sha256: media.sha256,
        }],
    };
    contract.validate().map_err(|_| {
        crate::CliFailure(xcss::server_cli::ErrorEnvelope::with_code(
            xcss::server_cli::ErrorCode::new("release_identity_unbound").expect("static code"),
            "This development binary is not bound to a complete source revision.",
        ))
    })?;
    Ok(serde_json::to_vec(&contract)?)
}
pub(crate) fn standard_identity() -> Result<xcss::contracts::ReleaseIdentity> {
    let identity = xcss::contracts::ReleaseIdentity {
        product: PRODUCT.into(),
        version: VERSION.into(),
        source_revision: env!("XCOS_SOURCE_REVISION").into(),
        target: env!("XCOS_BUILD_TARGET").into(),
        state_contract_sha256: sha256_hex(&state_contract_bytes()?),
    };
    identity.validate()?;
    Ok(identity)
}

const FIXED_DIRECTORIES: &[&str] = &["bin", "config", "deploy", "share"];
const FIXED_FILES: &[(&str, u32)] = &[
    ("bin/mediamtx", 0o555),
    (SERVER_BINARY, 0o555),
    ("config/mediamtx.lock", 0o444),
    ("config/mediamtx.yml", 0o444),
    ("deploy/.bootstrap-action.sh", 0o444),
    ("deploy/.start-action.sh", 0o444),
    ("deploy/.status-action.sh", 0o444),
    ("deploy/.stop-action.sh", 0o444),
    ("deploy/common.sh", 0o555),
    ("deploy/xcosctl", 0o555),
    ("share/web-assets.json", 0o444),
];

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub(crate) struct ReleaseIdentity {
    manifest_format: &'static str,
    application: &'static str,
    application_version: &'static str,
    source_revision: &'static str,
    target: &'static str,
    wire_protocol: String,
    api_prefix: String,
    schema_revision: i64,
    schema_sha256: &'static str,
    credential_envelope_revision: u32,
    credential_contract_sha256: String,
    static_contract_sha256: String,
    mediamtx_version: String,
    mediamtx_platform: String,
    mediamtx_sha256: String,
    mediamtx_config_sha256: String,
    release_contract_sha256: String,
}

#[derive(Debug)]
struct MediaMtxIdentity {
    version: String,
    platform: String,
    sha256: String,
}

#[derive(Clone, Debug)]
enum ManifestEntry {
    Directory {
        mode: u32,
    },
    File {
        mode: u32,
        size: u64,
        sha256: String,
    },
}

#[derive(Debug)]
struct FileHash {
    size: u64,
    sha256: String,
}

#[derive(Default)]
struct Counters {
    entries: usize,
    bytes: u64,
}

pub(crate) fn identity() -> Result<ReleaseIdentity> {
    let media = mediamtx_identity()?;
    let credential_contract_sha256 = credential_contract_sha256();
    let static_contract_sha256 = crate::static_assets::embedded_manifest_sha256();
    let mediamtx_config_sha256 = sha256_hex(MEDIAMTX_CONFIG);
    let contract = format!(
        "manifest_format={MANIFEST_FORMAT}\napplication={PRODUCT}\napplication_version={VERSION}\ntarget={}\nwire_protocol={}\napi_prefix={}\nschema_revision={}\nschema_sha256={}\ncredential_envelope_revision={CREDENTIAL_ENVELOPE_REVISION}\ncredential_contract_sha256={credential_contract_sha256}\nstatic_contract_sha256={static_contract_sha256}\nmediamtx_version={}\nmediamtx_platform={}\nmediamtx_sha256={}\nmediamtx_config_sha256={mediamtx_config_sha256}\n",
        env!("XCOS_BUILD_TARGET"),
        crate::protocol::CONTRACT.wire_protocol,
        crate::protocol::CONTRACT.api_prefix,
        crate::sqlite::CURRENT_SCHEMA_REVISION,
        crate::sqlite::CURRENT_SCHEMA_SHA256,
        media.version,
        media.platform,
        media.sha256,
    );
    Ok(ReleaseIdentity {
        manifest_format: MANIFEST_FORMAT,
        application: PRODUCT,
        application_version: VERSION,
        source_revision: env!("XCOS_SOURCE_REVISION"),
        target: env!("XCOS_BUILD_TARGET"),
        wire_protocol: crate::protocol::CONTRACT.wire_protocol.clone(),
        api_prefix: crate::protocol::CONTRACT.api_prefix.clone(),
        schema_revision: crate::sqlite::CURRENT_SCHEMA_REVISION,
        schema_sha256: crate::sqlite::CURRENT_SCHEMA_SHA256,
        credential_envelope_revision: CREDENTIAL_ENVELOPE_REVISION,
        credential_contract_sha256,
        static_contract_sha256,
        mediamtx_version: media.version,
        mediamtx_platform: media.platform,
        mediamtx_sha256: media.sha256,
        mediamtx_config_sha256,
        release_contract_sha256: sha256_hex(contract.as_bytes()),
    })
}

pub(crate) fn manifest_header() -> Result<String> {
    let identity = identity()?;
    Ok(format!(
        "format={}\napplication={}\napplication_version={}\nsource_revision={}\ntarget={}\nwire_protocol={}\napi_prefix={}\nschema_revision={}\nschema_sha256={}\ncredential_envelope_revision={}\ncredential_contract_sha256={}\nstatic_contract_sha256={}\nmediamtx_version={}\nmediamtx_platform={}\nmediamtx_sha256={}\nmediamtx_config_sha256={}\nrelease_contract_sha256={}\n",
        identity.manifest_format,
        identity.application,
        identity.application_version,
        identity.source_revision,
        identity.target,
        identity.wire_protocol,
        identity.api_prefix,
        identity.schema_revision,
        identity.schema_sha256,
        identity.credential_envelope_revision,
        identity.credential_contract_sha256,
        identity.static_contract_sha256,
        identity.mediamtx_version,
        identity.mediamtx_platform,
        identity.mediamtx_sha256,
        identity.mediamtx_config_sha256,
        identity.release_contract_sha256,
    ))
}

pub(crate) fn ensure_unbound_run() -> Result<()> {
    if env!("XCOS_SOURCE_REVISION") != "unbound" {
        return Err(
            crate::CliFailure(xcss::server_cli::ErrorEnvelope::with_code(
                xcss::server_cli::ErrorCode::new("release_root_required").expect("static code"),
                "A source-bound Xcos release requires run --release-root RELEASE_ROOT.",
            ))
            .into(),
        );
    }
    Ok(())
}

pub(crate) fn verify_release(root: &Path) -> Result<ReleaseIdentity> {
    verify_release_with_options(root, true, true)
}

fn verify_release_with_options(
    root: &Path,
    require_current_executable: bool,
    require_bound_source: bool,
) -> Result<ReleaseIdentity> {
    let root = validate_release_root(root)?;
    let production = root == Path::new(PRODUCTION_RELEASE_ROOT);
    if production {
        for directory in [
            "/opt",
            "/opt/isarmg",
            "/opt/isarmg/xcos",
            "/opt/isarmg/xcos/releases",
        ] {
            require_directory(
                Path::new(directory),
                0o755,
                true,
                "production release parent",
            )?;
        }
    }
    require_directory(&root, 0o555, production, "release root")?;

    let identity = identity()?;
    if require_bound_source {
        ensure!(
            is_source_revision(identity.source_revision),
            "release binary has no exact source revision binding"
        );
    }
    ensure!(
        identity.target == TARGET,
        "unsupported release build target"
    );

    if require_current_executable {
        let executing =
            fs::canonicalize(std::env::current_exe().context("resolve executing Xcos binary")?)?;
        ensure!(
            executing == root.join(SERVER_BINARY),
            "release must be verified by the Xcos binary physically contained by RELEASE_ROOT"
        );
    }

    let manifest_path = root.join(MANIFEST_NAME);
    let manifest_bytes = read_small_regular_file(
        &manifest_path,
        0o444,
        MAX_MANIFEST_BYTES,
        production,
        "release manifest",
    )?;
    let manifest =
        std::str::from_utf8(&manifest_bytes).context("release manifest must be canonical UTF-8")?;
    let expected_header = manifest_header()?;
    let records = manifest
        .strip_prefix(&expected_header)
        .context("release manifest identity differs from the executing binary")?;
    let expected = parse_entries(records)?;
    validate_exact_layout(&expected)?;

    let mut seen = BTreeSet::new();
    let mut counters = Counters::default();
    validate_directory(
        &root,
        Path::new(""),
        production,
        &expected,
        &mut seen,
        &mut counters,
    )?;
    ensure!(
        seen.len() == expected.len(),
        "release is missing one or more manifest entries"
    );

    let lock = read_small_regular_file(
        &root.join("config/mediamtx.lock"),
        0o444,
        1024 * 1024,
        production,
        "MediaMTX lock",
    )?;
    ensure!(
        lock == MEDIAMTX_LOCK,
        "release MediaMTX lock differs from the binary contract"
    );
    let config = read_small_regular_file(
        &root.join("config/mediamtx.yml"),
        0o444,
        1024 * 1024,
        production,
        "MediaMTX configuration",
    )?;
    ensure!(
        sha256_hex(&config) == identity.mediamtx_config_sha256,
        "release MediaMTX configuration differs from the binary contract"
    );
    let Some(ManifestEntry::File { sha256, .. }) = expected.get("bin/mediamtx") else {
        bail!("release manifest omits the MediaMTX binary");
    };
    ensure!(
        sha256 == &identity.mediamtx_sha256,
        "release MediaMTX binary differs from the pinned companion contract"
    );
    let web_manifest = read_small_regular_file(
        &root.join("share/web-assets.json"),
        0o444,
        1024 * 1024,
        production,
        "embedded Web inventory",
    )?;
    ensure!(
        web_manifest == crate::static_assets::MANIFEST.as_bytes(),
        "release Web inventory differs from the executing binary"
    );
    crate::static_assets::embedded_contract_sha256()?;

    Ok(identity)
}

fn validate_release_root(root: &Path) -> Result<PathBuf> {
    ensure!(root.is_absolute(), "RELEASE_ROOT must be absolute");
    ensure!(
        root.components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_))),
        "RELEASE_ROOT must be a normalized absolute path"
    );
    let named = fs::symlink_metadata(root).context("inspect RELEASE_ROOT")?;
    if named.file_type().is_symlink() {
        return validate_current_release_pointer(root);
    }
    ensure!(
        named.is_dir() && !named.file_type().is_symlink(),
        "RELEASE_ROOT must be a real directory"
    );
    let canonical = fs::canonicalize(root).context("resolve RELEASE_ROOT")?;
    ensure!(
        canonical.as_os_str() == root.as_os_str(),
        "RELEASE_ROOT and every parent component must be physical and normalized"
    );
    ensure!(
        canonical.ends_with(RELOCATABLE_RELEASE_SUFFIX),
        "RELEASE_ROOT must end in the physical release path for this Xcos version"
    );
    Ok(canonical)
}

/// One intentional deployment pointer; arbitrary symlinks inside release inputs stay forbidden.
fn validate_current_release_pointer(pointer: &Path) -> Result<PathBuf> {
    ensure!(
        pointer.ends_with("opt/isarmg/xcos/current"),
        "unsupported release pointer"
    );
    let installation = pointer
        .parent()
        .context("release installation directory is missing")?;
    ensure!(
        fs::canonicalize(installation)?.as_os_str() == installation.as_os_str(),
        "release pointer parent must be physical"
    );
    let production = installation == Path::new("/opt/isarmg/xcos");
    ensure!(
        pointer.as_os_str() == fs::canonicalize(installation)?.join("current").as_os_str(),
        "release pointer must use its exact physical spelling"
    );
    require_directory(
        installation,
        0o755,
        production,
        "release installation directory",
    )?;
    require_directory(
        &installation.join("releases"),
        0o755,
        production,
        "releases directory",
    )?;
    let before = fs::symlink_metadata(pointer)?;
    ensure!(
        before.file_type().is_symlink(),
        "release pointer must be a symlink"
    );
    #[cfg(unix)]
    ensure!(
        before.uid() == fs::metadata(installation)?.uid(),
        "release pointer owner differs from its installation"
    );
    let expected = installation.join("releases").join(VERSION);
    let target = fs::read_link(pointer)?;
    ensure!(
        target == expected || target == Path::new("releases").join(VERSION),
        "release pointer targets a different immutable release"
    );
    ensure!(
        fs::canonicalize(pointer)? == expected,
        "release pointer target must be physical"
    );
    let after = fs::symlink_metadata(pointer)?;
    #[cfg(unix)]
    ensure!(
        before.dev() == after.dev() && before.ino() == after.ino(),
        "release pointer changed during validation"
    );
    ensure!(
        fs::read_link(pointer)? == target,
        "release pointer changed during validation"
    );
    validate_release_root(&expected)
}

pub(crate) fn executing_release_root() -> Result<PathBuf> {
    let binary = fs::canonicalize(std::env::current_exe()?)?;
    let root = binary
        .parent()
        .and_then(Path::parent)
        .context("executing release root is unavailable")?;
    verify_release(root)?;
    Ok(root.to_path_buf())
}

fn parse_entries(records: &str) -> Result<BTreeMap<String, ManifestEntry>> {
    ensure!(
        !records.is_empty(),
        "release manifest has no payload records"
    );
    ensure!(
        records.ends_with('\n'),
        "release manifest must end in a newline"
    );
    let mut entries = BTreeMap::new();
    let mut previous: Option<&str> = None;
    for line in records.lines() {
        ensure!(!line.is_empty(), "release manifest contains a blank line");
        ensure!(
            entries.len() < MAX_ENTRIES,
            "release manifest exceeds the entry limit"
        );
        let fields: Vec<&str> = line.split(' ').collect();
        let (path, entry) = match fields.as_slice() {
            ["directory", mode, path] => {
                validate_relative_path(path)?;
                (
                    *path,
                    ManifestEntry::Directory {
                        mode: parse_mode(mode)?,
                    },
                )
            }
            ["file", mode, size, sha256, path] => {
                validate_relative_path(path)?;
                validate_sha256(sha256)?;
                let size = size.parse::<u64>().context("invalid release file size")?;
                ensure!(
                    size <= MAX_FILE_BYTES,
                    "release file exceeds its size limit"
                );
                (
                    *path,
                    ManifestEntry::File {
                        mode: parse_mode(mode)?,
                        size,
                        sha256: (*sha256).to_owned(),
                    },
                )
            }
            _ => bail!("release manifest contains an invalid or unknown record"),
        };
        ensure!(
            path != MANIFEST_NAME,
            "release manifest must not list itself"
        );
        if let Some(previous) = previous {
            ensure!(
                previous < path,
                "release manifest paths must be strictly sorted"
            );
        }
        previous = Some(path);
        ensure!(
            entries.insert(path.to_owned(), entry).is_none(),
            "release manifest contains a duplicate path"
        );
    }
    Ok(entries)
}

fn validate_exact_layout(entries: &BTreeMap<String, ManifestEntry>) -> Result<()> {
    for directory in FIXED_DIRECTORIES {
        ensure!(
            matches!(
                entries.get(*directory),
                Some(ManifestEntry::Directory { mode: 0o555 })
            ),
            "release manifest is missing required directory {directory}"
        );
    }
    for (file, expected_mode) in FIXED_FILES {
        ensure!(
            matches!(
                entries.get(*file),
                Some(ManifestEntry::File { mode, .. }) if mode == expected_mode
            ),
            "release manifest is missing required file {file}"
        );
    }

    for (path, entry) in entries {
        match entry {
            ManifestEntry::Directory { mode } => {
                ensure!(
                    *mode == 0o555 && FIXED_DIRECTORIES.contains(&path.as_str()),
                    "release contains an unexpected directory or mode: {path}"
                );
            }
            ManifestEntry::File { mode, .. } => {
                ensure!(
                    FIXED_FILES
                        .iter()
                        .any(|(fixed, expected)| *fixed == path && expected == mode),
                    "release contains an unexpected file or mode: {path}"
                );
            }
        }
    }
    Ok(())
}

fn validate_directory(
    root: &Path,
    relative: &Path,
    require_root_owned: bool,
    expected: &BTreeMap<String, ManifestEntry>,
    seen: &mut BTreeSet<String>,
    counters: &mut Counters,
) -> Result<()> {
    let directory = root.join(relative);
    let before = fs::symlink_metadata(&directory)?;
    ensure!(
        before.is_dir() && !before.file_type().is_symlink(),
        "release contains an invalid directory"
    );
    if !relative.as_os_str().is_empty() {
        let relative = normalized_relative(relative)?;
        let Some(ManifestEntry::Directory { mode }) = expected.get(&relative) else {
            bail!("release contains an unexpected directory: {relative}");
        };
        require_directory(
            &directory,
            *mode,
            require_root_owned,
            &format!("release directory {relative}"),
        )?;
        ensure!(seen.insert(relative), "duplicate release directory");
    }

    let mut children = fs::read_dir(&directory)?.collect::<std::io::Result<Vec<_>>>()?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let name = child
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("release contains a non-UTF-8 name"))?;
        validate_name(&name)?;
        let child_relative = relative.join(name);
        let child_string = normalized_relative(&child_relative)?;
        if child_string == MANIFEST_NAME {
            continue;
        }
        counters.entries += 1;
        ensure!(
            counters.entries <= MAX_ENTRIES,
            "release contains too many entries"
        );
        let metadata = fs::symlink_metadata(child.path())?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "release contains a symbolic link: {child_string}"
        );
        if metadata.is_dir() {
            validate_directory(
                root,
                &child_relative,
                require_root_owned,
                expected,
                seen,
                counters,
            )?;
            continue;
        }
        ensure!(
            metadata.is_file(),
            "release contains a special file: {child_string}"
        );
        let Some(ManifestEntry::File { mode, size, sha256 }) = expected.get(&child_string) else {
            bail!("release contains an unexpected file: {child_string}");
        };
        let actual = hash_regular_file(
            &child.path(),
            *mode,
            require_root_owned,
            &format!("release file {child_string}"),
        )?;
        ensure!(
            actual.size == *size && actual.sha256 == *sha256,
            "release file size or SHA-256 mismatch: {child_string}"
        );
        counters.bytes = counters
            .bytes
            .checked_add(*size)
            .context("release byte count overflow")?;
        ensure!(
            counters.bytes <= MAX_RELEASE_BYTES,
            "release exceeds its total size limit"
        );
        ensure!(seen.insert(child_string), "duplicate release file");
    }
    let after = fs::symlink_metadata(&directory)?;
    #[cfg(unix)]
    ensure!(
        before.dev() == after.dev() && before.ino() == after.ino(),
        "release directory changed during verification"
    );
    Ok(())
}

fn mediamtx_identity() -> Result<MediaMtxIdentity> {
    let lock = std::str::from_utf8(MEDIAMTX_LOCK).context("embedded MediaMTX lock is not UTF-8")?;
    let values: Vec<&str> = lock
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    ensure!(
        values.len() == 3,
        "embedded MediaMTX lock must contain exactly three fields"
    );
    let version = values[0]
        .strip_prefix("version=")
        .context("embedded MediaMTX lock has no version")?;
    let platform = values[1]
        .strip_prefix("platform=")
        .context("embedded MediaMTX lock has no platform")?;
    let sha256 = values[2]
        .strip_prefix("sha256=")
        .context("embedded MediaMTX lock has no SHA-256")?;
    ensure!(version == "v1.20.0", "unsupported MediaMTX version");
    ensure!(platform == "linux_amd64", "unsupported MediaMTX platform");
    validate_sha256(sha256)?;
    Ok(MediaMtxIdentity {
        version: version.to_owned(),
        platform: platform.to_owned(),
        sha256: sha256.to_owned(),
    })
}

fn hash_regular_file(
    path: &Path,
    mode: u32,
    require_root_owned: bool,
    label: &str,
) -> Result<FileHash> {
    let mut file = open_regular_file(path, mode, require_root_owned, label)?;
    let size = file.metadata()?.len();
    ensure!(size <= MAX_FILE_BYTES, "{label} exceeds its size limit");
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    revalidate_open_file(path, &file, size, label)?;
    Ok(FileHash {
        size,
        sha256: hex::encode(hasher.finalize()),
    })
}

fn read_small_regular_file(
    path: &Path,
    mode: u32,
    limit: u64,
    require_root_owned: bool,
    label: &str,
) -> Result<Vec<u8>> {
    let mut file = open_regular_file(path, mode, require_root_owned, label)?;
    let size = file.metadata()?.len();
    ensure!(size <= limit, "{label} exceeds its size limit");
    let mut bytes = Vec::with_capacity(size as usize);
    file.read_to_end(&mut bytes)?;
    revalidate_open_file(path, &file, size, label)?;
    Ok(bytes)
}

fn open_regular_file(
    path: &Path,
    expected_mode: u32,
    require_root_owned: bool,
    label: &str,
) -> Result<File> {
    let named = fs::symlink_metadata(path).with_context(|| format!("inspect {label}"))?;
    ensure!(
        named.is_file() && !named.file_type().is_symlink(),
        "{label} must be a regular non-symlink file"
    );
    #[cfg(unix)]
    {
        ensure!(
            named.nlink() == 1,
            "{label} must not have hard-link aliases"
        );
        ensure!(
            named.permissions().mode() & 0o7777 == expected_mode,
            "{label} has an unexpected mode"
        );
        if require_root_owned {
            ensure!(
                named.uid() == 0 && named.gid() == 0,
                "{label} must be owned by root"
            );
        }
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(
        (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NONBLOCK)
            .bits() as i32,
    );
    let file = options
        .open(path)
        .with_context(|| format!("open {label}"))?;
    revalidate_open_file(path, &file, named.len(), label)?;
    Ok(file)
}

fn revalidate_open_file(path: &Path, file: &File, expected_size: u64, label: &str) -> Result<()> {
    let opened = file.metadata()?;
    let named = fs::symlink_metadata(path)?;
    ensure!(
        opened.is_file()
            && named.is_file()
            && !named.file_type().is_symlink()
            && opened.len() == expected_size
            && named.len() == expected_size,
        "{label} changed while it was verified"
    );
    #[cfg(unix)]
    ensure!(
        opened.dev() == named.dev()
            && opened.ino() == named.ino()
            && opened.nlink() == 1
            && named.nlink() == 1
            && opened.mtime() == named.mtime()
            && opened.mtime_nsec() == named.mtime_nsec(),
        "{label} changed identity while it was verified"
    );
    Ok(())
}

fn require_directory(path: &Path, mode: u32, require_root_owned: bool, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path).with_context(|| format!("inspect {label}"))?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "{label} must be a real directory"
    );
    #[cfg(unix)]
    {
        ensure!(
            metadata.permissions().mode() & 0o7777 == mode,
            "{label} has an unexpected mode"
        );
        if require_root_owned {
            ensure!(
                metadata.uid() == 0 && metadata.gid() == 0,
                "{label} must be owned by root"
            );
        }
    }
    Ok(())
}

fn validate_relative_path(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= 1024,
        "invalid release path"
    );
    let path = Path::new(value);
    ensure!(!path.is_absolute(), "release paths must be relative");
    ensure!(
        path.components()
            .all(|component| matches!(component, Component::Normal(_))),
        "release path contains an unsafe component"
    );
    for component in path.components() {
        if let Component::Normal(name) = component {
            validate_name(name.to_str().context("release path is not UTF-8")?)?;
        }
    }
    ensure!(
        path.to_str() == Some(value),
        "release path is not canonical"
    );
    Ok(())
}

fn validate_name(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value != "."
            && value != ".."
            && value
                .bytes()
                .all(|byte| { byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-') }),
        "release contains a non-portable name"
    );
    Ok(())
}

fn normalized_relative(path: &Path) -> Result<String> {
    path.to_str()
        .context("release path is not UTF-8")
        .map(|value| value.replace(std::path::MAIN_SEPARATOR, "/"))
}

fn parse_mode(value: &str) -> Result<u32> {
    ensure!(
        value.len() == 3 && value.bytes().all(|byte| matches!(byte, b'0'..=b'7')),
        "release mode must be three octal digits"
    );
    u32::from_str_radix(value, 8).context("release mode is invalid")
}

fn validate_sha256(value: &str) -> Result<()> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')),
        "SHA-256 values must contain 64 lowercase hexadecimal digits"
    );
    Ok(())
}

fn is_source_revision(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn deployment_pointer_accepts_only_the_owned_current_immutable_root() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let temporary = tempfile::tempdir().unwrap();
        let installation = temporary.path().join("opt/isarmg/xcos");
        let releases = installation.join("releases");
        let physical = releases.join(VERSION);
        fs::create_dir_all(&physical).unwrap();
        fs::set_permissions(&installation, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&releases, fs::Permissions::from_mode(0o755)).unwrap();
        let current = installation.join("current");
        symlink(&physical, &current).unwrap();
        assert_eq!(validate_release_root(&current).unwrap(), physical);
        let arbitrary = installation.join("alias");
        symlink(&physical, &arbitrary).unwrap();
        assert!(validate_release_root(&arbitrary).is_err());
        fs::remove_file(&current).unwrap();
        symlink("releases/0.0.0", &current).unwrap();
        assert!(validate_release_root(&current).is_err());
        fs::remove_file(&current).unwrap();
        symlink(&physical, &current).unwrap();
        fs::set_permissions(&installation, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(validate_release_root(&current).is_err());
        fs::set_permissions(&installation, fs::Permissions::from_mode(0o755)).unwrap();
        fs::remove_file(&current).unwrap();
        let alias = temporary.path().join("untrusted-parent");
        symlink(&installation, &alias).unwrap();
        assert!(validate_release_root(&alias.join("current")).is_err());
    }

    #[test]
    fn identity_is_exact_and_manifest_header_is_canonical() {
        let identity = identity().unwrap();
        assert_eq!(identity.application, PRODUCT);
        assert_eq!(identity.application_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(identity.target, env!("XCOS_BUILD_TARGET"));
        assert_eq!(identity.wire_protocol, "xcos-wire-v1");
        assert_eq!(identity.api_prefix, "/api/v1");
        assert_eq!(
            identity.schema_revision,
            crate::sqlite::CURRENT_SCHEMA_REVISION
        );
        assert_eq!(identity.credential_envelope_revision, 1);
        assert!(manifest_header().unwrap().ends_with('\n'));
    }

    #[test]
    fn entry_parser_is_strict_and_sorted() {
        let valid = "directory 555 bin\nfile 555 3 abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd bin/app\n";
        assert!(parse_entries(valid).is_ok());
        assert!(parse_entries("directory 755 bin\n").is_ok());
        assert!(parse_entries("directory 555 ../bin\n").is_err());
        assert!(parse_entries("directory 555 z\ndirectory 555 a\n").is_err());
        assert!(parse_entries("compat 555 bin\n").is_err());
        assert!(parse_entries("directory 0555 bin\n").is_err());
    }

    #[test]
    fn companion_contract_explicitly_disables_unmanaged_protocols() {
        let config = std::str::from_utf8(MEDIAMTX_CONFIG).unwrap();
        for protocol in ["rtmp", "srt", "moq"] {
            let prefix = format!("{protocol}:");
            let declarations: Vec<_> = config
                .lines()
                .filter(|line| line.starts_with(&prefix))
                .collect();
            assert_eq!(declarations, [format!("{protocol}: no")]);
        }
    }

    #[test]
    fn source_revision_and_hashes_have_exact_encodings() {
        assert!(is_source_revision(
            "0123456789abcdef0123456789abcdef01234567"
        ));
        assert!(!is_source_revision("v0.3.0"));
        assert!(validate_sha256(&"a".repeat(64)).is_ok());
        assert!(validate_sha256(&"A".repeat(64)).is_err());
    }

    #[test]
    fn exact_layout_requires_inventory_and_rejects_external_web() {
        let mut entries = BTreeMap::new();
        for directory in FIXED_DIRECTORIES {
            entries.insert(
                (*directory).to_owned(),
                ManifestEntry::Directory { mode: 0o555 },
            );
        }
        for (file, mode) in FIXED_FILES {
            entries.insert(
                (*file).to_owned(),
                ManifestEntry::File {
                    mode: *mode,
                    size: 1,
                    sha256: "a".repeat(64),
                },
            );
        }
        assert!(validate_exact_layout(&entries).is_ok());
        entries.insert("web".into(), ManifestEntry::Directory { mode: 0o555 });
        assert!(validate_exact_layout(&entries).is_err());
    }
}
