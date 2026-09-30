use super::response::ApiError;
use gateway_plugin_sdk::{call::management::ManagementRequest, client::TypedCall};
use serde::de::DeserializeOwned;

pub(super) const MAXIMUM_BODY_BYTES: usize = 140 * 1024;

pub(super) fn require_no_query(request: &ManagementRequest) -> Result<(), ApiError> {
    if request.query.is_empty() {
        Ok(())
    } else {
        Err(ApiError::invalid("此接口不支持查询参数"))
    }
}

pub(super) fn decode_json<T: DeserializeOwned>(
    call: &TypedCall<ManagementRequest>,
) -> Result<T, ApiError> {
    let content_type = call.request.content_type.as_deref().unwrap_or_default();
    if !content_type
        .split(';')
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
    {
        return Err(ApiError::invalid("此接口需要 application/json 内容类型"));
    }
    serde_json::from_slice(&call.payload).map_err(|_| ApiError::invalid("请求正文不是有效 JSON"))
}

pub(super) fn bounded_id(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}
