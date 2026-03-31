use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::provider::{LlmError, ProviderKind, ResolvedProviderConfig};
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
        format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        )
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
        classify_with_openai_compatible_api(&self.client, &self.config, &self.chat_endpoint(), entries)
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
        format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        )
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
        classify_with_openai_compatible_api(&self.client, &self.config, &self.chat_endpoint(), entries)
            .await
    }
}

async fn classify_with_openai_compatible_api(
    client: &reqwest::Client,
    config: &ResolvedProviderConfig,
    chat_endpoint: &str,
    entries: &[UnknownEntry],
) -> Result<Vec<LlmClassification>, LlmError> {
    let api_key = config
        .api_key
        .as_deref()
        .ok_or(LlmError::MissingApiKey { provider: config.kind })?;
    let request = ChatRequest {
        model: config.model.clone(),
        messages: vec![Message {
            role: "user".to_string(),
            content: classification_prompt(entries),
        }],
        temperature: 0.0,
    };

    let response = client
        .post(chat_endpoint)
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&request)
        .send()
        .await
        .map_err(|error| LlmError::Request {
            provider: config.kind,
            message: error.to_string(),
        })?
        .error_for_status()
        .map_err(|error| LlmError::Request {
            provider: config.kind,
            message: error.to_string(),
        })?;

    let chat_response: ChatResponse = response.json().await.map_err(|error| LlmError::Response {
        provider: config.kind,
        message: error.to_string(),
    })?;

    parse_chat_response(config.kind, chat_response)
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
    use super::*;

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

    fn parse_chat_response(chat_response: ChatResponse) -> Result<Vec<LlmClassification>, LlmError> {
        super::parse_chat_response(ProviderKind::OpenRouter, chat_response)
    }
}

#[cfg(test)]
mod provider_defaults_tests {
    use crate::provider::{ProviderKind, ResolvedProviderConfig};
    use crate::llm::OpenAiClient;

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
