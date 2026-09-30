use std::fmt;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use gateway_plugin_sdk::{
    ErrorCode, PluginFault,
    call::host::{AuthCredential, HttpRequest},
    client::HostClient,
};
use serde_json::{Map, Value};
use uuid::Uuid;

pub(crate) const BASISPOINTS_URL: &str = "https://bps.openai.com/basispoints/api/responses";
pub(crate) const MAXIMUM_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug)]
pub(crate) enum BasispointsError {
    InvalidRequest(&'static str),
    UnsupportedResponse,
    Upstream,
    ResponseTooLarge,
}

impl fmt::Display for BasispointsError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidRequest(message) => *message,
            Self::UnsupportedResponse => "Basis Points 返回了不支持的内容类型",
            Self::Upstream => "Basis Points 请求失败",
            Self::ResponseTooLarge => "Basis Points 响应超过插件限制",
        };
        output.write_str(message)
    }
}

impl std::error::Error for BasispointsError {}

pub(crate) struct BasispointsResponse {
    pub(crate) status: u16,
    pub(crate) content_type: String,
    pub(crate) body: Vec<u8>,
}

pub(crate) async fn request(
    host: &HostClient,
    credential: &AuthCredential,
    request: Value,
) -> Result<BasispointsResponse, BasispointsError> {
    let token = credential
        .facts
        .material
        .get("accessToken")
        .or_else(|| credential.facts.material.get("access_token"))
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .ok_or(BasispointsError::InvalidRequest(
            "宿主账号没有可用 access token",
        ))?;
    let account_id = resolve_account_id(credential, token)?;
    let normalized = normalize_request(request)?;
    let body = serde_json::to_vec(&normalized)
        .map_err(|_| BasispointsError::InvalidRequest("请求编码失败"))?;
    let stream = normalized
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let response = host
        .http(
            HttpRequest {
                method: "POST".to_owned(),
                url: BASISPOINTS_URL.to_owned(),
                headers: vec![
                    ("authorization".to_owned(), format!("Bearer {token}")),
                    ("chatgpt-account-id".to_owned(), account_id.clone()),
                    ("x-openai-account-id".to_owned(), account_id),
                    ("x-basispoints-auth-mode".to_owned(), "chatgpt".to_owned()),
                    ("content-type".to_owned(), "application/json".to_owned()),
                    (
                        "accept".to_owned(),
                        if stream {
                            "text/event-stream".to_owned()
                        } else {
                            "application/json".to_owned()
                        },
                    ),
                    ("origin".to_owned(), "https://bps.openai.com".to_owned()),
                    (
                        "x-openai-internal-basispoints-client-agent-profile".to_owned(),
                        "excel".to_owned(),
                    ),
                    (
                        "x-openai-internal-basispoints-client-editor".to_owned(),
                        "excel".to_owned(),
                    ),
                    (
                        "x-openai-internal-basispoints-client-host".to_owned(),
                        "office".to_owned(),
                    ),
                    (
                        "x-openai-internal-basispoints-client-platform".to_owned(),
                        "excel".to_owned(),
                    ),
                    (
                        "x-openai-internal-basispoints-client-platform-class".to_owned(),
                        "PC".to_owned(),
                    ),
                    (
                        "x-openai-internal-basispoints-client-product".to_owned(),
                        "basispoints-excel-plugin".to_owned(),
                    ),
                    (
                        "x-openai-internal-basispoints-client-runtime".to_owned(),
                        "desktop".to_owned(),
                    ),
                    (
                        "x-openai-internal-basispoints-office-host".to_owned(),
                        "Excel".to_owned(),
                    ),
                    (
                        "x-openai-internal-basispoints-office-platform".to_owned(),
                        "PC".to_owned(),
                    ),
                    ("x-stainless-arch".to_owned(), "unknown".to_owned()),
                    ("x-stainless-lang".to_owned(), "rust".to_owned()),
                    ("x-stainless-os".to_owned(), "Unknown".to_owned()),
                    (
                        "x-stainless-package-version".to_owned(),
                        "6.31.0".to_owned(),
                    ),
                    ("x-stainless-retry-count".to_owned(), "0".to_owned()),
                    (
                        "x-stainless-runtime".to_owned(),
                        "browser:chrome".to_owned(),
                    ),
                    (
                        "user-agent".to_owned(),
                        "codex-proxy-basispoints-plugin/0.1.4".to_owned(),
                    ),
                ],
            },
            body,
        )
        .await
        .map_err(map_host_error)?;
    let content_type = response
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.as_str())
        .unwrap_or_default();
    let media_type = content_type
        .split(';')
        .next()
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if media_type != "application/json" && media_type != "text/event-stream" {
        let mut body = response.body;
        let _ = body.close().await;
        return Err(BasispointsError::UnsupportedResponse);
    }
    let body = response
        .body
        .collect(MAXIMUM_RESPONSE_BYTES)
        .await
        .map_err(map_body_error)?;
    Ok(BasispointsResponse {
        status: response.status,
        content_type: if content_type.is_empty() {
            media_type
        } else {
            content_type.to_owned()
        },
        body,
    })
}

fn normalize_request(mut request: Value) -> Result<Value, BasispointsError> {
    let object = request
        .as_object_mut()
        .ok_or(BasispointsError::InvalidRequest(
            "Responses 请求必须是 JSON 对象",
        ))?;
    let stream = object
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    object.insert(
        "model_selection".to_owned(),
        Value::String("explicit".to_owned()),
    );
    object.insert("stream".to_owned(), Value::Bool(stream));
    object.insert("store".to_owned(), Value::Bool(false));
    let effort = object
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .or_else(|| {
            object
                .get("reasoning")
                .and_then(|value| value.get("effort"))
                .and_then(Value::as_str)
        })
        .unwrap_or("medium");
    object.insert(
        "reasoning_effort".to_owned(),
        Value::String(normalize_effort(effort)),
    );
    let metadata = object
        .entry("metadata")
        .or_insert_with(|| Value::Object(Map::new()));
    if !metadata.is_object() {
        *metadata = Value::Object(Map::new());
    }
    let metadata = metadata
        .as_object_mut()
        .ok_or(BasispointsError::InvalidRequest("请求元数据格式无效"))?;
    metadata
        .entry("task_id")
        .or_insert_with(|| Value::String(Uuid::new_v4().to_string()));
    metadata
        .entry("turn_id")
        .or_insert_with(|| Value::String(Uuid::new_v4().to_string()));
    metadata
        .entry("agent_iteration")
        .or_insert_with(|| Value::String("0".to_owned()));
    if let Some(value) = metadata.get_mut("agent_iteration") {
        *value = Value::String(
            value
                .as_str()
                .map_or_else(|| value.to_string(), ToOwned::to_owned),
        );
    }
    Ok(request)
}

fn normalize_effort(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "x-high" | "extra-high" | "extra_high" | "max" => "xhigh".to_owned(),
        "low" | "medium" | "high" | "xhigh" | "ultra" => value.trim().to_ascii_lowercase(),
        _ => "medium".to_owned(),
    }
}

fn resolve_account_id(
    credential: &AuthCredential,
    token: &str,
) -> Result<String, BasispointsError> {
    let claim_account_id = token
        .split('.')
        .nth(1)
        .and_then(|encoded| URL_SAFE_NO_PAD.decode(encoded).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    let claim_account_id = claim_account_id
        .as_ref()
        .and_then(|claims| claims.get("https://api.openai.com/auth"))
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    if let Some(account_id) = credential.facts.upstream_account_id.as_deref() {
        if !valid_account_id(account_id) {
            return Err(BasispointsError::InvalidRequest("宿主账号 ID 无效"));
        }
        if let Some(claim_account_id) = claim_account_id.as_deref()
            && claim_account_id != account_id
        {
            return Err(BasispointsError::InvalidRequest(
                "宿主账号与 access token 不匹配",
            ));
        }
        return Ok(account_id.to_owned());
    }
    claim_account_id
        .filter(|value| valid_account_id(value))
        .ok_or(BasispointsError::InvalidRequest(
            "access token 缺少 ChatGPT 账号 ID",
        ))
}

fn valid_account_id(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

fn map_host_error(error: PluginFault) -> BasispointsError {
    match error.code {
        ErrorCode::Timeout | ErrorCode::Upstream | ErrorCode::Capacity => {
            BasispointsError::Upstream
        }
        _ => BasispointsError::Upstream,
    }
}

fn map_body_error(error: PluginFault) -> BasispointsError {
    if error.code == ErrorCode::Capacity {
        BasispointsError::ResponseTooLarge
    } else {
        BasispointsError::Upstream
    }
}
