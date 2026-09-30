use gateway_plugin_sdk::{
    PluginFault,
    call::host::{AuthCredential, AuthGetRequest},
    call::management::{ManagementRequest, ManagementResponse},
    client::{TypedCall, TypedReply},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::basispoints::{self, BasispointsError};

use super::{
    response::{ApiError, ApiResult, json_reply, raw_response},
    validation::{MAXIMUM_BODY_BYTES, bounded_id, decode_json, require_no_query},
};

#[derive(Clone, Default)]
pub struct PluginState;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BasispointsRequest {
    account_id: String,
    request: Value,
}

impl PluginState {
    pub fn new() -> Self {
        Self
    }
}

pub(crate) async fn handle(
    state: PluginState,
    call: TypedCall<ManagementRequest>,
) -> Result<TypedReply<ManagementResponse>, PluginFault> {
    route(state, call).await.or_else(ApiError::into_reply)
}

async fn route(state: PluginState, call: TypedCall<ManagementRequest>) -> ApiResult {
    if call.payload.len() > MAXIMUM_BODY_BYTES {
        return Err(ApiError::invalid("请求正文超过 140 KB 限制"));
    }
    require_no_query(&call.request)?;
    match (call.request.method.as_str(), call.request.path.as_str()) {
        ("GET", "api/status") => status(state).await,
        ("POST", "api/basispoints/responses") => basispoints_responses(state, call).await,
        _ => Err(ApiError::new(404, "not_found", "未找到插件管理接口")),
    }
}

async fn status(_state: PluginState) -> ApiResult {
    json_reply(&json!({
        "ready": true,
        "transport": "host.http",
    }))
}

async fn basispoints_responses(
    _state: PluginState,
    call: TypedCall<ManagementRequest>,
) -> ApiResult {
    let wrapper: BasispointsRequest = decode_json(&call)?;
    if !bounded_id(&wrapper.account_id) {
        return Err(ApiError::invalid("宿主账号标识无效"));
    }
    if !wrapper.request.is_object() {
        return Err(ApiError::invalid("Responses 请求必须是 JSON 对象"));
    }
    let credential = resolve_credential(&call, &wrapper.account_id).await?;
    let response = basispoints::request(&call.host, &credential, wrapper.request)
        .await
        .map_err(map_basispoints_error)?;
    raw_response(response.status, &response.content_type, response.body)
}

async fn resolve_credential(
    call: &TypedCall<ManagementRequest>,
    account_id: &str,
) -> Result<AuthCredential, ApiError> {
    let payload = serde_json::to_vec(&AuthGetRequest {
        account_id: account_id.to_owned(),
    })
    .map_err(|_| ApiError::new(500, "encoding", "账号请求编码失败"))?;
    let reply = call
        .host
        .call("host.auth.get", json!({}), payload)
        .await
        .map_err(|_| ApiError::new(503, "account_unavailable", "宿主账号凭据暂不可用"))?;
    if reply.result != json!({}) {
        return Err(ApiError::new(
            503,
            "account_unavailable",
            "宿主账号凭据响应无效",
        ));
    }
    let credential: AuthCredential = serde_json::from_slice(&reply.payload)
        .map_err(|_| ApiError::new(503, "account_unavailable", "宿主账号凭据响应无效"))?;
    if credential.account_id != account_id
        || credential.provider_id != "openai"
        || !credential
            .facts
            .authentication_kind
            .eq_ignore_ascii_case("oauth")
    {
        return Err(ApiError::invalid("宿主账号不是 OpenAI OAuth 账号"));
    }
    if credential.facts.material.contains_key("apiKey")
        || credential.facts.material.contains_key("api_key")
    {
        return Err(ApiError::invalid("宿主账号不是 OpenAI OAuth 账号"));
    }
    if !credential
        .facts
        .material
        .get("accessToken")
        .or_else(|| credential.facts.material.get("access_token"))
        .and_then(Value::as_str)
        .is_some_and(|token| !token.is_empty())
    {
        return Err(ApiError::invalid("宿主账号没有可用 access token"));
    }
    Ok(credential)
}

fn map_basispoints_error(error: BasispointsError) -> ApiError {
    match error {
        BasispointsError::InvalidRequest(message) => ApiError::invalid(message),
        BasispointsError::UnsupportedResponse => ApiError::new(
            502,
            "upstream_response",
            "Basis Points 返回了不支持的内容类型",
        ),
        BasispointsError::ResponseTooLarge => {
            ApiError::new(502, "upstream_response", "Basis Points 响应超过插件限制")
        }
        BasispointsError::Upstream => ApiError::new(502, "upstream", "Basis Points 请求失败"),
        BasispointsError::Relay(error) => ApiError::new(error.status, error.code, error.message),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn rejects_malformed_basispoints_wrapper_before_forwarding() {
        let unknown_field = serde_json::from_value::<super::BasispointsRequest>(json!({
            "accountId": "acct-1",
            "request": {},
            "unexpected": true,
        }));
        assert!(unknown_field.is_err());

        let scalar_request = serde_json::from_value::<super::BasispointsRequest>(json!({
            "accountId": "acct-1",
            "request": "not-an-object",
        }))
        .expect("wrapper itself should decode");
        assert!(!scalar_request.request.is_object());
    }
}
