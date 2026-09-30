use std::sync::Arc;

use codex_proxy_basispoints_plugin::{
    PLUGIN_ID, PluginState, WorkerClient, author_manifest, plugin,
};
use gateway_plugin_sdk::client::{PluginSession, SessionConfig, SessionError};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let session = PluginSession::accept(
        tokio::io::stdin(),
        tokio::io::stdout(),
        SessionConfig::default(),
    )
    .await?;
    let handshake = session.handshake();
    let manifest = author_manifest()?;
    if handshake.plugin_id != PLUGIN_ID
        || handshake.contributes != manifest.contributes
        || !handshake.configuration.is_object()
    {
        return Err(SessionError::Handshake.into());
    }
    let base_url = handshake
        .configuration
        .get("workerBaseUrl")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("http://127.0.0.1:28082");
    let digest = handshake
        .configuration
        .get("workerImageDigest")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let worker = Arc::new(WorkerClient::new(base_url)?);
    let state = PluginState::new(worker, digest);
    session.run(plugin(state)?).await?;
    Ok(())
}
