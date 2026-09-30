use codex_proxy_basispoints_plugin::{PLUGIN_ID, PluginState, author_manifest, plugin};
use gateway_plugin_sdk::client::{PluginSession, SessionConfig, SessionError};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("--third-party-notices") {
        print!("{}", include_str!("../../THIRD_PARTY_NOTICES.md"));
        return Ok(());
    }
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
    let state = PluginState::new();
    session.run(plugin(state)?).await?;
    Ok(())
}
