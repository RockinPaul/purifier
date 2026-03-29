mod app;
mod input;
mod ui;

use std::io;
use std::path::PathBuf;
use std::time::Duration;
use std::collections::HashMap;

use clap::Parser;
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use purifier_core::classifier::Classifier;
use purifier_core::llm::OpenRouterClient;
use purifier_core::rules::RulesEngine;
use purifier_core::scanner;
use purifier_core::types::{FileEntry, ScanEvent};

use app::{App, ScanStatus};
use input::InputResult;

/// Max scan events to process per frame to prevent input starvation
const MAX_EVENTS_PER_FRAME: usize = 1000;

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

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // If path provided via CLI, skip dir picker and start scanning immediately
    let mut app = App::new(cli.path.clone(), llm_enabled);

    let mut scan_rx: Option<crossbeam_channel::Receiver<ScanEvent>> = None;

    if let Some(path) = cli.path {
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
                                app.scan_status = ScanStatus::Complete;
                                app.total_size = total_size;
                                app.total_files = total_files;
                                app.skipped = skipped;

                                app.entries = build_tree(&app.scan_path, &mut path_children);
                                app.rebuild_flat_entries();
                            }
                        }
                        processed += 1;
                    }
                    Err(_) => break, // channel empty or disconnected
                }
            }
        }

        // Handle input — always polled every frame
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match input::handle_key(app, key) {
                    InputResult::StartScan(path) => {
                        // User selected a directory from picker — start scanning
                        path_children.clear();
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

fn build_tree(
    root: &PathBuf,
    path_children: &mut HashMap<PathBuf, Vec<FileEntry>>,
) -> Vec<FileEntry> {
    let mut entries = path_children.remove(root).unwrap_or_default();

    for entry in &mut entries {
        if entry.is_dir {
            entry.children = build_tree(&entry.path, path_children);
            entry.children
                .sort_by(|a, b| b.total_size().cmp(&a.total_size()));
        }
    }

    entries.sort_by(|a, b| b.total_size().cmp(&a.total_size()));
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
