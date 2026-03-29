use crossterm::event::{KeyCode, KeyEvent};
use purifier_core::types::FileEntry;

use crate::app::{App, AppScreen, View};

pub enum InputResult {
    None,
    StartScan(std::path::PathBuf),
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> InputResult {
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
            let path = if raw.starts_with("~/") {
                if let Some(home) = dirs::home_dir() {
                    home.join(&raw[2..])
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
    if app.show_delete_confirm {
        handle_delete_confirm(app, key);
        return;
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('1') => app.switch_view(View::BySize),
        KeyCode::Char('2') => app.switch_view(View::ByType),
        KeyCode::Char('3') => app.switch_view(View::BySafety),
        KeyCode::Char('4') => app.switch_view(View::ByAge),
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
                app.show_delete_confirm = true;
            }
        }
        _ => {}
    }
}

fn handle_delete_confirm(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            if let Some(flat) = app.selected_entry().cloned() {
                match purifier_core::delete_entry(&flat.path) {
                    Ok(freed) => {
                        app.freed_space += freed;
                        let index_path = flat.entry_index.clone();
                        remove_entry(&mut app.entries, &index_path);
                        app.rebuild_flat_entries();
                        if app.selected_index >= app.flat_entries.len() && !app.flat_entries.is_empty()
                        {
                            app.selected_index = app.flat_entries.len() - 1;
                        }
                    }
                    Err(_e) => {}
                }
            }
            app.show_delete_confirm = false;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.show_delete_confirm = false;
        }
        _ => {}
    }
}

fn remove_entry(entries: &mut Vec<FileEntry>, index_path: &[usize]) {
    if index_path.is_empty() {
        return;
    }
    if index_path.len() == 1 {
        if index_path[0] < entries.len() {
            entries.remove(index_path[0]);
        }
        return;
    }
    if let Some(parent) = entries.get_mut(index_path[0]) {
        remove_entry(&mut parent.children, &index_path[1..]);
    }
}
