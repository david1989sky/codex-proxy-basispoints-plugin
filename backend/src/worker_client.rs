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
            Self::Transport => "Basis Points Worker 暂时不可用",
            Self::InvalidResponse => "Basis Points Worker 返回了无效响应",
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
        let mut builder = self
            .client
            .request(method, url)
            .header("X-CPR-Basispoints", "1");
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
        let mut response = builder.send().await.map_err(|_| WorkerError::Transport)?;
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("application/json")
            .to_owned();
        if response
            .content_length()
            .is_some_and(|length| length > MAXIMUM_RESPONSE_BYTES as u64)
        {
            return Err(WorkerError::ResponseTooLarge);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| WorkerError::Transport)? {
            if chunk.len() > MAXIMUM_RESPONSE_BYTES.saturating_sub(bytes.len()) {
                return Err(WorkerError::ResponseTooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(WorkerResponse {
            status: status.as_u16(),
            content_type,
            body: bytes,
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
        health_payload(&response.body)
    }
}

fn worker_path(method: &str, path: &str) -> Option<String> {
    if method == "GET" && path == "api/status" {
        return Some("/health".to_owned());
    }
    if method == "POST" && path == "api/basispoints/responses" {
        return Some("/api/basispoints/responses".to_owned());
    }
    None
}

fn health_payload(body: &[u8]) -> Result<Value, WorkerError> {
    let value = serde_json::from_slice::<Value>(body).map_err(|_| WorkerError::InvalidResponse)?;
    if value.get("code").and_then(Value::as_u64) == Some(200) {
        return value
            .get("data")
            .filter(|value| value.is_object())
            .cloned()
            .ok_or(WorkerError::InvalidResponse);
    }
    if value.get("ready").is_some_and(Value::is_boolean) {
        return Ok(value);
    }
    Err(WorkerError::InvalidResponse)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use super::worker_path;
    use gateway_plugin_sdk::call::management::ManagementRequest;
    use reqwest::StatusCode;

    #[test]
    fn maps_only_supported_worker_paths() {
        assert_eq!(worker_path("GET", "api/status"), Some("/health".to_owned()));
        assert_eq!(
            worker_path("POST", "api/basispoints/responses"),
            Some("/api/basispoints/responses".to_owned())
        );
        assert_eq!(worker_path("GET", "api/basispoints/responses"), None);
        assert_eq!(worker_path("GET", "api/unknown"), None);
    }

    #[test]
    fn unwraps_only_the_worker_health_envelope() {
        let body = br#"{"code":200,"message":"ok","data":{"ready":true}}"#;
        assert_eq!(
            super::health_payload(body).expect("health envelope"),
            serde_json::json!({"ready": true})
        );
    }

    #[tokio::test]
    async fn forwards_basispoints_headers_and_json_content_type_to_worker() {
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
            assert!(request.contains("x-cpr-basispoints: 1"));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                .expect("response");
        });

        let client =
            super::WorkerClient::new(&format!("http://127.0.0.1:{port}")).expect("worker client");
        let request = ManagementRequest {
            method: "POST".to_owned(),
            path: "api/basispoints/responses".to_owned(),
            query: String::new(),
            content_type: Some("application/json".to_owned()),
            headers: Vec::new(),
        };
        let response = client.forward(&request, br#"{}"#).await.expect("response");

        assert_eq!(response.status, StatusCode::OK.as_u16());
        server.join().expect("server");
    }

    #[tokio::test]
    async fn preserves_raw_basispoints_response_body() {
        let expected_body = br#"{"code":200,"data":{"ready":true}}"#;
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
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                expected_body.len(),
                String::from_utf8_lossy(expected_body)
            );
            stream.write_all(response.as_bytes()).expect("response");
        });

        let client =
            super::WorkerClient::new(&format!("http://127.0.0.1:{port}")).expect("worker client");
        let request = ManagementRequest {
            method: "POST".to_owned(),
            path: "api/basispoints/responses".to_owned(),
            query: String::new(),
            content_type: Some("application/json".to_owned()),
            headers: Vec::new(),
        };
        let response = client
            .forward(&request, br#"{"accountId":"acct-1","request":{}}"#)
            .await
            .expect("response");

        assert_eq!(response.status, StatusCode::OK.as_u16());
        assert_eq!(response.body, expected_body);
        server.join().expect("server");
    }

    #[tokio::test]
    async fn rejects_oversized_worker_response_before_buffering() {
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
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2097153\r\nConnection: close\r\n\r\n")
                .expect("response");
        });

        let client =
            super::WorkerClient::new(&format!("http://127.0.0.1:{port}")).expect("worker client");
        let request = ManagementRequest {
            method: "POST".to_owned(),
            path: "api/basispoints/responses".to_owned(),
            query: String::new(),
            content_type: Some("application/json".to_owned()),
            headers: Vec::new(),
        };
        let result = client
            .forward(&request, br#"{"accountId":"acct-1","request":{}}"#)
            .await;

        assert!(matches!(result, Err(super::WorkerError::ResponseTooLarge)));
        server.join().expect("server");
    }
}
