mod app;
mod columns;
mod config;
mod input;
mod marks;
mod secrets;
mod ui;

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use clap::Parser;
use crossterm::event::{self, Event};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use purifier_core::classifier::{batch_unknowns, collect_unknowns, Classifier};
use purifier_core::llm::{LlmClassification, OpenAiClient, OpenRouterClient, UnknownEntry};
use purifier_core::provider::{
    LlmClient, LlmError, LlmRequestErrorKind, ProviderKind, ResolvedProviderConfig,
};
use purifier_core::rules::RulesEngine;
use purifier_core::scanner;
use purifier_core::size::SizeMode;
use purifier_core::types::{FileEntry, SafetyLevel, ScanEvent};

use app::SettingsDraft;
use app::{App, LlmStatus, ScanStatus};
use input::InputResult;
use secrets::{KeychainSecretStore, SecretStore};

/// Max scan events to process per frame to prevent input starvation
#[cfg_attr(not(test), allow(dead_code))]
const MAX_EVENTS_PER_FRAME: usize = 1000;
#[cfg_attr(not(test), allow(dead_code))]
const LLM_BATCH_SIZE: usize = 50;

type UnknownBatchSender = crossbeam_channel::Sender<Vec<UnknownEntry>>;
type LlmResultReceiver = crossbeam_channel::Receiver<Vec<LlmClassification>>;
type RuntimeConnectionReceiver = crossbeam_channel::Receiver<RuntimeConnectionEvent>;

struct ActiveScan {
    updates: crossbeam_channel::Receiver<ScanProcessingUpdate>,
    cancel: crossbeam_channel::Sender<()>,
}

#[derive(Debug)]
struct ScanProgressSnapshot {
    entries_scanned: u64,
    logical_bytes_found: u64,
    physical_bytes_found: u64,
    current_path: String,
}

#[derive(Debug)]
struct CompletedScanResult {
    entries: Vec<FileEntry>,
    total_entries: u64,
    total_logical_bytes: u64,
    total_physical_bytes: u64,
    skipped: u64,
}

#[derive(Debug)]
enum ScanProcessingUpdate {
    Progress(ScanProgressSnapshot),
    UnknownBatch(Vec<UnknownEntry>),
    Complete(CompletedScanResult),
}

enum RuntimeConnectionEvent {
    Validated {
        generation: u64,
        provider: ProviderKind,
        client: LlmClient,
    },
    Failed {
        generation: u64,
        provider: ProviderKind,
        detail: RuntimeConnectionFailure,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RuntimeConnectionFailure {
    MissingApiKey,
    Timeout,
    Http { status: u16, body: Option<String> },
    Network(String),
    InvalidResponse(String),
}

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

    /// LLM provider
    #[arg(long, value_parser = parse_provider_kind)]
    provider: Option<ProviderKind>,
}

#[derive(Debug, Clone, Default)]
struct EnvOverrides {
    openrouter_api_key: Option<String>,
    openai_api_key: Option<String>,
    anthropic_api_key: Option<String>,
    gemini_api_key: Option<String>,
    google_api_key: Option<String>,
}

#[derive(Debug, Clone)]
struct RuntimeConfig {
    scan_path: Option<PathBuf>,
    provider: Option<ResolvedProviderConfig>,
    show_onboarding: bool,
    llm_enabled: bool,
}

#[derive(Clone, Copy)]
struct SessionContext<'a> {
    cli: &'a Cli,
    env: &'a EnvOverrides,
}

fn parse_provider_kind(value: &str) -> Result<ProviderKind, String> {
    match value.to_ascii_lowercase().as_str() {
        "openrouter" => Ok(ProviderKind::OpenRouter),
        "openai" => Ok(ProviderKind::OpenAI),
        "anthropic" => Ok(ProviderKind::Anthropic),
        "google" | "gemini" => Ok(ProviderKind::Google),
        // TODO(#ollama-support): re-enable CLI selection when runtime support returns.
        "ollama" => Err("ollama is temporarily disabled".to_string()),
        _ => Err(format!("unsupported provider: {value}")),
    }
}

fn main() -> io::Result<()> {
    let cli = Cli::parse();
    let config_path = config::default_config_path();
    let saved_config = load_app_config(&config_path);
    let env = EnvOverrides::from_process();
    let mut secret_store = KeychainSecretStore;
    let (runtime_config, startup_warning) =
        load_runtime_config(&cli, saved_config.clone(), &env, &secret_store);
    if let Some(warning) = &startup_warning {
        eprintln!("{warning}");
    }

    // Load rules
    let mut rule_paths = Vec::new();
    if let Some(extra) = cli.rules.clone() {
        rule_paths.push(extra);
    }
    if let Some(path) = find_default_rules() {
        rule_paths.push(path);
    }

    let rules = RulesEngine::new(&rule_paths).unwrap_or_else(|e| {
        eprintln!("Warning: could not load rules: {e}");
        RulesEngine::new(&[]).unwrap()
    });

    let mut classifier = Classifier::new(rules, None);
    let initial_scan_path = runtime_config
        .scan_path
        .as_deref()
        .map(normalize_scan_path)
        .and_then(validate_scan_path);
    let initial_scan_profile = resolve_initial_scan_profile(cli.path.as_deref(), &saved_config);

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // If path provided via CLI, skip dir picker and start scanning immediately
    let mut app = App::new(
        initial_scan_path.clone(),
        runtime_config.llm_enabled,
        saved_config,
    );
    apply_runtime_config(&mut app, &runtime_config);
    let mut runtime_connection_rx =
        start_runtime_connection_check(&app, &classifier, &runtime_config);
    apply_startup_messages(&mut app, startup_warning, &cli, &env, &runtime_config);
    if runtime_config.show_onboarding {
        app.screen = app::AppScreen::Onboarding;
        app.open_onboarding();
    }

    let mut scan_rx: Option<ActiveScan> = None;

    if let Some(path) = initial_scan_path {
        app.scan_status = ScanStatus::Scanning;
        app.applied_scan_profile_name = initial_scan_profile
            .as_ref()
            .map(|profile| profile.name.clone());
        scan_rx = Some(start_scan_processing(
            path.clone(),
            scanner::scan_with_profile(&path, initial_scan_profile),
            classifier_rules(&classifier),
            should_mark_unknowns_pending(&app),
        ));
    }

    // Main loop
    let result = run_loop(
        &mut terminal,
        &mut app,
        &mut classifier,
        &mut scan_rx,
        &mut runtime_connection_rx,
        &config_path,
        &mut secret_store,
        SessionContext {
            cli: &cli,
            env: &env,
        },
    );

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

#[expect(
    clippy::too_many_arguments,
    reason = "Main loop coordinates terminal, scan, runtime validation, and session state"
)]
fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    classifier: &mut Classifier,
    scan_rx: &mut Option<ActiveScan>,
    runtime_connection_rx: &mut Option<RuntimeConnectionReceiver>,
    config_path: &Path,
    secrets: &mut impl SecretStore,
    session: SessionContext<'_>,
) -> io::Result<()> {
    let (mut unknown_tx, mut llm_result_rx) = start_llm_processing(classifier, app);
    let mut buffered_unknown_batches: Vec<Vec<UnknownEntry>> = Vec::new();
    let mut buffered_llm_results: Vec<LlmClassification> = Vec::new();

    loop {
        // 1. Drain ALL pending input events before draw — guarantees quit always works.
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Key(key) => {
                    match input::handle_key(app, key) {
                        InputResult::StartScan(path) => {
                            let path = normalize_scan_path(&path);
                            app.scan_path = path.clone();
                            let scan_profile = app.preferences.active_scan_profile().cloned();
                            app.applied_scan_profile_name =
                                scan_profile.as_ref().map(|profile| profile.name.clone());
                            if let Some(active_scan) = scan_rx.take() {
                                let _ = active_scan.cancel.send(());
                            }
                            *scan_rx = Some(start_scan_processing(
                                path.clone(),
                                scanner::scan_with_profile(&path, scan_profile),
                                classifier_rules(classifier),
                                should_mark_unknowns_pending(app),
                            ));
                            restart_llm_processing_for_new_scan(
                                classifier,
                                app,
                                &mut unknown_tx,
                                &mut llm_result_rx,
                                &mut buffered_unknown_batches,
                                &mut buffered_llm_results,
                            );
                        }
                        InputResult::SaveSettings(draft) => {
                            if apply_settings_save(
                                app,
                                config_path,
                                draft,
                                secrets,
                                classifier,
                                runtime_connection_rx,
                                session.cli,
                                session.env,
                            )
                            .is_ok()
                            {
                                (unknown_tx, llm_result_rx) = start_llm_processing(classifier, app);
                                if unknown_tx.is_some() {
                                    flush_unknown_batches(
                                        &mut buffered_unknown_batches,
                                        unknown_tx.as_ref(),
                                    );
                                    requeue_unknown_entries_for_visible_tree(
                                        app,
                                        unknown_tx.as_ref(),
                                    );
                                } else {
                                    reset_pending_llm_labels_in_state(app);
                                }
                            }
                        }
                        InputResult::SkipOnboarding => {
                            let _ = apply_onboarding_skip(
                                app,
                                config_path,
                                secrets,
                                classifier,
                                runtime_connection_rx,
                                session.cli,
                                session.env,
                            );
                        }
                        InputResult::None => {}
                    }
                    if app.should_quit {
                        if let Some(active_scan) = scan_rx.take() {
                            let _ = active_scan.cancel.send(());
                        }
                        return Ok(());
                    }
                }
                Event::Mouse(mouse) => {
                    input::handle_mouse(app, mouse);
                }
                _ => {}
            }
        }

        // 2. Process scan updates — capped per frame.
        let mut scan_completed = false;
        if let Some(rx) = scan_rx.as_ref() {
            let mut processed = 0;
            while processed < MAX_EVENTS_PER_FRAME {
                match rx.updates.try_recv() {
                    Ok(event) => {
                        match event {
                            ScanProcessingUpdate::UnknownBatch(batch) => {
                                if let Some(tx) = unknown_tx.as_ref() {
                                    let _ = tx.send(batch);
                                } else {
                                    buffered_unknown_batches.push(batch);
                                }
                            }
                            other => {
                                if apply_scan_update(app, other) {
                                    if !buffered_llm_results.is_empty() {
                                        apply_llm_results(
                                            app,
                                            std::mem::take(&mut buffered_llm_results),
                                        );
                                    }
                                    if unknown_tx.is_none() && !should_mark_unknowns_pending(app) {
                                        buffered_unknown_batches.clear();
                                        reset_pending_llm_labels_in_state(app);
                                    }
                                    scan_completed = true;
                                }
                            }
                        }
                        processed += 1;
                    }
                    Err(_) => break, // channel empty or disconnected
                }
            }
        }

        if scan_completed {
            *scan_rx = None;
        }

        if let Some(result_rx) = llm_result_rx.as_ref() {
            while let Ok(results) = result_rx.try_recv() {
                if app.scan_status == ScanStatus::Scanning && app.entries.is_empty() {
                    buffered_llm_results.extend(results);
                } else {
                    apply_llm_results(app, results);
                }
            }
        }

        let connection_event = runtime_connection_rx
            .as_ref()
            .and_then(|connection_rx| connection_rx.try_recv().ok());
        if let Some(event) = connection_event {
            apply_runtime_connection_event(app, classifier, event);
            *runtime_connection_rx = None;
            (unknown_tx, llm_result_rx) = start_llm_processing(classifier, app);
            if unknown_tx.is_some() {
                flush_unknown_batches(&mut buffered_unknown_batches, unknown_tx.as_ref());
                requeue_unknown_entries_for_visible_tree(app, unknown_tx.as_ref());
            } else {
                buffered_unknown_batches.clear();
                reset_pending_llm_labels_in_state(app);
            }
        }

        // 3. Precompute frame cache (sorted indices + preview analytics)
        app.refresh_frame_cache();

        // 4. Draw frame
        terminal.draw(|frame| ui::draw(frame, app))?;

        // 4. Wait briefly for next event (16ms ≈ 60fps)
        let _ = event::poll(Duration::from_millis(16));
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Settings save needs runtime refresh context and mutable app state"
)]
fn apply_settings_save(
    app: &mut App,
    config_path: &Path,
    draft: SettingsDraft,
    secrets: &mut impl SecretStore,
    classifier: &mut Classifier,
    runtime_connection_rx: &mut Option<RuntimeConnectionReceiver>,
    cli: &Cli,
    env: &EnvOverrides,
) -> Result<(), Box<dyn std::error::Error>> {
    match persist_settings(config_path, &mut app.preferences, draft.clone(), secrets) {
        Ok(()) => {
            app.sync_display_size_state();
            let runtime_override_active =
                refresh_runtime_state(app, classifier, runtime_connection_rx, secrets, cli, env);

            if draft_needs_live_validation(&draft)
                && !runtime_override_active
                && runtime_connection_rx.is_some()
            {
                app.settings_modal_is_saving = true;
                app.settings_modal_error = None;
                app.pending_settings_validation_generation = Some(app.llm_connection_generation);
            } else {
                app.close_preview_modal();
            }

            Ok(())
        }
        Err(error) => {
            let message = format!("Could not save settings: {error}");
            app.settings_modal_error = Some(message.clone());
            app.last_error = Some(message);
            Err(error)
        }
    }
}

fn draft_needs_live_validation(draft: &SettingsDraft) -> bool {
    draft.llm_enabled
        && matches!(
            draft.provider,
            ProviderKind::OpenRouter | ProviderKind::OpenAI
        )
}

fn apply_onboarding_skip(
    app: &mut App,
    config_path: &Path,
    secrets: &mut impl SecretStore,
    classifier: &mut Classifier,
    runtime_connection_rx: &mut Option<RuntimeConnectionReceiver>,
    cli: &Cli,
    env: &EnvOverrides,
) -> Result<(), Box<dyn std::error::Error>> {
    app.preferences.onboarding.first_launch_prompt_dismissed = true;
    app.preferences.save(config_path)?;
    refresh_runtime_state(app, classifier, runtime_connection_rx, secrets, cli, env);
    Ok(())
}

fn refresh_runtime_state(
    app: &mut App,
    classifier: &mut Classifier,
    runtime_connection_rx: &mut Option<RuntimeConnectionReceiver>,
    secrets: &mut impl SecretStore,
    cli: &Cli,
    env: &EnvOverrides,
) -> bool {
    let (runtime_config, warning) = load_runtime_config(cli, app.preferences.clone(), env, secrets);
    let runtime_override_active = runtime_overrides_active(cli, env, &runtime_config);
    apply_runtime_config(app, &runtime_config);
    classifier.set_llm_client(None);
    app.settings_modal_is_saving = false;
    app.settings_modal_error = None;
    app.pending_settings_validation_generation = None;
    app.last_error = None;
    *runtime_connection_rx = start_runtime_connection_check(app, classifier, &runtime_config);
    app.last_warning = combine_warnings(
        warning.map(normalize_warning),
        runtime_override_warning(cli, env, &runtime_config),
    );
    runtime_override_active
}

fn normalize_warning(warning: String) -> String {
    warning
        .strip_prefix("Warning: ")
        .unwrap_or(&warning)
        .to_string()
}

fn runtime_override_warning(
    cli: &Cli,
    env: &EnvOverrides,
    runtime_config: &RuntimeConfig,
) -> Option<String> {
    runtime_overrides_active(cli, env, runtime_config).then(|| {
        "Launch-time CLI/env overrides still control the live runtime; restart without overrides to use saved settings"
            .to_string()
    })
}

fn runtime_overrides_active(cli: &Cli, env: &EnvOverrides, runtime_config: &RuntimeConfig) -> bool {
    if cli.no_llm || cli.provider.is_some() || cli.api_key.is_some() {
        return true;
    }

    let Some(provider) = runtime_config.provider.as_ref() else {
        return false;
    };

    env.api_key_for(provider.kind).is_some() || env.base_url_for(provider.kind).is_some()
}

fn combine_warnings(primary: Option<String>, secondary: Option<String>) -> Option<String> {
    match (primary, secondary) {
        (Some(primary), Some(secondary)) => Some(format!("{primary} {secondary}")),
        (Some(primary), None) => Some(primary),
        (None, Some(secondary)) => Some(secondary),
        (None, None) => None,
    }
}

fn build_llm_client(provider: &ResolvedProviderConfig) -> LlmClient {
    match provider.kind {
        ProviderKind::OpenRouter => LlmClient::OpenRouter(OpenRouterClient::new(provider.clone())),
        ProviderKind::OpenAI => LlmClient::OpenAI(OpenAiClient::new(provider.clone())),
        _ => unreachable!("unsupported provider should have been filtered before runtime"),
    }
}

fn persist_settings(
    config_path: &Path,
    preferences: &mut crate::config::AppConfig,
    draft: SettingsDraft,
    secrets: &mut impl crate::secrets::SecretStore,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut updated = preferences.clone();
    updated.llm.enabled = draft.llm_enabled;
    updated.llm.active_provider = draft.provider;
    updated.ui.size_mode = draft.size_mode;
    updated.ui.last_selected_scan_profile = draft.selected_scan_profile.clone();
    updated.llm.providers.insert(
        draft.provider,
        purifier_core::provider::ProviderSettings {
            model: draft.model,
            base_url: draft.base_url,
        },
    );
    updated.onboarding.first_launch_prompt_dismissed = true;

    let previous_secret = if draft.api_key_edited {
        Some(secrets.load_api_key(draft.provider)?)
    } else {
        None
    };

    if draft.api_key_edited {
        if draft.api_key.is_empty() {
            secrets.delete_api_key(draft.provider)?;
        } else {
            secrets.save_api_key(draft.provider, &draft.api_key)?;
        }
    }

    if let Err(error) = updated.save(config_path) {
        if let Some(previous_secret) = previous_secret {
            restore_provider_secret(secrets, draft.provider, previous_secret)?;
        }
        return Err(Box::new(error));
    }
    *preferences = updated;
    Ok(())
}

fn restore_provider_secret(
    secrets: &mut impl SecretStore,
    provider: ProviderKind,
    previous_secret: Option<String>,
) -> Result<(), crate::secrets::SecretStoreError> {
    match previous_secret {
        Some(api_key) => secrets.save_api_key(provider, &api_key),
        None => secrets.delete_api_key(provider),
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

fn load_app_config(path: &Path) -> config::AppConfig {
    config::AppConfig::load_or_default(path).unwrap_or_else(|error| {
        eprintln!("Warning: could not load config {}: {error}", path.display());
        config::AppConfig::default()
    })
}

fn resolve_runtime_config(
    cli: &Cli,
    saved: config::AppConfig,
    env: &EnvOverrides,
    secrets: &impl SecretStore,
) -> Result<RuntimeConfig, secrets::SecretStoreError> {
    use purifier_core::provider::default_provider_settings;

    let llm_enabled = !cli.no_llm && saved.llm.enabled;
    if !llm_enabled {
        return Ok(RuntimeConfig::rules_only(saved, cli.path.clone()));
    }

    let provider_kind = cli.provider.unwrap_or(saved.llm.active_provider);
    let provider_settings = saved
        .llm
        .providers
        .get(&provider_kind)
        .cloned()
        .unwrap_or_else(|| default_provider_settings(provider_kind));
    let api_key = match cli
        .api_key
        .clone()
        .or_else(|| env.api_key_for(provider_kind))
    {
        Some(api_key) => Some(api_key),
        None if provider_kind == ProviderKind::Ollama => None,
        None => secrets.load_api_key(provider_kind)?,
    };
    let show_onboarding = llm_enabled
        && provider_kind != ProviderKind::Ollama
        && api_key.is_none()
        && !saved.onboarding.first_launch_prompt_dismissed;
    let provider = if llm_enabled && (!show_onboarding || provider_kind == ProviderKind::Ollama) {
        Some(ResolvedProviderConfig::new(
            provider_kind,
            api_key,
            provider_settings.model,
            env.base_url_for(provider_kind)
                .unwrap_or(provider_settings.base_url),
        ))
    } else {
        None
    };

    Ok(RuntimeConfig {
        scan_path: cli.path.clone().or(saved.ui.last_scan_path),
        provider,
        show_onboarding,
        llm_enabled,
    })
}

fn load_runtime_config(
    cli: &Cli,
    saved: config::AppConfig,
    env: &EnvOverrides,
    secrets: &impl SecretStore,
) -> (RuntimeConfig, Option<String>) {
    match resolve_runtime_config(cli, saved.clone(), env, secrets) {
        Ok(runtime_config) => finalize_runtime_config(runtime_config, &saved),
        Err(secrets::SecretStoreError::Read { provider, .. }) => (
            RuntimeConfig::rules_only(saved, cli.path.clone()),
            Some(format!(
                "Warning: failed to read API key for {provider:?}; continuing with rules-only classification"
            )),
        ),
        Err(error) => (
            RuntimeConfig::rules_only(saved, cli.path.clone()),
            Some(format!(
                "Warning: {error}; continuing with rules-only classification"
            )),
        ),
    }
}

fn finalize_runtime_config(
    runtime_config: RuntimeConfig,
    saved: &config::AppConfig,
) -> (RuntimeConfig, Option<String>) {
    if runtime_config.show_onboarding {
        return (
            RuntimeConfig {
                scan_path: runtime_config.scan_path,
                provider: None,
                show_onboarding: true,
                llm_enabled: false,
            },
            Some(
                "Warning: LLM classification is disabled until an API key is configured"
                    .to_string(),
            ),
        );
    }

    let Some(provider) = runtime_config.provider.as_ref() else {
        return (runtime_config, None);
    };

    if provider.kind == ProviderKind::Ollama {
        return (
            RuntimeConfig::rules_only(saved.clone(), runtime_config.scan_path),
            Some(
                "Warning: Ollama support is temporarily disabled; continuing with rules-only classification"
                    .to_string(),
            ),
        );
    }

    if provider.api_key.is_none() {
        return (
            RuntimeConfig::rules_only(saved.clone(), runtime_config.scan_path),
            Some(
                "Warning: LLM classification is disabled until an API key is configured"
                    .to_string(),
            ),
        );
    }

    if matches!(
        provider.kind,
        ProviderKind::OpenRouter | ProviderKind::OpenAI
    ) {
        return (runtime_config, None);
    }

    (
        RuntimeConfig::rules_only(saved.clone(), runtime_config.scan_path),
        Some(format!(
            "Warning: {:?} is not wired into runtime classification yet; continuing with rules-only classification",
            provider.kind
        )),
    )
}

impl EnvOverrides {
    fn from_process() -> Self {
        Self {
            openrouter_api_key: std::env::var("OPENROUTER_API_KEY").ok(),
            openai_api_key: std::env::var("OPENAI_API_KEY").ok(),
            anthropic_api_key: std::env::var("ANTHROPIC_API_KEY").ok(),
            gemini_api_key: std::env::var("GEMINI_API_KEY").ok(),
            google_api_key: std::env::var("GOOGLE_API_KEY").ok(),
        }
    }

    fn api_key_for(&self, provider: ProviderKind) -> Option<String> {
        match provider {
            ProviderKind::OpenRouter => self.openrouter_api_key.clone(),
            ProviderKind::OpenAI => self.openai_api_key.clone(),
            ProviderKind::Anthropic => self.anthropic_api_key.clone(),
            ProviderKind::Google => self.gemini_api_key.clone().or(self.google_api_key.clone()),
            ProviderKind::Ollama => None,
        }
    }

    fn base_url_for(&self, provider: ProviderKind) -> Option<String> {
        let _ = provider;
        None
    }
}

impl RuntimeConfig {
    fn rules_only(saved: config::AppConfig, scan_path: Option<PathBuf>) -> Self {
        Self {
            scan_path: scan_path.or(saved.ui.last_scan_path),
            provider: None,
            show_onboarding: false,
            llm_enabled: false,
        }
    }
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Startup preference sync remains covered by unit tests"
    )
)]
fn apply_startup_config(app: &mut App, config: &config::AppConfig) {
    app.preferences = config.clone();
    app.columns.sort_key = config.ui.sort_key;
}

fn apply_startup_messages(
    app: &mut App,
    startup_warning: Option<String>,
    cli: &Cli,
    env: &EnvOverrides,
    runtime_config: &RuntimeConfig,
) {
    app.last_warning = combine_warnings(
        startup_warning.map(normalize_warning),
        runtime_override_warning(cli, env, runtime_config),
    );
}

fn apply_runtime_config(app: &mut App, runtime_config: &RuntimeConfig) {
    app.llm_enabled = runtime_config.llm_enabled;
    app.llm_online = false;
    app.llm_connection_generation = app.llm_connection_generation.wrapping_add(1);
    app.llm_status = if runtime_config.show_onboarding {
        LlmStatus::NeedsSetup
    } else if !runtime_config.llm_enabled {
        LlmStatus::Disabled
    } else if let Some(provider) = runtime_config.provider.as_ref() {
        LlmStatus::Connecting(provider.kind)
    } else {
        LlmStatus::Disabled
    };
}

fn start_runtime_connection_check(
    app: &App,
    classifier: &Classifier,
    runtime_config: &RuntimeConfig,
) -> Option<RuntimeConnectionReceiver> {
    let provider = runtime_config.provider.as_ref()?;

    if !runtime_config.llm_enabled
        || !matches!(
            provider.kind,
            ProviderKind::OpenRouter | ProviderKind::OpenAI
        )
    {
        return None;
    }

    let _ = classifier;
    let _ = app;
    let generation = app.llm_connection_generation;
    let provider = provider.clone();
    let client = build_llm_client(&provider);
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to create tokio runtime for LLM connection validation");

        let event = match runtime.block_on(client.clone().validate_connection()) {
            Ok(()) => RuntimeConnectionEvent::Validated {
                generation,
                provider: provider.kind,
                client,
            },
            Err(error) => RuntimeConnectionEvent::Failed {
                generation,
                provider: provider.kind,
                detail: connection_failure(&error),
            },
        };
        let _ = tx.send(event);
    });
    Some(rx)
}

fn apply_runtime_connection_event(
    app: &mut App,
    classifier: &mut Classifier,
    event: RuntimeConnectionEvent,
) {
    match event {
        RuntimeConnectionEvent::Validated {
            generation,
            provider,
            client,
        } => {
            if !matches_runtime_connection_target(app, generation, provider) {
                return;
            }
            let closes_modal = app.pending_settings_validation_generation == Some(generation);
            classifier.set_llm_client(Some(client));
            app.llm_online = true;
            app.llm_status = LlmStatus::Ready(provider);
            app.last_error = None;

            if closes_modal {
                app.close_preview_modal();
            }
        }
        RuntimeConnectionEvent::Failed {
            generation,
            provider,
            detail,
        } => {
            if !matches_runtime_connection_target(app, generation, provider) {
                return;
            }
            let keeps_modal_open = app.pending_settings_validation_generation == Some(generation);
            classifier.set_llm_client(None);
            app.llm_online = false;
            app.llm_status = LlmStatus::Error(connection_failure_status(provider, &detail));
            let message = if keeps_modal_open {
                format!(
                    "{:?} connection failed: {}. Update the API key or provider and save again.",
                    provider,
                    connection_failure_detail(&detail)
                )
            } else {
                format!(
                    "{:?} connection failed: {}. Check the API key, model, base URL, or network, then update settings after scan completion.",
                    provider,
                    connection_failure_detail(&detail)
                )
            };
            app.last_error = Some(message.clone());

            if keeps_modal_open {
                app.settings_modal_is_saving = false;
                app.settings_modal_error = Some(message);
                app.pending_settings_validation_generation = None;
            }
        }
    }
}

fn matches_runtime_connection_target(app: &App, generation: u64, provider: ProviderKind) -> bool {
    app.llm_enabled
        && app.llm_connection_generation == generation
        && app.llm_status == LlmStatus::Connecting(provider)
}

fn connection_failure(error: &LlmError) -> RuntimeConnectionFailure {
    match error {
        LlmError::MissingApiKey { .. } => RuntimeConnectionFailure::MissingApiKey,
        LlmError::Request { kind, .. } => match kind {
            LlmRequestErrorKind::Timeout => RuntimeConnectionFailure::Timeout,
            LlmRequestErrorKind::Http { status, body } => RuntimeConnectionFailure::Http {
                status: *status,
                body: body.clone(),
            },
            LlmRequestErrorKind::Network { message } => {
                RuntimeConnectionFailure::Network(message.clone())
            }
        },
        LlmError::Response { message, .. } => {
            RuntimeConnectionFailure::InvalidResponse(message.clone())
        }
    }
}

fn connection_failure_detail(detail: &RuntimeConnectionFailure) -> String {
    match detail {
        RuntimeConnectionFailure::MissingApiKey => "API key missing".to_string(),
        RuntimeConnectionFailure::Timeout => "request timed out".to_string(),
        RuntimeConnectionFailure::Http {
            status,
            body: Some(body),
        } => format!("HTTP {} {} - {body}", status, http_status_reason(*status)),
        RuntimeConnectionFailure::Http { status, body: None } => {
            format!("HTTP {} {}", status, http_status_reason(*status))
        }
        RuntimeConnectionFailure::Network(message)
        | RuntimeConnectionFailure::InvalidResponse(message) => message.clone(),
    }
}

fn connection_failure_status(provider: ProviderKind, detail: &RuntimeConnectionFailure) -> String {
    let provider_name = format!("{:?}", provider);
    match detail {
        RuntimeConnectionFailure::MissingApiKey => format!("{provider_name} setup incomplete"),
        RuntimeConnectionFailure::Timeout => format!("{provider_name} timed out"),
        RuntimeConnectionFailure::Http {
            status: 401 | 403, ..
        } => {
            format!("{provider_name} auth failed")
        }
        RuntimeConnectionFailure::Http { status, .. } if *status == 404 => {
            format!("{provider_name} bad base URL")
        }
        RuntimeConnectionFailure::Http { status, body } if *status == 400 => {
            if body
                .as_deref()
                .is_some_and(|body| body.to_ascii_lowercase().contains("model"))
            {
                format!("{provider_name} model failed")
            } else {
                format!("{provider_name} request failed")
            }
        }
        RuntimeConnectionFailure::Http { .. } => format!("{provider_name} request failed"),
        RuntimeConnectionFailure::Network(_) => format!("{provider_name} network failed"),
        RuntimeConnectionFailure::InvalidResponse(_) => format!("{provider_name} response failed"),
    }
}

fn http_status_reason(status: u16) -> &'static str {
    match status {
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    }
}

fn validate_scan_path(path: PathBuf) -> Option<PathBuf> {
    path.is_dir().then_some(path)
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "Covered by startup path resolution tests")
)]
fn resolve_initial_scan_path(
    cli_path: Option<&Path>,
    config: &config::AppConfig,
) -> Option<PathBuf> {
    cli_path
        .map(normalize_scan_path)
        .and_then(validate_scan_path)
        .or_else(|| {
            config
                .ui
                .last_scan_path
                .as_deref()
                .map(normalize_scan_path)
                .and_then(validate_scan_path)
        })
}

fn resolve_initial_scan_profile(
    cli_path: Option<&Path>,
    config: &config::AppConfig,
) -> Option<purifier_core::ScanProfile> {
    if cli_path.is_some() {
        None
    } else {
        config.active_scan_profile().cloned()
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
    (Some(unknown_tx), Some(result_rx))
}

fn restart_llm_processing_for_new_scan(
    classifier: &Classifier,
    app: &mut App,
    unknown_tx: &mut Option<UnknownBatchSender>,
    llm_result_rx: &mut Option<LlmResultReceiver>,
    buffered_unknown_batches: &mut Vec<Vec<UnknownEntry>>,
    buffered_llm_results: &mut Vec<LlmClassification>,
) {
    buffered_unknown_batches.clear();
    buffered_llm_results.clear();
    *unknown_tx = None;
    *llm_result_rx = None;
    (*unknown_tx, *llm_result_rx) = start_llm_processing(classifier, app);
}

fn start_scan_processing(
    scan_root: PathBuf,
    scan_rx: crossbeam_channel::Receiver<ScanEvent>,
    rules: RulesEngine,
    mark_unknown_pending: bool,
) -> ActiveScan {
    let (update_tx, updates) = crossbeam_channel::unbounded();
    let (cancel, cancel_rx) = crossbeam_channel::bounded(1);

    std::thread::spawn(move || {
        let mut path_children: HashMap<PathBuf, Vec<FileEntry>> = HashMap::new();
        let mut pending_unknowns = Vec::new();

        loop {
            crossbeam_channel::select! {
                recv(cancel_rx) -> _ => break,
                recv(scan_rx) -> message => match message {
                    Ok(ScanEvent::Entry {
                        path,
                        sizes,
                        file_identity,
                        is_dir,
                        modified,
                    }) => {
                        let mut entry = FileEntry::new_with_sizes(
                            path.clone(),
                            sizes,
                            file_identity,
                            is_dir,
                            modified,
                        );
                        classify_entry_with_rules(&rules, &mut entry);
                        queue_scan_unknown_entry(
                            &mut entry,
                            &mut pending_unknowns,
                            &update_tx,
                            mark_unknown_pending,
                        );

                        if let Some(parent) = path.parent() {
                            path_children
                                .entry(parent.to_path_buf())
                                .or_default()
                                .push(entry);
                        }
                    }
                    Ok(ScanEvent::Progress {
                        entries_scanned,
                        logical_bytes_found,
                        physical_bytes_found,
                        current_path,
                    }) => {
                        if update_tx
                            .send(ScanProcessingUpdate::Progress(ScanProgressSnapshot {
                                entries_scanned,
                                logical_bytes_found,
                                physical_bytes_found,
                                current_path,
                            }))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(ScanEvent::ScanComplete {
                        total_entries,
                        total_logical_bytes,
                        total_physical_bytes,
                        skipped,
                    }) => {
                        flush_scan_unknown_batches(&mut pending_unknowns, &update_tx);
                        let entries = build_tree(
                            &scan_root,
                            &mut path_children,
                            &HashSet::new(),
                            &HashSet::new(),
                        );

                        let _ = update_tx.send(ScanProcessingUpdate::Complete(CompletedScanResult {
                            entries,
                            total_entries,
                            total_logical_bytes,
                            total_physical_bytes,
                            skipped,
                        }));
                        break;
                    }
                    Err(_) => break,
                }
            }
        }
    });

    ActiveScan { updates, cancel }
}

fn classifier_rules(classifier: &Classifier) -> RulesEngine {
    classifier.rules().clone()
}

fn apply_scan_update(app: &mut App, update: ScanProcessingUpdate) -> bool {
    match update {
        ScanProcessingUpdate::Progress(progress) => {
            app.files_scanned = progress.entries_scanned;
            app.logical_bytes_found = progress.logical_bytes_found;
            app.physical_bytes_found = progress.physical_bytes_found;
            app.sync_display_size_state();
            app.current_scan_dir = progress.current_path;
            false
        }
        ScanProcessingUpdate::UnknownBatch(_) => false,
        ScanProcessingUpdate::Complete(result) => {
            app.scan_status = ScanStatus::Complete;
            app.total_logical_size = result.total_logical_bytes;
            app.total_physical_size = result.total_physical_bytes;
            app.sync_display_size_state();
            app.total_files = result.total_entries;
            app.skipped = result.skipped;
            app.entries = result.entries;
            app.rebuild_size_cache();
            app.invalidate_caches();
            true
        }
    }
}

fn classify_entry_with_rules(rules: &RulesEngine, entry: &mut FileEntry) {
    if let Some(rule_match) = rules.classify(&entry.path) {
        entry.category = rule_match.category;
        entry.safety = rule_match.safety;
        entry.safety_reason = rule_match.reason;
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn queue_unknown_entry(
    entry: &mut FileEntry,
    pending_unknowns: &mut Vec<UnknownEntry>,
    unknown_tx: Option<&crossbeam_channel::Sender<Vec<UnknownEntry>>>,
    mark_pending_without_worker: bool,
) {
    if entry.safety != SafetyLevel::Unknown {
        return;
    }

    if unknown_tx.is_some() || mark_pending_without_worker {
        entry.safety_reason = "Analyzing with LLM...".to_string();
    }

    if let Some(tx) = unknown_tx {
        pending_unknowns.push(UnknownEntry {
            path: entry.path.clone(),
            size: entry.total_size(SizeMode::Logical),
            is_dir: entry.is_dir,
            age_days: entry.modified.and_then(age_in_days),
        });

        if pending_unknowns.len() >= LLM_BATCH_SIZE {
            let batch = pending_unknowns.drain(..LLM_BATCH_SIZE).collect();
            let _ = tx.send(batch);
        }
    }
}

fn requeue_unknown_entries_for_visible_tree(
    app: &mut App,
    unknown_tx: Option<&UnknownBatchSender>,
) {
    let Some(tx) = unknown_tx else {
        return;
    };

    if app.entries.is_empty() {
        return;
    }

    mark_unknown_entries_as_pending(&mut app.entries);
    for batch in batch_unknowns(collect_unknowns(&app.entries)) {
        let _ = tx.send(batch);
    }
}

fn flush_unknown_batches(
    buffered_unknown_batches: &mut Vec<Vec<UnknownEntry>>,
    unknown_tx: Option<&UnknownBatchSender>,
) {
    let Some(tx) = unknown_tx else {
        return;
    };

    for batch in buffered_unknown_batches.drain(..) {
        let _ = tx.send(batch);
    }
}

fn queue_scan_unknown_entry(
    entry: &mut FileEntry,
    pending_unknowns: &mut Vec<UnknownEntry>,
    update_tx: &crossbeam_channel::Sender<ScanProcessingUpdate>,
    mark_unknown_pending: bool,
) {
    if entry.safety != SafetyLevel::Unknown {
        return;
    }

    if mark_unknown_pending {
        entry.safety_reason = "Analyzing with LLM...".to_string();
    }

    pending_unknowns.push(UnknownEntry {
        path: entry.path.clone(),
        size: entry.total_size(SizeMode::Logical),
        is_dir: entry.is_dir,
        age_days: entry.modified.and_then(age_in_days),
    });

    if pending_unknowns.len() >= LLM_BATCH_SIZE {
        let batch = pending_unknowns.drain(..LLM_BATCH_SIZE).collect();
        let _ = update_tx.send(ScanProcessingUpdate::UnknownBatch(batch));
    }
}

fn flush_scan_unknown_batches(
    pending_unknowns: &mut Vec<UnknownEntry>,
    update_tx: &crossbeam_channel::Sender<ScanProcessingUpdate>,
) {
    if pending_unknowns.is_empty() {
        return;
    }

    let batch = std::mem::take(pending_unknowns);
    let _ = update_tx.send(ScanProcessingUpdate::UnknownBatch(batch));
}

fn should_mark_unknowns_pending(app: &App) -> bool {
    !matches!(
        app.llm_status,
        LlmStatus::Disabled | LlmStatus::NeedsSetup | LlmStatus::Error(_)
    )
}

fn mark_unknown_entries_as_pending(entries: &mut [FileEntry]) {
    for entry in entries {
        if entry.safety == SafetyLevel::Unknown {
            entry.safety_reason = "Analyzing with LLM...".to_string();
        }
        mark_unknown_entries_as_pending(&mut entry.children);
    }
}

fn reset_pending_llm_labels(entries: &mut [FileEntry]) {
    for entry in entries {
        if entry.safety == SafetyLevel::Unknown && entry.safety_reason == "Analyzing with LLM..." {
            entry.safety_reason = "Could not classify — review manually".to_string();
        }
        reset_pending_llm_labels(&mut entry.children);
    }
}

fn reset_pending_llm_labels_in_state(app: &mut App) {
    reset_pending_llm_labels(&mut app.entries);
}

fn apply_llm_results(app: &mut App, results: Vec<LlmClassification>) {
    let mut applied_any = false;

    for result in results {
        applied_any |= update_entry_classification(&mut app.entries, &result);
    }

    if applied_any {
        app.llm_classified_count += 1;
        app.invalidate_caches();
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

#[cfg_attr(not(test), allow(dead_code))]
fn age_in_days(modified: SystemTime) -> Option<i64> {
    SystemTime::now()
        .duration_since(modified)
        .ok()
        .map(|duration| (duration.as_secs() / 86_400) as i64)
}

#[cfg_attr(not(test), allow(dead_code))]
fn refresh_scan_snapshot(app: &mut App, path_children: &HashMap<PathBuf, Vec<FileEntry>>) {
    if app.scan_status != ScanStatus::Scanning {
        return;
    }

    let _ = path_children;
}

#[allow(dead_code)]
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
                .sort_by_key(|child| std::cmp::Reverse(child.total_size(SizeMode::Logical)));
        }
    }

    entries.sort_by_key(|entry| std::cmp::Reverse(entry.total_size(SizeMode::Logical)));
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
                .sort_by_key(|child| std::cmp::Reverse(child.total_size(SizeMode::Logical)));
        }
    }

    entries.sort_by_key(|entry| std::cmp::Reverse(entry.total_size(SizeMode::Logical)));
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
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;

    use crate::{Cli, EnvOverrides};
    use purifier_core::llm::{LlmClassification, OpenRouterClient, UnknownEntry};
    use purifier_core::provider::{LlmClient, ProviderKind, ResolvedProviderConfig};
    use purifier_core::rules::RulesEngine;
    use purifier_core::size::SizeMode;
    use purifier_core::types::{Category, FileEntry, SafetyLevel, ScanEvent};

    use super::{
        apply_llm_results, apply_runtime_config, apply_runtime_connection_event, apply_scan_update,
        apply_startup_config, apply_startup_messages, build_tree, build_tree_snapshot,
        load_app_config, normalize_scan_path, queue_unknown_entry, refresh_scan_snapshot,
        requeue_unknown_entries_for_visible_tree, reset_pending_llm_labels,
        reset_pending_llm_labels_in_state, resolve_initial_scan_path, resolve_initial_scan_profile,
        restart_llm_processing_for_new_scan, start_llm_processing, start_runtime_connection_check,
        start_scan_processing, RuntimeConfig, ScanProcessingUpdate, ScanProgressSnapshot,
    };
    use crate::app::{App, LlmStatus, ScanStatus};
    use crate::config::AppConfig;
    use purifier_core::classifier::Classifier;
    use purifier_core::{Filter, FilterTest, ScanProfile};

    fn spawn_http_server(
        status_line: &'static str,
        body: &'static str,
        delay_before_response: Option<std::time::Duration>,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        let address = listener
            .local_addr()
            .expect("test server should have a local address");

        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("test server should accept");
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer);
            if let Some(delay) = delay_before_response {
                std::thread::sleep(delay);
            }
            let response = format!(
                "HTTP/1.1 {status_line}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("test server should respond");
        });

        format!("http://{address}")
    }

    #[test]
    fn start_llm_processing_should_not_mark_app_online_just_for_worker_startup() {
        let classifier = Classifier::new(
            RulesEngine::new(&[]).expect("rules engine should initialize"),
            Some(LlmClient::OpenRouter(OpenRouterClient::new(
                ResolvedProviderConfig::new(
                    ProviderKind::OpenRouter,
                    Some("test-key".to_string()),
                    "google/gemini-2.0-flash-001".to_string(),
                    "https://openrouter.ai/api/v1".to_string(),
                ),
            ))),
        );
        let mut app = App::new(Some(PathBuf::from("/scan")), true, AppConfig::default());
        app.llm_status = LlmStatus::Connecting(ProviderKind::OpenRouter);

        let (unknown_tx, result_rx) = start_llm_processing(&classifier, &mut app);

        assert!(unknown_tx.is_some(), "worker sender should exist");
        assert!(result_rx.is_some(), "worker receiver should exist");
        assert!(!app.llm_online, "worker startup should not mark LLM online");
        assert_eq!(
            app.llm_status,
            LlmStatus::Connecting(ProviderKind::OpenRouter)
        );
    }

    #[test]
    fn restart_llm_processing_for_new_scan_should_replace_old_result_receiver() {
        let classifier = Classifier::new(
            RulesEngine::new(&[]).expect("rules engine should initialize"),
            Some(LlmClient::OpenRouter(OpenRouterClient::new(
                ResolvedProviderConfig::new(
                    ProviderKind::OpenRouter,
                    Some("test-key".to_string()),
                    "google/gemini-2.0-flash-001".to_string(),
                    "https://openrouter.ai/api/v1".to_string(),
                ),
            ))),
        );
        let mut app = App::new(Some(PathBuf::from("/scan")), true, AppConfig::default());
        let (old_unknown_tx, _old_unknown_rx) = crossbeam_channel::unbounded();
        let (old_result_tx, old_result_rx) = crossbeam_channel::unbounded();
        old_result_tx
            .send(vec![LlmClassification {
                path: PathBuf::from("/stale"),
                category: Category::Unknown,
                safety: SafetyLevel::Unknown,
                reason: "stale".to_string(),
            }])
            .expect("stale result should be queued");

        let mut unknown_tx = Some(old_unknown_tx);
        let mut llm_result_rx = Some(old_result_rx);
        let mut buffered_unknown_batches = vec![vec![UnknownEntry {
            path: PathBuf::from("/stale"),
            size: 1,
            is_dir: false,
            age_days: None,
        }]];
        let mut buffered_llm_results = vec![LlmClassification {
            path: PathBuf::from("/stale"),
            category: Category::Unknown,
            safety: SafetyLevel::Unknown,
            reason: "stale".to_string(),
        }];

        restart_llm_processing_for_new_scan(
            &classifier,
            &mut app,
            &mut unknown_tx,
            &mut llm_result_rx,
            &mut buffered_unknown_batches,
            &mut buffered_llm_results,
        );

        assert!(buffered_unknown_batches.is_empty());
        assert!(buffered_llm_results.is_empty());
        assert!(unknown_tx.is_some(), "new worker sender should exist");
        assert!(llm_result_rx.is_some(), "new worker receiver should exist");
        assert!(
            llm_result_rx
                .as_ref()
                .expect("new result receiver should exist")
                .try_recv()
                .is_err(),
            "new result receiver should not expose stale queued results"
        );
    }

    #[test]
    fn start_llm_processing_should_preserve_needs_setup_state_without_llm_client() {
        let classifier = Classifier::new(
            RulesEngine::new(&[]).expect("rules engine should initialize"),
            None,
        );
        let mut app = App::new(Some(PathBuf::from("/scan")), false, AppConfig::default());
        app.llm_status = LlmStatus::NeedsSetup;

        let (unknown_tx, result_rx) = start_llm_processing(&classifier, &mut app);

        assert!(unknown_tx.is_none());
        assert!(result_rx.is_none());
        assert_eq!(app.llm_status, LlmStatus::NeedsSetup);
        assert!(!app.llm_online);
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

        queue_unknown_entry(&mut entry, &mut pending, Some(&unknown_tx), false);

        let batch = unknown_rx.try_recv().expect("full batch should flush");
        assert_eq!(batch.len(), 50, "batch should contain 50 entries");
        assert!(
            pending.is_empty(),
            "pending batch should be empty after flush"
        );
        assert_eq!(entry.safety_reason, "Analyzing with LLM...");
    }

    #[test]
    fn requeue_unknown_entries_for_visible_tree_should_enqueue_existing_unknowns_while_scanning() {
        let (unknown_tx, unknown_rx) = crossbeam_channel::unbounded();
        let mut app = App::new(Some(PathBuf::from("/scan")), true, AppConfig::default());
        app.scan_status = ScanStatus::Scanning;
        app.entries = vec![FileEntry::new(
            PathBuf::from("/scan/unknown"),
            10,
            false,
            None,
        )];

        requeue_unknown_entries_for_visible_tree(&mut app, Some(&unknown_tx));

        let batch: Vec<purifier_core::llm::UnknownEntry> = unknown_rx
            .try_recv()
            .expect("unknown entries should be re-queued");
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].path, PathBuf::from("/scan/unknown"));
        assert_eq!(app.entries[0].safety_reason, "Analyzing with LLM...");
    }

    #[test]
    fn queue_unknown_entry_should_mark_connecting_rows_pending_before_failure_reset() {
        let mut entry = FileEntry::new(PathBuf::from("/scan/unknown"), 10, false, None);
        let mut pending = Vec::new();

        queue_unknown_entry(&mut entry, &mut pending, None, true);

        assert!(
            pending.is_empty(),
            "connecting rows should not queue without a worker"
        );
        assert_eq!(entry.safety_reason, "Analyzing with LLM...");

        let mut entries = vec![entry];
        reset_pending_llm_labels(&mut entries);

        assert_eq!(
            entries[0].safety_reason,
            "Could not classify — review manually"
        );
    }

    #[test]
    fn reset_pending_llm_labels_should_restore_unknown_rows_when_no_worker_is_live() {
        let mut app = App::new(Some(PathBuf::from("/scan")), true, AppConfig::default());
        app.entries = vec![FileEntry::new(
            PathBuf::from("/scan/unknown"),
            10,
            false,
            None,
        )];
        app.entries[0].safety_reason = "Analyzing with LLM...".to_string();

        reset_pending_llm_labels(&mut app.entries);

        assert_eq!(
            app.entries[0].safety_reason,
            "Could not classify — review manually"
        );
    }

    #[test]
    fn reset_pending_llm_labels_in_state_should_restore_visible_tree_rows_when_no_worker_is_live() {
        let mut app = App::new(Some(PathBuf::from("/scan")), true, AppConfig::default());
        app.entries = vec![{
            let mut entry = FileEntry::new(PathBuf::from("/scan/unknown"), 10, false, None);
            entry.safety_reason = "Analyzing with LLM...".to_string();
            entry
        }];

        reset_pending_llm_labels_in_state(&mut app);

        assert_eq!(
            app.entries[0].safety_reason,
            "Could not classify — review manually"
        );
    }

    #[test]
    fn apply_llm_results_should_update_matching_path_only() {
        let mut app = App::new(Some(PathBuf::from("/scan")), true, AppConfig::default());
        app.entries = vec![
            FileEntry::new(PathBuf::from("/scan/match"), 10, false, None),
            FileEntry::new(PathBuf::from("/scan/other"), 20, false, None),
        ];

        apply_llm_results(
            &mut app,
            vec![LlmClassification {
                path: PathBuf::from("/scan/match"),
                category: Category::Cache,
                safety: SafetyLevel::Safe,
                reason: "Recreated automatically".to_string(),
            }],
        );

        assert_eq!(app.entries[0].safety, SafetyLevel::Safe);
        assert_eq!(app.entries[0].category, Category::Cache);
        assert_eq!(app.entries[0].safety_reason, "Recreated automatically");
        assert_eq!(app.entries[1].safety, SafetyLevel::Unknown);
    }

    #[test]
    fn refresh_scan_snapshot_should_keep_entries_hidden_before_scan_complete() {
        let path_children = HashMap::from([(
            PathBuf::from("/scan"),
            vec![FileEntry::new(
                PathBuf::from("/scan/cache"),
                10,
                false,
                None,
            )],
        )]);
        let mut app = App::new(Some(PathBuf::from("/scan")), false, AppConfig::default());
        app.scan_status = ScanStatus::Scanning;

        refresh_scan_snapshot(&mut app, &path_children);

        assert!(
            app.entries.is_empty(),
            "live entries should stay hidden until scan completion"
        );
    }

    #[test]
    fn apply_llm_results_should_not_refresh_hidden_rows_while_scanning() {
        let mut app = App::new(Some(PathBuf::from("/scan")), true, AppConfig::default());
        app.scan_status = ScanStatus::Scanning;

        apply_scan_update(
            &mut app,
            ScanProcessingUpdate::Progress(ScanProgressSnapshot {
                entries_scanned: 12,
                logical_bytes_found: 4096,
                physical_bytes_found: 8192,
                current_path: "/scan/cache".to_string(),
            }),
        );

        assert_eq!(app.files_scanned, 12);
        assert_eq!(app.bytes_found, 8192);
        assert_eq!(app.current_scan_dir, "/scan/cache");

        assert!(
            app.entries.is_empty(),
            "progress snapshots should not rebuild hidden scan rows"
        );
    }

    #[test]
    fn start_scan_processing_should_return_immediately_while_scan_worker_runs() {
        let (scan_tx, scan_rx) = crossbeam_channel::unbounded();
        let started_at = std::time::Instant::now();

        let handle = start_scan_processing(
            PathBuf::from("/scan"),
            scan_rx,
            RulesEngine::new(&[]).expect("rules engine should initialize"),
            true,
        );

        assert!(started_at.elapsed() < std::time::Duration::from_millis(100));

        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            let _ = scan_tx.send(ScanEvent::ScanComplete {
                total_entries: 0,
                total_logical_bytes: 0,
                total_physical_bytes: 0,
                skipped: 0,
            });
        });

        let update = handle
            .updates
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("processor should eventually finish");
        assert!(matches!(update, ScanProcessingUpdate::Complete(_)));
    }

    #[test]
    fn start_scan_processing_should_build_final_tree_on_completion() {
        let (scan_tx, scan_rx) = crossbeam_channel::unbounded();
        let handle = start_scan_processing(
            PathBuf::from("/scan"),
            scan_rx,
            RulesEngine::new(&[]).expect("rules engine should initialize"),
            true,
        );

        scan_tx
            .send(ScanEvent::Entry {
                path: PathBuf::from("/scan/dir"),
                sizes: purifier_core::size::EntrySizes {
                    logical_bytes: 0,
                    physical_bytes: 0,
                    accounted_physical_bytes: 0,
                },
                file_identity: None,
                is_dir: true,
                modified: None,
            })
            .expect("directory event should send");
        scan_tx
            .send(ScanEvent::Entry {
                path: PathBuf::from("/scan/dir/file"),
                sizes: purifier_core::size::EntrySizes {
                    logical_bytes: 5,
                    physical_bytes: 5,
                    accounted_physical_bytes: 5,
                },
                file_identity: None,
                is_dir: false,
                modified: None,
            })
            .expect("file event should send");
        scan_tx
            .send(ScanEvent::ScanComplete {
                total_entries: 2,
                total_logical_bytes: 5,
                total_physical_bytes: 5,
                skipped: 0,
            })
            .expect("complete event should send");

        let result = loop {
            let update = handle
                .updates
                .recv_timeout(std::time::Duration::from_secs(1))
                .expect("processor should emit completion update");
            if let ScanProcessingUpdate::Complete(result) = update {
                break result;
            }
        };

        assert_eq!(result.total_entries, 2);
        assert_eq!(result.total_logical_bytes, 5);
        assert_eq!(result.total_physical_bytes, 5);
        assert_eq!(result.skipped, 0);
        assert_eq!(
            collect_paths(&result.entries),
            vec![PathBuf::from("/scan/dir"), PathBuf::from("/scan/dir/file")]
        );
    }

    #[test]
    fn start_scan_processing_should_emit_unknown_batches_before_complete() {
        let (scan_tx, scan_rx) = crossbeam_channel::unbounded();
        let handle = start_scan_processing(
            PathBuf::from("/scan"),
            scan_rx,
            RulesEngine::new(&[]).expect("rules engine should initialize"),
            true,
        );

        scan_tx
            .send(ScanEvent::Entry {
                path: PathBuf::from("/scan/unknown"),
                sizes: purifier_core::size::EntrySizes {
                    logical_bytes: 5,
                    physical_bytes: 5,
                    accounted_physical_bytes: 5,
                },
                file_identity: None,
                is_dir: false,
                modified: None,
            })
            .expect("unknown entry should send");
        scan_tx
            .send(ScanEvent::ScanComplete {
                total_entries: 1,
                total_logical_bytes: 5,
                total_physical_bytes: 5,
                skipped: 0,
            })
            .expect("complete event should send");

        let first_update = handle
            .updates
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("processor should emit unknown batch before completion");
        let second_update = handle
            .updates
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("processor should emit completion update");

        let ScanProcessingUpdate::UnknownBatch(batch) = first_update else {
            panic!("expected unknown batch before completion");
        };
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].path, PathBuf::from("/scan/unknown"));

        assert!(matches!(second_update, ScanProcessingUpdate::Complete(_)));
    }

    #[test]
    fn build_tree_snapshot_should_nest_children_under_directories() {
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
        let expanded_paths = std::collections::HashSet::new();
        let deleted_paths = std::collections::HashSet::new();

        let entries = build_tree_snapshot(
            &PathBuf::from("/scan"),
            &path_children,
            &expanded_paths,
            &deleted_paths,
        );

        assert_eq!(entries.len(), 1, "root should have one directory child");
        assert_eq!(
            entries[0].children.len(),
            1,
            "directory should have one nested child"
        );
    }

    #[test]
    fn build_tree_snapshot_should_hide_deleted_paths() {
        let path_children = HashMap::from([(
            PathBuf::from("/scan"),
            vec![FileEntry::new(
                PathBuf::from("/scan/cache"),
                10,
                false,
                None,
            )],
        )]);
        let expanded_paths = std::collections::HashSet::new();
        let mut deleted_paths = std::collections::HashSet::new();
        deleted_paths.insert(PathBuf::from("/scan/cache"));

        let entries = build_tree_snapshot(
            &PathBuf::from("/scan"),
            &path_children,
            &expanded_paths,
            &deleted_paths,
        );

        assert!(
            entries.is_empty(),
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

    #[test]
    fn resolve_initial_scan_path_should_fall_back_to_config_last_scan_path() {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let last_scan_path = tempdir.path().join("purifier-config");
        fs::create_dir(&last_scan_path).expect("config path should be created");
        let config = AppConfig {
            ui: crate::config::UiConfig {
                last_scan_path: Some(last_scan_path.clone()),
                size_mode: SizeMode::Physical,
                scan_profiles: Vec::new(),
                last_selected_scan_profile: None,
                ..crate::config::UiConfig::default()
            },
            ..AppConfig::default()
        };

        let resolved = resolve_initial_scan_path(None, &config);

        assert_eq!(resolved, Some(last_scan_path));
    }

    #[test]
    fn resolve_initial_scan_path_should_ignore_missing_config_last_scan_path() {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let config = AppConfig {
            ui: crate::config::UiConfig {
                last_scan_path: Some(tempdir.path().join("missing-dir")),
                size_mode: SizeMode::Physical,
                scan_profiles: Vec::new(),
                last_selected_scan_profile: None,
                ..crate::config::UiConfig::default()
            },
            ..AppConfig::default()
        };

        let resolved = resolve_initial_scan_path(None, &config);

        assert_eq!(resolved, None);
    }

    #[test]
    fn resolve_initial_scan_profile_should_ignore_saved_profile_for_explicit_cli_path() {
        let config = AppConfig {
            ui: crate::config::UiConfig {
                last_scan_path: Some(PathBuf::from("/tmp/saved")),
                size_mode: SizeMode::Physical,
                scan_profiles: vec![ScanProfile {
                    name: "exclude-node-modules".to_string(),
                    exclude: Some(Filter::single(FilterTest::PathGlob(
                        "**/node_modules/**".to_string(),
                    ))),
                    mask: None,
                    display_filter: None,
                }],
                last_selected_scan_profile: Some("exclude-node-modules".to_string()),
                ..crate::config::UiConfig::default()
            },
            ..AppConfig::default()
        };

        let resolved =
            resolve_initial_scan_profile(Some(std::path::Path::new("/tmp/cli")), &config);

        assert_eq!(resolved, None);
    }

    #[test]
    fn resolve_initial_scan_profile_should_use_saved_profile_for_persisted_scan_path() {
        let config = AppConfig {
            ui: crate::config::UiConfig {
                last_scan_path: Some(PathBuf::from("/tmp/saved")),
                size_mode: SizeMode::Physical,
                scan_profiles: vec![ScanProfile {
                    name: "exclude-node-modules".to_string(),
                    exclude: Some(Filter::single(FilterTest::PathGlob(
                        "**/node_modules/**".to_string(),
                    ))),
                    mask: None,
                    display_filter: None,
                }],
                last_selected_scan_profile: Some("exclude-node-modules".to_string()),
                ..crate::config::UiConfig::default()
            },
            ..AppConfig::default()
        };

        let resolved = resolve_initial_scan_profile(None, &config);

        assert_eq!(resolved, Some(config.ui.scan_profiles[0].clone()));
    }

    #[test]
    fn load_app_config_should_return_defaults_when_config_is_invalid() {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let config_path = tempdir.path().join("config.toml");
        fs::write(&config_path, "not = [valid toml").expect("invalid config should be written");

        let config = load_app_config(&config_path);

        assert_eq!(config, AppConfig::default());
    }

    #[test]
    fn apply_startup_config_should_use_persisted_sort_key() {
        let mut app = App::new(None, false, AppConfig::default());
        let config = AppConfig {
            ui: crate::config::UiConfig {
                sort_key: crate::columns::SortKey::Safety,
                last_scan_path: None,
                size_mode: SizeMode::Physical,
                scan_profiles: Vec::new(),
                last_selected_scan_profile: None,
            },
            ..AppConfig::default()
        };

        apply_startup_config(&mut app, &config);

        assert_eq!(app.columns.sort_key, crate::columns::SortKey::Safety);
    }

    #[test]
    fn apply_startup_messages_should_surface_override_warning_in_app_state() {
        let mut app = App::new(None, false, AppConfig::default());
        let runtime_config = RuntimeConfig {
            scan_path: None,
            provider: None,
            show_onboarding: false,
            llm_enabled: false,
        };
        let cli = Cli {
            path: None,
            rules: None,
            no_llm: true,
            api_key: None,
            provider: None,
        };

        apply_startup_messages(
            &mut app,
            None,
            &cli,
            &EnvOverrides::default(),
            &runtime_config,
        );

        assert_eq!(
            app.last_warning.as_deref(),
            Some(
                "Launch-time CLI/env overrides still control the live runtime; restart without overrides to use saved settings"
            )
        );
    }

    #[test]
    fn apply_runtime_config_should_mark_live_provider_as_connecting_before_validation() {
        let runtime_config = RuntimeConfig {
            scan_path: Some(PathBuf::from("/scan")),
            provider: Some(ResolvedProviderConfig::new(
                ProviderKind::OpenRouter,
                Some("test-key".to_string()),
                "google/gemini-2.0-flash-001".to_string(),
                "https://openrouter.ai/api/v1".to_string(),
            )),
            show_onboarding: false,
            llm_enabled: true,
        };
        let mut app = App::new(Some(PathBuf::from("/scan")), true, AppConfig::default());

        apply_runtime_config(&mut app, &runtime_config);

        assert_eq!(
            app.llm_status,
            LlmStatus::Connecting(ProviderKind::OpenRouter)
        );
        assert!(app.llm_enabled);
        assert!(!app.llm_online);
    }

    #[test]
    fn start_runtime_connection_check_should_return_immediately_while_validation_runs() {
        let runtime_config = RuntimeConfig {
            scan_path: Some(PathBuf::from("/scan")),
            provider: Some(ResolvedProviderConfig::new(
                ProviderKind::OpenRouter,
                Some("test-key".to_string()),
                "google/gemini-2.0-flash-001".to_string(),
                spawn_http_server(
                    "200 OK",
                    r#"{"choices":[{"message":{"content":"[{\"path\":\"/tmp/purifier-validation\",\"category\":\"BuildArtifact\",\"safety\":\"Safe\",\"reason\":\"Validation probe\"}]"}}]}"#,
                    Some(std::time::Duration::from_millis(250)),
                ),
            )),
            show_onboarding: false,
            llm_enabled: true,
        };
        let mut app = App::new(Some(PathBuf::from("/scan")), true, AppConfig::default());
        let classifier = Classifier::new(RulesEngine::new(&[]).unwrap(), None);

        apply_runtime_config(&mut app, &runtime_config);
        let started_at = std::time::Instant::now();
        let validation_rx = start_runtime_connection_check(&app, &classifier, &runtime_config);

        assert!(validation_rx.is_some());
        assert!(started_at.elapsed() < std::time::Duration::from_millis(100));
        assert_eq!(
            app.llm_status,
            LlmStatus::Connecting(ProviderKind::OpenRouter)
        );
        assert!(!app.llm_online);
    }

    #[test]
    fn scan_processing_and_runtime_validation_should_stay_non_blocking_while_scan_input_remains_responsive(
    ) {
        let mut app = App::new(Some(PathBuf::from("/scan")), true, AppConfig::default());
        app.scan_status = ScanStatus::Scanning;

        let (scan_tx, scan_rx) = crossbeam_channel::unbounded();
        let scan_started_at = std::time::Instant::now();
        let active_scan = start_scan_processing(
            PathBuf::from("/scan"),
            scan_rx,
            RulesEngine::new(&[]).unwrap(),
            true,
        );

        let runtime_config = RuntimeConfig {
            scan_path: Some(PathBuf::from("/scan")),
            provider: Some(ResolvedProviderConfig::new(
                ProviderKind::OpenRouter,
                Some("test-key".to_string()),
                "google/gemini-2.0-flash-001".to_string(),
                spawn_http_server(
                    "200 OK",
                    r#"{"choices":[{"message":{"content":"[{\"path\":\"/tmp/purifier-validation\",\"category\":\"BuildArtifact\",\"safety\":\"Safe\",\"reason\":\"Validation probe\"}]"}}]}"#,
                    Some(std::time::Duration::from_millis(250)),
                ),
            )),
            show_onboarding: false,
            llm_enabled: true,
        };
        let classifier = Classifier::new(RulesEngine::new(&[]).unwrap(), None);

        apply_runtime_config(&mut app, &runtime_config);
        let validation_started_at = std::time::Instant::now();
        let validation_rx = start_runtime_connection_check(&app, &classifier, &runtime_config);

        assert!(scan_started_at.elapsed() < std::time::Duration::from_millis(100));
        assert!(validation_started_at.elapsed() < std::time::Duration::from_millis(100));
        assert!(validation_rx.is_some());

        crate::input::handle_key(
            &mut app,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('q'),
                crossterm::event::KeyModifiers::NONE,
            ),
        );

        assert!(app.should_quit);

        let _ = active_scan.cancel.send(());
        drop(scan_tx);
    }

    #[test]
    fn apply_runtime_connection_event_should_promote_connecting_provider_to_ready() {
        let provider = ResolvedProviderConfig::new(
            ProviderKind::OpenRouter,
            Some("test-key".to_string()),
            "google/gemini-2.0-flash-001".to_string(),
            "https://openrouter.ai/api/v1".to_string(),
        );
        let mut app = App::new(Some(PathBuf::from("/scan")), true, AppConfig::default());
        app.llm_status = LlmStatus::Connecting(ProviderKind::OpenRouter);
        app.llm_connection_generation = 1;
        let mut classifier = Classifier::new(RulesEngine::new(&[]).unwrap(), None);

        apply_runtime_connection_event(
            &mut app,
            &mut classifier,
            super::RuntimeConnectionEvent::Validated {
                generation: 1,
                provider: ProviderKind::OpenRouter,
                client: LlmClient::OpenRouter(OpenRouterClient::new(provider.clone())),
            },
        );

        assert_eq!(app.llm_status, LlmStatus::Ready(ProviderKind::OpenRouter));
        assert!(app.llm_online);
        assert!(classifier.has_llm());
    }

    #[test]
    fn apply_runtime_connection_event_should_surface_concise_diagnostic_failure_details() {
        let mut app = App::new(Some(PathBuf::from("/scan")), true, AppConfig::default());
        app.llm_status = LlmStatus::Connecting(ProviderKind::OpenRouter);
        app.llm_connection_generation = 2;
        let mut classifier = Classifier::new(RulesEngine::new(&[]).unwrap(), None);

        apply_runtime_connection_event(
            &mut app,
            &mut classifier,
            super::RuntimeConnectionEvent::Failed {
                generation: 2,
                provider: ProviderKind::OpenRouter,
                detail: super::RuntimeConnectionFailure::Http {
                    status: 400,
                    body: Some("The model gpt-missing does not exist".to_string()),
                },
            },
        );

        assert_eq!(
            app.llm_status,
            LlmStatus::Error("OpenRouter model failed".to_string())
        );
        assert!(!app.llm_online);
        assert!(!classifier.has_llm());
        assert_eq!(
            app.last_error.as_deref(),
            Some(
                "OpenRouter connection failed: HTTP 400 Bad Request - The model gpt-missing does not exist. Check the API key, model, base URL, or network, then update settings after scan completion."
            )
        );
    }

    #[test]
    fn apply_runtime_connection_event_should_ignore_stale_results_after_generation_changes() {
        let provider = ResolvedProviderConfig::new(
            ProviderKind::OpenRouter,
            Some("test-key".to_string()),
            "google/gemini-2.0-flash-001".to_string(),
            "https://openrouter.ai/api/v1".to_string(),
        );
        let mut app = App::new(Some(PathBuf::from("/scan")), true, AppConfig::default());
        app.llm_status = LlmStatus::Connecting(ProviderKind::OpenAI);
        app.llm_connection_generation = 9;
        let mut classifier = Classifier::new(RulesEngine::new(&[]).unwrap(), None);

        apply_runtime_connection_event(
            &mut app,
            &mut classifier,
            super::RuntimeConnectionEvent::Validated {
                generation: 8,
                provider: ProviderKind::OpenRouter,
                client: LlmClient::OpenRouter(OpenRouterClient::new(provider)),
            },
        );

        assert_eq!(app.llm_status, LlmStatus::Connecting(ProviderKind::OpenAI));
        assert!(!app.llm_online);
        assert!(!classifier.has_llm());
    }

    #[test]
    fn apply_runtime_connection_event_should_surface_invalid_response_failures() {
        let mut app = App::new(Some(PathBuf::from("/scan")), true, AppConfig::default());
        app.llm_status = LlmStatus::Connecting(ProviderKind::OpenAI);
        app.llm_connection_generation = 3;
        let mut classifier = Classifier::new(RulesEngine::new(&[]).unwrap(), None);

        apply_runtime_connection_event(
            &mut app,
            &mut classifier,
            super::RuntimeConnectionEvent::Failed {
                generation: 3,
                provider: ProviderKind::OpenAI,
                detail: super::RuntimeConnectionFailure::InvalidResponse(
                    "LLM validation response contained no choices".to_string(),
                ),
            },
        );

        assert_eq!(
            app.llm_status,
            LlmStatus::Error("OpenAI response failed".to_string())
        );
        assert_eq!(
            app.last_error.as_deref(),
            Some(
                "OpenAI connection failed: LLM validation response contained no choices. Check the API key, model, base URL, or network, then update settings after scan completion."
            )
        );
    }

    #[test]
    fn apply_runtime_config_should_mark_llm_disabled_for_rules_only_runtime() {
        let runtime_config = RuntimeConfig {
            scan_path: Some(PathBuf::from("/scan")),
            provider: None,
            show_onboarding: false,
            llm_enabled: false,
        };
        let mut app = App::new(Some(PathBuf::from("/scan")), true, AppConfig::default());

        apply_runtime_config(&mut app, &runtime_config);

        assert_eq!(app.llm_status, LlmStatus::Disabled);
        assert!(!app.llm_enabled);
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

#[cfg(test)]
mod resolve_runtime_config_tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::AppConfig;
    use crate::secrets::{FakeSecretStore, SecretStore, SecretStoreError};
    use purifier_core::provider::ProviderKind;

    struct PanicSecretStore;

    impl SecretStore for PanicSecretStore {
        fn load_api_key(&self, provider: ProviderKind) -> Result<Option<String>, SecretStoreError> {
            Err(SecretStoreError::Read {
                provider,
                message: "secret store should not be touched".to_string(),
            })
        }

        fn save_api_key(
            &mut self,
            provider: ProviderKind,
            _api_key: &str,
        ) -> Result<(), SecretStoreError> {
            Err(SecretStoreError::Write {
                provider,
                message: "not used in this test".to_string(),
            })
        }

        fn delete_api_key(&mut self, provider: ProviderKind) -> Result<(), SecretStoreError> {
            Err(SecretStoreError::Delete {
                provider,
                message: "not used in this test".to_string(),
            })
        }
    }

    #[test]
    fn resolve_runtime_config_should_prefer_cli_over_env_and_saved_settings() {
        let cli = Cli {
            path: Some(PathBuf::from("/scan")),
            rules: None,
            no_llm: false,
            api_key: Some("cli-key".to_string()),
            provider: Some(ProviderKind::Anthropic),
        };
        let mut saved = AppConfig::default();
        saved.llm.active_provider = ProviderKind::OpenAI;
        let env = EnvOverrides {
            openrouter_api_key: Some("env-key".to_string()),
            openai_api_key: None,
            anthropic_api_key: Some("env-anthropic".to_string()),
            gemini_api_key: None,
            google_api_key: None,
        };
        let mut store = FakeSecretStore::default();
        store
            .save_api_key(ProviderKind::Anthropic, "saved-key")
            .unwrap();

        assert_eq!(saved.llm.active_provider, ProviderKind::OpenAI);
        assert_eq!(
            store.load_api_key(ProviderKind::Anthropic).unwrap(),
            Some("saved-key".to_string())
        );

        let resolved = resolve_runtime_config(&cli, saved, &env, &store).unwrap();

        assert_eq!(
            resolved.provider.as_ref().unwrap().kind,
            ProviderKind::Anthropic
        );
        assert_eq!(
            resolved.provider.as_ref().unwrap().api_key.as_deref(),
            Some("cli-key")
        );
    }

    #[test]
    fn resolve_runtime_config_should_not_touch_secret_store_when_llm_disabled() {
        let cli = Cli {
            path: Some(PathBuf::from("/scan")),
            rules: None,
            no_llm: true,
            api_key: None,
            provider: Some(ProviderKind::OpenAI),
        };

        let resolved = resolve_runtime_config(
            &cli,
            AppConfig::default(),
            &EnvOverrides::default(),
            &PanicSecretStore,
        )
        .unwrap();

        assert!(!resolved.llm_enabled);
        assert!(resolved.provider.is_none());
    }

    #[test]
    fn resolve_runtime_config_should_not_let_secret_lookup_override_cli_api_key() {
        let cli = Cli {
            path: Some(PathBuf::from("/scan")),
            rules: None,
            no_llm: false,
            api_key: Some("cli-key".to_string()),
            provider: Some(ProviderKind::OpenRouter),
        };
        let mut saved = AppConfig::default();
        saved.llm.enabled = true;
        saved.onboarding.first_launch_prompt_dismissed = true;

        let resolved =
            resolve_runtime_config(&cli, saved, &EnvOverrides::default(), &PanicSecretStore)
                .expect("cli key should bypass secret-store reads");

        assert!(resolved.llm_enabled);
        assert_eq!(
            resolved
                .provider
                .as_ref()
                .and_then(|provider| provider.api_key.as_deref()),
            Some("cli-key")
        );
    }

    #[test]
    fn parse_provider_kind_should_reject_ollama_while_support_is_todo() {
        let error = parse_provider_kind("ollama").expect_err("ollama should be disabled");

        assert!(error.contains("temporarily disabled"));
    }

    #[test]
    fn finalize_runtime_config_should_keep_openai_enabled_when_api_key_is_present() {
        let runtime_config = RuntimeConfig {
            scan_path: Some(PathBuf::from("/scan")),
            provider: Some(ResolvedProviderConfig::new(
                ProviderKind::OpenAI,
                Some("sk-test".to_string()),
                "gpt-4o-mini".to_string(),
                "https://api.openai.com/v1".to_string(),
            )),
            show_onboarding: false,
            llm_enabled: true,
        };

        let (resolved, warning) = finalize_runtime_config(runtime_config, &AppConfig::default());

        assert!(resolved.llm_enabled);
        assert_eq!(
            resolved.provider.as_ref().map(|provider| provider.kind),
            Some(ProviderKind::OpenAI)
        );
        assert_eq!(warning, None);
    }

    #[test]
    fn finalize_runtime_config_should_warn_and_disable_ollama_while_support_is_todo() {
        let runtime_config = RuntimeConfig {
            scan_path: Some(PathBuf::from("/scan")),
            provider: Some(ResolvedProviderConfig::new(
                ProviderKind::Ollama,
                None,
                "llama3.1:8b".to_string(),
                "http://127.0.0.1:11434".to_string(),
            )),
            show_onboarding: false,
            llm_enabled: true,
        };

        let (resolved, warning) = finalize_runtime_config(runtime_config, &AppConfig::default());

        assert!(!resolved.llm_enabled);
        assert!(resolved.provider.is_none());
        assert_eq!(
            warning,
            Some(
                "Warning: Ollama support is temporarily disabled; continuing with rules-only classification"
                    .to_string()
            )
        );
    }

    #[test]
    fn load_runtime_config_should_warn_and_disable_llm_when_secret_lookup_fails() {
        let cli = Cli {
            path: Some(PathBuf::from("/scan")),
            rules: None,
            no_llm: false,
            api_key: None,
            provider: Some(ProviderKind::OpenRouter),
        };
        let mut saved = AppConfig::default();
        saved.llm.enabled = true;
        saved.onboarding.first_launch_prompt_dismissed = true;

        let (resolved, warning) =
            load_runtime_config(&cli, saved, &EnvOverrides::default(), &PanicSecretStore);

        assert!(!resolved.llm_enabled);
        assert!(resolved.provider.is_none());
        assert_eq!(warning, Some("Warning: failed to read API key for OpenRouter; continuing with rules-only classification".to_string()));
    }

    #[test]
    fn finalize_runtime_config_should_warn_and_disable_llm_when_openrouter_key_is_missing() {
        let runtime_config = RuntimeConfig {
            scan_path: Some(PathBuf::from("/scan")),
            provider: Some(ResolvedProviderConfig::new(
                ProviderKind::OpenRouter,
                None,
                "google/gemini-2.0-flash-001".to_string(),
                "https://openrouter.ai/api/v1".to_string(),
            )),
            show_onboarding: false,
            llm_enabled: true,
        };

        let (resolved, warning) = finalize_runtime_config(runtime_config, &AppConfig::default());

        assert!(!resolved.llm_enabled);
        assert!(resolved.provider.is_none());
        assert_eq!(
            warning,
            Some(
                "Warning: LLM classification is disabled until an API key is configured"
                    .to_string()
            )
        );
    }

    #[test]
    fn finalize_runtime_config_should_preserve_onboarding_gate_while_disabling_runtime_llm() {
        let runtime_config = RuntimeConfig {
            scan_path: Some(PathBuf::from("/scan")),
            provider: None,
            show_onboarding: true,
            llm_enabled: true,
        };

        let (resolved, warning) = finalize_runtime_config(runtime_config, &AppConfig::default());

        assert!(resolved.show_onboarding);
        assert!(!resolved.llm_enabled);
        assert!(resolved.provider.is_none());
        assert_eq!(
            warning,
            Some(
                "Warning: LLM classification is disabled until an API key is configured"
                    .to_string()
            )
        );
    }
}

#[cfg(test)]
mod persist_settings_tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use purifier_core::classifier::Classifier;
    use purifier_core::provider::ProviderKind;
    use purifier_core::rules::RulesEngine;
    use purifier_core::{Filter, FilterTest, ScanProfile, SizeMode};

    use super::persist_settings;
    use crate::app::{App, LlmStatus, PreviewMode, SettingsDraft};
    use crate::config::AppConfig;
    use crate::secrets::{FakeSecretStore, SecretStore, SecretStoreError};

    fn spawn_http_server(
        status_line: &'static str,
        body: &'static str,
        delay_before_response: Option<std::time::Duration>,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        let address = listener
            .local_addr()
            .expect("test server should have a local address");

        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("test server should accept");
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer);
            if let Some(delay) = delay_before_response {
                std::thread::sleep(delay);
            }
            let response = format!(
                "HTTP/1.1 {status_line}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("test server should respond");
        });

        format!("http://{address}")
    }

    struct FailingSecretStore;

    impl SecretStore for FailingSecretStore {
        fn load_api_key(
            &self,
            _provider: ProviderKind,
        ) -> Result<Option<String>, SecretStoreError> {
            Ok(None)
        }

        fn save_api_key(
            &mut self,
            provider: ProviderKind,
            _api_key: &str,
        ) -> Result<(), SecretStoreError> {
            Err(SecretStoreError::Write {
                provider,
                message: "boom".to_string(),
            })
        }

        fn delete_api_key(&mut self, provider: ProviderKind) -> Result<(), SecretStoreError> {
            Err(SecretStoreError::Delete {
                provider,
                message: "boom".to_string(),
            })
        }
    }

    #[test]
    fn persist_settings_should_store_provider_preferences_and_api_key() {
        let tempdir = tempfile::tempdir().unwrap();
        let config_path = tempdir.path().join("config.toml");
        let mut preferences = AppConfig::default();
        preferences.onboarding.first_launch_prompt_dismissed = false;
        let draft = SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: "or-key".to_string(),
            api_key_edited: true,
            api_key_editing: false,
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            llm_enabled: true,
            size_mode: SizeMode::Logical,
            selected_scan_profile: Some("exclude-node-modules".to_string()),
        };
        preferences.ui.scan_profiles = vec![ScanProfile {
            name: "exclude-node-modules".to_string(),
            exclude: Some(Filter::single(FilterTest::PathGlob(
                "**/node_modules/**".to_string(),
            ))),
            mask: None,
            display_filter: None,
        }];
        let mut secrets = FakeSecretStore::default();

        persist_settings(&config_path, &mut preferences, draft, &mut secrets).unwrap();

        let provider = preferences
            .llm
            .providers
            .get(&ProviderKind::OpenRouter)
            .unwrap();

        assert_eq!(preferences.llm.active_provider, ProviderKind::OpenRouter);
        assert_eq!(provider.model, "google/gemini-2.0-flash-001");
        assert_eq!(provider.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(
            secrets.load_api_key(ProviderKind::OpenRouter).unwrap(),
            Some("or-key".to_string())
        );
        assert!(preferences.onboarding.first_launch_prompt_dismissed);
        assert!(config_path.exists());
        let loaded = AppConfig::load_or_default(&config_path).unwrap();
        assert_eq!(loaded.llm.active_provider, ProviderKind::OpenRouter);
        assert_eq!(loaded.ui.size_mode, SizeMode::Logical);
        assert_eq!(
            loaded.ui.last_selected_scan_profile.as_deref(),
            Some("exclude-node-modules")
        );
        assert!(loaded.onboarding.first_launch_prompt_dismissed);
    }

    #[test]
    fn persist_settings_should_keep_existing_secret_when_api_key_is_untouched() {
        let tempdir = tempfile::tempdir().unwrap();
        let config_path = tempdir.path().join("config.toml");
        let mut preferences = AppConfig::default();
        let draft = SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: String::new(),
            api_key_edited: false,
            api_key_editing: false,
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            llm_enabled: true,
            size_mode: SizeMode::Physical,
            selected_scan_profile: None,
        };
        let mut secrets = FakeSecretStore::default();
        secrets
            .save_api_key(ProviderKind::OpenRouter, "saved-key")
            .unwrap();

        persist_settings(&config_path, &mut preferences, draft, &mut secrets).unwrap();

        assert_eq!(
            secrets.load_api_key(ProviderKind::OpenRouter).unwrap(),
            Some("saved-key".to_string())
        );
    }

    #[test]
    fn persist_settings_should_restore_previous_secret_when_config_write_fails() {
        let tempdir = tempfile::tempdir().unwrap();
        let parent_path = tempdir.path().join("parent-file");
        std::fs::write(&parent_path, "not a directory").unwrap();
        let config_path = parent_path.join("config.toml");
        let mut preferences = AppConfig::default();
        let draft = SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: "new-key".to_string(),
            api_key_edited: true,
            api_key_editing: false,
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            llm_enabled: true,
            size_mode: SizeMode::Physical,
            selected_scan_profile: None,
        };
        let mut secrets = FakeSecretStore::default();
        secrets
            .save_api_key(ProviderKind::OpenRouter, "old-key")
            .unwrap();

        let result = persist_settings(&config_path, &mut preferences, draft, &mut secrets);

        assert!(result.is_err());
        assert_eq!(
            secrets.load_api_key(ProviderKind::OpenRouter).unwrap(),
            Some("old-key".to_string())
        );
    }

    #[test]
    fn save_settings_should_keep_modal_open_when_persistence_fails() {
        let tempdir = tempfile::tempdir().unwrap();
        let config_path = tempdir.path().join("config.toml");
        let draft = SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: "or-key".to_string(),
            api_key_edited: true,
            api_key_editing: false,
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: spawn_http_server(
                "200 OK",
                r#"{"choices":[{"message":{"content":"[{\"path\":\"/tmp/purifier-validation\",\"category\":\"BuildArtifact\",\"safety\":\"Safe\",\"reason\":\"Validation probe\"}]"}}]}"#,
                None,
            ),
            llm_enabled: true,
            size_mode: SizeMode::Physical,
            selected_scan_profile: None,
        };
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            true,
            AppConfig::default(),
        );
        app.preview_mode = PreviewMode::Settings(draft.clone());

        let mut classifier = Classifier::new(RulesEngine::new(&[]).unwrap(), None);
        let mut runtime_connection_rx = None;
        let result = super::apply_settings_save(
            &mut app,
            &config_path,
            draft,
            &mut FailingSecretStore,
            &mut classifier,
            &mut runtime_connection_rx,
            &super::Cli {
                path: None,
                rules: None,
                no_llm: false,
                api_key: None,
                provider: None,
            },
            &super::EnvOverrides::default(),
        );

        assert!(result.is_err());
        assert!(matches!(app.preview_mode, PreviewMode::Settings(_)));
        assert_eq!(
            app.last_error.as_deref(),
            Some("Could not save settings: failed to write key for OpenRouter: boom")
        );
    }

    #[test]
    fn apply_settings_save_should_update_live_app_state_conservatively_after_success() {
        let tempdir = tempfile::tempdir().unwrap();
        let config_path = tempdir.path().join("config.toml");
        let draft = SettingsDraft {
            provider: ProviderKind::Anthropic,
            api_key: "anthropic-key".to_string(),
            api_key_edited: true,
            api_key_editing: false,
            model: "claude-3-5-haiku-latest".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            llm_enabled: true,
            size_mode: SizeMode::Physical,
            selected_scan_profile: None,
        };
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            true,
            AppConfig::default(),
        );
        app.preview_mode = PreviewMode::Settings(draft.clone());
        app.llm_status = LlmStatus::Ready(ProviderKind::OpenRouter);
        app.llm_enabled = true;
        let mut secrets = FakeSecretStore::default();

        let mut classifier = Classifier::new(RulesEngine::new(&[]).unwrap(), None);
        let mut runtime_connection_rx = None;

        super::apply_settings_save(
            &mut app,
            &config_path,
            draft,
            &mut secrets,
            &mut classifier,
            &mut runtime_connection_rx,
            &super::Cli {
                path: None,
                rules: None,
                no_llm: false,
                api_key: None,
                provider: None,
            },
            &super::EnvOverrides::default(),
        )
        .unwrap();

        assert!(matches!(app.preview_mode, PreviewMode::Analytics));
        assert_eq!(app.preferences.llm.active_provider, ProviderKind::Anthropic);
        assert!(!app.llm_enabled);
        assert_eq!(app.llm_status, LlmStatus::Disabled);
        assert!(app.last_error.is_none());
        assert!(runtime_connection_rx.is_none());
        assert_eq!(
            app.last_warning.as_deref(),
            Some(
                "Anthropic is not wired into runtime classification yet; continuing with rules-only classification"
            )
        );
    }

    #[test]
    fn apply_settings_save_should_refresh_live_openrouter_runtime_after_success() {
        let tempdir = tempfile::tempdir().unwrap();
        let config_path = tempdir.path().join("config.toml");
        let draft = SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: "or-key".to_string(),
            api_key_edited: true,
            api_key_editing: false,
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: spawn_http_server(
                "200 OK",
                r#"{"choices":[{"message":{"content":"[{\"path\":\"/tmp/purifier-validation\",\"category\":\"BuildArtifact\",\"safety\":\"Safe\",\"reason\":\"Validation probe\"}]"}}]}"#,
                None,
            ),
            llm_enabled: true,
            size_mode: SizeMode::Physical,
            selected_scan_profile: None,
        };
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            false,
            AppConfig::default(),
        );
        app.preview_mode = PreviewMode::Settings(draft.clone());
        let mut classifier = Classifier::new(RulesEngine::new(&[]).unwrap(), None);
        let mut secrets = FakeSecretStore::default();
        let mut runtime_connection_rx = None;

        super::apply_settings_save(
            &mut app,
            &config_path,
            draft,
            &mut secrets,
            &mut classifier,
            &mut runtime_connection_rx,
            &super::Cli {
                path: None,
                rules: None,
                no_llm: false,
                api_key: None,
                provider: None,
            },
            &super::EnvOverrides::default(),
        )
        .unwrap();

        assert!(matches!(app.preview_mode, PreviewMode::Settings(_)));
        assert_eq!(
            app.preferences.llm.active_provider,
            ProviderKind::OpenRouter
        );
        assert!(app.llm_enabled);
        assert_eq!(
            app.llm_status,
            LlmStatus::Connecting(ProviderKind::OpenRouter)
        );
        assert!(!classifier.has_llm());
        let event = runtime_connection_rx
            .as_ref()
            .expect("validation should start for live provider")
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("validation result should arrive");
        super::apply_runtime_connection_event(&mut app, &mut classifier, event);
        assert!(matches!(app.preview_mode, PreviewMode::Analytics));
        assert_eq!(app.llm_status, LlmStatus::Ready(ProviderKind::OpenRouter));
        assert!(classifier.has_llm());
    }

    #[test]
    fn apply_settings_save_should_surface_live_connection_failures_after_persisting() {
        let tempdir = tempfile::tempdir().unwrap();
        let config_path = tempdir.path().join("config.toml");
        let draft = SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: "bad-key".to_string(),
            api_key_edited: true,
            api_key_editing: false,
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: spawn_http_server(
                "401 Unauthorized",
                r#"{"error":{"message":"Invalid API key"}}"#,
                None,
            ),
            llm_enabled: true,
            size_mode: SizeMode::Physical,
            selected_scan_profile: None,
        };
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            false,
            AppConfig::default(),
        );
        app.preview_mode = PreviewMode::Settings(draft.clone());
        let mut classifier = Classifier::new(RulesEngine::new(&[]).unwrap(), None);
        let mut secrets = FakeSecretStore::default();
        let mut runtime_connection_rx = None;

        super::apply_settings_save(
            &mut app,
            &config_path,
            draft,
            &mut secrets,
            &mut classifier,
            &mut runtime_connection_rx,
            &super::Cli {
                path: None,
                rules: None,
                no_llm: false,
                api_key: None,
                provider: None,
            },
            &super::EnvOverrides::default(),
        )
        .unwrap();

        assert!(matches!(app.preview_mode, PreviewMode::Settings(_)));
        assert!(app.llm_enabled);
        assert_eq!(
            app.llm_status,
            LlmStatus::Connecting(ProviderKind::OpenRouter)
        );
        assert!(!app.llm_online);
        assert!(!classifier.has_llm());
        let event = runtime_connection_rx
            .as_ref()
            .expect("validation should start for live provider")
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("validation result should arrive");
        super::apply_runtime_connection_event(&mut app, &mut classifier, event);
        assert_eq!(
            app.llm_status,
            LlmStatus::Error("OpenRouter auth failed".to_string())
        );
        assert_eq!(
            app.settings_modal_error.as_deref(),
            Some(
                "OpenRouter connection failed: HTTP 401 Unauthorized - Invalid API key. Update the API key or provider and save again."
            )
        );
        assert!(matches!(app.preview_mode, PreviewMode::Settings(_)));
    }

    #[test]
    fn apply_settings_save_should_only_mark_live_provider_ready_after_connection_succeeds() {
        let tempdir = tempfile::tempdir().unwrap();
        let config_path = tempdir.path().join("config.toml");
        let draft = SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: "or-key".to_string(),
            api_key_edited: true,
            api_key_editing: false,
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: spawn_http_server(
                "200 OK",
                r#"{"choices":[{"message":{"content":"[{\"path\":\"/tmp/purifier-validation\",\"category\":\"BuildArtifact\",\"safety\":\"Safe\",\"reason\":\"Validation probe\"}]"}}]}"#,
                None,
            ),
            llm_enabled: true,
            size_mode: SizeMode::Physical,
            selected_scan_profile: None,
        };
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            false,
            AppConfig::default(),
        );
        app.preview_mode = PreviewMode::Settings(draft.clone());
        let mut classifier = Classifier::new(RulesEngine::new(&[]).unwrap(), None);
        let mut secrets = FakeSecretStore::default();
        let mut runtime_connection_rx = None;

        super::apply_settings_save(
            &mut app,
            &config_path,
            draft,
            &mut secrets,
            &mut classifier,
            &mut runtime_connection_rx,
            &super::Cli {
                path: None,
                rules: None,
                no_llm: false,
                api_key: None,
                provider: None,
            },
            &super::EnvOverrides::default(),
        )
        .unwrap();

        assert!(matches!(app.preview_mode, PreviewMode::Settings(_)));
        assert!(app.llm_enabled);
        assert_eq!(
            app.llm_status,
            LlmStatus::Connecting(ProviderKind::OpenRouter)
        );
        assert!(!app.llm_online);
        assert!(!classifier.has_llm());
        let event = runtime_connection_rx
            .as_ref()
            .expect("validation should start for live provider")
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("validation result should arrive");
        super::apply_runtime_connection_event(&mut app, &mut classifier, event);
        assert!(matches!(app.preview_mode, PreviewMode::Analytics));
        assert_eq!(app.llm_status, LlmStatus::Ready(ProviderKind::OpenRouter));
        assert!(app.llm_online);
        assert!(classifier.has_llm());
        assert!(app.last_error.is_none());
    }

    #[test]
    fn apply_settings_save_should_preserve_session_cli_overrides_during_runtime_refresh() {
        let tempdir = tempfile::tempdir().unwrap();
        let config_path = tempdir.path().join("config.toml");
        let draft = SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: "or-key".to_string(),
            api_key_edited: true,
            api_key_editing: false,
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            llm_enabled: true,
            size_mode: SizeMode::Physical,
            selected_scan_profile: None,
        };
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            false,
            AppConfig::default(),
        );
        app.preview_mode = PreviewMode::Settings(draft.clone());
        let mut classifier = Classifier::new(RulesEngine::new(&[]).unwrap(), None);
        let mut secrets = FakeSecretStore::default();
        let mut runtime_connection_rx = None;
        let cli = super::Cli {
            path: None,
            rules: None,
            no_llm: true,
            api_key: None,
            provider: None,
        };

        super::apply_settings_save(
            &mut app,
            &config_path,
            draft,
            &mut secrets,
            &mut classifier,
            &mut runtime_connection_rx,
            &cli,
            &super::EnvOverrides::default(),
        )
        .unwrap();

        assert!(!app.llm_enabled);
        assert_eq!(app.llm_status, LlmStatus::Disabled);
        assert!(!classifier.has_llm());
        assert!(runtime_connection_rx.is_none());
        assert_eq!(
            app.last_warning.as_deref(),
            Some(
                "Launch-time CLI/env overrides still control the live runtime; restart without overrides to use saved settings"
            )
        );
    }

    #[test]
    fn apply_settings_save_should_close_modal_when_cli_override_changes_live_runtime_target() {
        let tempdir = tempfile::tempdir().unwrap();
        let config_path = tempdir.path().join("config.toml");
        let draft = SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: "or-key".to_string(),
            api_key_edited: true,
            api_key_editing: false,
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            llm_enabled: true,
            size_mode: SizeMode::Physical,
            selected_scan_profile: None,
        };
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            false,
            AppConfig::default(),
        );
        app.preview_mode = PreviewMode::Settings(draft.clone());
        let mut classifier = Classifier::new(RulesEngine::new(&[]).unwrap(), None);
        let mut secrets = FakeSecretStore::default();
        let mut runtime_connection_rx = None;
        let cli = super::Cli {
            path: None,
            rules: None,
            no_llm: false,
            api_key: Some("cli-openai-key".to_string()),
            provider: Some(ProviderKind::OpenAI),
        };

        super::apply_settings_save(
            &mut app,
            &config_path,
            draft,
            &mut secrets,
            &mut classifier,
            &mut runtime_connection_rx,
            &cli,
            &super::EnvOverrides::default(),
        )
        .unwrap();

        assert!(matches!(app.preview_mode, PreviewMode::Analytics));
        assert!(!app.settings_modal_is_saving);
        assert!(app.pending_settings_validation_generation.is_none());
        assert_eq!(app.llm_status, LlmStatus::Connecting(ProviderKind::OpenAI));
        assert!(runtime_connection_rx.is_some());
        assert_eq!(
            app.last_warning.as_deref(),
            Some(
                "Launch-time CLI/env overrides still control the live runtime; restart without overrides to use saved settings"
            )
        );
    }

    #[test]
    fn apply_settings_save_should_close_modal_when_env_override_controls_live_runtime() {
        let tempdir = tempfile::tempdir().unwrap();
        let config_path = tempdir.path().join("config.toml");
        let draft = SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: "saved-or-key".to_string(),
            api_key_edited: true,
            api_key_editing: false,
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            llm_enabled: true,
            size_mode: SizeMode::Physical,
            selected_scan_profile: None,
        };
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            false,
            AppConfig::default(),
        );
        app.preview_mode = PreviewMode::Settings(draft.clone());
        let mut classifier = Classifier::new(RulesEngine::new(&[]).unwrap(), None);
        let mut secrets = FakeSecretStore::default();
        let mut runtime_connection_rx = None;
        let env = super::EnvOverrides {
            openrouter_api_key: Some("env-openrouter-key".to_string()),
            ..super::EnvOverrides::default()
        };

        super::apply_settings_save(
            &mut app,
            &config_path,
            draft,
            &mut secrets,
            &mut classifier,
            &mut runtime_connection_rx,
            &super::Cli {
                path: None,
                rules: None,
                no_llm: false,
                api_key: None,
                provider: None,
            },
            &env,
        )
        .unwrap();

        assert!(matches!(app.preview_mode, PreviewMode::Analytics));
        assert!(!app.settings_modal_is_saving);
        assert!(app.pending_settings_validation_generation.is_none());
        assert_eq!(
            app.llm_status,
            LlmStatus::Connecting(ProviderKind::OpenRouter)
        );
        assert!(runtime_connection_rx.is_some());
        assert_eq!(
            app.last_warning.as_deref(),
            Some(
                "Launch-time CLI/env overrides still control the live runtime; restart without overrides to use saved settings"
            )
        );
    }

    #[test]
    fn apply_onboarding_skip_should_refresh_runtime_state_after_dismissal() {
        let tempdir = tempfile::tempdir().unwrap();
        let config_path = tempdir.path().join("config.toml");
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            true,
            AppConfig::default(),
        );
        app.llm_status = LlmStatus::NeedsSetup;
        app.preferences.onboarding.first_launch_prompt_dismissed = false;
        let mut classifier = Classifier::new(RulesEngine::new(&[]).unwrap(), None);
        let mut secrets = FakeSecretStore::default();
        let mut runtime_connection_rx = None;

        super::apply_onboarding_skip(
            &mut app,
            &config_path,
            &mut secrets,
            &mut classifier,
            &mut runtime_connection_rx,
            &super::Cli {
                path: None,
                rules: None,
                no_llm: false,
                api_key: None,
                provider: None,
            },
            &super::EnvOverrides::default(),
        )
        .unwrap();

        assert!(app.preferences.onboarding.first_launch_prompt_dismissed);
        assert_eq!(app.llm_status, LlmStatus::Disabled);
        assert!(!app.llm_enabled);
        assert!(runtime_connection_rx.is_none());
    }
}
