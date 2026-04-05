use std::fs;
use std::path::{Path, PathBuf};

use purifier_core::built_in_scan_profiles;
use purifier_core::provider::{default_provider_settings, ProviderKind, ProviderSettingsMap};
use purifier_core::size::SizeMode;
use purifier_core::ScanProfile;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::columns::SortKey;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AppConfig {
    pub ui: UiConfig,
    pub llm: LlmConfig,
    pub onboarding: OnboardingConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub sort_key: SortKey,
    pub last_scan_path: Option<PathBuf>,
    pub size_mode: SizeMode,
    pub scan_profiles: Vec<ScanProfile>,
    pub last_selected_scan_profile: Option<String>,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            sort_key: SortKey::default(),
            last_scan_path: None,
            size_mode: SizeMode::Physical,
            scan_profiles: built_in_scan_profiles(),
            last_selected_scan_profile: None,
        }
    }
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

impl AppConfig {
    pub fn active_scan_profile(&self) -> Option<&ScanProfile> {
        let selected = self.ui.last_selected_scan_profile.as_deref()?;
        self.ui
            .scan_profiles
            .iter()
            .find(|profile| profile.name == selected)
    }

    pub fn load_or_default(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let raw = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.display().to_string(),
            source,
        })?;
        let config = toml::from_str(&raw).map_err(|source| ConfigError::Parse {
            path: path.display().to_string(),
            source,
        })?;

        Ok(Self::with_built_in_scan_profiles(config))
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| ConfigError::CreateDir {
                path: parent.display().to_string(),
                source,
            })?;
        }

        let mut persisted = self.clone();
        persisted.ui.scan_profiles = persisted_custom_scan_profiles(&persisted.ui.scan_profiles);

        let raw = toml::to_string_pretty(&persisted)?;
        fs::write(path, raw).map_err(|source| ConfigError::Write {
            path: path.display().to_string(),
            source,
        })
    }

    fn with_built_in_scan_profiles(mut config: Self) -> Self {
        config.ui.scan_profiles = merge_scan_profiles(config.ui.scan_profiles);
        config
    }
}

fn merge_scan_profiles(profiles: Vec<ScanProfile>) -> Vec<ScanProfile> {
    let mut merged = built_in_scan_profiles();

    for profile in profiles {
        if merged.iter().all(|existing| existing.name != profile.name) {
            merged.push(profile);
        }
    }

    merged
}

fn persisted_custom_scan_profiles(profiles: &[ScanProfile]) -> Vec<ScanProfile> {
    let built_ins = built_in_scan_profiles();

    profiles
        .iter()
        .filter(|profile| {
            built_ins
                .iter()
                .all(|built_in| built_in.name != profile.name)
        })
        .cloned()
        .collect()
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
    use crate::columns::SortKey;
    use purifier_core::{Filter, FilterTest, ScanProfile, SizeMode};

    #[test]
    fn load_or_default_should_return_defaults_when_file_is_missing() {
        let tempdir = tempfile::tempdir().unwrap();
        let config_path = tempdir.path().join("config.toml");

        let config = AppConfig::load_or_default(&config_path).unwrap();

        assert_eq!(config.ui.sort_key, SortKey::Size);
        assert_eq!(config.ui.last_scan_path, None);
        assert_eq!(config.ui.size_mode, SizeMode::Physical);
        assert_eq!(
            config
                .ui
                .scan_profiles
                .iter()
                .map(|profile| profile.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Full scan", "Fast developer scan"]
        );
        assert_eq!(config.ui.last_selected_scan_profile, None);
        assert!(config.llm.enabled);
        assert_eq!(config.llm.active_provider, ProviderKind::OpenRouter);
        assert!(!config.onboarding.first_launch_prompt_dismissed);
    }

    #[test]
    fn save_should_round_trip_last_scan_path_and_sort_key() {
        let tempdir = tempfile::tempdir().unwrap();
        let config_path = tempdir.path().join("config.toml");
        let config = AppConfig {
            ui: UiConfig {
                sort_key: SortKey::Safety,
                last_scan_path: Some(PathBuf::from("/tmp/purifier")),
                size_mode: SizeMode::Physical,
                scan_profiles: Vec::new(),
                last_selected_scan_profile: None,
            },
            llm: LlmConfig::default(),
            onboarding: OnboardingConfig::default(),
        };

        config.save(&config_path).unwrap();
        let loaded = AppConfig::load_or_default(&config_path).unwrap();

        assert_eq!(loaded.ui.sort_key, SortKey::Safety);
        assert_eq!(
            loaded.ui.last_scan_path,
            Some(PathBuf::from("/tmp/purifier"))
        );
    }

    #[test]
    fn save_should_round_trip_size_mode_and_scan_profiles() {
        let tempdir = tempfile::tempdir().unwrap();
        let config_path = tempdir.path().join("config.toml");
        let config = AppConfig {
            ui: UiConfig {
                sort_key: SortKey::Size,
                last_scan_path: Some(PathBuf::from("/tmp/project")),
                size_mode: SizeMode::Logical,
                scan_profiles: vec![ScanProfile {
                    name: "exclude-node-modules".to_string(),
                    exclude: Some(Filter::single(FilterTest::PathGlob(
                        "**/node_modules/**".to_string(),
                    ))),
                    mask: None,
                    display_filter: None,
                }],
                last_selected_scan_profile: Some("exclude-node-modules".to_string()),
            },
            llm: LlmConfig::default(),
            onboarding: OnboardingConfig::default(),
        };

        config.save(&config_path).unwrap();
        let loaded = AppConfig::load_or_default(&config_path).unwrap();

        assert_eq!(loaded.ui.size_mode, SizeMode::Logical);
        assert_eq!(
            loaded
                .ui
                .scan_profiles
                .iter()
                .map(|profile| profile.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Full scan", "Fast developer scan", "exclude-node-modules"]
        );
        assert_eq!(
            loaded.ui.last_selected_scan_profile,
            Some("exclude-node-modules".to_string())
        );
    }

    #[test]
    fn load_or_default_should_preserve_older_config_values_while_defaulting_new_ui_fields() {
        let tempdir = tempfile::tempdir().unwrap();
        let config_path = tempdir.path().join("config.toml");

        fs::write(
            &config_path,
            r#"
[ui]
sort_key = "Safety"
last_scan_path = "/tmp/legacy-scan"

[llm]
enabled = false
active_provider = "OpenAI"

[llm.providers.OpenRouter]
model = "openrouter/model"
base_url = "https://openrouter.example"

[llm.providers.OpenAI]
model = "openai/model"
base_url = "https://openai.example"

[llm.providers.Anthropic]
model = "anthropic/model"
base_url = "https://anthropic.example"

[llm.providers.Google]
model = "google/model"
base_url = "https://google.example"

[onboarding]
first_launch_prompt_dismissed = true
"#,
        )
        .unwrap();

        let config = AppConfig::load_or_default(&config_path).unwrap();

        assert_eq!(config.ui.sort_key, SortKey::Safety);
        assert_eq!(
            config.ui.last_scan_path,
            Some(PathBuf::from("/tmp/legacy-scan"))
        );
        assert_eq!(config.ui.size_mode, SizeMode::Physical);
        assert_eq!(
            config
                .ui
                .scan_profiles
                .iter()
                .map(|profile| profile.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Full scan", "Fast developer scan"]
        );
        assert_eq!(config.ui.last_selected_scan_profile, None);
        assert!(!config.llm.enabled);
        assert_eq!(config.llm.active_provider, ProviderKind::OpenAI);
        assert!(config.onboarding.first_launch_prompt_dismissed);
    }

    #[test]
    fn load_or_default_should_append_built_ins_without_dropping_custom_profiles() {
        let tempdir = tempfile::tempdir().unwrap();
        let config_path = tempdir.path().join("config.toml");

        fs::write(
            &config_path,
            r#"
[ui]
scan_profiles = [
  { name = "Project only", exclude = { Single = { PathGlob = "**/generated/**" } } }
]
last_selected_scan_profile = "Project only"

[llm]
enabled = true
active_provider = "OpenRouter"

[llm.providers.OpenRouter]
model = "openrouter/model"
base_url = "https://openrouter.example"

[llm.providers.OpenAI]
model = "openai/model"
base_url = "https://openai.example"

[llm.providers.Anthropic]
model = "anthropic/model"
base_url = "https://anthropic.example"

[llm.providers.Google]
model = "google/model"
base_url = "https://google.example"

[onboarding]
first_launch_prompt_dismissed = false
"#,
        )
        .unwrap();

        let config = AppConfig::load_or_default(&config_path).unwrap();
        let names = config
            .ui
            .scan_profiles
            .iter()
            .map(|profile| profile.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec!["Full scan", "Fast developer scan", "Project only"]
        );
        assert_eq!(
            config.ui.last_selected_scan_profile.as_deref(),
            Some("Project only")
        );
    }

    #[test]
    fn save_should_not_persist_built_in_scan_profiles() {
        let tempdir = tempfile::tempdir().unwrap();
        let config_path = tempdir.path().join("config.toml");
        let config = AppConfig::default();

        config.save(&config_path).unwrap();
        let raw = fs::read_to_string(&config_path).unwrap();

        assert!(!raw.contains("Full scan"));
        assert!(!raw.contains("Fast developer scan"));
    }

    #[test]
    fn load_or_default_should_ignore_stale_persisted_built_in_profiles() {
        let tempdir = tempfile::tempdir().unwrap();
        let config_path = tempdir.path().join("config.toml");

        fs::write(
            &config_path,
            r#"
[ui]
scan_profiles = [
  { name = "Fast developer scan", exclude = { Single = { PathGlob = "**/stale-built-in/**" } } }
]
last_selected_scan_profile = "Fast developer scan"

[llm]
enabled = true
active_provider = "OpenRouter"

[llm.providers.OpenRouter]
model = "openrouter/model"
base_url = "https://openrouter.example"

[llm.providers.OpenAI]
model = "openai/model"
base_url = "https://openai.example"

[llm.providers.Anthropic]
model = "anthropic/model"
base_url = "https://anthropic.example"

[llm.providers.Google]
model = "google/model"
base_url = "https://google.example"

[onboarding]
first_launch_prompt_dismissed = false
"#,
        )
        .unwrap();

        let config = AppConfig::load_or_default(&config_path).unwrap();
        let fast_profile = config
            .ui
            .scan_profiles
            .iter()
            .find(|profile| profile.name == "Fast developer scan")
            .unwrap();

        assert!(fast_profile.should_exclude(Path::new("/tmp/app/node_modules/react/index.js")));
        assert!(
            !fast_profile.should_exclude(Path::new("/tmp/stale-built-in/file.txt")),
            "stale persisted built-in should not override the current built-in definition"
        );
    }
}
