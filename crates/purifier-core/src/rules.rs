use std::borrow::Cow;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::types::{Category, SafetyLevel};

#[derive(Debug, Deserialize)]
struct RuleFile {
    rules: Vec<RuleDef>,
}

#[derive(Debug, Deserialize)]
struct RuleDef {
    pattern: String,
    category: Category,
    safety: SafetyLevel,
    reason: String,
}

#[derive(Debug)]
struct CompiledRule {
    pattern: glob::Pattern,
    category: Category,
    safety: SafetyLevel,
    reason: String,
}

#[derive(Debug)]
pub struct RulesEngine {
    rules: Vec<CompiledRule>,
}

#[derive(Debug, Clone)]
pub struct RuleMatch {
    pub category: Category,
    pub safety: SafetyLevel,
    pub reason: String,
}

impl RulesEngine {
    pub fn new(rule_paths: &[PathBuf]) -> Result<Self, Box<dyn std::error::Error>> {
        let mut rules = Vec::new();

        for path in rule_paths {
            let content = std::fs::read_to_string(path)?;
            let file: RuleFile = toml::from_str(&content)?;

            for def in file.rules {
                let expanded = expand_tilde(&def.pattern);
                let pattern = glob::Pattern::new(&expanded)?;
                rules.push(CompiledRule {
                    pattern,
                    category: def.category,
                    safety: def.safety,
                    reason: def.reason,
                });
            }
        }

        Ok(Self { rules })
    }

    pub fn classify(&self, path: &Path) -> Option<RuleMatch> {
        let normalized_path = normalize_for_matching(path);
        let path_str = normalized_path.to_string_lossy();

        for rule in &self.rules {
            if rule.pattern.matches(&path_str)
                || matches_any_ancestor(normalized_path.as_ref(), &rule.pattern)
            {
                return Some(RuleMatch {
                    category: rule.category,
                    safety: rule.safety,
                    reason: rule.reason.clone(),
                });
            }
        }

        None
    }
}

fn normalize_for_matching(path: &Path) -> Cow<'_, Path> {
    if path.is_absolute() {
        Cow::Borrowed(path)
    } else if let Ok(canonical) = path.canonicalize() {
        Cow::Owned(canonical)
    } else if let Ok(cwd) = std::env::current_dir() {
        Cow::Owned(cwd.join(path))
    } else {
        Cow::Borrowed(path)
    }
}

fn matches_any_ancestor(path: &Path, pattern: &glob::Pattern) -> bool {
    let mut current = path;
    while let Some(parent) = current.parent() {
        if pattern.matches_path(current) {
            return true;
        }
        if parent == current {
            break;
        }
        current = parent;
    }
    false
}

fn expand_tilde(pattern: &str) -> String {
    if pattern.starts_with("~/") || pattern == "~" {
        if let Some(home) = dirs::home_dir() {
            return pattern.replacen('~', &home.to_string_lossy(), 1);
        }
    }
    pattern.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_test_rules(dir: &Path) -> PathBuf {
        let path = dir.join("test-rules.toml");
        fs::write(
            &path,
            r#"
[[rules]]
pattern = "**/node_modules"
category = "BuildArtifact"
safety = "Safe"
reason = "npm dependencies"

[[rules]]
pattern = "**/target/debug"
category = "BuildArtifact"
safety = "Safe"
reason = "Rust debug output"

[[rules]]
pattern = "**/.git"
category = "System"
safety = "Unsafe"
reason = "Git repository"
"#,
        )
        .unwrap();
        path
    }

    #[test]
    fn test_rule_matching() {
        let dir = tempfile::tempdir().unwrap();
        let rules_path = write_test_rules(dir.path());
        let engine = RulesEngine::new(&[rules_path]).unwrap();

        let m = engine
            .classify(Path::new("/home/user/project/node_modules"))
            .expect("should match node_modules");
        assert_eq!(m.category, Category::BuildArtifact);
        assert_eq!(m.safety, SafetyLevel::Safe);

        let m = engine
            .classify(Path::new("/home/user/project/target/debug"))
            .expect("should match target/debug");
        assert_eq!(m.category, Category::BuildArtifact);

        let m = engine
            .classify(Path::new("/home/user/project/.git"))
            .expect("should match .git");
        assert_eq!(m.category, Category::System);
        assert_eq!(m.safety, SafetyLevel::Unsafe);
    }

    #[test]
    fn test_unknown_path() {
        let dir = tempfile::tempdir().unwrap();
        let rules_path = write_test_rules(dir.path());
        let engine = RulesEngine::new(&[rules_path]).unwrap();

        let result = engine.classify(Path::new("/home/user/random_file.txt"));
        assert!(result.is_none());
    }

    #[test]
    fn test_multiple_rule_files() {
        let dir = tempfile::tempdir().unwrap();
        let rules1 = write_test_rules(dir.path());

        let rules2_path = dir.path().join("extra-rules.toml");
        fs::write(
            &rules2_path,
            r#"
[[rules]]
pattern = "**/*.log"
category = "Cache"
safety = "Safe"
reason = "Log file"
"#,
        )
        .unwrap();

        // Custom rules first — they take priority
        let engine = RulesEngine::new(&[rules2_path, rules1]).unwrap();

        let m = engine
            .classify(Path::new("/var/log/app.log"))
            .expect("should match .log");
        assert_eq!(m.category, Category::Cache);
        assert_eq!(m.safety, SafetyLevel::Safe);
    }

    #[test]
    fn test_relative_paths_match_absolute_rules_after_normalization() {
        let cwd = std::env::current_dir().unwrap();
        let dir = tempfile::tempdir_in(&cwd).unwrap();
        let cache_dir = dir.path().join("cache");
        fs::create_dir(&cache_dir).unwrap();

        let rules_path = dir.path().join("absolute-rules.toml");
        fs::write(
            &rules_path,
            format!(
                r#"
[[rules]]
pattern = "{}"
category = "Cache"
safety = "Safe"
reason = "Absolute cache path"
"#,
                cache_dir.display()
            ),
        )
        .unwrap();

        let relative_cache = cache_dir.strip_prefix(&cwd).unwrap();
        let engine = RulesEngine::new(&[rules_path]).unwrap();

        let m = engine
            .classify(relative_cache)
            .expect("relative path should match absolute rule");
        assert_eq!(m.category, Category::Cache);
        assert_eq!(m.safety, SafetyLevel::Safe);
    }

    #[test]
    fn test_first_match_wins_after_relative_path_normalization() {
        let cwd = std::env::current_dir().unwrap();
        let dir = tempfile::tempdir_in(&cwd).unwrap();
        let cache_dir = dir.path().join("cache");
        fs::create_dir(&cache_dir).unwrap();

        let rules_path = dir.path().join("ordered-rules.toml");
        fs::write(
            &rules_path,
            format!(
                r#"
[[rules]]
pattern = "{}"
category = "AppData"
safety = "Unsafe"
reason = "Specific absolute path"

[[rules]]
pattern = "**/cache"
category = "Cache"
safety = "Safe"
reason = "Generic cache path"
"#,
                cache_dir.display()
            ),
        )
        .unwrap();

        let relative_cache = cache_dir.strip_prefix(&cwd).unwrap();
        let engine = RulesEngine::new(&[rules_path]).unwrap();

        let m = engine
            .classify(relative_cache)
            .expect("relative path should still respect first match");
        assert_eq!(m.category, Category::AppData);
        assert_eq!(m.safety, SafetyLevel::Unsafe);
    }
}
