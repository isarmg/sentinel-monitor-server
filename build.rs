use std::{collections::BTreeSet, env, fs, path::PathBuf};

const UNBOUND_MANIFEST: &str = "format=sentinel-static-layout-v1\napplication=sentinel-monitor\napplication_version=0.2.14\nunbound=true\n";
const FOUNDATION_SOURCE: &str = "git+https://github.com/isarmg/sarmg-foundation-server.git?rev=";

fn locked_foundation_revision(lockfile: &str) -> String {
    let revisions = lockfile
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("source = \"")?.strip_suffix('"'))
        .filter_map(|source| source.strip_prefix(FOUNDATION_SOURCE))
        .map(|source| source.split_once('#').expect("locked Foundation source"))
        .map(|(requested, locked)| {
            assert_eq!(requested, locked, "Foundation revision must be immutable");
            assert!(
                locked.len() == 40 && locked.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "Foundation revision must be full hexadecimal"
            );
            locked.to_owned()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        revisions.len(),
        1,
        "all Foundation crates must share one revision"
    );
    revisions
        .into_iter()
        .next()
        .expect("one Foundation revision")
}

fn main() {
    println!("cargo:rerun-if-env-changed=SENTINEL_STATIC_MANIFEST_PATH");
    println!("cargo:rerun-if-env-changed=SENTINEL_SOURCE_REVISION");

    let target = env::var("TARGET").expect("Cargo must provide TARGET");
    assert_eq!(
        target, "x86_64-unknown-linux-gnu",
        "Sentinel server builds support only x86_64-unknown-linux-gnu"
    );
    let source_revision =
        env::var("SENTINEL_SOURCE_REVISION").unwrap_or_else(|_| "unbound".to_owned());
    assert!(
        source_revision == "unbound"
            || (source_revision.len() == 40
                && source_revision
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))),
        "SENTINEL_SOURCE_REVISION must be unbound or a full lowercase 40-hex Git commit"
    );
    println!("cargo:rustc-env=SENTINEL_BUILD_TARGET={target}");
    println!("cargo:rustc-env=SENTINEL_SOURCE_REVISION={source_revision}");
    let foundation_revision =
        locked_foundation_revision(&fs::read_to_string("Cargo.lock").expect("read Cargo.lock"));
    println!("cargo:rustc-env=SARMG_FOUNDATION_REVISION={foundation_revision}");
    println!("cargo:rerun-if-changed=Cargo.lock");

    let manifest = match env::var_os("SENTINEL_STATIC_MANIFEST_PATH") {
        Some(path) => {
            let path = PathBuf::from(path);
            assert!(
                path.is_absolute(),
                "SENTINEL_STATIC_MANIFEST_PATH must be absolute"
            );
            println!("cargo:rerun-if-changed={}", path.display());
            let manifest = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read static manifest: {error}"));
            assert!(
                manifest.len() <= 1024 * 1024,
                "static manifest exceeds the 1 MiB build limit"
            );
            manifest
        }
        None => UNBOUND_MANIFEST.to_string(),
    };

    let output = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"))
        .join("sentinel-static-layout.manifest");
    fs::write(output, manifest).expect("write embedded static manifest");
}
