use serde::Deserialize;
use std::sync::LazyLock;

// The browser client and Rust server share these protocol constants instead of duplicating API paths.
const CONTRACT_SOURCE: &str = include_str!("../web/src/protocol-contract.json");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolContract {
    pub wire_protocol: String,
    pub edge_protocol: String,
    pub api_prefix: String,
    pub media_auth_path: String,
    pub media_jwt_protocol: String,
    pub media_jwt_issuer: String,
    pub media_jwt_audience: String,
    pub media_jwt_kind: String,
}

pub static CONTRACT: LazyLock<ProtocolContract> = LazyLock::new(|| {
    let contract: ProtocolContract = serde_json::from_str(CONTRACT_SOURCE)
        .expect("embedded protocol contract must be valid JSON");
    assert_eq!(contract.wire_protocol, "xcos-wire-v2");
    assert_eq!(
        contract.edge_protocol,
        crate::models::CLIENT_PAIRING_PROTOCOL
    );
    assert_eq!(contract.api_prefix, "/api/v1");
    assert_eq!(contract.media_auth_path, "/internal/v1/media/auth");
    assert_eq!(contract.media_jwt_protocol, "xcos-media-jwt-v1");
    assert_eq!(contract.media_jwt_issuer, "xcos/1.0.0");
    assert_eq!(contract.media_jwt_audience, "xcos-mediamtx/1.20.0");
    assert_eq!(contract.media_jwt_kind, "media");
    contract
});

#[cfg(test)]
mod tests {
    use super::CONTRACT;

    #[test]
    fn rust_web_and_mediamtx_share_the_current_protocol_contract() {
        assert_eq!(CONTRACT.media_jwt_issuer, "xcos/1.0.0");

        let web = include_str!("../web/src/api.ts");
        assert!(web.contains("export function apiPath"));
        assert!(!web.contains("\"/api/"));
        let vite = include_str!("../web/vite.config.ts");
        assert!(vite.contains("\"/api/v1\": \"http://127.0.0.1:8080\""));
        assert!(!vite.contains("\"/api\":"));

        let routes = include_str!("routes/mod.rs");
        assert!(routes.contains(".nest(&CONTRACT.api_prefix, api)"));
        assert!(routes.contains(".route(&CONTRACT.media_auth_path, post(media_auth))"));

        let config = include_str!("../config/mediamtx.yml");
        assert!(
            config.lines().any(|line| line
                .strip_prefix("authHTTPAddress: ")
                .is_some_and(|url| url.ends_with(&CONTRACT.media_auth_path))),
            "MediaMTX config must use the embedded current callback path"
        );
    }
}
