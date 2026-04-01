use std::time::SystemTime;

use crossbeam_channel::{Receiver, Sender};

use crate::llm::{LlmClassification, UnknownEntry};
use crate::provider::LlmClient;
use crate::rules::RulesEngine;
use crate::size::SizeMode;
use crate::types::{FileEntry, SafetyLevel};

const LLM_BATCH_SIZE: usize = 50;

pub struct Classifier {
    rules: RulesEngine,
    llm_client: Option<LlmClient>,
}

impl Classifier {
    pub fn new(rules: RulesEngine, llm_client: Option<LlmClient>) -> Self {
        Self { rules, llm_client }
    }

    pub fn set_llm_client(&mut self, llm_client: Option<LlmClient>) {
        self.llm_client = llm_client;
    }

    pub fn rules(&self) -> &RulesEngine {
        &self.rules
    }

    fn worker_llm_client(&self) -> Option<LlmClient> {
        self.llm_client.clone()
    }

    pub fn classify_entry(&self, entry: &mut FileEntry) {
        if let Some(rule_match) = self.rules.classify(&entry.path) {
            entry.category = rule_match.category;
            entry.safety = rule_match.safety;
            entry.safety_reason = rule_match.reason;
        }
        // If no rule matched, entry stays Unknown — will be queued for LLM
    }

    pub fn start_llm_classifier(
        &self,
        unknown_rx: Receiver<Vec<UnknownEntry>>,
        result_tx: Sender<Vec<LlmClassification>>,
    ) {
        let Some(client) = self.worker_llm_client() else {
            return;
        };

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to create tokio runtime");

            for batch in unknown_rx {
                let results = rt.block_on(client.classify_batch(batch));
                if result_tx.send(results).is_err() {
                    break;
                }
            }
        });
    }

    pub fn has_llm(&self) -> bool {
        self.llm_client.is_some()
    }
}

pub fn collect_unknowns(entries: &[FileEntry]) -> Vec<UnknownEntry> {
    let mut unknowns = Vec::new();
    collect_unknowns_recursive(entries, &mut unknowns);
    unknowns
}

fn collect_unknowns_recursive(entries: &[FileEntry], out: &mut Vec<UnknownEntry>) {
    for entry in entries {
        if entry.safety == SafetyLevel::Unknown {
            let age_days = entry.modified.and_then(|m| {
                SystemTime::now()
                    .duration_since(m)
                    .ok()
                    .map(|d| (d.as_secs() / 86400) as i64)
            });

            out.push(UnknownEntry {
                path: entry.path.clone(),
                size: entry.total_size(SizeMode::Logical),
                is_dir: entry.is_dir,
                age_days,
            });
        }
        collect_unknowns_recursive(&entry.children, out);
    }
}

pub fn batch_unknowns(unknowns: Vec<UnknownEntry>) -> Vec<Vec<UnknownEntry>> {
    unknowns
        .chunks(LLM_BATCH_SIZE)
        .map(|chunk| chunk.to_vec())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{LlmClient, ProviderKind, ResolvedProviderConfig};
    use crate::rules::RulesEngine;
    use crate::types::Category;
    use std::path::PathBuf;

    fn test_engine() -> RulesEngine {
        let dir = tempfile::tempdir().unwrap();
        let rules_path = dir.path().join("rules.toml");
        std::fs::write(
            &rules_path,
            r#"
[[rules]]
pattern = "**/node_modules"
category = "BuildArtifact"
safety = "Safe"
reason = "npm dependencies"
"#,
        )
        .unwrap();
        // Leak the tempdir so the file persists for the test
        let path = rules_path.clone();
        std::mem::forget(dir);
        RulesEngine::new(&[path]).unwrap()
    }

    #[test]
    fn test_classify_known_entry() {
        let engine = test_engine();
        let classifier = Classifier::new(engine, None);

        let mut entry = FileEntry::new(
            PathBuf::from("/home/user/project/node_modules"),
            1000,
            true,
            None,
        );

        classifier.classify_entry(&mut entry);
        assert_eq!(entry.category, Category::BuildArtifact);
        assert_eq!(entry.safety, SafetyLevel::Safe);
    }

    #[test]
    fn test_classify_unknown_entry() {
        let engine = test_engine();
        let classifier = Classifier::new(engine, None);

        let mut entry = FileEntry::new(PathBuf::from("/home/user/random_stuff"), 500, true, None);

        classifier.classify_entry(&mut entry);
        assert_eq!(entry.category, Category::Unknown);
        assert_eq!(entry.safety, SafetyLevel::Unknown);
    }

    #[test]
    fn test_batch_unknowns() {
        let entries: Vec<UnknownEntry> = (0..120)
            .map(|i| UnknownEntry {
                path: PathBuf::from(format!("/path/{i}")),
                size: 100,
                is_dir: false,
                age_days: Some(30),
            })
            .collect();

        let batches = batch_unknowns(entries);
        assert_eq!(batches.len(), 3); // 50 + 50 + 20
        assert_eq!(batches[0].len(), 50);
        assert_eq!(batches[1].len(), 50);
        assert_eq!(batches[2].len(), 20);
    }

    #[test]
    fn start_llm_classifier_should_clone_full_client_configuration() {
        let client = LlmClient::OpenRouter(crate::llm::OpenRouterClient::new(
            ResolvedProviderConfig::new(
                ProviderKind::OpenRouter,
                Some("test-key".to_string()),
                "google/gemini-2.0-flash-001".to_string(),
                "https://openrouter.ai/api/v1".to_string(),
            ),
        ));
        let classifier =
            Classifier::new(crate::rules::RulesEngine::new(&[]).unwrap(), Some(client));

        let worker_client = classifier
            .worker_llm_client()
            .expect("worker should clone the configured LLM client");

        match worker_client {
            LlmClient::OpenRouter(client) => {
                let config = client.config();
                assert_eq!(config.kind, ProviderKind::OpenRouter);
                assert_eq!(config.api_key.as_deref(), Some("test-key"));
                assert_eq!(config.model, "google/gemini-2.0-flash-001");
                assert_eq!(config.base_url, "https://openrouter.ai/api/v1");
            }
            _ => panic!("expected openrouter worker client"),
        }
    }

    #[test]
    fn set_llm_client_should_replace_runtime_client() {
        let mut classifier = Classifier::new(crate::rules::RulesEngine::new(&[]).unwrap(), None);

        classifier.set_llm_client(Some(LlmClient::OpenRouter(
            crate::llm::OpenRouterClient::new(ResolvedProviderConfig::new(
                ProviderKind::OpenRouter,
                Some("test-key".to_string()),
                "google/gemini-2.0-flash-001".to_string(),
                "https://openrouter.ai/api/v1".to_string(),
            )),
        )));

        assert!(classifier.has_llm());
    }
}
