use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use purifier_core::provider::{default_provider_settings, ProviderKind};
use purifier_core::size::SizeMode;

use crate::app::{App, AppScreen, PreviewMode, ScanStatus, SettingsDraft};

pub enum InputResult {
    None,
    StartScan(PathBuf),
    SaveSettings(SettingsDraft),
    SkipOnboarding,
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> InputResult {
    match app.screen {
        AppScreen::Onboarding => handle_onboarding(app, key),
        AppScreen::DirPicker => {
            if app.dir_picker_typing {
                handle_custom_input(app, key)
            } else {
                handle_dir_picker(app, key)
            }
        }
        AppScreen::Main => handle_main(app, key),
    }
}

pub fn handle_mouse(app: &mut App, mouse: MouseEvent) {
    if !matches!(app.screen, AppScreen::Main) {
        return;
    }
    if !matches!(app.preview_mode, PreviewMode::Analytics) {
        return;
    }

    let count = app.current_children_count();
    match mouse.kind {
        MouseEventKind::ScrollDown => app.columns.move_selection(1, count),
        MouseEventKind::ScrollUp => app.columns.move_selection(-1, count),
        _ => {}
    }
}

// -- Main screen --

fn handle_main(app: &mut App, key: KeyEvent) -> InputResult {
    match &app.preview_mode {
        PreviewMode::Analytics => handle_main_analytics(app, key),
        PreviewMode::DeleteConfirm(_) => {
            handle_delete_confirm(app, key);
            InputResult::None
        }
        PreviewMode::BatchReview => {
            handle_batch_review(app, key);
            InputResult::None
        }
        PreviewMode::Settings(_) => handle_settings(app, key),
        PreviewMode::Onboarding(_) => handle_onboarding_preview(app, key),
    }
}

fn handle_main_analytics(app: &mut App, key: KeyEvent) -> InputResult {
    if app.scan_status == ScanStatus::Scanning {
        if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
            app.should_quit = true;
        }
        return InputResult::None;
    }

    let count = app.current_children_count();

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,

        // Navigation
        KeyCode::Char('j') | KeyCode::Down => app.columns.move_selection(1, count),
        KeyCode::Char('k') | KeyCode::Up => app.columns.move_selection(-1, count),
        KeyCode::Char('g') => app.columns.jump_top(),
        KeyCode::Char('G') => app.columns.jump_bottom(count),

        // Enter directory / step into
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
            if let Some(entry) = app.selected_entry() {
                if entry.is_dir {
                    let path = entry.path.clone();
                    app.columns.enter(path);
                }
            }
        }

        // Go back to parent
        KeyCode::Char('h') | KeyCode::Left => {
            app.columns.back();
        }

        // Go to home directory
        KeyCode::Char('~') => {
            if let Some(home) = dirs::home_dir() {
                if home.starts_with(&app.scan_path) || app.scan_path.starts_with(&home) {
                    app.columns
                        .navigate_to(&home, &app.entries, app.size_mode());
                }
            }
        }

        // Sort
        KeyCode::Char('s') => {
            app.columns.sort_key = app.columns.sort_key.cycle();
            app.preferences.ui.sort_key = app.columns.sort_key;
        }

        // Size mode toggle
        KeyCode::Char('i') => {
            app.preferences.ui.size_mode = match app.size_mode() {
                SizeMode::Physical => SizeMode::Logical,
                SizeMode::Logical => SizeMode::Physical,
            };
            app.sync_display_size_state();
        }

        // Delete
        KeyCode::Char('d') => {
            if let Some(path) = app.selected_path() {
                app.preview_mode = PreviewMode::DeleteConfirm(path);
            }
        }

        // Mark for batch
        KeyCode::Char(' ') => {
            if let Some(path) = app.selected_path() {
                app.marks.toggle(&path);
            }
        }

        // Execute batch
        KeyCode::Char('x') => {
            if !app.marks.is_empty() {
                app.batch_review_selected = 0;
                app.preview_mode = PreviewMode::BatchReview;
            }
        }

        // Clear marks
        KeyCode::Char('u') => {
            app.marks.clear();
        }

        // Settings
        KeyCode::Char(',') => {
            app.open_settings();
        }

        _ => {}
    }

    InputResult::None
}

fn handle_delete_confirm(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            let path = match &app.preview_mode {
                PreviewMode::DeleteConfirm(p) => p.clone(),
                _ => return,
            };
            execute_single_delete(app, &path);
            app.preview_mode = PreviewMode::Analytics;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.preview_mode = PreviewMode::Analytics;
        }
        _ => {}
    }
}

fn handle_batch_review(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            let paths = app.marks.paths();
            for path in &paths {
                execute_single_delete(app, path);
            }
            app.marks.clear();
            app.preview_mode = PreviewMode::Analytics;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.preview_mode = PreviewMode::Analytics;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let count = app.marks.count();
            if count > 0 && app.batch_review_selected < count - 1 {
                app.batch_review_selected += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if app.batch_review_selected > 0 {
                app.batch_review_selected -= 1;
            }
        }
        KeyCode::Char(' ') => {
            // Unmark individual from batch review
            let paths = app.marks.paths();
            if let Some(path) = paths.get(app.batch_review_selected) {
                app.marks.remove(path);
                if app.marks.is_empty() {
                    app.preview_mode = PreviewMode::Analytics;
                } else if app.batch_review_selected >= app.marks.count() {
                    app.batch_review_selected = app.marks.count().saturating_sub(1);
                }
            }
        }
        _ => {}
    }
}

fn execute_single_delete(app: &mut App, path: &std::path::Path) {
    // Get sizes before delete for accounting
    // Sizes are accounted for by the delete_entry outcome
    let _ = app.entry_at_path(path);

    match purifier_core::delete_entry(path) {
        Ok(outcome) => {
            app.delete_stats.logical_bytes_removed += outcome.logical_bytes_removed;
            app.delete_stats.physical_bytes_estimated += outcome.physical_bytes_estimated;
            app.delete_stats.physical_bytes_freed += outcome.physical_bytes_freed;
            app.total_logical_size = app
                .total_logical_size
                .saturating_sub(outcome.logical_bytes_removed);
            app.total_physical_size = app
                .total_physical_size
                .saturating_sub(outcome.physical_bytes_estimated);
            app.sync_display_size_state();
            app.total_files = app.total_files.saturating_sub(outcome.entries_removed);
            app.last_error = None;
            app.mark_deleted(path);
            app.remove_entry_by_path(path);
            app.rebuild_size_cache();
            // Adjust selection if it's now out of bounds
            let count = app.current_children_count();
            if count > 0 && app.columns.current().selected_index >= count {
                app.columns.current_mut().selected_index = count - 1;
            }
        }
        Err(error) => {
            app.last_error = Some(format!("Could not delete {}: {error}", path.display()));
        }
    }
}

// -- Settings in preview pane --

fn handle_settings(app: &mut App, key: KeyEvent) -> InputResult {
    if app.settings_modal_is_saving {
        return InputResult::None;
    }

    // Check if we're in API key editing mode
    let is_editing = matches!(&app.preview_mode, PreviewMode::Settings(d) | PreviewMode::Onboarding(d) if d.api_key_editing);

    if is_editing {
        return handle_api_key_editing(app, key);
    }

    if key.code == KeyCode::Enter {
        let draft = match &app.preview_mode {
            PreviewMode::Settings(d) => d.clone(),
            _ => return InputResult::None,
        };
        if draft.api_key_editing {
            if let PreviewMode::Settings(d) = &mut app.preview_mode {
                d.api_key_editing = false;
            }
            return InputResult::None;
        }
        app.settings_modal_error = None;
        app.last_error = None;
        return InputResult::SaveSettings(draft);
    }

    if key.code == KeyCode::Esc {
        app.close_preview_modal();
        return InputResult::None;
    }

    // Provider switching
    let provider_switch = match key.code {
        KeyCode::Char('1') => Some(ProviderKind::OpenRouter),
        KeyCode::Char('2') => Some(ProviderKind::OpenAI),
        KeyCode::Char('3') => Some(ProviderKind::Anthropic),
        KeyCode::Char('4') => Some(ProviderKind::Google),
        _ => None,
    };

    if let Some(provider) = provider_switch {
        let settings = app
            .preferences
            .llm
            .providers
            .get(&provider)
            .cloned()
            .unwrap_or_else(|| default_provider_settings(provider));
        if let PreviewMode::Settings(draft) = &mut app.preview_mode {
            apply_provider_defaults(draft, provider, settings);
            app.settings_modal_error = None;
            app.last_error = None;
        }
        return InputResult::None;
    }

    match key.code {
        KeyCode::Char('a') => {
            if let PreviewMode::Settings(draft) = &mut app.preview_mode {
                draft.api_key_editing = true;
                app.settings_modal_error = None;
                app.last_error = None;
            }
        }
        KeyCode::Char('m') => {
            if let PreviewMode::Settings(draft) = &mut app.preview_mode {
                draft.size_mode = match draft.size_mode {
                    SizeMode::Physical => SizeMode::Logical,
                    SizeMode::Logical => SizeMode::Physical,
                };
            }
        }
        KeyCode::Char('p') => {
            if let PreviewMode::Settings(draft) = &mut app.preview_mode {
                draft.selected_scan_profile = next_scan_profile_name(
                    &app.preferences.ui.scan_profiles,
                    draft.selected_scan_profile.as_deref(),
                );
            }
        }
        _ => {}
    }

    InputResult::None
}

// -- Onboarding screen (standalone) --

fn handle_onboarding(app: &mut App, key: KeyEvent) -> InputResult {
    // If preview mode isn't Onboarding yet, set it up
    if !matches!(app.preview_mode, PreviewMode::Onboarding(_)) {
        app.open_onboarding();
    }

    let is_editing = matches!(&app.preview_mode, PreviewMode::Onboarding(d) if d.api_key_editing);
    if is_editing {
        return handle_api_key_editing(app, key);
    }

    if key.code == KeyCode::Enter {
        let draft = match &app.preview_mode {
            PreviewMode::Onboarding(d) => d.clone(),
            _ => return InputResult::None,
        };

        if draft.api_key.is_empty() {
            app.settings_modal_error =
                Some("Enter an API key or press Esc to skip".to_string());
            app.last_error = Some("Enter an API key or press Esc to skip".to_string());
            return InputResult::None;
        }

        app.settings_modal_error = None;
        app.last_error = None;
        app.screen = AppScreen::DirPicker;
        return InputResult::SaveSettings(draft);
    }

    if key.code == KeyCode::Esc {
        app.close_preview_modal();
        app.screen = AppScreen::DirPicker;
        return InputResult::SkipOnboarding;
    }

    // Provider switching
    let provider_switch = match key.code {
        KeyCode::Char('1') => Some(ProviderKind::OpenRouter),
        KeyCode::Char('2') => Some(ProviderKind::OpenAI),
        KeyCode::Char('3') => Some(ProviderKind::Anthropic),
        KeyCode::Char('4') => Some(ProviderKind::Google),
        _ => None,
    };

    if let Some(provider) = provider_switch {
        let settings = app
            .preferences
            .llm
            .providers
            .get(&provider)
            .cloned()
            .unwrap_or_else(|| default_provider_settings(provider));
        if let PreviewMode::Onboarding(draft) = &mut app.preview_mode {
            apply_provider_defaults(draft, provider, settings);
            app.settings_modal_error = None;
            app.last_error = None;
        }
        return InputResult::None;
    }

    if key.code == KeyCode::Char('a') {
        if let PreviewMode::Onboarding(draft) = &mut app.preview_mode {
            draft.api_key_editing = true;
            app.settings_modal_error = None;
            app.last_error = None;
        }
    }

    InputResult::None
}

fn handle_onboarding_preview(app: &mut App, key: KeyEvent) -> InputResult {
    // Delegate to the onboarding handler since the logic is the same
    handle_onboarding(app, key)
}

// -- API key editing (shared between settings and onboarding) --

fn handle_api_key_editing(app: &mut App, key: KeyEvent) -> InputResult {
    let draft = match &mut app.preview_mode {
        PreviewMode::Settings(d) | PreviewMode::Onboarding(d) => d,
        _ => return InputResult::None,
    };

    match key.code {
        KeyCode::Tab | KeyCode::Esc => {
            draft.api_key_editing = false;
        }
        KeyCode::Enter => {
            draft.api_key_editing = false;
        }
        KeyCode::Char(c) => {
            draft.api_key.push(c);
            draft.api_key_edited = true;
            app.settings_modal_error = None;
            app.last_error = None;
        }
        KeyCode::Backspace | KeyCode::Delete => {
            draft.api_key.pop();
            draft.api_key_edited = true;
            app.settings_modal_error = None;
            app.last_error = None;
        }
        _ => {}
    }

    InputResult::None
}

// -- Dir picker --

fn handle_dir_picker(app: &mut App, key: KeyEvent) -> InputResult {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => {
            app.should_quit = true;
            InputResult::None
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if app.dir_picker_selected < app.dir_picker_options.len().saturating_sub(1) {
                app.dir_picker_selected += 1;
            }
            InputResult::None
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if app.dir_picker_selected > 0 {
                app.dir_picker_selected -= 1;
            }
            InputResult::None
        }
        KeyCode::Enter => {
            let path = app.dir_picker_options[app.dir_picker_selected].clone();
            app.start_scan_with_path(path.clone());
            InputResult::StartScan(path)
        }
        KeyCode::Char('/') => {
            app.dir_picker_typing = true;
            app.dir_picker_custom.clear();
            InputResult::None
        }
        _ => InputResult::None,
    }
}

fn handle_custom_input(app: &mut App, key: KeyEvent) -> InputResult {
    match key.code {
        KeyCode::Esc => {
            app.dir_picker_typing = false;
            app.dir_picker_custom.clear();
            InputResult::None
        }
        KeyCode::Enter => {
            let raw = app.dir_picker_custom.trim().to_string();
            if raw.is_empty() {
                app.dir_picker_typing = false;
                return InputResult::None;
            }
            let path = if let Some(stripped) = raw.strip_prefix("~/") {
                if let Some(home) = dirs::home_dir() {
                    home.join(stripped)
                } else {
                    PathBuf::from(&raw)
                }
            } else {
                PathBuf::from(&raw)
            };

            if path.exists() && path.is_dir() {
                app.dir_picker_typing = false;
                app.start_scan_with_path(path.clone());
                InputResult::StartScan(path)
            } else {
                InputResult::None
            }
        }
        KeyCode::Backspace => {
            app.dir_picker_custom.pop();
            InputResult::None
        }
        KeyCode::Char(c) => {
            app.dir_picker_custom.push(c);
            InputResult::None
        }
        _ => InputResult::None,
    }
}

// -- Helpers --

fn apply_provider_defaults(
    draft: &mut SettingsDraft,
    provider: ProviderKind,
    settings: purifier_core::provider::ProviderSettings,
) {
    draft.provider = provider;
    draft.api_key.clear();
    draft.api_key_edited = false;
    draft.api_key_editing = false;
    draft.model = settings.model;
    draft.base_url = settings.base_url;
}

fn next_scan_profile_name(
    profiles: &[purifier_core::ScanProfile],
    current: Option<&str>,
) -> Option<String> {
    if profiles.is_empty() {
        return None;
    }

    let next_index = current
        .and_then(|selected| profiles.iter().position(|profile| profile.name == selected))
        .map_or(0, |index| (index + 1) % (profiles.len() + 1));

    if next_index == profiles.len() {
        None
    } else {
        Some(profiles[next_index].name.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use purifier_core::types::FileEntry;
    use std::path::PathBuf;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn app_with_entries() -> App {
        let mut app = App::new(Some(PathBuf::from("/")), false, AppConfig::default());
        app.scan_status = ScanStatus::Complete;
        app.entries = vec![
            {
                let mut e = FileEntry::new(PathBuf::from("/large"), 100, true, None);
                e.children = vec![
                    FileEntry::new(PathBuf::from("/large/a"), 60, false, None),
                    FileEntry::new(PathBuf::from("/large/b"), 40, false, None),
                ];
                e
            },
            FileEntry::new(PathBuf::from("/small"), 10, false, None),
        ];
        app.rebuild_size_cache();
        app
    }

    #[test]
    fn h_on_root_column_is_noop() {
        let mut app = app_with_entries();
        assert_eq!(app.columns.depth(), 1);
        handle_key(&mut app, key(KeyCode::Char('h')));
        assert_eq!(app.columns.depth(), 1);
    }

    #[test]
    fn l_on_dir_enters_and_h_goes_back() {
        let mut app = app_with_entries();
        // selected is /large (biggest)
        handle_key(&mut app, key(KeyCode::Char('l')));
        assert_eq!(app.columns.depth(), 2);
        assert_eq!(app.columns.current_path(), PathBuf::from("/large"));

        handle_key(&mut app, key(KeyCode::Char('h')));
        assert_eq!(app.columns.depth(), 1);
    }

    #[test]
    fn l_on_file_does_not_enter() {
        let mut app = app_with_entries();
        // Move to second entry (file /small)
        handle_key(&mut app, key(KeyCode::Char('j')));
        handle_key(&mut app, key(KeyCode::Char('l')));
        assert_eq!(app.columns.depth(), 1); // still at root
    }

    #[test]
    fn d_sets_delete_confirm_n_cancels() {
        let mut app = app_with_entries();
        handle_key(&mut app, key(KeyCode::Char('d')));
        assert!(matches!(app.preview_mode, PreviewMode::DeleteConfirm(_)));

        handle_key(&mut app, key(KeyCode::Char('n')));
        assert!(matches!(app.preview_mode, PreviewMode::Analytics));
    }

    #[test]
    fn space_toggles_mark_x_enters_batch_review() {
        let mut app = app_with_entries();
        handle_key(&mut app, key(KeyCode::Char(' ')));
        assert_eq!(app.marks.count(), 1);

        handle_key(&mut app, key(KeyCode::Char('x')));
        assert!(matches!(app.preview_mode, PreviewMode::BatchReview));

        handle_key(&mut app, key(KeyCode::Esc));
        assert!(matches!(app.preview_mode, PreviewMode::Analytics));
    }

    #[test]
    fn comma_opens_settings_esc_cancels() {
        let mut app = app_with_entries();
        handle_key(&mut app, key(KeyCode::Char(',')));
        assert!(matches!(app.preview_mode, PreviewMode::Settings(_)));

        handle_key(&mut app, key(KeyCode::Esc));
        assert!(matches!(app.preview_mode, PreviewMode::Analytics));
    }

    #[test]
    fn s_cycles_sort_key() {
        let mut app = app_with_entries();
        assert_eq!(app.columns.sort_key, crate::columns::SortKey::Size);

        handle_key(&mut app, key(KeyCode::Char('s')));
        assert_eq!(app.columns.sort_key, crate::columns::SortKey::Safety);
    }
}
