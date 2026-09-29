use gateway_plugin_sdk::{Capability, Manifest};

const AUTHOR_MANIFEST: &[u8] = include_bytes!("../../plugin.json");

#[test]
fn author_manifest_declares_management_and_private_state() {
    let manifest = Manifest::from_author_slice(AUTHOR_MANIFEST).expect("valid author manifest");
    assert_eq!(
        manifest.plugin_id().expect("derived plugin id"),
        "david1989sky.codex-proxy-twofa"
    );
    assert_eq!(manifest.version.to_string(), "0.1.0");
    assert_eq!(
        manifest.engines.codex_proxy_rs.to_string(),
        ">=3.18.1, <4.0.0"
    );
    assert!(manifest.contributes.contains_key(&Capability::Management));
    assert!(manifest.resources.contains_key("web/index.html"));
    assert_eq!(manifest.state.len(), 1);
    assert_eq!(manifest.state[0].namespace, "twofa");
}
