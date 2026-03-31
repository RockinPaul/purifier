use crossterm::event::{KeyCode, KeyEvent};
use purifier_core::provider::{default_provider_settings, ProviderKind};

use crate::app::{App, AppModal, AppScreen, ScanStatus, SettingsDraft, View};

pub enum InputResult {
    None,
    StartScan(std::path::PathBuf),
    SaveSettings(SettingsDraft),
    SkipOnboarding,
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> InputResult {
    let had_modal = app.modal.is_some();
    let modal_result = handle_modal(app, key);
    if had_modal {
        return modal_result;
    }

    match app.screen {
        AppScreen::DirPicker => handle_dir_picker(app, key),
        AppScreen::Main => {
            handle_main(app, key);
            InputResult::None
        }
    }
}

fn handle_dir_picker(app: &mut App, key: KeyEvent) -> InputResult {
    if app.dir_picker_typing {
        return handle_custom_input(app, key);
    }

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
                    std::path::PathBuf::from(&raw)
                }
            } else {
                std::path::PathBuf::from(&raw)
            };

            if path.exists() && path.is_dir() {
                app.dir_picker_typing = false;
                app.start_scan_with_path(path.clone());
                InputResult::StartScan(path)
            } else {
                // Invalid path — stay in typing mode
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

fn handle_main(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('1') => app.switch_view(View::BySize),
        KeyCode::Char('2') => app.switch_view(View::ByType),
        KeyCode::Char('3') => app.switch_view(View::BySafety),
        KeyCode::Char('4') => app.switch_view(View::ByAge),
        KeyCode::Char('s') if app.scan_status == ScanStatus::Complete => app.open_settings(),
        KeyCode::Char('j') | KeyCode::Down => app.move_down(),
        KeyCode::Char('k') | KeyCode::Up => app.move_up(),
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => app.toggle_expand(),
        KeyCode::Char('h') | KeyCode::Left => {
            if let Some(flat) = app.selected_entry() {
                if flat.is_dir && flat.expanded {
                    app.toggle_expand();
                }
            }
        }
        KeyCode::Char('d') => {
            if app.selected_entry().is_some() {
                app.open_delete_confirm();
            }
        }
        _ => {}
    }
}

fn handle_modal(app: &mut App, key: KeyEvent) -> InputResult {
    match app.modal.as_ref() {
        Some(AppModal::DeleteConfirm) if matches!(app.screen, AppScreen::Main) => {
            handle_delete_confirm(app, key);
            InputResult::None
        }
        Some(AppModal::DeleteConfirm) => InputResult::None,
        Some(AppModal::Settings(_)) | Some(AppModal::Onboarding(_)) => {
            handle_settings_modal(app, key)
        }
        None => InputResult::None,
    }
}

fn handle_settings_modal(app: &mut App, key: KeyEvent) -> InputResult {
    if key.code == KeyCode::Enter {
        let is_onboarding = matches!(app.modal, Some(AppModal::Onboarding(_)));
        let Some(AppModal::Settings(draft) | AppModal::Onboarding(draft)) = app.modal.as_mut()
        else {
            return InputResult::None;
        };
        if draft.api_key_editing {
            draft.api_key_editing = false;
            return InputResult::None;
        }

        if is_onboarding && draft.api_key.is_empty() {
            app.last_error = Some("Enter an API key or press Esc to skip onboarding".to_string());
            return InputResult::None;
        }

        app.last_error = None;
        return InputResult::SaveSettings(draft.clone());
    }

    if matches!(app.modal, Some(AppModal::Onboarding(_))) && key.code == KeyCode::Esc {
        app.modal = None;
        app.last_error = None;
        return InputResult::SkipOnboarding;
    }

    if key.code == KeyCode::Esc {
        app.modal = None;
        return InputResult::None;
    }

    let provider_switch = match key.code {
        KeyCode::Char('1') => Some(ProviderKind::OpenRouter),
        KeyCode::Char('2') => Some(ProviderKind::OpenAI),
        KeyCode::Char('3') => Some(ProviderKind::Anthropic),
        KeyCode::Char('4') => Some(ProviderKind::Google),
        _ => None,
    };
    let provider_settings = provider_switch.map(|provider| {
        (
            provider,
            app.preferences
                .llm
                .providers
                .get(&provider)
                .cloned()
                .unwrap_or_else(|| default_provider_settings(provider)),
        )
    });

    let Some(AppModal::Settings(draft) | AppModal::Onboarding(draft)) = app.modal.as_mut() else {
        return InputResult::None;
    };

    if draft.api_key_editing {
        match key.code {
            KeyCode::Tab => {
                draft.api_key_editing = false;
            }
            KeyCode::Char(c) => {
                draft.api_key.push(c);
                draft.api_key_edited = true;
                app.last_error = None;
            }
            KeyCode::Backspace | KeyCode::Delete => {
                draft.api_key.pop();
                draft.api_key_edited = true;
            }
            _ => {}
        }

        return InputResult::None;
    }

    match key.code {
        KeyCode::Char('1') | KeyCode::Char('2') | KeyCode::Char('3') | KeyCode::Char('4') => {
            let Some((provider, settings)) = provider_settings else {
                return InputResult::None;
            };
            apply_provider_defaults(draft, provider, settings);
        }
        KeyCode::Char('a') => {
            draft.api_key_editing = true;
            app.last_error = None;
        }
        _ => {}
    }

    InputResult::None
}

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

fn handle_delete_confirm(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            if let Some(flat) = app.selected_entry().cloned() {
                match purifier_core::delete_entry(&flat.path) {
                    Ok(freed) => {
                        app.freed_space += freed;
                        app.last_error = None;
                        app.mark_deleted(&flat.path);
                        app.remove_entry_by_path(&flat.path);
                        app.rebuild_flat_entries();
                        if app.selected_index >= app.flat_entries.len()
                            && !app.flat_entries.is_empty()
                        {
                            app.selected_index = app.flat_entries.len() - 1;
                        }
                    }
                    Err(error) => {
                        app.last_error =
                            Some(format!("Could not delete {}: {error}", flat.path.display()));
                    }
                }
            }
            app.close_modal();
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.close_modal();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use purifier_core::types::FileEntry;

    use super::handle_key;
    use crate::app::App;

    #[test]
    fn confirm_delete_should_keep_entry_and_record_error_when_delete_fails() {
        let mut app = App::new(
            Some(PathBuf::from("/")),
            false,
            crate::config::AppConfig::default(),
        );
        app.entries = vec![FileEntry::new(
            PathBuf::from("/definitely/missing"),
            1,
            false,
            None,
        )];
        app.rebuild_flat_entries();
        app.open_delete_confirm();

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
        );

        assert_eq!(app.entries.len(), 1, "failed delete should keep entry");
        assert!(
            app.last_error.is_some(),
            "failed delete should record an error"
        );
        assert!(
            app.modal.is_none(),
            "confirmation should close after handling"
        );
    }
}

#[cfg(test)]
mod settings_input_tests {
    use std::path::PathBuf;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use purifier_core::types::FileEntry;

    use super::handle_key;
    use crate::app::{App, AppModal};
    use crate::config::AppConfig;

    #[test]
    fn pressing_s_should_open_settings_modal_when_scan_is_complete() {
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            true,
            AppConfig::default(),
        );
        app.scan_status = crate::app::ScanStatus::Complete;

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
        );

        assert!(matches!(app.modal, Some(AppModal::Settings(_))));
    }

    #[test]
    fn pressing_escape_should_close_settings_modal() {
        let mut app = App::new(Some(PathBuf::from("/")), true, AppConfig::default());
        app.open_settings();

        handle_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert!(app.modal.is_none());
        assert!(!app.should_quit);
    }

    #[test]
    fn pressing_escape_should_close_onboarding_modal() {
        let mut app = App::new(Some(PathBuf::from("/")), true, AppConfig::default());
        app.open_onboarding();

        handle_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert!(app.modal.is_none());
        assert!(!app.should_quit);
    }

    #[test]
    fn dir_picker_should_route_escape_to_onboarding_modal_before_screen_input() {
        let mut app = App::new(None, true, AppConfig::default());
        app.open_onboarding();

        handle_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert!(app.modal.is_none());
        assert!(!app.should_quit);
        assert!(matches!(app.screen, crate::app::AppScreen::DirPicker));
    }

    #[test]
    fn dir_picker_should_not_move_selection_while_onboarding_modal_is_open() {
        let mut app = App::new(None, true, AppConfig::default());
        app.open_onboarding();
        let initial_selection = app.dir_picker_selected;

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        );

        assert_eq!(app.dir_picker_selected, initial_selection);
        assert!(matches!(app.modal, Some(AppModal::Onboarding(_))));
    }

    #[test]
    fn pressing_d_should_open_delete_confirm_without_secondary_flag() {
        let mut app = App::new(Some(PathBuf::from("/")), false, AppConfig::default());
        app.entries = vec![FileEntry::new(PathBuf::from("/tmp/file"), 1, false, None)];
        app.rebuild_flat_entries();

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
        );

        assert!(matches!(app.modal, Some(AppModal::DeleteConfirm)));
    }
}

#[cfg(test)]
mod modal_submit_tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use purifier_core::provider::{default_provider_settings, ProviderKind};

    use super::{handle_key, InputResult};
    use crate::app::{App, AppModal, ScanStatus, SettingsDraft};
    use crate::config::AppConfig;

    #[test]
    fn onboarding_save_should_return_draft_without_closing_modal() {
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            true,
            AppConfig::default(),
        );
        app.scan_status = ScanStatus::Idle;
        app.modal = Some(AppModal::Onboarding(SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: "or-key".to_string(),
            api_key_edited: true,
            api_key_editing: false,
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            llm_enabled: true,
        }));

        let result = handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let InputResult::SaveSettings(draft) = result else {
            panic!("save should return settings draft");
        };
        assert!(matches!(app.modal, Some(AppModal::Onboarding(_))));
        assert_eq!(draft.provider, ProviderKind::OpenRouter);
    }

    #[test]
    fn provider_hotkey_should_refresh_model_and_base_url_for_selected_provider() {
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            true,
            AppConfig::default(),
        );
        app.modal = Some(AppModal::Settings(SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: "or-key".to_string(),
            api_key_edited: true,
            api_key_editing: false,
            model: "custom-openrouter-model".to_string(),
            base_url: "https://wrong.example.com".to_string(),
            llm_enabled: true,
        }));

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE),
        );

        let default_settings = default_provider_settings(ProviderKind::OpenAI);
        let Some(AppModal::Settings(draft)) = app.modal.as_ref() else {
            panic!("settings modal should remain open");
        };
        assert_eq!(draft.provider, ProviderKind::OpenAI);
        assert_eq!(draft.model, default_settings.model);
        assert_eq!(draft.base_url, default_settings.base_url);
    }

    #[test]
    fn provider_hotkey_should_prefer_saved_provider_settings_over_defaults() {
        let mut config = AppConfig::default();
        config.llm.providers.insert(
            ProviderKind::OpenAI,
            purifier_core::provider::ProviderSettings {
                model: "saved-gpt".to_string(),
                base_url: "https://saved.openai.example/v1".to_string(),
            },
        );
        let mut app = App::new(Some(std::path::PathBuf::from("/")), true, config);
        app.modal = Some(AppModal::Settings(SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: "or-key".to_string(),
            api_key_edited: true,
            api_key_editing: false,
            model: "custom-openrouter-model".to_string(),
            base_url: "https://wrong.example.com".to_string(),
            llm_enabled: true,
        }));

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE),
        );

        let Some(AppModal::Settings(draft)) = app.modal.as_ref() else {
            panic!("settings modal should remain open");
        };
        assert_eq!(draft.provider, ProviderKind::OpenAI);
        assert_eq!(draft.model, "saved-gpt");
        assert_eq!(draft.base_url, "https://saved.openai.example/v1");
    }

    #[test]
    fn provider_hotkey_should_clear_api_key_state_when_switching_providers() {
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            true,
            AppConfig::default(),
        );
        app.modal = Some(AppModal::Settings(SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: "or-key".to_string(),
            api_key_edited: true,
            api_key_editing: true,
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            llm_enabled: true,
        }));

        handle_key(&mut app, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE),
        );

        let Some(AppModal::Settings(draft)) = app.modal.as_ref() else {
            panic!("settings modal should remain open");
        };
        assert_eq!(draft.provider, ProviderKind::OpenAI);
        assert!(draft.api_key.is_empty());
        assert!(!draft.api_key_edited);
        assert!(!draft.api_key_editing);
    }

    #[test]
    fn api_key_editing_should_return_typed_key_on_save() {
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            true,
            AppConfig::default(),
        );
        app.modal = Some(AppModal::Settings(SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: String::new(),
            api_key_edited: false,
            api_key_editing: false,
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            llm_enabled: true,
        }));

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
        );
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
        );
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
        );
        handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let result = handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let InputResult::SaveSettings(draft) = result else {
            panic!("save should return settings draft");
        };
        assert_eq!(draft.api_key, "sk");
        assert!(draft.api_key_edited);
    }

    #[test]
    fn save_should_not_claim_live_llm_ready_without_runtime_client() {
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            false,
            AppConfig::default(),
        );
        app.llm_status = crate::app::LlmStatus::NeedsSetup;
        app.modal = Some(AppModal::Onboarding(SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: "or-key".to_string(),
            api_key_edited: true,
            api_key_editing: false,
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            llm_enabled: true,
        }));

        handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(app.llm_status, crate::app::LlmStatus::NeedsSetup);
        assert!(!app.llm_enabled);
    }

    #[test]
    fn onboarding_save_should_require_api_key_for_supported_providers() {
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            true,
            AppConfig::default(),
        );
        app.modal = Some(AppModal::Onboarding(SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: String::new(),
            api_key_edited: false,
            api_key_editing: false,
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            llm_enabled: true,
        }));

        let result = handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert!(matches!(result, InputResult::None));
        assert!(matches!(app.modal, Some(AppModal::Onboarding(_))));
        assert_eq!(
            app.last_error.as_deref(),
            Some("Enter an API key or press Esc to skip onboarding")
        );
    }

    #[test]
    fn onboarding_skip_should_clear_previous_validation_error() {
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            true,
            AppConfig::default(),
        );
        app.last_error = Some("Enter an API key or press Esc to skip onboarding".to_string());
        app.modal = Some(AppModal::Onboarding(SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: String::new(),
            api_key_edited: false,
            api_key_editing: false,
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            llm_enabled: true,
        }));

        let result = handle_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert!(matches!(result, InputResult::SkipOnboarding));
        assert!(app.modal.is_none());
        assert!(app.last_error.is_none());
    }

    #[test]
    fn onboarding_validation_error_should_clear_when_modal_becomes_valid_again() {
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            true,
            AppConfig::default(),
        );
        app.last_error = Some("Enter an API key or press Esc to skip onboarding".to_string());
        app.modal = Some(AppModal::Onboarding(SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: String::new(),
            api_key_edited: false,
            api_key_editing: false,
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            llm_enabled: true,
        }));

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
        );

        assert!(app.last_error.is_none());
    }

    #[test]
    fn provider_hotkey_five_should_leave_provider_unchanged_when_ollama_is_disabled() {
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            true,
            AppConfig::default(),
        );
        app.modal = Some(AppModal::Settings(SettingsDraft {
            provider: ProviderKind::OpenAI,
            api_key: String::new(),
            api_key_edited: false,
            api_key_editing: false,
            model: "gpt-4o-mini".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            llm_enabled: true,
        }));

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('5'), KeyModifiers::NONE),
        );

        match app.modal.as_ref() {
            Some(AppModal::Settings(draft)) => {
                assert_eq!(draft.provider, ProviderKind::OpenAI);
                assert_eq!(draft.model, "gpt-4o-mini");
            }
            other => panic!("expected settings modal, got {other:?}"),
        }
    }
}
