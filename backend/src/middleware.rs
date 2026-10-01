use gateway_plugin_sdk::{
    ErrorCode, PluginFault,
    call::{
        host::{AuthCredential, AuthGetRequest, AuthListRequest, AuthListResult},
        middleware::{
            MiddlewareBodyFrame, MiddlewareBodyFraming, MiddlewareHeader, MiddlewareMount,
        },
    },
    client::{MiddlewareBody, MiddlewareResponse, RequestCall},
};
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    basispoints::{self, BasispointsError},
    management::PluginState,
    sse::SseFrameSplitter,
    usage::UsageStats,
};

/// Intercepts only the configured OpenAI Responses models at the selected
/// attempt. Unsupported models continue through the native RS provider.
pub(crate) async fn handle(
    state: PluginState,
    call: RequestCall,
) -> Result<MiddlewareResponse, PluginFault> {
    let model = call
        .request
        .head
        .model
        .clone()
        .or_else(|| model_from_body(&call.request.body));
    if !should_intercept(
        &state,
        call.request.head.mount,
        &call.request.head.operation,
        &call.request.head.protocol,
        model.as_deref(),
    ) {
        return call.next.run(call.request).await;
    }

    let request: Value = serde_json::from_slice(&call.request.body)
        .map_err(|_| fault(ErrorCode::InvalidInput, "Responses 请求正文无效"))?;
    let credential =
        resolve_credential(&call.host, call.request.head.account_id.as_deref()).await?;
    state.usage.begin(now_ms());
    let response = match basispoints::request(&call.host, &credential, request).await {
        Ok(response) => response,
        Err(error) => {
            state.usage.record_failure();
            return Err(map_basispoints_error(error));
        }
    };
    record_response(&state.usage, response.status);
    // BPS can reject an otherwise valid OAuth account with a policy response
    // (currently HTTP 403). Delegate to RS's native provider so enabling the
    // plugin does not turn the whole model route into a middleware 502.
    if response.status == 403 {
        return call.next.run(call.request).await;
    }
    let (is_sse, framing) = response_framing(response.status, &response.content_type);
    let frames = if is_sse {
        sse_frames(&response.body)
    } else {
        vec![MiddlewareBodyFrame::new(response.body, true)]
    };
    let headers = vec![
        MiddlewareHeader {
            name: "content-type".to_owned(),
            value: response.content_type.into_bytes(),
        },
        MiddlewareHeader {
            name: "cache-control".to_owned(),
            value: b"no-store".to_vec(),
        },
        MiddlewareHeader {
            name: "x-content-type-options".to_owned(),
            value: b"nosniff".to_vec(),
        },
    ];
    Ok(MiddlewareResponse::direct(
        call.request.head.protocol,
        response.status,
        headers,
        MiddlewareBody::from_frames(framing, frames),
    ))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn record_response(usage: &UsageStats, status: u16) {
    if (200..300).contains(&status) {
        usage.record_success();
    } else {
        usage.record_failure();
    }
}

fn should_intercept(
    state: &PluginState,
    mount: MiddlewareMount,
    operation: &str,
    protocol: &str,
    model: Option<&str>,
) -> bool {
    mount == MiddlewareMount::Request
        && operation == "generate"
        && protocol == "openai"
        && state.config.accepts(model)
}

fn model_from_body(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("model")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
}

fn sse_frames(body: &[u8]) -> Vec<MiddlewareBodyFrame> {
    let mut splitter = SseFrameSplitter::new();
    let mut frames = splitter.feed(body);
    if let Some(frame) = splitter.finish() {
        frames.push(frame);
    }
    if frames.is_empty() {
        return vec![MiddlewareBodyFrame::new(Vec::new(), true)];
    }
    let last = frames.len() - 1;
    frames
        .into_iter()
        .enumerate()
        .map(|(index, frame)| MiddlewareBodyFrame::new(frame.bytes, index == last || frame.done))
        .collect()
}

/// Match RS's response framing contract, including error responses.
///
/// The host runtime expects `RawBytes` for every non-2xx short-circuit
/// response. Returning `JsonDocument` for a JSON 403/5xx is rejected as an
/// invalid middleware response and becomes the generic 502 seen by clients.
fn response_framing(status: u16, content_type: &str) -> (bool, MiddlewareBodyFraming) {
    let is_success = (200..300).contains(&status);
    let is_sse = is_success
        && content_type
            .split(';')
            .next()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"));
    let framing = if !is_success {
        MiddlewareBodyFraming::RawBytes
    } else if is_sse {
        MiddlewareBodyFraming::SseEvent
    } else {
        MiddlewareBodyFraming::JsonDocument
    };
    (is_sse, framing)
}

async fn resolve_credential(
    host: &gateway_plugin_sdk::client::HostClient,
    account_id: Option<&str>,
) -> Result<AuthCredential, PluginFault> {
    if let Some(account_id) = account_id {
        return get_credential(host, account_id).await;
    }
    let payload = serde_json::to_vec(&AuthListRequest {
        provider_id: Some("openai".to_owned()),
        cursor: None,
        limit: 100,
    })
    .map_err(|_| fault(ErrorCode::Fault, "账号列表请求编码失败"))?;
    let reply = host
        .call("host.auth.list", serde_json::json!({}), payload)
        .await
        .map_err(|_| fault(ErrorCode::Upstream, "宿主账号列表暂不可用"))?;
    if reply.result != serde_json::json!({}) {
        return Err(fault(ErrorCode::Upstream, "宿主账号列表响应无效"));
    }
    let accounts: AuthListResult = serde_json::from_slice(&reply.payload)
        .map_err(|_| fault(ErrorCode::Upstream, "宿主账号列表响应无效"))?;
    for account in accounts.accounts {
        if account.provider_id != "openai"
            || !account.enabled
            || !account.authentication_kind.eq_ignore_ascii_case("oauth")
        {
            continue;
        }
        if let Ok(credential) = get_credential(host, &account.account_id).await {
            return Ok(credential);
        }
    }
    Err(fault(
        ErrorCode::Upstream,
        "宿主没有可用的 OpenAI OAuth 账号",
    ))
}

async fn get_credential(
    host: &gateway_plugin_sdk::client::HostClient,
    account_id: &str,
) -> Result<AuthCredential, PluginFault> {
    let payload = serde_json::to_vec(&AuthGetRequest {
        account_id: account_id.to_owned(),
    })
    .map_err(|_| fault(ErrorCode::Fault, "账号请求编码失败"))?;
    let reply = host
        .call("host.auth.get", json!({}), payload)
        .await
        .map_err(|_| fault(ErrorCode::Upstream, "宿主账号凭据暂不可用"))?;
    if reply.result != json!({}) {
        return Err(fault(ErrorCode::Upstream, "宿主账号凭据响应无效"));
    }
    let credential: AuthCredential = serde_json::from_slice(&reply.payload)
        .map_err(|_| fault(ErrorCode::Upstream, "宿主账号凭据响应无效"))?;
    if credential.account_id != account_id
        || credential.provider_id != "openai"
        || !credential
            .facts
            .authentication_kind
            .eq_ignore_ascii_case("oauth")
    {
        return Err(fault(
            ErrorCode::InvalidInput,
            "宿主账号不是 OpenAI OAuth 账号",
        ));
    }
    if credential.facts.material.contains_key("apiKey")
        || credential.facts.material.contains_key("api_key")
    {
        return Err(fault(
            ErrorCode::InvalidInput,
            "宿主账号不是 OpenAI OAuth 账号",
        ));
    }
    let has_token = credential
        .facts
        .material
        .get("accessToken")
        .or_else(|| credential.facts.material.get("access_token"))
        .and_then(Value::as_str)
        .is_some_and(|token| !token.is_empty());
    if !has_token {
        return Err(fault(
            ErrorCode::InvalidInput,
            "宿主账号没有可用 access token",
        ));
    }
    Ok(credential)
}

fn map_basispoints_error(error: BasispointsError) -> PluginFault {
    match error {
        BasispointsError::InvalidRequest(message) => fault(ErrorCode::InvalidInput, message),
        BasispointsError::ResponseTooLarge => {
            fault(ErrorCode::Capacity, "Basis Points 响应超过插件限制")
        }
        BasispointsError::UnsupportedResponse => {
            fault(ErrorCode::Upstream, "Basis Points 返回了不支持的内容类型")
        }
        BasispointsError::Upstream => fault(ErrorCode::Upstream, "Basis Points HTTP 请求失败"),
        BasispointsError::Relay(_) => fault(ErrorCode::Upstream, "Basis Points 工具中继失败"),
    }
}

fn fault(code: ErrorCode, message: &'static str) -> PluginFault {
    PluginFault::new(code, message)
}

#[cfg(test)]
mod tests {
    use super::{
        MiddlewareBodyFraming, MiddlewareMount, response_framing, should_intercept, sse_frames,
    };
    use crate::{PluginState, usage::UsageStats};

    #[test]
    fn request_stage_matches_generate_openai_model() {
        let state = PluginState::new(serde_json::json!({
            "enabled": true,
            "models": ["gpt-6-astra"]
        }));
        assert!(should_intercept(
            &state,
            MiddlewareMount::Request,
            "generate",
            "openai",
            Some("gpt-6-astra")
        ));
        assert!(!should_intercept(
            &state,
            MiddlewareMount::Attempt,
            "generate",
            "openai",
            Some("gpt-6-astra")
        ));
        assert!(!should_intercept(
            &state,
            MiddlewareMount::Request,
            "generate",
            "openai",
            Some("gpt-6-sol")
        ));
    }

    #[test]
    fn sse_frames_preserve_complete_event_boundaries() {
        let frames = sse_frames(b"event: response.output_text.delta\ndata: {}\n\n");
        assert_eq!(frames.len(), 1);
        assert!(frames[0].terminal);
    }

    #[test]
    fn non_success_responses_use_raw_bytes_framing() {
        assert_eq!(
            response_framing(403, "application/json; charset=utf-8"),
            (false, MiddlewareBodyFraming::RawBytes)
        );
        assert_eq!(
            response_framing(200, "text/event-stream"),
            (true, MiddlewareBodyFraming::SseEvent)
        );
        assert_eq!(
            response_framing(200, "application/json"),
            (false, MiddlewareBodyFraming::JsonDocument)
        );
    }

    #[test]
    fn response_statuses_classify_usage_attempts() {
        let usage = UsageStats::new();

        for status in [200, 201, 299] {
            usage.begin(1);
            super::record_response(&usage, status);
        }
        for status in [400, 403, 500] {
            usage.begin(1);
            super::record_response(&usage, status);
        }

        let snapshot = usage.snapshot();
        assert_eq!(snapshot.total_requests, 6);
        assert_eq!(snapshot.successful_requests, 3);
        assert_eq!(snapshot.failed_requests, 3);
    }

    #[test]
    fn unselected_models_do_not_start_usage_attempts() {
        let state = PluginState::new(serde_json::json!({
            "enabled": true,
            "models": ["gpt-6-astra"]
        }));

        assert!(!should_intercept(
            &state,
            MiddlewareMount::Request,
            "generate",
            "openai",
            Some("gpt-6-sol")
        ));
        assert_eq!(state.usage.snapshot().total_requests, 0);
    }
}
