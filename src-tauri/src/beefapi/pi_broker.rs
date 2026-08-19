use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use futures::StreamExt;
use reqwest::{header::ACCEPT_ENCODING, Client, StatusCode};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use uuid::Uuid;

use crate::settings::ModelProvider;

use super::provider::EphemeralModelProvider;

const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;
const LOCAL_AUTHORIZATION: &str = "Bearer beefex-parent-broker";

struct BrokerConfig {
    upstream_url: String,
    credential: String,
    group: String,
}

impl BrokerConfig {
    fn from_provider(provider: EphemeralModelProvider) -> Result<(Self, String), String> {
        let routing_group = encode_beefex_group_header(provider.routing_group());
        if routing_group.is_empty() {
            return Err("managed_provider_group_missing".to_string());
        }
        let ModelProvider {
            base_url,
            mut api_keys,
            enabled_models,
            ..
        } = provider.into_inner();
        let credential = api_keys
            .drain(..)
            .next()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "managed_provider_credential_missing".to_string())?;
        let model = enabled_models
            .into_iter()
            .next()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "managed_provider_model_missing".to_string())?;
        let upstream_url = format!("{}/responses", base_url.trim_end_matches('/'));
        Ok((
            Self {
                upstream_url,
                credential,
                group: routing_group,
            },
            model,
        ))
    }
}

fn encode_beefex_group_header(group: &str) -> String {
    let mut encoded = String::new();
    for byte in group.trim().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                encoded.push(byte as char)
            }
            b' ' => encoded.push_str("%20"),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

pub(crate) struct PiProviderBroker {
    endpoint: String,
    model: String,
    authorization_rejected: Arc<AtomicBool>,
    task: JoinHandle<()>,
}

impl std::fmt::Debug for PiProviderBroker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PiProviderBroker")
            .field("endpoint", &"http://127.0.0.1:<ephemeral>/<redacted>/v1")
            .field("model", &self.model)
            .field("credential", &"<redacted>")
            .finish()
    }
}

impl PiProviderBroker {
    pub(crate) async fn start(
        http: Client,
        provider: EphemeralModelProvider,
    ) -> Result<Self, String> {
        let (config, model) = BrokerConfig::from_provider(provider)?;
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| "pi_provider_broker_bind_failed".to_string())?;
        let address = listener
            .local_addr()
            .map_err(|_| "pi_provider_broker_bind_failed".to_string())?;
        let capability = Uuid::new_v4().simple().to_string();
        let request_path = format!("/{capability}/v1/responses");
        let endpoint = format!("http://{address}/{capability}/v1");
        let config = Arc::new(config);
        let authorization_rejected = Arc::new(AtomicBool::new(false));
        let rejected = authorization_rejected.clone();
        let task = tokio::spawn(async move {
            while let Ok((stream, peer)) = listener.accept().await {
                if !peer.ip().is_loopback() {
                    continue;
                }
                let http = http.clone();
                let config = config.clone();
                let request_path = request_path.clone();
                let rejected = rejected.clone();
                tokio::spawn(async move {
                    if let Err(reason) =
                        handle_connection(stream, &http, &config, &request_path, &rejected).await
                    {
                        eprintln!("[pi-broker] request failed: {reason}");
                    }
                });
            }
        });
        Ok(Self {
            endpoint,
            model,
            authorization_rejected,
            task,
        })
    }

    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub(crate) fn model(&self) -> &str {
        &self.model
    }

    pub(crate) fn authorization_rejected(&self) -> bool {
        self.authorization_rejected.load(Ordering::SeqCst)
    }
}

impl Drop for PiProviderBroker {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct IncomingRequest {
    path: String,
    authorization: Option<String>,
    content_type: Option<String>,
    body: Vec<u8>,
}

async fn read_request(stream: &mut TcpStream) -> Result<IncomingRequest, &'static str> {
    let mut bytes = Vec::new();
    let header_end = loop {
        if bytes.len() > MAX_HEADER_BYTES {
            return Err("headers_too_large");
        }
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).await.map_err(|_| "read_failed")?;
        if read == 0 {
            return Err("request_incomplete");
        }
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = std::str::from_utf8(&bytes[..header_end]).map_err(|_| "headers_invalid")?;
    let mut lines = headers.split("\r\n");
    let mut request_line = lines.next().unwrap_or_default().split_whitespace();
    if request_line.next() != Some("POST") {
        return Err("method_not_allowed");
    }
    let path = request_line.next().unwrap_or_default().to_string();
    let mut content_length = None;
    let mut authorization = None;
    let mut content_type = None;
    for line in lines.filter(|line| !line.is_empty()) {
        let Some((name, value)) = line.split_once(':') else {
            return Err("headers_invalid");
        };
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => {
                content_length = value.trim().parse::<usize>().ok();
            }
            "authorization" => authorization = Some(value.trim().to_string()),
            "content-type" => content_type = Some(value.trim().to_string()),
            "transfer-encoding" if !value.trim().eq_ignore_ascii_case("identity") => {
                return Err("chunked_request_unsupported")
            }
            _ => {}
        }
    }
    let content_length = content_length.ok_or("content_length_required")?;
    if content_length > MAX_BODY_BYTES {
        return Err("body_too_large");
    }
    while bytes.len() - header_end < content_length {
        let mut chunk = [0_u8; 8192];
        let read = stream.read(&mut chunk).await.map_err(|_| "read_failed")?;
        if read == 0 {
            return Err("request_incomplete");
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    Ok(IncomingRequest {
        path,
        authorization,
        content_type,
        body: bytes[header_end..header_end + content_length].to_vec(),
    })
}

async fn handle_connection(
    mut stream: TcpStream,
    http: &Client,
    config: &BrokerConfig,
    request_path: &str,
    authorization_rejected: &AtomicBool,
) -> Result<(), String> {
    let request = match read_request(&mut stream).await {
        Ok(request) => request,
        Err(reason) => {
            write_error(&mut stream, 400, reason).await?;
            return Ok(());
        }
    };
    if request.path != request_path || request.authorization.as_deref() != Some(LOCAL_AUTHORIZATION)
    {
        write_error(&mut stream, 404, "broker_capability_not_found").await?;
        return Ok(());
    }

    let mut upstream = http
        .post(&config.upstream_url)
        .bearer_auth(&config.credential)
        .header("x-beefex-group", &config.group)
        .header(ACCEPT_ENCODING, "identity");
    if let Some(content_type) = request.content_type {
        upstream = upstream.header("content-type", content_type);
    }
    let response = match upstream.body(request.body).send().await {
        Ok(response) => response,
        Err(_) => {
            write_error(&mut stream, 502, "pi_provider_broker_upstream_failed").await?;
            return Ok(());
        }
    };
    let status = response.status();
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        authorization_rejected.store(true, Ordering::SeqCst);
    }
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/json");
    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nTransfer-Encoding: chunked\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        status.as_u16(),
        status.canonical_reason().unwrap_or("Upstream"),
        content_type,
    );
    stream
        .write_all(header.as_bytes())
        .await
        .map_err(|_| "pi_provider_broker_client_write_failed".to_string())?;
    let mut chunks = response.bytes_stream();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|_| "pi_provider_broker_upstream_read_failed".to_string())?;
        if chunk.is_empty() {
            continue;
        }
        stream
            .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
            .await
            .map_err(|_| "pi_provider_broker_client_write_failed".to_string())?;
        stream
            .write_all(&chunk)
            .await
            .map_err(|_| "pi_provider_broker_client_write_failed".to_string())?;
        stream
            .write_all(b"\r\n")
            .await
            .map_err(|_| "pi_provider_broker_client_write_failed".to_string())?;
    }
    stream
        .write_all(b"0\r\n\r\n")
        .await
        .map_err(|_| "pi_provider_broker_client_write_failed".to_string())?;
    Ok(())
}

async fn write_error(stream: &mut TcpStream, status: u16, reason: &str) -> Result<(), String> {
    let body = serde_json::json!({ "error": { "message": reason } }).to_string();
    let response = format!(
        "HTTP/1.1 {status} Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len(),
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|_| "pi_provider_broker_client_write_failed".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    use tokio::time::{timeout, Duration};

    use crate::beefapi::{
        credential_store::SecretCredential,
        provider::hydrate_managed_provider,
        types::{SafeAccountMetadata, REQUIRED_GROUP},
    };

    const SECRET: &str = "fixture-secret-never-child-visible";

    fn provider(base_url: String) -> EphemeralModelProvider {
        hydrate_managed_provider(
            &SafeAccountMetadata {
                email: "ender@example.com".to_string(),
                group: REQUIRED_GROUP.to_string(),
                default_model: "gpt-5.6-sol".to_string(),
                allowed_models: vec!["gpt-5.6-sol".to_string()],
                model_groups: Default::default(),
                key_name: "Beefex".to_string(),
                base_url,
            },
            &SecretCredential::new(SECRET.to_string()),
            None,
        )
        .unwrap()
    }

    async fn mock_upstream() -> (String, tokio::sync::oneshot::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (send, receive) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = vec![0_u8; 16 * 1024];
            let read = stream.read(&mut bytes).await.unwrap();
            let request = String::from_utf8_lossy(&bytes[..read]).to_string();
            let _ = send.send(request);
            let body = "data: {\"type\":\"response.completed\"}\n\n";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        (format!("http://{address}/v1"), receive)
    }

    #[tokio::test]
    async fn broker_injects_parent_credential_and_streams_without_exposing_it_in_debug() {
        let (base_url, request) = mock_upstream().await;
        let broker = PiProviderBroker::start(Client::new(), provider(base_url))
            .await
            .unwrap();
        assert!(!format!("{broker:?}").contains(SECRET));
        let response = Client::new()
            .post(format!("{}/responses", broker.endpoint()))
            .header("authorization", LOCAL_AUTHORIZATION)
            .json(&serde_json::json!({ "model": broker.model(), "stream": true }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response
            .text()
            .await
            .unwrap()
            .contains("response.completed"));
        let request = request.await.unwrap().to_ascii_lowercase();
        assert!(request.contains(&format!("authorization: bearer {SECRET}")));
        assert!(request.contains("x-beefex-group: gpt-pro"));
    }

    #[tokio::test]
    async fn broker_encodes_anthropic_family_group_for_the_selected_model() {
        let (base_url, request) = mock_upstream().await;
        let mut model_groups = std::collections::BTreeMap::new();
        model_groups.insert("claude-fable-5".to_string(), "claude max".to_string());
        let provider = hydrate_managed_provider(
            &SafeAccountMetadata {
                email: "ender@example.com".to_string(),
                group: REQUIRED_GROUP.to_string(),
                default_model: "gpt-5.6-sol".to_string(),
                allowed_models: vec!["gpt-5.6-sol".to_string(), "claude-fable-5".to_string()],
                model_groups,
                key_name: "Beefex".to_string(),
                base_url,
            },
            &SecretCredential::new(SECRET.to_string()),
            Some("claude-fable-5"),
        )
        .unwrap();
        let broker = PiProviderBroker::start(Client::new(), provider)
            .await
            .unwrap();
        let response = Client::new()
            .post(format!("{}/responses", broker.endpoint()))
            .header("authorization", LOCAL_AUTHORIZATION)
            .json(&serde_json::json!({ "model": broker.model(), "stream": true }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let request = request.await.unwrap().to_ascii_lowercase();
        assert!(request.contains("x-beefex-group: claude%20max"));
        assert_eq!(encode_beefex_group_header("claude max"), "claude%20max");
    }

    #[test]
    fn encoded_groups_match_beefapi_query_unescape_fixtures() {
        // Keep these strings identical to middleware.TestBeefexRequestedGroupMatchesDesktopEncoderFixtures.
        assert_eq!(encode_beefex_group_header("gpt-pro"), "gpt-pro");
        assert_eq!(encode_beefex_group_header("claude max"), "claude%20max");
        assert_eq!(
            encode_beefex_group_header("claude 特惠"),
            "claude%20%E7%89%B9%E6%83%A0"
        );
        assert_eq!(encode_beefex_group_header("grok"), "grok");
    }

    #[tokio::test]
    async fn broker_rejects_unknown_capability_without_reaching_upstream() {
        let (base_url, request) = mock_upstream().await;
        let broker = PiProviderBroker::start(Client::new(), provider(base_url))
            .await
            .unwrap();
        let mut wrong = url::Url::parse(broker.endpoint()).unwrap();
        wrong.set_path("/wrong/v1/responses");
        let response = Client::new()
            .post(wrong)
            .header("authorization", LOCAL_AUTHORIZATION)
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(timeout(Duration::from_millis(100), request).await.is_err());
    }

    #[tokio::test]
    async fn broker_marks_upstream_credential_rejection_for_parent_cleanup() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = vec![0_u8; 16 * 1024];
            let _ = stream.read(&mut bytes).await.unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                )
                .await
                .unwrap();
        });
        let broker =
            PiProviderBroker::start(Client::new(), provider(format!("http://{address}/v1")))
                .await
                .unwrap();
        let response = Client::new()
            .post(format!("{}/responses", broker.endpoint()))
            .header("authorization", LOCAL_AUTHORIZATION)
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(broker.authorization_rejected());
    }

    #[tokio::test]
    async fn broker_returns_redacted_bad_gateway_when_upstream_is_unreachable() {
        let broker = PiProviderBroker::start(
            Client::builder()
                .connect_timeout(Duration::from_millis(100))
                .build()
                .unwrap(),
            provider("http://127.0.0.1:1/v1".to_string()),
        )
        .await
        .unwrap();
        let response = Client::new()
            .post(format!("{}/responses", broker.endpoint()))
            .header("authorization", LOCAL_AUTHORIZATION)
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = response.text().await.unwrap();
        assert!(body.contains("pi_provider_broker_upstream_failed"));
        assert!(!body.contains(SECRET));
    }
}
