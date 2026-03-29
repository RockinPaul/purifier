mod app;
mod input;
mod ui;

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use clap::Parser;
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use purifier_core::classifier::Classifier;
use purifier_core::llm::{LlmClassification, OpenRouterClient, UnknownEntry};
use purifier_core::rules::RulesEngine;
use purifier_core::scanner;
use purifier_core::types::{FileEntry, SafetyLevel, ScanEvent};

use app::{App, ScanStatus};
use input::InputResult;

/// Max scan events to process per frame to prevent input starvation
const MAX_EVENTS_PER_FRAME: usize = 1000;
const LLM_BATCH_SIZE: usize = 50;

type UnknownBatchSender = crossbeam_channel::Sender<Vec<UnknownEntry>>;
type LlmResultReceiver = crossbeam_channel::Receiver<Vec<LlmClassification>>;

#[derive(Parser)]
#[command(name = "purifier", about = "Disk cleanup with safety intelligence")]
struct Cli {
    /// Path to scan (shows directory picker if omitted)
    path: Option<PathBuf>,

    /// Additional rules file
    #[arg(long)]
    rules: Option<PathBuf>,

    /// Disable LLM classification
    #[arg(long)]
    no_llm: bool,

    /// OpenRouter API key (also reads OPENROUTER_API_KEY env)
    #[arg(long)]
    api_key: Option<String>,
}

fn main() -> io::Result<()> {
    let cli = Cli::parse();

    let api_key = cli
        .api_key
        .or_else(|| std::env::var("OPENROUTER_API_KEY").ok());
    let llm_enabled = !cli.no_llm && api_key.is_some();

    // Load rules
    let mut rule_paths = Vec::new();
    if let Some(extra) = cli.rules {
        rule_paths.push(extra);
    }
    if let Some(path) = find_default_rules() {
        rule_paths.push(path);
    }

    let rules = RulesEngine::new(&rule_paths).unwrap_or_else(|e| {
        eprintln!("Warning: could not load rules: {e}");
        RulesEngine::new(&[]).unwrap()
    });

    let llm_client = if llm_enabled {
        api_key.map(OpenRouterClient::new)
    } else {
        None
    };

    let classifier = Classifier::new(rules, llm_client);
    let initial_scan_path = cli.path.as_deref().map(normalize_scan_path);

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // If path provided via CLI, skip dir picker and start scanning immediately
    let mut app = App::new(initial_scan_path.clone(), llm_enabled);

    let mut scan_rx: Option<crossbeam_channel::Receiver<ScanEvent>> = None;

    if let Some(path) = initial_scan_path {
        app.scan_status = ScanStatus::Scanning;
        scan_rx = Some(scanner::scan(&path));
    }

    // Main loop
    let result = run_loop(&mut terminal, &mut app, &classifier, &mut scan_rx);

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    classifier: &Classifier,
    scan_rx: &mut Option<crossbeam_channel::Receiver<ScanEvent>>,
) -> io::Result<()> {
    let mut path_children: HashMap<PathBuf, Vec<FileEntry>> = HashMap::new();
    let (unknown_tx, llm_result_rx) = start_llm_processing(classifier, app);
    let mut pending_unknowns = Vec::new();

    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;

        // Process scan events — capped per frame to prevent input starvation
        if let Some(rx) = scan_rx.as_ref() {
            let mut processed = 0;
            while processed < MAX_EVENTS_PER_FRAME {
                match rx.try_recv() {
                    Ok(event) => {
                        match event {
                            ScanEvent::Entry {
                                path,
                                size,
                                is_dir,
                                modified,
                            } => {
                                let mut entry =
                                    FileEntry::new(path.clone(), size, is_dir, modified);
                                classifier.classify_entry(&mut entry);
                                queue_unknown_entry(
                                    &mut entry,
                                    &mut pending_unknowns,
                                    unknown_tx.as_ref(),
                                );

                                if let Some(parent) = path.parent() {
                                    path_children
                                        .entry(parent.to_path_buf())
                                        .or_default()
                                        .push(entry);
                                } else {
                                    app.entries.push(entry);
                                }
                            }
                            ScanEvent::Progress {
                                files_scanned,
                                bytes_found,
                                current_dir,
                            } => {
                                app.files_scanned = files_scanned;
                                app.bytes_found = bytes_found;
                                app.current_scan_dir = current_dir;
                            }
                            ScanEvent::ScanComplete {
                                total_size,
                                total_files,
                                skipped,
                            } => {
                                flush_pending_unknowns(&mut pending_unknowns, unknown_tx.as_ref());
                                app.scan_status = ScanStatus::Complete;
                                app.total_size = total_size;
                                app.total_files = total_files;
                                app.skipped = skipped;

                                app.entries = build_tree(
                                    &app.scan_path,
                                    &mut path_children,
                                    &app.expanded_paths,
                                    &app.deleted_paths,
                                );
                                app.rebuild_flat_entries();
                            }
                        }
                        processed += 1;
                    }
                    Err(_) => break, // channel empty or disconnected
                }
            }

            if processed > 0 && app.scan_status == ScanStatus::Scanning {
                refresh_scan_snapshot(app, &path_children);
            }
        }

        if let Some(result_rx) = llm_result_rx.as_ref() {
            while let Ok(results) = result_rx.try_recv() {
                apply_llm_results(&mut path_children, app, results);
            }
        }

        // Handle input — always polled every frame
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match input::handle_key(app, key) {
                    InputResult::StartScan(path) => {
                        // User selected a directory from picker — start scanning
                        let path = normalize_scan_path(&path);
                        path_children.clear();
                        pending_unknowns.clear();
                        app.scan_path = path.clone();
                        *scan_rx = Some(scanner::scan(&path));
                    }
                    InputResult::None => {}
                }
                if app.should_quit {
                    return Ok(());
                }
            }
        }
    }
}

fn normalize_scan_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else if let Ok(canonical) = path.canonicalize() {
        canonical
    } else if let Ok(cwd) = std::env::current_dir() {
        cwd.join(path)
    } else {
        path.to_path_buf()
    }
}

fn start_llm_processing(
    classifier: &Classifier,
    app: &mut App,
) -> (Option<UnknownBatchSender>, Option<LlmResultReceiver>) {
    if !classifier.has_llm() {
        app.llm_online = false;
        return (None, None);
    }

    let (unknown_tx, unknown_rx) = crossbeam_channel::unbounded();
    let (result_tx, result_rx) = crossbeam_channel::unbounded();
    classifier.start_llm_classifier(unknown_rx, result_tx);
    app.llm_online = true;
    (Some(unknown_tx), Some(result_rx))
}

fn queue_unknown_entry(
    entry: &mut FileEntry,
    pending_unknowns: &mut Vec<UnknownEntry>,
    unknown_tx: Option<&crossbeam_channel::Sender<Vec<UnknownEntry>>>,
) {
    if entry.safety != SafetyLevel::Unknown {
        return;
    }

    if let Some(tx) = unknown_tx {
        entry.safety_reason = "Analyzing with LLM...".to_string();
        pending_unknowns.push(UnknownEntry {
            path: entry.path.clone(),
            size: entry.size,
            is_dir: entry.is_dir,
            age_days: entry.modified.and_then(age_in_days),
        });

        if pending_unknowns.len() >= LLM_BATCH_SIZE {
            let batch = pending_unknowns.drain(..LLM_BATCH_SIZE).collect();
            let _ = tx.send(batch);
        }
    }
}

fn flush_pending_unknowns(
    pending_unknowns: &mut Vec<UnknownEntry>,
    unknown_tx: Option<&crossbeam_channel::Sender<Vec<UnknownEntry>>>,
) {
    if pending_unknowns.is_empty() {
        return;
    }

    if let Some(tx) = unknown_tx {
        let batch = std::mem::take(pending_unknowns);
        let _ = tx.send(batch);
    }
}

fn apply_llm_results(
    path_children: &mut HashMap<PathBuf, Vec<FileEntry>>,
    app: &mut App,
    results: Vec<LlmClassification>,
) {
    let mut applied_any = false;

    for result in results {
        let mut applied = false;

        for entries in path_children.values_mut() {
            if update_entry_classification(entries, &result) {
                applied = true;
                break;
            }
        }

        if !applied && update_entry_classification(&mut app.entries, &result) {
            applied = true;
        }

        applied_any |= applied;
    }

    if applied_any && app.scan_status == ScanStatus::Scanning {
        refresh_scan_snapshot(app, path_children);
    } else if applied_any && !app.entries.is_empty() {
        app.rebuild_flat_entries();
    }
}

fn update_entry_classification(entries: &mut [FileEntry], result: &LlmClassification) -> bool {
    for entry in entries {
        if entry.path == result.path {
            entry.category = result.category;
            entry.safety = result.safety;
            entry.safety_reason = result.reason.clone();
            return true;
        }

        if update_entry_classification(&mut entry.children, result) {
            return true;
        }
    }

    false
}

fn age_in_days(modified: SystemTime) -> Option<i64> {
    SystemTime::now()
        .duration_since(modified)
        .ok()
        .map(|duration| (duration.as_secs() / 86_400) as i64)
}

fn refresh_scan_snapshot(app: &mut App, path_children: &HashMap<PathBuf, Vec<FileEntry>>) {
    if app.scan_status != ScanStatus::Scanning {
        return;
    }

    app.entries = build_tree_snapshot(
        &app.scan_path,
        path_children,
        &app.expanded_paths,
        &app.deleted_paths,
    );
    app.rebuild_flat_entries();
}

fn build_tree_snapshot(
    root: &Path,
    path_children: &HashMap<PathBuf, Vec<FileEntry>>,
    expanded_paths: &std::collections::HashSet<PathBuf>,
    deleted_paths: &std::collections::HashSet<PathBuf>,
) -> Vec<FileEntry> {
    let Some(children) = path_children.get(root) else {
        return Vec::new();
    };

    let mut entries = children
        .iter()
        .filter(|entry| !deleted_paths.contains(&entry.path))
        .cloned()
        .collect::<Vec<_>>();

    for entry in &mut entries {
        if entry.is_dir {
            entry.expanded = expanded_paths.contains(&entry.path);
            entry.children =
                build_tree_snapshot(&entry.path, path_children, expanded_paths, deleted_paths);
            entry
                .children
                .sort_by_key(|child| std::cmp::Reverse(child.total_size()));
        }
    }

    entries.sort_by_key(|entry| std::cmp::Reverse(entry.total_size()));
    entries
}

fn build_tree(
    root: &PathBuf,
    path_children: &mut HashMap<PathBuf, Vec<FileEntry>>,
    expanded_paths: &std::collections::HashSet<PathBuf>,
    deleted_paths: &std::collections::HashSet<PathBuf>,
) -> Vec<FileEntry> {
    let mut entries = path_children
        .remove(root)
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| !deleted_paths.contains(&entry.path))
        .collect::<Vec<_>>();

    for entry in &mut entries {
        if entry.is_dir {
            entry.expanded = expanded_paths.contains(&entry.path);
            entry.children = build_tree(&entry.path, path_children, expanded_paths, deleted_paths);
            entry
                .children
                .sort_by_key(|child| std::cmp::Reverse(child.total_size()));
        }
    }

    entries.sort_by_key(|entry| std::cmp::Reverse(entry.total_size()));
    entries
}

fn find_default_rules() -> Option<PathBuf> {
    let cwd_rules = PathBuf::from("rules/default.toml");
    if cwd_rules.exists() {
        return Some(cwd_rules);
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let exe_rules = dir.join("../../rules/default.toml");
            if exe_rules.exists() {
                return Some(exe_rules);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use purifier_core::llm::{LlmClassification, OpenRouterClient};
    use purifier_core::rules::RulesEngine;
    use purifier_core::types::{Category, FileEntry, SafetyLevel};

    use super::{
        apply_llm_results, build_tree, build_tree_snapshot, normalize_scan_path,
        queue_unknown_entry, refresh_scan_snapshot, start_llm_processing,
    };
    use crate::app::{App, ScanStatus};
    use purifier_core::classifier::Classifier;

    #[test]
    fn start_llm_processing_should_mark_app_online_when_classifier_has_llm() {
        let classifier = Classifier::new(
            RulesEngine::new(&[]).expect("rules engine should initialize"),
            Some(OpenRouterClient::new("test-key".to_string())),
        );
        let mut app = App::new(Some(PathBuf::from("/scan")), true);

        let (unknown_tx, result_rx) = start_llm_processing(&classifier, &mut app);

        assert!(unknown_tx.is_some(), "worker sender should exist");
        assert!(result_rx.is_some(), "worker receiver should exist");
        assert!(app.llm_online, "LLM should be marked online");
    }

    #[test]
    fn queue_unknown_entry_should_flush_full_batch_to_worker() {
        let (unknown_tx, unknown_rx) = crossbeam_channel::unbounded();
        let mut pending = (0..49)
            .map(|index| purifier_core::llm::UnknownEntry {
                path: PathBuf::from(format!("/scan/{index}")),
                size: 1,
                is_dir: false,
                age_days: None,
            })
            .collect::<Vec<_>>();
        let mut entry = FileEntry::new(PathBuf::from("/scan/49"), 1, false, None);

        queue_unknown_entry(&mut entry, &mut pending, Some(&unknown_tx));

        let batch = unknown_rx.try_recv().expect("full batch should flush");
        assert_eq!(batch.len(), 50, "batch should contain 50 entries");
        assert!(
            pending.is_empty(),
            "pending batch should be empty after flush"
        );
        assert_eq!(entry.safety_reason, "Analyzing with LLM...");
    }

    #[test]
    fn apply_llm_results_should_update_matching_path_only() {
        let parent = PathBuf::from("/scan");
        let mut path_children = HashMap::from([(
            parent,
            vec![
                FileEntry::new(PathBuf::from("/scan/match"), 10, false, None),
                FileEntry::new(PathBuf::from("/scan/other"), 20, false, None),
            ],
        )]);
        let mut app = App::new(Some(PathBuf::from("/scan")), true);

        apply_llm_results(
            &mut path_children,
            &mut app,
            vec![LlmClassification {
                path: PathBuf::from("/scan/match"),
                category: Category::Cache,
                safety: SafetyLevel::Safe,
                reason: "Recreated automatically".to_string(),
            }],
        );

        let entries = path_children
            .get(&PathBuf::from("/scan"))
            .expect("entries should remain under parent");
        assert_eq!(entries[0].safety, SafetyLevel::Safe);
        assert_eq!(entries[0].category, Category::Cache);
        assert_eq!(entries[0].safety_reason, "Recreated automatically");
        assert_eq!(entries[1].safety, SafetyLevel::Unknown);
    }

    #[test]
    fn refresh_scan_snapshot_should_make_entries_visible_before_scan_complete() {
        let path_children = HashMap::from([(
            PathBuf::from("/scan"),
            vec![FileEntry::new(
                PathBuf::from("/scan/cache"),
                10,
                false,
                None,
            )],
        )]);
        let mut app = App::new(Some(PathBuf::from("/scan")), false);
        app.scan_status = ScanStatus::Scanning;

        refresh_scan_snapshot(&mut app, &path_children);

        assert_eq!(app.flat_entries.len(), 1);
        assert_eq!(app.flat_entries[0].path, PathBuf::from("/scan/cache"));
    }

    #[test]
    fn apply_llm_results_should_refresh_visible_rows_while_scanning() {
        let mut path_children = HashMap::from([(
            PathBuf::from("/scan"),
            vec![FileEntry::new(
                PathBuf::from("/scan/cache"),
                10,
                false,
                None,
            )],
        )]);
        let mut app = App::new(Some(PathBuf::from("/scan")), true);
        app.scan_status = ScanStatus::Scanning;
        refresh_scan_snapshot(&mut app, &path_children);

        apply_llm_results(
            &mut path_children,
            &mut app,
            vec![LlmClassification {
                path: PathBuf::from("/scan/cache"),
                category: Category::Cache,
                safety: SafetyLevel::Safe,
                reason: "Recreated automatically".to_string(),
            }],
        );

        assert_eq!(app.flat_entries[0].safety, SafetyLevel::Safe);
        assert_eq!(app.flat_entries[0].safety_reason, "Recreated automatically");
    }

    #[test]
    fn refresh_scan_snapshot_should_preserve_expanded_paths() {
        let path_children = HashMap::from([
            (
                PathBuf::from("/scan"),
                vec![FileEntry::new(PathBuf::from("/scan/dir"), 1, true, None)],
            ),
            (
                PathBuf::from("/scan/dir"),
                vec![FileEntry::new(
                    PathBuf::from("/scan/dir/file"),
                    5,
                    false,
                    None,
                )],
            ),
        ]);
        let mut app = App::new(Some(PathBuf::from("/scan")), false);
        app.scan_status = ScanStatus::Scanning;
        app.expanded_paths.insert(PathBuf::from("/scan/dir"));

        refresh_scan_snapshot(&mut app, &path_children);

        assert_eq!(
            app.flat_entries.len(),
            2,
            "expanded child should stay visible"
        );
        assert!(
            app.flat_entries[0].expanded,
            "directory should stay expanded"
        );
    }

    #[test]
    fn refresh_scan_snapshot_should_hide_deleted_paths() {
        let path_children = HashMap::from([(
            PathBuf::from("/scan"),
            vec![FileEntry::new(
                PathBuf::from("/scan/cache"),
                10,
                false,
                None,
            )],
        )]);
        let mut app = App::new(Some(PathBuf::from("/scan")), false);
        app.scan_status = ScanStatus::Scanning;
        app.deleted_paths.insert(PathBuf::from("/scan/cache"));

        refresh_scan_snapshot(&mut app, &path_children);

        assert!(
            app.flat_entries.is_empty(),
            "deleted item should stay hidden"
        );
    }

    #[test]
    fn build_tree_snapshot_should_match_final_build_tree() {
        let mut path_children = HashMap::from([
            (
                PathBuf::from("/scan"),
                vec![FileEntry::new(PathBuf::from("/scan/dir"), 1, true, None)],
            ),
            (
                PathBuf::from("/scan/dir"),
                vec![FileEntry::new(
                    PathBuf::from("/scan/dir/file"),
                    5,
                    false,
                    None,
                )],
            ),
        ]);
        let expanded_paths = std::collections::HashSet::new();
        let deleted_paths = std::collections::HashSet::new();
        let snapshot = build_tree_snapshot(
            &PathBuf::from("/scan"),
            &path_children,
            &expanded_paths,
            &deleted_paths,
        );
        let final_tree = build_tree(
            &PathBuf::from("/scan"),
            &mut path_children,
            &expanded_paths,
            &deleted_paths,
        );

        assert_eq!(collect_paths(&snapshot), collect_paths(&final_tree));
    }

    #[test]
    fn normalize_scan_path_should_convert_relative_paths_to_absolute() {
        let cwd = std::env::current_dir().expect("cwd should exist");
        let relative = PathBuf::from(".");

        let normalized = normalize_scan_path(&relative);

        assert!(normalized.is_absolute());
        assert_eq!(normalized, cwd);
    }

    #[test]
    fn normalize_scan_path_should_leave_absolute_paths_unchanged() {
        let absolute = PathBuf::from("/tmp/purifier-absolute-test");

        assert_eq!(normalize_scan_path(&absolute), absolute);
    }

    fn collect_paths(entries: &[FileEntry]) -> Vec<PathBuf> {
        let mut paths = Vec::new();

        for entry in entries {
            paths.push(entry.path.clone());
            paths.extend(collect_paths(&entry.children));
        }

        paths
    }
}
