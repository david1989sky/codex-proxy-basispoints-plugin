use gateway_plugin_sdk::{Capability, Manifest, Stage};

const AUTHOR_MANIFEST: &[u8] = include_bytes!("../../plugin.json");

#[test]
fn author_manifest_declares_basispoints_management_contract() {
    let mut manifest: Manifest = serde_json::from_slice(AUTHOR_MANIFEST).expect("manifest json");
    if let Err(error) = manifest.normalize_author() {
        panic!("manifest error: {error:?}");
    }
    assert_eq!(
        manifest.plugin_id().expect("derived plugin id"),
        "david1989sky.codex-proxy-basispoints"
    );
    assert_eq!(manifest.version.to_string(), env!("CARGO_PKG_VERSION"));
    assert_eq!(
        manifest.engines.codex_proxy_rs.to_string(),
        ">=3.18.1, <4.0.0"
    );
    assert!(manifest.contributes.contains_key(&Capability::Management));
    let middleware = &manifest.contributes[&Capability::Middleware];
    assert_eq!(middleware.version, 3);
    assert_eq!(middleware.stages, [Stage::Request]);
    assert_eq!(middleware.input_formats, ["openai"]);
    assert_eq!(middleware.output_formats, ["openai"]);
    assert_eq!(manifest.resources.len(), 3);
    assert_eq!(
        manifest.resources.get("web/index.html").map(String::as_str),
        Some("text/html")
    );
    assert_eq!(
        manifest.resources.get("web/app.js").map(String::as_str),
        Some("text/javascript")
    );
    assert_eq!(
        manifest.resources.get("web/app.css").map(String::as_str),
        Some("text/css")
    );
    assert!(manifest.state.is_empty());
}
