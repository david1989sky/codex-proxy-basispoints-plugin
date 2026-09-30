use gateway_plugin_sdk::client::RequestCall;
use gateway_plugin_sdk::client::{AuthorError, ComposedPlugin, PluginBuilder};

use crate::{PluginState, management, middleware};

pub fn plugin(state: PluginState) -> Result<ComposedPlugin, AuthorError> {
    let middleware_state = state.clone();
    PluginBuilder::from_json(include_bytes!("../../plugin.json"))?
        .management(management::registration(), move |call| {
            let state = state.clone();
            async move { management::handle(state, call).await }
        })?
        .middleware::<RequestCall, _, _>(move |call| {
            let state = middleware_state.clone();
            async move { middleware::handle(state, call).await }
        })?
        .build()
}
