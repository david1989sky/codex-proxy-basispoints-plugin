use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use gateway_plugin_sdk::{
    PluginFault,
    call::{
        host::StatePutRequest,
        management::{ManagementRequest, ManagementResponse},
    },
    client::{TypedCall, TypedReply},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::worker_client::{WorkerClient, WorkerError, WorkerResponse};

use super::{
    response::{ApiError, ApiResult, json_reply, raw_json, raw_response},
    validation::{
        MAXIMUM_BODY_BYTES, bounded_id, decode_json, require_empty_object, require_no_query,
    },
};

#[derive(Clone)]
pub struct PluginState {
    pub(crate) worker: Arc<WorkerClient>,
    pub(crate) worker_image_digest: String,
}

#[derive(Clone, Serialize)]
pub(crate) struct MigrationMarker {
    pub(crate) source: String,
    pub(crate) record_count: u32,
    pub(crate) updated_at: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MigrationState {
    migration_version: String,
    legacy_vault_mounted: bool,
    record_count: u32,
    updated_at: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BasispointsRequest {
    account_id: String,
    request: Value,
}

const MIGRATION_NAMESPACE: &str = "twofa";
const MIGRATION_KEY: &str = "migration";
const MIGRATION_VERSION: &str = "legacy-vault-v1";
const MIGRATION_SOURCE: &str = "legacy-cpr-twofa-vault";

impl PluginState {
    pub fn new(worker: Arc<WorkerClient>, worker_image_digest: String) -> Self {
        Self {
            worker,
            worker_image_digest,
        }
    }
}

#[derive(Deserialize)]
#[serde(
    tag = "operation",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Operation {
    GetCredentials {
        account_id: String,
    },
    DeleteCredentials {
        account_id: String,
    },
    Reauthorize {
        account_id: String,
        submission_id: String,
        text: Option<String>,
    },
    StartTask {
        text: String,
        submission_id: String,
        settings: Value,
        outbound_proxy_id: Option<String>,
    },
    GetTask {
        task_id: String,
    },
    CancelTask {
        task_id: String,
    },
    RetryTask {
        task_id: String,
    },
    DeleteTask {
        task_id: String,
    },
    GetScreen {
        task_id: String,
        item_id: String,
    },
    SendInput {
        task_id: String,
        item_id: String,
        data: Value,
    },
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
        ("GET", "api/migration") => migration(state, call).await,
        ("POST", "api/migration/import") => import_migration(state, call).await,
        ("POST", "api/basispoints/responses") => basispoints_responses(state, call).await,
        ("POST", "api/request") => request(state, call).await,
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
        "legacyVaultMounted": true,
    }))
}

async fn migration(_state: PluginState, call: TypedCall<ManagementRequest>) -> ApiResult {
    let result =
        crate::host_calls::get_state(&call.host, MIGRATION_NAMESPACE, MIGRATION_KEY).await?;
    let marker = match result.record {
        Some(record) => {
            if record.schema_version != 1 {
                return Err(ApiError::new(500, "invalid_state", "迁移状态版本不受支持"));
            }
            let value = serde_json::from_value::<MigrationState>(record.value)
                .map_err(|_| ApiError::new(500, "invalid_state", "迁移状态无效"))?;
            Some(MigrationMarker {
                source: MIGRATION_SOURCE.to_owned(),
                record_count: value.record_count,
                updated_at: value.updated_at,
            })
        }
        None => None,
    };
    json_reply(&json!({
        "legacyVaultMounted": true,
        "imported": marker.is_some(),
        "marker": marker,
    }))
}

async fn import_migration(state: PluginState, call: TypedCall<ManagementRequest>) -> ApiResult {
    let value: Value = decode_json(&call)?;
    require_empty_object(&value)?;
    state
        .worker
        .health()
        .await
        .map_err(|error| ApiError::from_worker(503, error.to_string()))?;
    let existing =
        crate::host_calls::get_state(&call.host, MIGRATION_NAMESPACE, MIGRATION_KEY).await?;
    let result = match existing.record {
        Some(record) => serde_json::from_value::<MigrationState>(record.value)
            .map_err(|_| ApiError::new(500, "invalid_state", "迁移状态无效"))?,
        None => {
            let value = MigrationState {
                migration_version: MIGRATION_VERSION.to_owned(),
                legacy_vault_mounted: true,
                record_count: 0,
                updated_at: now(),
            };
            crate::host_calls::put_state(
                &call.host,
                &StatePutRequest {
                    namespace: MIGRATION_NAMESPACE.to_owned(),
                    key: MIGRATION_KEY.to_owned(),
                    value: serde_json::to_value(&value)?,
                    expected_version: None,
                },
            )
            .await?;
            value
        }
    };
    let marker = MigrationMarker {
        source: MIGRATION_SOURCE.to_owned(),
        record_count: result.record_count,
        updated_at: result.updated_at,
    };
    json_reply(&json!({ "imported": true, "marker": marker }))
}

async fn request(state: PluginState, call: TypedCall<ManagementRequest>) -> ApiResult {
    let operation: Operation = decode_json(&call)?;
    let (method, path, body) = operation_request(operation)?;
    let synthetic = ManagementRequest {
        method: method.to_owned(),
        path,
        query: String::new(),
        content_type: (!body.is_empty()).then(|| "application/json".to_owned()),
        headers: call.request.headers,
    };
    let response = state
        .worker
        .forward(&synthetic, &body)
        .await
        .map_err(|error| ApiError::from_worker(503, error.to_string()))?;
    forward_response(response)
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

fn operation_request(operation: Operation) -> Result<(&'static str, String, Vec<u8>), ApiError> {
    let request = match operation {
        Operation::GetCredentials { account_id } => (
            "GET",
            format!("api/accounts/{account_id}/credentials"),
            Vec::new(),
        ),
        Operation::DeleteCredentials { account_id } => (
            "DELETE",
            format!("api/accounts/{account_id}/credentials"),
            Vec::new(),
        ),
        Operation::Reauthorize {
            account_id,
            submission_id,
            text,
        } => {
            let mut body = serde_json::Map::new();
            body.insert("submissionId".to_owned(), Value::String(submission_id));
            if let Some(text) = text {
                body.insert("text".to_owned(), Value::String(text));
            }
            (
                "POST",
                format!("api/accounts/{account_id}/reauthorize"),
                serde_json::to_vec(&body)
                    .map_err(|_| ApiError::new(500, "encoding", "请求编码失败"))?,
            )
        }
        Operation::StartTask {
            text,
            submission_id,
            settings,
            outbound_proxy_id,
        } => {
            let body = json!({ "text": text, "submissionId": submission_id, "settings": settings, "outboundProxyId": outbound_proxy_id });
            (
                "POST",
                "api/tasks".to_owned(),
                serde_json::to_vec(&body)
                    .map_err(|_| ApiError::new(500, "encoding", "请求编码失败"))?,
            )
        }
        Operation::GetTask { task_id } => ("GET", format!("api/tasks/{task_id}"), Vec::new()),
        Operation::CancelTask { task_id } => {
            ("POST", format!("api/tasks/{task_id}/cancel"), Vec::new())
        }
        Operation::RetryTask { task_id } => {
            ("POST", format!("api/tasks/{task_id}/retry"), Vec::new())
        }
        Operation::DeleteTask { task_id } => ("DELETE", format!("api/tasks/{task_id}"), Vec::new()),
        Operation::GetScreen { task_id, item_id } => (
            "GET",
            format!("api/tasks/{task_id}/items/{item_id}/screen"),
            Vec::new(),
        ),
        Operation::SendInput {
            task_id,
            item_id,
            data,
        } => (
            "POST",
            format!("api/tasks/{task_id}/items/{item_id}/input"),
            serde_json::to_vec(&data)
                .map_err(|_| ApiError::new(500, "encoding", "请求编码失败"))?,
        ),
    };
    validate_operation_request(&request.1, &request.2)?;
    Ok(request)
}

fn validate_operation_request(path: &str, body: &[u8]) -> Result<(), ApiError> {
    for segment in path.split('/') {
        if matches!(
            segment,
            "api"
                | "accounts"
                | "tasks"
                | "items"
                | "credentials"
                | "reauthorize"
                | "cancel"
                | "retry"
                | "screen"
                | "input"
        ) {
            continue;
        }
        if !bounded_id(segment) {
            return Err(ApiError::invalid("账号或任务标识无效"));
        }
    }
    if body.len() > MAXIMUM_BODY_BYTES {
        return Err(ApiError::invalid("请求正文超过 140 KB 限制"));
    }
    Ok(())
}

fn forward_response(response: WorkerResponse) -> ApiResult {
    if !response
        .content_type
        .to_ascii_lowercase()
        .starts_with("application/json")
    {
        return Err(ApiError::new(
            502,
            "worker_response",
            "Worker 返回了不支持的内容类型",
        ));
    }
    raw_json(response.status, response.body)
        .map_err(|_| ApiError::new(500, "encoding", "管理接口响应编码失败"))
}

fn now() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{Operation, operation_request};
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

    #[test]
    fn operation_request_rejects_invalid_identifier() {
        let operation = Operation::GetTask {
            task_id: "invalid id".to_owned(),
        };
        assert!(operation_request(operation).is_err());
    }

    #[test]
    fn decodes_start_task_payload_with_camel_case_fields() {
        let payload = json!({
            "operation": "startTask",
            "text": "test@example.invalid----invalid-password----JBSWY3DPEHPK3PXP",
            "submissionId": "58bc8ea3-1c36-4a70-aff4-f7ba6402403e",
            "settings": {
                "enabled": true,
                "concurrencyLimit": null,
                "weight": 1,
                "groupIds": []
            }
        });

        let operation: Operation = match serde_json::from_value(payload) {
            Ok(operation) => operation,
            Err(error) => panic!("startTask payload: {error}"),
        };
        let result = operation_request(operation);
        assert!(result.is_ok(), "worker request should be accepted");
        let (method, path, body) = result.unwrap_or_else(|_| unreachable!());

        assert_eq!(method, "POST");
        assert_eq!(path, "api/tasks");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body)
                .unwrap_or_else(|error| { panic!("worker body: {error}") }),
            json!({
                "text": "test@example.invalid----invalid-password----JBSWY3DPEHPK3PXP",
                "submissionId": "58bc8ea3-1c36-4a70-aff4-f7ba6402403e",
                "settings": {
                    "enabled": true,
                    "concurrencyLimit": null,
                    "weight": 1,
                    "groupIds": []
                },
                "outboundProxyId": null
            })
        );
    }
}
