//! Foundation-generated inventory and bytes are part of this executable.
include!(concat!(env!("OUT_DIR"), "/xcss-web-assets.rs"));

pub fn embedded_contract_sha256() -> anyhow::Result<String> {
    xcss_web_assets::verify_embedded(ASSETS, MANIFEST, DIGEST)?;
    Ok(DIGEST.to_owned())
}

pub(crate) fn embedded_manifest_sha256() -> String {
    DIGEST.to_owned()
}

pub(crate) async fn serve(
    axum::extract::State(state): axum::extract::State<crate::AppState>,
    request: axum::extract::Request,
) -> axum::response::Response {
    let response = if let Some(directory) = state.web_directory.as_deref() {
        directory.response(request.uri().path(), request.method(), request.headers())
    } else {
        xcss_web_assets::response(
            ASSETS,
            request.uri().path(),
            request.method(),
            request.headers(),
        )
    };
    response.map(axum::body::Body::from)
}
