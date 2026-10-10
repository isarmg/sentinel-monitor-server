use std::{collections::BTreeSet, env, fs, path::PathBuf};

const XCSS_SOURCE: &str = "git+https://github.com/isarmg/xcss.git?rev=";

fn locked_xcss_revision(lockfile: &str) -> String {
    let revisions = lockfile
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("source = \"")?.strip_suffix('"'))
        .filter_map(|source| source.strip_prefix(XCSS_SOURCE))
        .map(|source| source.split_once('#').expect("locked xcss source"))
        .map(|(requested, locked)| {
            assert_eq!(requested, locked, "xcss revision must be immutable");
            assert!(
                locked.len() == 40 && locked.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "xcss revision must be full hexadecimal"
            );
            locked.to_owned()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        revisions.len(),
        1,
        "the xcss package must use one immutable revision"
    );
    revisions.into_iter().next().expect("one xcss revision")
}

fn main() {
    println!("cargo:rerun-if-env-changed=XCSS_WEB_DIST");
    println!("cargo:rerun-if-env-changed=XCOS_SOURCE_REVISION");

    let target = env::var("TARGET").expect("Cargo must provide TARGET");
    assert_eq!(
        target, "x86_64-unknown-linux-gnu",
        "Xcos server builds support only x86_64-unknown-linux-gnu"
    );
    let source_revision = env::var("XCOS_SOURCE_REVISION").unwrap_or_else(|_| "unbound".to_owned());
    assert!(
        source_revision == "unbound"
            || (source_revision.len() == 40
                && source_revision
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))),
        "XCOS_SOURCE_REVISION must be unbound or a full lowercase 40-hex Git commit"
    );
    println!("cargo:rustc-env=XCOS_BUILD_TARGET={target}");
    println!("cargo:rustc-env=XCOS_SOURCE_REVISION={source_revision}");
    let xcss_revision =
        locked_xcss_revision(&fs::read_to_string("Cargo.lock").expect("read Cargo.lock"));
    println!("cargo:rustc-env=XCSS_REVISION={xcss_revision}");
    println!("cargo:rerun-if-changed=Cargo.lock");

    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"));
    let protocol_path = root.join("web/src/protocol-contract.json");
    println!("cargo:rerun-if-changed={}", protocol_path.display());
    let protocol: serde_json::Value =
        serde_json::from_slice(&fs::read(protocol_path).expect("read product protocol authority"))
            .expect("product protocol must be valid JSON");
    let edge = protocol["edge_protocol"]
        .as_str()
        .expect("edge protocol identity is required");
    assert_eq!(
        edge, "xcos-edge-v1",
        "only the current product edge contract is implemented"
    );
    println!("cargo:rustc-env=XCOS_EDGE_PROTOCOL={edge}");
    let web_root = env::var_os("XCSS_WEB_DIST")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("web/dist"));
    xcss::web_assets::build::generate(&web_root).expect("build current embedded Web assets");
}
