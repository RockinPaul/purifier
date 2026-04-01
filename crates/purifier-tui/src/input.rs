use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use purifier_core::provider::{default_provider_settings, ProviderKind};
use purifier_core::size::SizeMode;
use ratatui::layout::Rect;

use crate::app::{App, AppModal, AppScreen, ScanStatus, SettingsDraft, View};
use crate::ui::{self, MainLayout};

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

pub fn handle_mouse(app: &mut App, mouse: MouseEvent, layout: MainLayout) {
    if app.modal.is_some() || !matches!(app.screen, AppScreen::Main) {
        return;
    }

    if app.scan_status == ScanStatus::Scanning {
        return;
    }

    if !rect_contains(layout.main, mouse.column, mouse.row) {
        return;
    }

    if app.scan_status == ScanStatus::Scanning
        && rect_contains(
            ui::tree_view::scanning_overlay_area(layout.main),
            mouse.column,
            mouse.row,
        )
    {
        return;
    }

    match mouse.kind {
        MouseEventKind::ScrollDown => app.move_down(),
        MouseEventKind::ScrollUp => app.move_up(),
        _ => {}
    }
}

fn rect_contains(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
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
    if app.scan_status == ScanStatus::Scanning {
        if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
            app.should_quit = true;
        }
        return;
    }

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
    if app.settings_modal_is_saving {
        return InputResult::None;
    }

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
            app.settings_modal_error =
                Some("Enter an API key or press Esc to skip onboarding".to_string());
            app.last_error = Some("Enter an API key or press Esc to skip onboarding".to_string());
            return InputResult::None;
        }

        app.settings_modal_error = None;
        app.last_error = None;
        return InputResult::SaveSettings(draft.clone());
    }

    if matches!(app.modal, Some(AppModal::Onboarding(_))) && key.code == KeyCode::Esc {
        app.close_modal();
        app.last_error = None;
        return InputResult::SkipOnboarding;
    }

    if key.code == KeyCode::Esc {
        app.close_modal();
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

        return InputResult::None;
    }

    match key.code {
        KeyCode::Char('1') | KeyCode::Char('2') | KeyCode::Char('3') | KeyCode::Char('4') => {
            let Some((provider, settings)) = provider_settings else {
                return InputResult::None;
            };
            apply_provider_defaults(draft, provider, settings);
            app.settings_modal_error = None;
            app.last_error = None;
        }
        KeyCode::Char('a') => {
            draft.api_key_editing = true;
            app.settings_modal_error = None;
            app.last_error = None;
        }
        KeyCode::Char('m') => {
            draft.size_mode = match draft.size_mode {
                SizeMode::Physical => SizeMode::Logical,
                SizeMode::Logical => SizeMode::Physical,
            };
        }
        KeyCode::Char('p') => {
            draft.selected_scan_profile = next_scan_profile_name(
                &app.preferences.ui.scan_profiles,
                draft.selected_scan_profile.as_deref(),
            );
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

fn handle_delete_confirm(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            if let Some(flat) = app.selected_entry().cloned() {
                match purifier_core::delete_entry(&flat.path) {
                    Ok(outcome) => {
                        app.delete_stats.logical_bytes_removed += outcome.logical_bytes_removed;
                        app.delete_stats.physical_bytes_estimated +=
                            outcome.physical_bytes_estimated;
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

    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;

    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use purifier_core::size::EntrySizes;
    use purifier_core::types::FileEntry;

    use super::{handle_key, handle_mouse};
    use crate::app::App;
    use crate::ui;

    fn scroll_down_event() -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn scroll_up_event() -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn scroll_down_at(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

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

    #[test]
    fn confirm_delete_should_track_logical_removed_and_physical_freed_bytes() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let file_path = dir.path().join("delete-me.bin");
        std::fs::write(&file_path, vec![0_u8; 8192]).expect("test file should be written");

        let mut app = App::new(
            Some(PathBuf::from("/")),
            false,
            crate::config::AppConfig::default(),
        );
        app.entries = vec![FileEntry::new_with_sizes(
            file_path.clone(),
            EntrySizes {
                logical_bytes: 8192,
                physical_bytes: 8192,
                accounted_physical_bytes: 8192,
            },
            None,
            false,
            None,
        )];
        app.rebuild_flat_entries();
        app.open_delete_confirm();

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
        );

        assert!(
            app.entries.is_empty(),
            "successful delete should remove entry"
        );
        assert_eq!(app.delete_stats.logical_bytes_removed, 8192);
        assert!(
            app.delete_stats.physical_bytes_freed > 0
                || app.delete_stats.physical_bytes_estimated > 0,
            "successful delete should keep physical freed-space information visible"
        );
        assert!(
            !file_path.exists(),
            "file should be removed from disk after confirmation"
        );
    }

    #[test]
    fn confirm_delete_should_update_total_size_and_file_count() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let file_path = dir.path().join("delete-me.bin");
        std::fs::write(&file_path, vec![0_u8; 8192]).expect("test file should be written");

        let mut app = App::new(
            Some(PathBuf::from("/")),
            false,
            crate::config::AppConfig::default(),
        );
        app.entries = vec![
            FileEntry::new_with_sizes(
                file_path.clone(),
                EntrySizes {
                    logical_bytes: 8192,
                    physical_bytes: 8192,
                    accounted_physical_bytes: 8192,
                },
                None,
                false,
                None,
            ),
            FileEntry::new(PathBuf::from("/keep"), 1024, false, None),
        ];
        app.total_logical_size = 9216;
        app.total_physical_size = 9216;
        app.sync_display_size_state();
        app.total_files = 2;
        app.rebuild_flat_entries();
        app.selected_index = 0;
        app.open_delete_confirm();

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
        );

        assert_eq!(app.total_size, 1024);
        assert_eq!(app.total_files, 1);
    }

    #[cfg(unix)]
    #[test]
    fn confirm_delete_should_keep_physical_total_when_other_hard_links_survive() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let original = dir.path().join("file.txt");
        let linked = dir.path().join("file-copy.txt");
        std::fs::write(&original, b"hello").expect("file should be written");
        std::fs::hard_link(&original, &linked).expect("hard link should be created");

        let physical = std::fs::metadata(&original)
            .expect("metadata should load")
            .blocks()
            * 512;

        let mut app = App::new(
            Some(PathBuf::from("/")),
            false,
            crate::config::AppConfig::default(),
        );
        app.entries = vec![
            FileEntry::new_with_sizes(
                original.clone(),
                EntrySizes {
                    logical_bytes: 5,
                    physical_bytes: physical,
                    accounted_physical_bytes: physical,
                },
                None,
                false,
                None,
            ),
            FileEntry::new_with_sizes(
                linked.clone(),
                EntrySizes {
                    logical_bytes: 5,
                    physical_bytes: physical,
                    accounted_physical_bytes: 0,
                },
                None,
                false,
                None,
            ),
        ];
        app.total_logical_size = 10;
        app.total_physical_size = physical;
        app.sync_display_size_state();
        app.total_files = 2;
        app.rebuild_flat_entries();
        app.selected_index = 0;
        app.open_delete_confirm();

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
        );

        assert_eq!(app.total_physical_size, physical);
        assert_eq!(app.total_logical_size, 5);
        assert_eq!(app.total_files, 1);
    }

    #[test]
    fn mouse_wheel_should_move_main_selection() {
        let mut app = App::new(
            Some(PathBuf::from("/")),
            false,
            crate::config::AppConfig::default(),
        );
        app.entries = vec![
            FileEntry::new(PathBuf::from("/a"), 3, false, None),
            FileEntry::new(PathBuf::from("/b"), 2, false, None),
            FileEntry::new(PathBuf::from("/c"), 1, false, None),
        ];
        app.rebuild_flat_entries();
        let layout = ui::main_layout(ratatui::layout::Rect::new(0, 0, 80, 20));

        handle_mouse(
            &mut app,
            scroll_down_at(layout.main.x + 1, layout.main.y + 1),
            layout,
        );
        assert_eq!(
            app.selected_index, 1,
            "scroll down should advance selection"
        );
        handle_mouse(
            &mut app,
            MouseEvent {
                column: layout.main.x + 1,
                row: layout.main.y + 1,
                ..scroll_up_event()
            },
            layout,
        );
        handle_mouse(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: layout.main.x + 1,
                row: layout.main.y + 1,
                modifiers: KeyModifiers::NONE,
            },
            layout,
        );

        assert_eq!(
            app.selected_index, 0,
            "scroll down then up should restore selection"
        );
    }

    #[test]
    fn mouse_wheel_should_not_move_selection_when_modal_or_non_main_interaction_is_active() {
        let mut settings_app = App::new(
            Some(PathBuf::from("/")),
            false,
            crate::config::AppConfig::default(),
        );
        settings_app.entries = vec![
            FileEntry::new(PathBuf::from("/a"), 3, false, None),
            FileEntry::new(PathBuf::from("/b"), 2, false, None),
        ];
        settings_app.rebuild_flat_entries();
        settings_app.open_settings();
        let layout = ui::main_layout(ratatui::layout::Rect::new(0, 0, 80, 20));

        handle_mouse(
            &mut settings_app,
            scroll_down_at(layout.main.x + 1, layout.main.y + 1),
            layout,
        );

        assert_eq!(
            settings_app.selected_index, 0,
            "settings modal should block wheel navigation"
        );

        let mut delete_confirm_app = App::new(
            Some(PathBuf::from("/")),
            false,
            crate::config::AppConfig::default(),
        );
        delete_confirm_app.entries = vec![
            FileEntry::new(PathBuf::from("/a"), 3, false, None),
            FileEntry::new(PathBuf::from("/b"), 2, false, None),
        ];
        delete_confirm_app.rebuild_flat_entries();
        delete_confirm_app.open_delete_confirm();

        handle_mouse(
            &mut delete_confirm_app,
            scroll_down_at(layout.main.x + 1, layout.main.y + 1),
            layout,
        );

        assert_eq!(
            delete_confirm_app.selected_index, 0,
            "delete confirm should block wheel navigation"
        );

        let mut dir_picker_app = App::new(None, false, crate::config::AppConfig::default());
        dir_picker_app.selected_index = 1;
        dir_picker_app.dir_picker_typing = true;

        handle_mouse(&mut dir_picker_app, scroll_down_event(), layout);

        assert_eq!(
            dir_picker_app.selected_index, 1,
            "wheel input should not mutate hidden main selection while typing in dir picker"
        );
    }

    #[test]
    fn mouse_wheel_should_only_scroll_when_pointer_is_over_main_tree_pane() {
        let mut app = App::new(
            Some(PathBuf::from("/")),
            false,
            crate::config::AppConfig::default(),
        );
        app.entries = vec![
            FileEntry::new(PathBuf::from("/a"), 3, false, None),
            FileEntry::new(PathBuf::from("/b"), 2, false, None),
            FileEntry::new(PathBuf::from("/c"), 1, false, None),
        ];
        app.rebuild_flat_entries();
        let layout = ui::main_layout(ratatui::layout::Rect::new(0, 0, 80, 20));

        handle_mouse(
            &mut app,
            scroll_down_at(layout.main.x + 1, layout.main.y + 1),
            layout,
        );
        assert_eq!(
            app.selected_index, 1,
            "tree pane should accept wheel scroll"
        );

        handle_mouse(
            &mut app,
            scroll_down_at(layout.tabs.x + 1, layout.tabs.y + 1),
            layout,
        );
        assert_eq!(app.selected_index, 1, "tab bar should ignore wheel scroll");

        handle_mouse(
            &mut app,
            scroll_down_at(layout.info.x + 1, layout.info.y + 1),
            layout,
        );
        assert_eq!(
            app.selected_index, 1,
            "info pane should ignore wheel scroll"
        );

        handle_mouse(
            &mut app,
            scroll_down_at(layout.status.x + 1, layout.status.y),
            layout,
        );
        assert_eq!(
            app.selected_index, 1,
            "status bar should ignore wheel scroll"
        );
    }

    #[test]
    fn mouse_wheel_should_ignore_pointer_inside_scanning_overlay() {
        let mut app = App::new(
            Some(PathBuf::from("/")),
            false,
            crate::config::AppConfig::default(),
        );
        app.entries = vec![
            FileEntry::new(PathBuf::from("/a"), 3, false, None),
            FileEntry::new(PathBuf::from("/b"), 2, false, None),
            FileEntry::new(PathBuf::from("/c"), 1, false, None),
        ];
        app.rebuild_flat_entries();
        app.scan_status = crate::app::ScanStatus::Scanning;
        let layout = ui::main_layout(ratatui::layout::Rect::new(0, 0, 80, 20));
        let overlay = ui::tree_view::scanning_overlay_area(layout.main);

        handle_mouse(
            &mut app,
            scroll_down_at(overlay.x + 1, overlay.y + 1),
            layout,
        );

        assert_eq!(
            app.selected_index, 0,
            "centered scan overlay should block wheel scroll"
        );
    }

    #[test]
    fn scanning_state_should_ignore_main_list_navigation_keys() {
        let mut app = App::new(
            Some(PathBuf::from("/")),
            false,
            crate::config::AppConfig::default(),
        );
        app.scan_status = crate::app::ScanStatus::Scanning;
        app.entries = vec![
            FileEntry::new(PathBuf::from("/a"), 3, false, None),
            FileEntry::new(PathBuf::from("/b"), 2, false, None),
        ];
        app.rebuild_flat_entries();

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        );
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
        );

        assert_eq!(
            app.selected_index, 0,
            "keyboard navigation should stay blocked while scanning"
        );
        assert!(
            app.modal.is_none(),
            "delete confirm should stay closed while scanning"
        );
    }

    #[test]
    fn mouse_wheel_should_not_move_selection_while_scanning() {
        let mut app = App::new(
            Some(PathBuf::from("/")),
            false,
            crate::config::AppConfig::default(),
        );
        app.scan_status = crate::app::ScanStatus::Scanning;
        app.entries = vec![
            FileEntry::new(PathBuf::from("/a"), 3, false, None),
            FileEntry::new(PathBuf::from("/b"), 2, false, None),
        ];
        app.rebuild_flat_entries();
        let layout = ui::main_layout(ratatui::layout::Rect::new(0, 0, 80, 20));

        handle_mouse(
            &mut app,
            scroll_down_at(layout.main.x + 1, layout.main.y + 1),
            layout,
        );

        assert_eq!(
            app.selected_index, 0,
            "wheel scrolling should stay blocked while scanning"
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
            size_mode: purifier_core::SizeMode::Physical,
            selected_scan_profile: None,
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
            size_mode: purifier_core::SizeMode::Physical,
            selected_scan_profile: None,
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
    fn saving_modal_should_ignore_keyboard_edits_and_navigation() {
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            true,
            AppConfig::default(),
        );
        app.modal = Some(AppModal::Settings(SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: "old-key".to_string(),
            api_key_edited: true,
            api_key_editing: false,
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            llm_enabled: true,
            size_mode: purifier_core::SizeMode::Physical,
            selected_scan_profile: None,
        }));
        app.settings_modal_is_saving = true;
        app.settings_modal_error = Some("still validating".to_string());
        app.last_error = Some("still validating".to_string());

        assert!(matches!(
            handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)
            ),
            InputResult::None
        ));
        assert!(matches!(
            handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE)
            ),
            InputResult::None
        ));
        assert!(matches!(
            handle_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            InputResult::None
        ));

        let Some(AppModal::Settings(draft)) = app.modal.as_ref() else {
            panic!("settings modal should stay open while saving");
        };
        assert_eq!(draft.provider, ProviderKind::OpenRouter);
        assert_eq!(draft.api_key, "old-key");
        assert!(!draft.api_key_editing);
        assert!(app.settings_modal_is_saving);
        assert_eq!(
            app.settings_modal_error.as_deref(),
            Some("still validating")
        );
        assert_eq!(app.last_error.as_deref(), Some("still validating"));
    }

    #[test]
    fn editing_api_key_should_clear_inline_and_global_errors_together() {
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            true,
            AppConfig::default(),
        );
        app.modal = Some(AppModal::Settings(SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: "bad".to_string(),
            api_key_edited: true,
            api_key_editing: true,
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            llm_enabled: true,
            size_mode: purifier_core::SizeMode::Physical,
            selected_scan_profile: None,
        }));
        app.settings_modal_error = Some("inline error".to_string());
        app.last_error = Some("inline error".to_string());

        let result = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        );

        assert!(matches!(result, InputResult::None));
        let Some(AppModal::Settings(draft)) = app.modal.as_ref() else {
            panic!("settings modal should stay open");
        };
        assert_eq!(draft.api_key, "badx");
        assert!(app.settings_modal_error.is_none());
        assert!(app.last_error.is_none());
    }

    #[test]
    fn backspace_while_editing_api_key_should_clear_inline_and_global_errors_together() {
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            true,
            AppConfig::default(),
        );
        app.modal = Some(AppModal::Settings(SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: "bad".to_string(),
            api_key_edited: true,
            api_key_editing: true,
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            llm_enabled: true,
            size_mode: purifier_core::SizeMode::Physical,
            selected_scan_profile: None,
        }));
        app.settings_modal_error = Some("inline error".to_string());
        app.last_error = Some("inline error".to_string());

        let result = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        );

        assert!(matches!(result, InputResult::None));
        let Some(AppModal::Settings(draft)) = app.modal.as_ref() else {
            panic!("settings modal should stay open");
        };
        assert_eq!(draft.api_key, "ba");
        assert!(app.settings_modal_error.is_none());
        assert!(app.last_error.is_none());
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
            size_mode: purifier_core::SizeMode::Physical,
            selected_scan_profile: None,
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
            size_mode: purifier_core::SizeMode::Physical,
            selected_scan_profile: None,
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
            size_mode: purifier_core::SizeMode::Physical,
            selected_scan_profile: None,
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
    fn settings_modal_shortcuts_should_update_size_mode_and_selected_profile() {
        let mut config = AppConfig::default();
        config.ui.scan_profiles = vec![
            purifier_core::ScanProfile {
                name: "exclude-node-modules".to_string(),
                exclude: None,
                mask: None,
                display_filter: None,
            },
            purifier_core::ScanProfile {
                name: "cache-only".to_string(),
                exclude: None,
                mask: None,
                display_filter: None,
            },
        ];
        let mut app = App::new(Some(std::path::PathBuf::from("/")), true, config);
        app.open_settings();

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE),
        );
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
        );

        let Some(AppModal::Settings(draft)) = app.modal.as_ref() else {
            panic!("settings modal should stay open");
        };
        assert_eq!(draft.size_mode, purifier_core::SizeMode::Logical);
        assert_eq!(
            draft.selected_scan_profile.as_deref(),
            Some("exclude-node-modules")
        );
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
            size_mode: purifier_core::SizeMode::Physical,
            selected_scan_profile: None,
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
            size_mode: purifier_core::SizeMode::Physical,
            selected_scan_profile: None,
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
            size_mode: purifier_core::SizeMode::Physical,
            selected_scan_profile: None,
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
            size_mode: purifier_core::SizeMode::Physical,
            selected_scan_profile: None,
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
            size_mode: purifier_core::SizeMode::Physical,
            selected_scan_profile: None,
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
