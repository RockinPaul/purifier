use std::fs;
use std::path::{Path, PathBuf};

use purifier_core::provider::{default_provider_settings, ProviderKind, ProviderSettingsMap};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::app::View;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("failed to serialize config: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("failed to create config directory {path}: {source}")]
    CreateDir {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write config {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    pub ui: UiConfig,
    pub llm: LlmConfig,
    pub onboarding: OnboardingConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiConfig {
    pub default_view: View,
    pub last_scan_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmConfig {
    pub enabled: bool,
    pub active_provider: ProviderKind,
    pub providers: ProviderSettingsMap,
}

impl Default for LlmConfig {
    fn default() -> Self {
        let mut providers = ProviderSettingsMap::new();
        for kind in [
            ProviderKind::OpenRouter,
            ProviderKind::OpenAI,
            ProviderKind::Anthropic,
            ProviderKind::Google,
        ] {
            providers.insert(kind, default_provider_settings(kind));
        }

        Self {
            enabled: true,
            active_provider: ProviderKind::OpenRouter,
            providers,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OnboardingConfig {
    pub first_launch_prompt_dismissed: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            ui: UiConfig {
                default_view: View::BySize,
                last_scan_path: None,
            },
            llm: LlmConfig::default(),
            onboarding: OnboardingConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn load_or_default(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let raw = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.display().to_string(),
            source,
        })?;
        toml::from_str(&raw).map_err(|source| ConfigError::Parse {
            path: path.display().to_string(),
            source,
        })
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| ConfigError::CreateDir {
                path: parent.display().to_string(),
                source,
            })?;
        }

        let raw = toml::to_string_pretty(self)?;
        fs::write(path, raw).map_err(|source| ConfigError::Write {
            path: path.display().to_string(),
            source,
        })
    }
}

pub fn default_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("purifier")
        .join("config.toml")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::app::View;

    #[test]
    fn load_or_default_should_return_defaults_when_file_is_missing() {
        let tempdir = tempfile::tempdir().unwrap();
        let config_path = tempdir.path().join("config.toml");

        let config = AppConfig::load_or_default(&config_path).unwrap();

        assert_eq!(config.ui.default_view, View::BySize);
        assert_eq!(config.ui.last_scan_path, None);
        assert!(config.llm.enabled);
        assert_eq!(config.llm.active_provider, ProviderKind::OpenRouter);
        assert!(!config.onboarding.first_launch_prompt_dismissed);
    }

    #[test]
    fn save_should_round_trip_last_scan_path_and_default_view() {
        let tempdir = tempfile::tempdir().unwrap();
        let config_path = tempdir.path().join("config.toml");
        let config = AppConfig {
            ui: UiConfig {
                default_view: View::BySafety,
                last_scan_path: Some(PathBuf::from("/tmp/purifier")),
            },
            llm: LlmConfig::default(),
            onboarding: OnboardingConfig::default(),
        };

        config.save(&config_path).unwrap();
        let loaded = AppConfig::load_or_default(&config_path).unwrap();

        assert_eq!(loaded.ui.default_view, View::BySafety);
        assert_eq!(
            loaded.ui.last_scan_path,
            Some(PathBuf::from("/tmp/purifier"))
        );
    }
}
