use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::llm::{LlmClassification, OpenAiClient, OpenRouterClient, UnknownEntry};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ProviderKind {
    OpenRouter,
    OpenAI,
    Anthropic,
    Google,
    // TODO(#ollama-support): restore live Ollama runtime support without breaking provider UX.
    Ollama,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSettings {
    pub model: String,
    pub base_url: String,
}

pub type ProviderSettingsMap = BTreeMap<ProviderKind, ProviderSettings>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProviderConfig {
    pub kind: ProviderKind,
    pub api_key: Option<String>,
    pub model: String,
    pub base_url: String,
}

impl ResolvedProviderConfig {
    pub fn new(
        kind: ProviderKind,
        api_key: Option<String>,
        model: String,
        base_url: String,
    ) -> Self {
        Self {
            kind,
            api_key,
            model,
            base_url,
        }
    }
}

impl ProviderKind {
    pub fn keychain_account(self) -> &'static str {
        match self {
            ProviderKind::OpenRouter => "openrouter",
            ProviderKind::OpenAI => "openai",
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Google => "google",
            ProviderKind::Ollama => "ollama",
        }
    }
}

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("provider {provider:?} is missing an API key")]
    MissingApiKey { provider: ProviderKind },
    #[error("request to {provider:?} failed: {message}")]
    Request {
        provider: ProviderKind,
        message: String,
    },
    #[error("response from {provider:?} was invalid: {message}")]
    Response {
        provider: ProviderKind,
        message: String,
    },
}

#[derive(Clone)]
pub enum LlmClient {
    OpenRouter(OpenRouterClient),
    OpenAI(OpenAiClient),
}

impl LlmClient {
    pub fn kind(&self) -> ProviderKind {
        match self {
            Self::OpenRouter(_) => ProviderKind::OpenRouter,
            Self::OpenAI(_) => ProviderKind::OpenAI,
        }
    }

    pub async fn classify_batch(&self, entries: Vec<UnknownEntry>) -> Vec<LlmClassification> {
        match self {
            Self::OpenRouter(client) => client.classify_batch(entries).await,
            Self::OpenAI(client) => client.classify_batch(entries).await,
        }
    }
}

pub fn default_provider_settings(kind: ProviderKind) -> ProviderSettings {
    match kind {
        ProviderKind::OpenRouter => ProviderSettings {
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
        },
        ProviderKind::OpenAI => ProviderSettings {
            model: "gpt-4o-mini".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
        },
        ProviderKind::Anthropic => ProviderSettings {
            model: "claude-3-5-haiku-latest".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
        },
        ProviderKind::Google => ProviderSettings {
            model: "gemini-2.0-flash".to_string(),
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
        },
        // TODO(#ollama-support): re-enable Ollama defaults for new configs when runtime support returns.
        ProviderKind::Ollama => ProviderSettings {
            model: "llama3.1:8b".to_string(),
            base_url: "http://127.0.0.1:11434".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{OpenAiClient, OpenRouterClient};

    #[test]
    fn provider_kind_should_return_expected_keychain_account_names() {
        assert_eq!(ProviderKind::OpenRouter.keychain_account(), "openrouter");
        assert_eq!(ProviderKind::Ollama.keychain_account(), "ollama");
    }

    #[test]
    fn llm_client_should_report_openrouter_kind() {
        let client = LlmClient::OpenRouter(OpenRouterClient::new(ResolvedProviderConfig::new(
            ProviderKind::OpenRouter,
            Some("test-key".to_string()),
            "google/gemini-2.0-flash-001".to_string(),
            "https://openrouter.ai/api/v1".to_string(),
        )));

        assert_eq!(client.kind(), ProviderKind::OpenRouter);
    }

    #[test]
    fn llm_client_should_report_openai_kind() {
        let client = LlmClient::OpenAI(OpenAiClient::new(ResolvedProviderConfig::new(
            ProviderKind::OpenAI,
            Some("sk-test".to_string()),
            "gpt-4o-mini".to_string(),
            "https://api.openai.com/v1".to_string(),
        )));

        assert_eq!(client.kind(), ProviderKind::OpenAI);
    }
}
