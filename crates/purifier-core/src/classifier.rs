use std::time::SystemTime;

use crossbeam_channel::{Receiver, Sender};

use crate::llm::{LlmClassification, OpenRouterClient, UnknownEntry};
use crate::rules::RulesEngine;
use crate::types::{FileEntry, SafetyLevel};

const LLM_BATCH_SIZE: usize = 50;

pub struct Classifier {
    rules: RulesEngine,
    llm_client: Option<OpenRouterClient>,
}

impl Classifier {
    pub fn new(rules: RulesEngine, llm_client: Option<OpenRouterClient>) -> Self {
        Self { rules, llm_client }
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
        let client = match &self.llm_client {
            Some(c) => OpenRouterClient::new(c.api_key().to_string()),
            None => return,
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
                size: entry.size,
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

        let mut entry = FileEntry::new(
            PathBuf::from("/home/user/random_stuff"),
            500,
            true,
            None,
        );

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
}
