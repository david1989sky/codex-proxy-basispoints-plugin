use gateway_plugin_sdk::{
    ErrorCode, PluginFault, call::management::ManagementResponse, client::TypedReply,
};
use serde::Serialize;
use serde_json::json;

pub(super) const JSON_CONTENT_TYPE: &str = "application/json";
pub(super) type ApiResult = Result<TypedReply<ManagementResponse>, ApiError>;

#[derive(Serialize)]
pub(super) struct ApiError {
    #[serde(skip)]
    status: u16,
    code: &'static str,
    message: String,
}

impl ApiError {
    pub fn new(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(400, "invalid_request", message)
    }

    pub fn from_worker(status: u16, message: impl Into<String>) -> Self {
        let code = match status {
            401 => "unauthorized",
            403 => "forbidden",
            404 => "not_found",
            409 => "conflict",
            413 => "payload_too_large",
            429 => "busy",
            503 => "worker_unavailable",
            _ => "worker_error",
        };
        Self::new(status, code, message)
    }

    pub fn into_reply(self) -> Result<TypedReply<ManagementResponse>, PluginFault> {
        encode(self.status, &json!({ "error": self }))
    }
}

impl From<PluginFault> for ApiError {
    fn from(error: PluginFault) -> Self {
        if error.code == ErrorCode::Conflict {
            Self::new(409, "conflict", "迁移状态已更新，请重新加载后再试")
        } else {
            Self::new(502, "host_callback", "宿主状态操作未完成")
        }
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(_: serde_json::Error) -> Self {
        Self::new(500, "encoding", "管理接口响应编码失败")
    }
}

pub(super) fn json_reply(body: &impl Serialize) -> ApiResult {
    encode(200, body).map_err(|_| ApiError::new(500, "encoding", "管理接口响应编码失败"))
}

pub(super) fn raw_json(
    status: u16,
    body: Vec<u8>,
) -> Result<TypedReply<ManagementResponse>, PluginFault> {
    raw_response(status, JSON_CONTENT_TYPE, body)
        .map_err(|_| PluginFault::new(ErrorCode::Fault, "管理接口响应编码失败"))
}

pub(super) fn raw_response(
    status: u16,
    content_type: &str,
    body: Vec<u8>,
) -> Result<TypedReply<ManagementResponse>, ApiError> {
    let media_type = content_type
        .split(';')
        .next()
        .map(str::trim)
        .unwrap_or_default();
    if !media_type.eq_ignore_ascii_case("application/json")
        && !media_type.eq_ignore_ascii_case("text/event-stream")
    {
        return Err(ApiError::new(
            502,
            "worker_response",
            "Worker 返回了不支持的内容类型",
        ));
    }
    Ok(TypedReply::new(ManagementResponse {
        status,
        content_type: content_type.to_owned(),
        headers: vec![
            gateway_plugin_sdk::call::middleware::MiddlewareHeader {
                name: "Cache-Control".to_owned(),
                value: b"no-store".to_vec(),
            },
            gateway_plugin_sdk::call::middleware::MiddlewareHeader {
                name: "X-Content-Type-Options".to_owned(),
                value: b"nosniff".to_vec(),
            },
        ],
    })
    .with_payload(body))
}

pub(super) fn encode(
    status: u16,
    body: &impl Serialize,
) -> Result<TypedReply<ManagementResponse>, PluginFault> {
    let payload = serde_json::to_vec(body)
        .map_err(|_| PluginFault::new(ErrorCode::Fault, "管理接口响应编码失败"))?;
    Ok(TypedReply::new(ManagementResponse {
        status,
        content_type: JSON_CONTENT_TYPE.to_owned(),
        headers: vec![
            gateway_plugin_sdk::call::middleware::MiddlewareHeader {
                name: "Cache-Control".to_owned(),
                value: b"no-store".to_vec(),
            },
            gateway_plugin_sdk::call::middleware::MiddlewareHeader {
                name: "X-Content-Type-Options".to_owned(),
                value: b"nosniff".to_vec(),
            },
        ],
    })
    .with_payload(payload))
}

#[cfg(test)]
mod tests {
    use super::raw_response;

    #[test]
    fn preserves_allowed_sse_content_type_and_security_headers() {
        let reply = match raw_response(
            200,
            "Text/Event-Stream; charset=utf-8",
            b"data: hello\n\n".to_vec(),
        ) {
            Ok(reply) => reply,
            Err(_) => panic!("SSE response should be accepted"),
        };

        assert_eq!(
            reply.result.content_type,
            "Text/Event-Stream; charset=utf-8"
        );
        assert_eq!(reply.result.status, 200);
        assert_eq!(reply.payload, b"data: hello\n\n");
        assert_eq!(reply.result.headers.len(), 2);
    }

    #[test]
    fn rejects_unallowlisted_content_types() {
        let result = raw_response(200, "text/plain", b"secret".to_vec());
        let error = match result {
            Ok(_) => panic!("text/plain must be rejected"),
            Err(error) => error,
        };

        assert_eq!(error.status, 502);
        assert_eq!(error.code, "worker_response");
        assert_eq!(error.message, "Worker 返回了不支持的内容类型");
    }
}
