use gateway_plugin_sdk::{
    ErrorCode, PluginFault,
    call::host::{StateGetRequest, StateGetResult, StatePutRequest, StatePutResult},
    client::{HostClient, SessionError},
};
use serde::{Serialize, de::DeserializeOwned};

pub(crate) async fn get_state(
    host: &HostClient,
    namespace: &str,
    key: &str,
) -> Result<StateGetResult, PluginFault> {
    metadata(
        host,
        "host.state.get",
        &StateGetRequest {
            namespace: namespace.to_owned(),
            key: key.to_owned(),
        },
    )
    .await
    .map(|(result, _)| result)
}

pub(crate) async fn put_state(
    host: &HostClient,
    request: &StatePutRequest,
) -> Result<StatePutResult, PluginFault> {
    metadata(host, "host.state.put", request)
        .await
        .map(|(result, _)| result)
}

async fn metadata<I, O>(
    host: &HostClient,
    method: &str,
    input: &I,
) -> Result<(O, Vec<u8>), PluginFault>
where
    I: Serialize,
    O: DeserializeOwned,
{
    let reply = host
        .call(
            method,
            serde_json::to_value(input).map_err(|_| invalid_callback())?,
            Vec::new(),
        )
        .await
        .map_err(SessionError::into_plugin_fault)?;
    let result = serde_json::from_value(reply.result).map_err(|_| invalid_callback())?;
    Ok((result, reply.payload))
}

fn invalid_callback() -> PluginFault {
    PluginFault::new(ErrorCode::Fault, "宿主回调返回了无效响应")
}
