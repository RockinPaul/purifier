#[cfg(test)]
use std::collections::HashMap;

use keyring::Entry;
use purifier_core::provider::ProviderKind;
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
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

pub struct KeychainSecretStore;

impl KeychainSecretStore {
    fn entry(
        provider: ProviderKind,
        operation: SecretOperation,
    ) -> Result<Entry, SecretStoreError> {
        Entry::new("io.github.rockinpaul.purifier", provider.keychain_account())
            .map_err(|error| secret_store_error(operation, provider, error.to_string()))
    }
}

impl SecretStore for KeychainSecretStore {
    fn load_api_key(&self, provider: ProviderKind) -> Result<Option<String>, SecretStoreError> {
        match Self::entry(provider, SecretOperation::Read)?.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(secret_store_error(
                SecretOperation::Read,
                provider,
                error.to_string(),
            )),
        }
    }

    fn save_api_key(
        &mut self,
        provider: ProviderKind,
        api_key: &str,
    ) -> Result<(), SecretStoreError> {
        Self::entry(provider, SecretOperation::Write)?
            .set_password(api_key)
            .map_err(|error| {
                secret_store_error(SecretOperation::Write, provider, error.to_string())
            })
    }

    fn delete_api_key(&mut self, provider: ProviderKind) -> Result<(), SecretStoreError> {
        match Self::entry(provider, SecretOperation::Delete)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(secret_store_error(
                SecretOperation::Delete,
                provider,
                error.to_string(),
            )),
        }
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
