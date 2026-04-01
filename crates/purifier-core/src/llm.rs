use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::provider::{LlmError, LlmRequestErrorKind, ProviderKind, ResolvedProviderConfig};
use crate::types::{Category, SafetyLevel};

#[derive(Debug, Clone)]
pub struct UnknownEntry {
    pub path: PathBuf,
    pub size: u64,
    pub is_dir: bool,
    pub age_days: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct LlmClassification {
    pub path: PathBuf,
    pub category: Category,
    pub safety: SafetyLevel,
    pub reason: String,
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    content: String,
}

#[derive(Debug, Deserialize)]
struct ClassificationItem {
    path: String,
    category: Category,
    safety: SafetyLevel,
    reason: String,
}

#[derive(Clone)]
pub struct OpenRouterClient {
    config: ResolvedProviderConfig,
    client: reqwest::Client,
}

impl OpenRouterClient {
    pub fn new(config: ResolvedProviderConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    pub fn config(&self) -> &ResolvedProviderConfig {
        &self.config
    }

    pub fn chat_endpoint(&self) -> String {
        openai_compatible_chat_endpoint(&self.config.base_url)
    }

    pub async fn validate_connection(&self) -> Result<(), LlmError> {
        validate_openai_compatible_connection(&self.client, &self.config, &self.chat_endpoint())
            .await
    }

    pub async fn classify_batch(&self, entries: Vec<UnknownEntry>) -> Vec<LlmClassification> {
        match self.do_classify_batch(&entries).await {
            Ok(results) => results,
            Err(_) => entries
                .into_iter()
                .map(|e| LlmClassification {
                    path: e.path,
                    category: Category::Unknown,
                    safety: SafetyLevel::Unknown,
                    reason: "Could not classify — review manually".to_string(),
                })
                .collect(),
        }
    }

    async fn do_classify_batch(
        &self,
        entries: &[UnknownEntry],
    ) -> Result<Vec<LlmClassification>, LlmError> {
        classify_with_openai_compatible_api(
            &self.client,
            &self.config,
            &self.chat_endpoint(),
            entries,
        )
        .await
    }
}

#[derive(Clone)]
pub struct OpenAiClient {
    config: ResolvedProviderConfig,
    client: reqwest::Client,
}

impl OpenAiClient {
    pub fn new(config: ResolvedProviderConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    pub fn chat_endpoint(&self) -> String {
        openai_compatible_chat_endpoint(&self.config.base_url)
    }

    pub async fn validate_connection(&self) -> Result<(), LlmError> {
        validate_openai_compatible_connection(&self.client, &self.config, &self.chat_endpoint())
            .await
    }

    pub async fn classify_batch(&self, entries: Vec<UnknownEntry>) -> Vec<LlmClassification> {
        match self.do_classify_batch(&entries).await {
            Ok(results) => results,
            Err(_) => fallback_classifications(entries),
        }
    }

    async fn do_classify_batch(
        &self,
        entries: &[UnknownEntry],
    ) -> Result<Vec<LlmClassification>, LlmError> {
        classify_with_openai_compatible_api(
            &self.client,
            &self.config,
            &self.chat_endpoint(),
            entries,
        )
        .await
    }
}

async fn classify_with_openai_compatible_api(
    client: &reqwest::Client,
    config: &ResolvedProviderConfig,
    chat_endpoint: &str,
    entries: &[UnknownEntry],
) -> Result<Vec<LlmClassification>, LlmError> {
    let api_key = config.api_key.as_deref().ok_or(LlmError::MissingApiKey {
        provider: config.kind,
    })?;
    let request = classification_request(config, entries);

    let response =
        send_openai_compatible_request(client, config, chat_endpoint, api_key, &request, None)
            .await?;

    let chat_response: ChatResponse =
        response.json().await.map_err(|error| LlmError::Response {
            provider: config.kind,
            message: error.to_string(),
        })?;

    parse_chat_response(config.kind, chat_response)
}

async fn validate_openai_compatible_connection(
    client: &reqwest::Client,
    config: &ResolvedProviderConfig,
    chat_endpoint: &str,
) -> Result<(), LlmError> {
    let api_key = config.api_key.as_deref().ok_or(LlmError::MissingApiKey {
        provider: config.kind,
    })?;
    let request = validation_request(config);

    let response = send_openai_compatible_request(
        client,
        config,
        chat_endpoint,
        api_key,
        &request,
        Some(validation_timeout()),
    )
    .await?;

    let chat_response: ChatResponse =
        response.json().await.map_err(|error| LlmError::Response {
            provider: config.kind,
            message: error.to_string(),
        })?;

    let parsed = parse_chat_response(config.kind, chat_response)?;
    if parsed
        .iter()
        .any(|item| item.path == Path::new("/tmp/purifier-validation"))
    {
        Ok(())
    } else {
        Err(LlmError::Response {
            provider: config.kind,
            message: io::Error::new(
                io::ErrorKind::InvalidData,
                "LLM validation response did not classify the synthetic path",
            )
            .to_string(),
        })
    }
}

fn openai_compatible_chat_endpoint(base_url: &str) -> String {
    let base_url = base_url.trim_end_matches('/');
    if base_url.ends_with("/chat/completions") {
        base_url.to_string()
    } else {
        format!("{base_url}/chat/completions")
    }
}

fn classification_request(
    config: &ResolvedProviderConfig,
    entries: &[UnknownEntry],
) -> ChatRequest {
    ChatRequest {
        model: config.model.clone(),
        messages: vec![Message {
            role: "user".to_string(),
            content: classification_prompt(entries),
        }],
        temperature: 0.0,
        max_tokens: None,
    }
}

fn validation_request(config: &ResolvedProviderConfig) -> ChatRequest {
    ChatRequest {
        model: config.model.clone(),
        messages: vec![Message {
            role: "user".to_string(),
            content: validation_prompt(),
        }],
        temperature: 0.0,
        max_tokens: None,
    }
}

fn validation_prompt() -> String {
    "Classify this single path for a connection check. Return the same JSON array schema used for runtime classification with path, category, safety, and reason. Respond with ONLY a JSON array like [{\"path\":\"/tmp/purifier-validation\",\"category\":\"BuildArtifact\",\"safety\":\"Safe\",\"reason\":\"Validation probe\"}]. Path: /tmp/purifier-validation, Size: 1024 bytes, Type: File, Age: 1 day.".to_string()
}

async fn send_openai_compatible_request(
    client: &reqwest::Client,
    config: &ResolvedProviderConfig,
    chat_endpoint: &str,
    api_key: &str,
    request: &ChatRequest,
    timeout: Option<Duration>,
) -> Result<reqwest::Response, LlmError> {
    let mut request_builder = client
        .post(chat_endpoint)
        .header("Authorization", format!("Bearer {api_key}"))
        .json(request);
    if let Some(timeout) = timeout {
        request_builder = request_builder.timeout(timeout);
    }

    let response = request_builder
        .send()
        .await
        .map_err(|error| request_error(config.kind, error))?;

    let status = response.status();
    if status.is_success() {
        Ok(response)
    } else {
        let body = response
            .text()
            .await
            .ok()
            .and_then(|body| summarize_http_error_body(&body));
        Err(http_request_error(config.kind, status.as_u16(), body))
    }
}

fn request_error(provider: ProviderKind, error: reqwest::Error) -> LlmError {
    let kind = if error.is_timeout() {
        LlmRequestErrorKind::Timeout
    } else {
        LlmRequestErrorKind::Network {
            message: error.to_string(),
        }
    };

    LlmError::Request { provider, kind }
}

fn http_request_error(provider: ProviderKind, status: u16, body: Option<String>) -> LlmError {
    LlmError::Request {
        provider,
        kind: LlmRequestErrorKind::Http { status, body },
    }
}

fn summarize_http_error_body(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(message) = json
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(|message| message.as_str())
        {
            return Some(message.trim().to_string());
        }

        if let Some(message) = json.get("message").and_then(|message| message.as_str()) {
            return Some(message.trim().to_string());
        }
    }

    let summary = trimmed.lines().next()?.trim();
    (!summary.is_empty()).then(|| summary.to_string())
}

fn validation_timeout() -> Duration {
    #[cfg(test)]
    {
        Duration::from_millis(150)
    }

    #[cfg(not(test))]
    {
        Duration::from_secs(5)
    }
}

fn fallback_classifications(entries: Vec<UnknownEntry>) -> Vec<LlmClassification> {
    entries
        .into_iter()
        .map(|entry| LlmClassification {
            path: entry.path,
            category: Category::Unknown,
            safety: SafetyLevel::Unknown,
            reason: "Could not classify — review manually".to_string(),
        })
        .collect()
}

fn classification_prompt(entries: &[UnknownEntry]) -> String {
    let paths_description: String = entries
        .iter()
        .map(|entry| {
            let kind = if entry.is_dir { "dir" } else { "file" };
            let age = entry
                .age_days
                .map(|days| format!(", {days} days old"))
                .unwrap_or_default();
            format!(
                "- {} ({}, {} bytes{})",
                entry.path.display(),
                kind,
                entry.size,
                age
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"You are a macOS filesystem expert. Classify each path below.

For each path, respond with a JSON array of objects with these fields:
- "path": the exact path string
- "category": one of "BuildArtifact", "Cache", "Download", "AppData", "Media", "System", "Unknown"
- "safety": one of "Safe", "Caution", "Unsafe"
- "reason": a one-line explanation of what this is and whether it's safe to delete

Paths to classify:
{paths_description}

Respond with ONLY a JSON array, no markdown formatting, no explanation."#
    )
}

fn parse_chat_response(
    provider: ProviderKind,
    chat_response: ChatResponse,
) -> Result<Vec<LlmClassification>, LlmError> {
    let content = chat_response
        .choices
        .first()
        .map(|choice| choice.message.content.as_str())
        .ok_or_else(|| LlmError::Response {
            provider,
            message: io::Error::new(
                io::ErrorKind::InvalidData,
                "LLM response contained no choices",
            )
            .to_string(),
        })?;

    parse_response_content(provider, content)
}

fn parse_response_content(
    provider: ProviderKind,
    content: &str,
) -> Result<Vec<LlmClassification>, LlmError> {
    // Strip markdown code fences if present
    let json_str = content
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let items: Vec<ClassificationItem> =
        serde_json::from_str(json_str).map_err(|error| LlmError::Response {
            provider,
            message: error.to_string(),
        })?;

    Ok(items
        .into_iter()
        .map(|item| LlmClassification {
            path: PathBuf::from(item.path),
            category: item.category,
            safety: item.safety,
            reason: item.reason,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use super::*;

    fn spawn_http_server(status_line: &'static str, body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        let address = listener
            .local_addr()
            .expect("test server should have a local address");

        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("test server should accept");
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 {status_line}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("test server should respond");
        });

        format!("http://{address}")
    }

    fn spawn_http_server_with_capture(
        status_line: &'static str,
        body: &'static str,
        captured_request: Arc<Mutex<Vec<u8>>>,
        delay_before_response: Option<std::time::Duration>,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        let address = listener
            .local_addr()
            .expect("test server should have a local address");
        let captured_request_for_thread = Arc::clone(&captured_request);

        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("test server should accept");
            let mut buffer = [0_u8; 4096];
            let bytes_read = stream.read(&mut buffer).expect("test server should read");
            *captured_request_for_thread
                .lock()
                .expect("capture lock should succeed") = buffer[..bytes_read].to_vec();
            if let Some(delay) = delay_before_response {
                thread::sleep(delay);
            }
            let response = format!(
                "HTTP/1.1 {status_line}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("test server should respond");
        });

        format!("http://{address}")
    }

    fn response_with_content(content: &str) -> ChatResponse {
        ChatResponse {
            choices: vec![Choice {
                message: ResponseMessage {
                    content: content.to_string(),
                },
            }],
        }
    }

    #[test]
    fn parse_chat_response_should_return_error_when_choices_are_empty() {
        let response = ChatResponse {
            choices: Vec::new(),
        };

        assert!(parse_chat_response(response).is_err());
    }

    #[test]
    fn parse_chat_response_should_parse_markdown_wrapped_json() {
        let response = response_with_content(
            "```json\n[{\"path\":\"/tmp/cache\",\"category\":\"Cache\",\"safety\":\"Safe\",\"reason\":\"Recreated automatically\"}]\n```",
        );

        let parsed = parse_chat_response(response).expect("markdown-wrapped JSON should parse");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].path, PathBuf::from("/tmp/cache"));
        assert_eq!(parsed[0].category, Category::Cache);
        assert_eq!(parsed[0].safety, SafetyLevel::Safe);
    }

    #[test]
    fn parse_chat_response_should_return_error_for_malformed_json() {
        let response = response_with_content("[{\"path\":\"/tmp/cache\"");

        assert!(parse_chat_response(response).is_err());
    }

    #[test]
    fn parse_chat_response_should_parse_valid_json() {
        let response = response_with_content(
            "[{\"path\":\"/tmp/build\",\"category\":\"BuildArtifact\",\"safety\":\"Safe\",\"reason\":\"Build output\"}]",
        );

        let parsed = parse_chat_response(response).expect("valid JSON should parse");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].path, PathBuf::from("/tmp/build"));
        assert_eq!(parsed[0].category, Category::BuildArtifact);
        assert_eq!(parsed[0].safety, SafetyLevel::Safe);
        assert_eq!(parsed[0].reason, "Build output");
    }

    #[tokio::test]
    async fn openai_client_should_validate_connection_with_minimal_chat_request_for_configured_model(
    ) {
        let captured_request = Arc::new(Mutex::new(Vec::new()));
        let base_url = spawn_http_server_with_capture(
            "200 OK",
            r#"{"choices":[{"message":{"content":"[{\"path\":\"/tmp/purifier-validation\",\"category\":\"BuildArtifact\",\"safety\":\"Safe\",\"reason\":\"Validation probe\"}]"}}]}"#,
            Arc::clone(&captured_request),
            None,
        );
        let client = OpenAiClient::new(ResolvedProviderConfig::new(
            ProviderKind::OpenAI,
            Some("sk-test".to_string()),
            "gpt-4o-mini".to_string(),
            base_url,
        ));

        let result = client.validate_connection().await;

        assert!(result.is_ok(), "expected connection validation to succeed");

        let request = String::from_utf8(
            captured_request
                .lock()
                .expect("capture lock should succeed")
                .clone(),
        )
        .expect("request should be valid UTF-8");
        assert!(request.starts_with("POST /chat/completions HTTP/1.1"));
        assert!(request.contains(r#""model":"gpt-4o-mini""#));
        assert!(request.contains("Classify this single path for a connection check."));
        assert!(request.contains("/tmp/purifier-validation"));
        assert!(request.contains(r#"\"category\""#));
        assert!(request.contains("Respond with ONLY a JSON array"));
        assert!(!request.contains("You are a macOS filesystem expert. Classify each path below."));
        assert!(!request.contains(r#""max_tokens""#));
    }

    #[tokio::test]
    async fn openrouter_client_should_return_request_error_when_connection_validation_fails() {
        let base_url = spawn_http_server(
            "401 Unauthorized",
            r#"{"error":{"message":"Invalid API key"}}"#,
        );
        let client = OpenRouterClient::new(ResolvedProviderConfig::new(
            ProviderKind::OpenRouter,
            Some("bad-key".to_string()),
            "google/gemini-2.0-flash-001".to_string(),
            base_url,
        ));

        let error = client
            .validate_connection()
            .await
            .expect_err("expected unauthorized validation to fail");

        assert!(matches!(
            error,
            LlmError::Request {
                provider: ProviderKind::OpenRouter,
                kind: crate::provider::LlmRequestErrorKind::Http { status: 401, .. },
                ..
            }
        ));
    }

    #[tokio::test]
    async fn openai_client_should_preserve_http_body_detail_for_model_failures() {
        let base_url = spawn_http_server(
            "400 Bad Request",
            r#"{"error":{"message":"The model gpt-missing does not exist"}}"#,
        );
        let client = OpenAiClient::new(ResolvedProviderConfig::new(
            ProviderKind::OpenAI,
            Some("sk-test".to_string()),
            "gpt-missing".to_string(),
            base_url,
        ));

        let error = client
            .validate_connection()
            .await
            .expect_err("expected model validation to fail");

        assert!(matches!(
            error,
            LlmError::Request {
                provider: ProviderKind::OpenAI,
                kind: crate::provider::LlmRequestErrorKind::Http {
                    status: 400,
                    body: Some(ref body),
                },
                ..
            } if body.contains("gpt-missing")
        ));
    }

    #[tokio::test]
    async fn openai_client_should_use_base_url_directly_when_it_already_points_to_chat_completions()
    {
        let captured_request = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        let address = listener
            .local_addr()
            .expect("test server should have a local address");
        let captured_request_for_thread = Arc::clone(&captured_request);

        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("test server should accept");
            let mut buffer = [0_u8; 4096];
            let bytes_read = stream.read(&mut buffer).expect("test server should read");
            *captured_request_for_thread
                .lock()
                .expect("capture lock should succeed") = buffer[..bytes_read].to_vec();
            let body = r#"{"choices":[{"message":{"content":"[{\"path\":\"/tmp/purifier-validation\",\"category\":\"BuildArtifact\",\"safety\":\"Safe\",\"reason\":\"Validation probe\"}]"}}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("test server should respond");
        });

        let client = OpenAiClient::new(ResolvedProviderConfig::new(
            ProviderKind::OpenAI,
            Some("sk-test".to_string()),
            "gpt-4o-mini".to_string(),
            format!("http://{address}/chat/completions"),
        ));

        client
            .validate_connection()
            .await
            .expect("expected direct chat endpoint to validate");

        let request = String::from_utf8(
            captured_request
                .lock()
                .expect("capture lock should succeed")
                .clone(),
        )
        .expect("request should be valid UTF-8");
        assert!(request.starts_with("POST /chat/completions HTTP/1.1"));
    }

    #[tokio::test]
    async fn openrouter_client_should_time_out_slow_connection_validation_requests() {
        let captured_request = Arc::new(Mutex::new(Vec::new()));
        let base_url = spawn_http_server_with_capture(
            "200 OK",
            r#"{"choices":[{"message":{"content":"ok"}}]}"#,
            Arc::clone(&captured_request),
            Some(std::time::Duration::from_millis(400)),
        );
        let client = OpenRouterClient::new(ResolvedProviderConfig::new(
            ProviderKind::OpenRouter,
            Some("or-key".to_string()),
            "google/gemini-2.0-flash-001".to_string(),
            base_url,
        ));

        let error = client
            .validate_connection()
            .await
            .expect_err("expected slow validation to time out");

        match error {
            LlmError::Request {
                kind: LlmRequestErrorKind::Timeout,
                ..
            } => {}
            other => panic!("expected request timeout, got {other:?}"),
        }
    }

    fn parse_chat_response(
        chat_response: ChatResponse,
    ) -> Result<Vec<LlmClassification>, LlmError> {
        super::parse_chat_response(ProviderKind::OpenRouter, chat_response)
    }
}

#[cfg(test)]
mod provider_defaults_tests {
    use crate::llm::OpenAiClient;
    use crate::provider::{ProviderKind, ResolvedProviderConfig};

    #[test]
    fn openai_client_should_use_openai_chat_endpoint() {
        let config = ResolvedProviderConfig::new(
            ProviderKind::OpenAI,
            Some("sk-test".to_string()),
            "gpt-4o-mini".to_string(),
            "https://api.openai.com/v1".to_string(),
        );

        let client = OpenAiClient::new(config.clone());

        assert_eq!(
            client.chat_endpoint(),
            "https://api.openai.com/v1/chat/completions"
        );
    }
}
