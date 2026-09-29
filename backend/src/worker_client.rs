use std::{fmt, time::Duration};

use gateway_plugin_sdk::call::management::ManagementRequest;
use reqwest::{Client, Method, StatusCode, Url, header::HeaderValue};
use serde_json::Value;

const MAXIMUM_REQUEST_BYTES: usize = 140 * 1024;
const MAXIMUM_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone)]
pub struct WorkerClient {
    base_url: Url,
    client: Client,
}

pub struct WorkerResponse {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

#[derive(Debug)]
pub enum WorkerError {
    InvalidBaseUrl,
    InvalidPath,
    RequestTooLarge,
    ResponseTooLarge,
    Transport,
    InvalidResponse,
}

impl fmt::Display for WorkerError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidBaseUrl => "Worker 地址必须是本机 HTTP 地址",
            Self::InvalidPath => "插件请求路径不受支持",
            Self::RequestTooLarge => "请求正文超过 Worker 限制",
            Self::ResponseTooLarge => "Worker 响应超过插件限制",
            Self::Transport => "2FA Worker 暂时不可用",
            Self::InvalidResponse => "2FA Worker 返回了无效响应",
        };
        output.write_str(message)
    }
}

impl std::error::Error for WorkerError {}

impl WorkerClient {
    pub fn new(base_url: &str) -> Result<Self, WorkerError> {
        let base_url = Url::parse(base_url).map_err(|_| WorkerError::InvalidBaseUrl)?;
        if base_url.scheme() != "http"
            || !matches!(base_url.host_str(), Some("127.0.0.1" | "localhost"))
            || base_url.query().is_some()
            || base_url.fragment().is_some()
        {
            return Err(WorkerError::InvalidBaseUrl);
        }
        let client = Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(45))
            .build()
            .map_err(|_| WorkerError::Transport)?;
        Ok(Self { base_url, client })
    }

    pub async fn forward(
        &self,
        request: &ManagementRequest,
        payload: &[u8],
    ) -> Result<WorkerResponse, WorkerError> {
        if payload.len() > MAXIMUM_REQUEST_BYTES {
            return Err(WorkerError::RequestTooLarge);
        }
        let path = worker_path(&request.method, &request.path).ok_or(WorkerError::InvalidPath)?;
        let mut url = self.base_url.clone();
        url.set_path(&path);
        if !request.query.is_empty() {
            url.set_query(Some(&request.query));
        }
        let method =
            Method::from_bytes(request.method.as_bytes()).map_err(|_| WorkerError::InvalidPath)?;
        let mut builder = self.client.request(method, url).header("X-CPR-TwoFA", "1");
        for header in &request.headers {
            if (header.name.eq_ignore_ascii_case("cookie")
                || header.name.eq_ignore_ascii_case("origin"))
                && let Ok(value) = HeaderValue::from_bytes(&header.value)
            {
                builder = builder.header(header.name.as_str(), value);
            }
        }
        if let Some(content_type) = request.content_type.as_deref() {
            builder = builder.header(reqwest::header::CONTENT_TYPE, content_type);
        }
        if !payload.is_empty() {
            builder = builder.body(payload.to_vec());
        }
        let response = builder.send().await.map_err(|_| WorkerError::Transport)?;
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("application/json")
            .to_owned();
        let bytes = response.bytes().await.map_err(|_| WorkerError::Transport)?;
        if bytes.len() > MAXIMUM_RESPONSE_BYTES {
            return Err(WorkerError::ResponseTooLarge);
        }
        Ok(WorkerResponse {
            status: status.as_u16(),
            content_type,
            body: unwrap_worker_body(status, &bytes),
        })
    }

    pub async fn health(&self) -> Result<Value, WorkerError> {
        let request = ManagementRequest {
            method: "GET".to_owned(),
            path: "api/status".to_owned(),
            query: String::new(),
            content_type: None,
            headers: vec![],
        };
        let response = self.forward(&request, &[]).await?;
        if response.status != StatusCode::OK.as_u16() {
            return Err(WorkerError::Transport);
        }
        serde_json::from_slice(&response.body).map_err(|_| WorkerError::InvalidResponse)
    }
}

fn worker_path(method: &str, path: &str) -> Option<String> {
    if method == "GET" && path == "api/status" {
        return Some("/health".to_owned());
    }
    if path == "api/migration" || path == "api/migration/import" {
        return None;
    }
    let allowed = path.strip_prefix("api/")?;
    if let Some(account) = allowed.strip_suffix("/credentials")
        && account.starts_with("accounts/")
    {
        return Some(format!("/api/admin/twofa/{account}"));
    }
    let mapped = if allowed == "tasks"
        || allowed.starts_with("tasks/")
        || allowed == "accounts"
        || allowed.starts_with("accounts/")
    {
        format!("/api/admin/twofa/{allowed}")
    } else {
        return None;
    };
    Some(mapped)
}

fn unwrap_worker_body(status: StatusCode, bytes: &[u8]) -> Vec<u8> {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return bytes.to_vec();
    };
    if status.is_success()
        && value.get("code").and_then(Value::as_u64) == Some(200)
        && value.get("data").is_some()
    {
        return serde_json::to_vec(value.get("data").unwrap_or(&Value::Null)).unwrap_or_default();
    }
    if !status.is_success() {
        let message = value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("2FA Worker 请求失败");
        return serde_json::to_vec(&serde_json::json!({
            "error": { "code": format!("worker_{}", status.as_u16()), "message": message }
        }))
        .unwrap_or_default();
    }
    bytes.to_vec()
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use super::{unwrap_worker_body, worker_path};
    use gateway_plugin_sdk::call::management::ManagementRequest;
    use reqwest::StatusCode;

    #[test]
    fn maps_only_supported_worker_paths() {
        assert_eq!(worker_path("GET", "api/status"), Some("/health".to_owned()));
        assert_eq!(
            worker_path("GET", "api/accounts/a/credentials"),
            Some("/api/admin/twofa/accounts/a".to_owned())
        );
        assert_eq!(
            worker_path("POST", "api/tasks/id/cancel"),
            Some("/api/admin/twofa/tasks/id/cancel".to_owned())
        );
        assert_eq!(worker_path("GET", "api/unknown"), None);
    }

    #[test]
    fn unwraps_worker_envelopes_without_exposing_unknown_fields() {
        let body = br#"{"code":200,"message":"ok","data":{"ready":true}}"#;
        assert_eq!(
            unwrap_worker_body(StatusCode::OK, body),
            br#"{"ready":true}"#
        );
        let error = r#"{"code":403,"message":"来源验证失败","data":null}"#.as_bytes();
        assert!(
            String::from_utf8(unwrap_worker_body(StatusCode::FORBIDDEN, error))
                .unwrap()
                .contains("来源验证失败")
        );
    }

    #[tokio::test]
    async fn forwards_json_content_type_to_worker() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listener");
        let port = listener.local_addr().expect("listener address").port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let size = stream.read(&mut chunk).expect("read request");
                if size == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..size]);
            }
            let request = String::from_utf8(request).expect("request headers");
            assert!(request.contains("content-type: application/json"));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                .expect("response");
        });

        let client =
            super::WorkerClient::new(&format!("http://127.0.0.1:{port}")).expect("worker client");
        let request = ManagementRequest {
            method: "POST".to_owned(),
            path: "api/tasks".to_owned(),
            query: String::new(),
            content_type: Some("application/json".to_owned()),
            headers: Vec::new(),
        };
        let response = client.forward(&request, br#"{}"#).await.expect("response");

        assert_eq!(response.status, StatusCode::OK.as_u16());
        server.join().expect("server");
    }
}
