use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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

pub struct OpenRouterClient {
    api_key: String,
    client: reqwest::Client,
    model: String,
}

impl OpenRouterClient {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
            model: "google/gemini-2.0-flash-001".to_string(),
        }
    }

    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    pub async fn classify_batch(
        &self,
        entries: Vec<UnknownEntry>,
    ) -> Vec<LlmClassification> {
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
    ) -> Result<Vec<LlmClassification>, Box<dyn std::error::Error + Send + Sync>> {
        let paths_description: String = entries
            .iter()
            .map(|e| {
                let kind = if e.is_dir { "dir" } else { "file" };
                let age = e
                    .age_days
                    .map(|d| format!(", {d} days old"))
                    .unwrap_or_default();
                format!(
                    "- {} ({}, {} bytes{})",
                    e.path.display(),
                    kind,
                    e.size,
                    age
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            r#"You are a macOS filesystem expert. Classify each path below.

For each path, respond with a JSON array of objects with these fields:
- "path": the exact path string
- "category": one of "BuildArtifact", "Cache", "Download", "AppData", "Media", "System", "Unknown"
- "safety": one of "Safe", "Caution", "Unsafe"
- "reason": a one-line explanation of what this is and whether it's safe to delete

Paths to classify:
{paths_description}

Respond with ONLY a JSON array, no markdown formatting, no explanation."#
        );

        let request = ChatRequest {
            model: self.model.clone(),
            messages: vec![Message {
                role: "user".to_string(),
                content: prompt,
            }],
            temperature: 0.0,
        };

        let response = self
            .client
            .post("https://openrouter.ai/api/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request)
            .send()
            .await?
            .error_for_status()?;

        let chat_response: ChatResponse = response.json().await?;
        parse_chat_response(chat_response)
    }
}

fn parse_chat_response(
    chat_response: ChatResponse,
) -> Result<Vec<LlmClassification>, Box<dyn std::error::Error + Send + Sync>> {
    let content = chat_response
        .choices
        .first()
        .map(|choice| choice.message.content.as_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "LLM response contained no choices"))?;

    // Strip markdown code fences if present
    let json_str = content
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let items: Vec<ClassificationItem> = serde_json::from_str(json_str)?;

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
        let response = ChatResponse { choices: Vec::new() };

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
}
