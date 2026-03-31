pub mod classifier;
pub mod llm;
pub mod provider;
pub mod rules;
pub mod scanner;
pub mod types;

pub use provider::{
    LlmClient, LlmError, ProviderKind, ProviderSettings, ProviderSettingsMap,
    ResolvedProviderConfig,
};
pub use types::{Category, FileEntry, SafetyLevel, ScanEvent};

use std::path::Path;

pub fn delete_entry(path: &Path) -> Result<u64, std::io::Error> {
    let metadata = std::fs::metadata(path)?;
    let size = if metadata.is_dir() {
        dir_size(path)
    } else {
        metadata.len()
    };

    if metadata.is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }

    Ok(size)
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.is_dir() {
                total += dir_size(&entry.path());
            } else {
                total += meta.len();
            }
        }
    }
    total
}
