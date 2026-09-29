use gateway_plugin_sdk::client::{AuthorError, ComposedPlugin, PluginBuilder};

use crate::{PluginState, management};

pub fn plugin(state: PluginState) -> Result<ComposedPlugin, AuthorError> {
    PluginBuilder::from_json(include_bytes!("../../plugin.json"))?
        .management(management::registration(), move |call| {
            let state = state.clone();
            async move { management::handle(state, call).await }
        })?
        .build()
}
