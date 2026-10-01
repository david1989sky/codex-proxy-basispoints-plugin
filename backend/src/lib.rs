mod app;
mod basispoints;
mod config;
mod management;
mod middleware;
pub mod relay;
pub mod sse;
mod tool_adapter;
mod tool_response;
pub(crate) mod usage;

pub const PLUGIN_ID: &str = "david1989sky.codex-proxy-basispoints";

pub fn author_manifest() -> Result<gateway_plugin_sdk::Manifest, gateway_plugin_sdk::ManifestError>
{
    gateway_plugin_sdk::Manifest::from_author_slice(include_bytes!("../../plugin.json"))
}

pub use app::plugin;
pub use management::PluginState;
