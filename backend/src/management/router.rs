use std::sync::Arc;

use gateway_plugin_sdk::{
    PluginFault,
    call::management::{ManagementRequest, ManagementResponse},
    client::{TypedCall, TypedReply},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::worker_client::{WorkerClient, WorkerError};

use super::{
    response::{ApiError, ApiResult, json_reply, raw_response},
    validation::{MAXIMUM_BODY_BYTES, bounded_id, decode_json, require_no_query},
};

#[derive(Clone)]
pub struct PluginState {
    pub(crate) worker: Arc<WorkerClient>,
    pub(crate) worker_image_digest: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BasispointsRequest {
    account_id: String,
    request: Value,
}

impl PluginState {
    pub fn new(worker: Arc<WorkerClient>, worker_image_digest: String) -> Self {
        Self {
            worker,
            worker_image_digest,
        }
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

async fn status(state: PluginState) -> ApiResult {
    let health = state
        .worker
        .health()
        .await
        .map_err(|error| ApiError::from_worker(503, error.to_string()))?;
    json_reply(&json!({
        "ready": health.get("ready").and_then(Value::as_bool).unwrap_or(false),
        "workerImageDigest": state.worker_image_digest,
    }))
}

async fn basispoints_responses(
    state: PluginState,
    call: TypedCall<ManagementRequest>,
) -> ApiResult {
    let wrapper: BasispointsRequest = decode_json(&call)?;
    if !bounded_id(&wrapper.account_id) {
        return Err(ApiError::invalid("宿主账号标识无效"));
    }
    if !wrapper.request.is_object() {
        return Err(ApiError::invalid("Responses 请求必须是 JSON 对象"));
    }
    let body =
        serde_json::to_vec(&wrapper).map_err(|_| ApiError::new(500, "encoding", "请求编码失败"))?;
    let synthetic = ManagementRequest {
        method: "POST".to_owned(),
        path: "api/basispoints/responses".to_owned(),
        query: String::new(),
        content_type: Some("application/json".to_owned()),
        headers: call.request.headers,
    };
    let response = state
        .worker
        .forward(&synthetic, &body)
        .await
        .map_err(map_basispoints_worker_error)?;
    raw_response(response.status, &response.content_type, response.body)
}

fn map_basispoints_worker_error(error: WorkerError) -> ApiError {
    match error {
        WorkerError::RequestTooLarge => {
            ApiError::new(413, "payload_too_large", "请求正文超过 Worker 限制")
        }
        WorkerError::ResponseTooLarge => {
            ApiError::new(502, "worker_response", "Worker 响应超过插件限制")
        }
        WorkerError::InvalidBaseUrl
        | WorkerError::InvalidPath
        | WorkerError::Transport
        | WorkerError::InvalidResponse => {
            ApiError::new(503, "worker_unavailable", "Worker 暂时不可用")
        }
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
