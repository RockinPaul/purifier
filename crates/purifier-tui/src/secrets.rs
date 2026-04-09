#[cfg(test)]
use std::collections::HashMap;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use purifier_core::provider::ProviderKind;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
enum SecretOperation {
    Read,
    Write,
    Delete,
}

fn secret_store_error(
    operation: SecretOperation,
    provider: ProviderKind,
    message: String,
) -> SecretStoreError {
    match operation {
        SecretOperation::Read => SecretStoreError::Read { provider, message },
        SecretOperation::Write => SecretStoreError::Write { provider, message },
        SecretOperation::Delete => SecretStoreError::Delete { provider, message },
    }
}

#[derive(Debug, Error)]
pub enum SecretStoreError {
    #[error("failed to read key for {provider:?}: {message}")]
    Read {
        provider: ProviderKind,
        message: String,
    },
    #[error("failed to write key for {provider:?}: {message}")]
    Write {
        provider: ProviderKind,
        message: String,
    },
    #[error("failed to delete key for {provider:?}: {message}")]
    Delete {
        provider: ProviderKind,
        message: String,
    },
}

pub trait SecretStore {
    fn load_api_key(&self, provider: ProviderKind) -> Result<Option<String>, SecretStoreError>;
    fn save_api_key(
        &mut self,
        provider: ProviderKind,
        api_key: &str,
    ) -> Result<(), SecretStoreError>;
    fn delete_api_key(&mut self, provider: ProviderKind) -> Result<(), SecretStoreError>;
}

/// File-based secret store. Stores API keys in a TOML file next to the config.
///
/// The `keyring` v3 crate on macOS has a bug where `Entry::new()` creates
/// non-portable credential references that don't survive across process restarts.
/// This file-based approach is reliable and consistent.
///
/// The file is created with restrictive permissions (0600 on Unix).
#[derive(Debug, Serialize, Deserialize, Default)]
struct SecretsFile {
    #[serde(default)]
    api_keys: BTreeMap<String, String>,
}

pub struct FileSecretStore {
    path: PathBuf,
}

impl FileSecretStore {
    pub fn new(config_dir: &Path) -> Self {
        Self {
            path: config_dir.join("secrets.toml"),
        }
    }

    fn read_file(&self) -> SecretsFile {
        match std::fs::read_to_string(&self.path) {
            Ok(contents) => toml::from_str(&contents).unwrap_or_default(),
            Err(_) => SecretsFile::default(),
        }
    }

    fn write_file(&self, secrets: &SecretsFile) -> Result<(), String> {
        // Ensure parent directory exists
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create secrets directory: {e}"))?;
        }

        let contents =
            toml::to_string_pretty(secrets).map_err(|e| format!("failed to serialize: {e}"))?;
        std::fs::write(&self.path, &contents)
            .map_err(|e| format!("failed to write secrets file: {e}"))?;

        // Set restrictive permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            let _ = std::fs::set_permissions(&self.path, perms);
        }

        Ok(())
    }
}

impl SecretStore for FileSecretStore {
    fn load_api_key(&self, provider: ProviderKind) -> Result<Option<String>, SecretStoreError> {
        let secrets = self.read_file();
        Ok(secrets
            .api_keys
            .get(provider.keychain_account())
            .cloned())
    }

    fn save_api_key(
        &mut self,
        provider: ProviderKind,
        api_key: &str,
    ) -> Result<(), SecretStoreError> {
        let mut secrets = self.read_file();
        secrets
            .api_keys
            .insert(provider.keychain_account().to_string(), api_key.to_string());
        self.write_file(&secrets).map_err(|message| {
            secret_store_error(SecretOperation::Write, provider, message)
        })
    }

    fn delete_api_key(&mut self, provider: ProviderKind) -> Result<(), SecretStoreError> {
        let mut secrets = self.read_file();
        secrets.api_keys.remove(provider.keychain_account());
        self.write_file(&secrets).map_err(|message| {
            secret_store_error(SecretOperation::Delete, provider, message)
        })
    }
}

#[cfg(test)]
#[derive(Debug, Default)]
pub struct FakeSecretStore {
    keys: HashMap<ProviderKind, String>,
}

#[cfg(test)]
impl SecretStore for FakeSecretStore {
    fn load_api_key(&self, provider: ProviderKind) -> Result<Option<String>, SecretStoreError> {
        Ok(self.keys.get(&provider).cloned())
    }

    fn save_api_key(
        &mut self,
        provider: ProviderKind,
        api_key: &str,
    ) -> Result<(), SecretStoreError> {
        self.keys.insert(provider, api_key.to_string());
        Ok(())
    }

    fn delete_api_key(&mut self, provider: ProviderKind) -> Result<(), SecretStoreError> {
        self.keys.remove(&provider);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use purifier_core::provider::ProviderKind;

    use super::*;

    #[test]
    fn fake_secret_store_should_round_trip_provider_keys() {
        let mut store = FakeSecretStore::default();

        store.save_api_key(ProviderKind::OpenAI, "sk-test").unwrap();

        assert_eq!(
            store.load_api_key(ProviderKind::OpenAI).unwrap(),
            Some("sk-test".to_string())
        );
    }

    #[test]
    fn fake_secret_store_should_delete_provider_keys() {
        let mut store = FakeSecretStore::default();
        store.save_api_key(ProviderKind::OpenAI, "sk-test").unwrap();

        store.delete_api_key(ProviderKind::OpenAI).unwrap();

        assert_eq!(store.load_api_key(ProviderKind::OpenAI).unwrap(), None);
    }

    #[test]
    fn file_secret_store_should_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = FileSecretStore::new(dir.path());
        let test_key = "sk-test-round-trip-12345";

        store
            .save_api_key(ProviderKind::OpenRouter, test_key)
            .unwrap();

        // Read back with a NEW store instance (simulates app restart)
        let store2 = FileSecretStore::new(dir.path());
        let loaded = store2.load_api_key(ProviderKind::OpenRouter).unwrap();
        assert_eq!(loaded.as_deref(), Some(test_key));
    }

    #[test]
    fn file_secret_store_should_delete() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = FileSecretStore::new(dir.path());

        store
            .save_api_key(ProviderKind::OpenAI, "sk-delete-me")
            .unwrap();
        store.delete_api_key(ProviderKind::OpenAI).unwrap();

        let loaded = store.load_api_key(ProviderKind::OpenAI).unwrap();
        assert_eq!(loaded, None);
    }

    #[test]
    fn file_secret_store_should_handle_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSecretStore::new(dir.path());

        let loaded = store.load_api_key(ProviderKind::OpenRouter).unwrap();
        assert_eq!(loaded, None);
    }

    #[test]
    fn file_secret_store_should_persist_multiple_providers() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = FileSecretStore::new(dir.path());

        store
            .save_api_key(ProviderKind::OpenRouter, "or-key")
            .unwrap();
        store
            .save_api_key(ProviderKind::OpenAI, "oai-key")
            .unwrap();

        // New instance
        let store2 = FileSecretStore::new(dir.path());
        assert_eq!(
            store2.load_api_key(ProviderKind::OpenRouter).unwrap(),
            Some("or-key".to_string())
        );
        assert_eq!(
            store2.load_api_key(ProviderKind::OpenAI).unwrap(),
            Some("oai-key".to_string())
        );
    }

    #[test]
    fn secret_store_error_for_operation_should_match_write_and_delete() {
        let write_error = secret_store_error(
            SecretOperation::Write,
            ProviderKind::Anthropic,
            "write failed".to_string(),
        );
        let delete_error = secret_store_error(
            SecretOperation::Delete,
            ProviderKind::Google,
            "delete failed".to_string(),
        );

        assert!(matches!(
            write_error,
            SecretStoreError::Write {
                provider: ProviderKind::Anthropic,
                ..
            }
        ));
        assert!(matches!(
            delete_error,
            SecretStoreError::Delete {
                provider: ProviderKind::Google,
                ..
            }
        ));
    }
}
