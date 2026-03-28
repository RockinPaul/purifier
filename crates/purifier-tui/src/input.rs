use crossterm::event::{KeyCode, KeyEvent};
use purifier_core::types::FileEntry;

use crate::app::{App, View};

pub fn handle_key(app: &mut App, key: KeyEvent) {
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
            // Collapse: if on expanded dir, collapse it; otherwise go to parent
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
                        // Remove entry from tree
                        let index_path = flat.entry_index.clone();
                        remove_entry(&mut app.entries, &index_path);
                        app.rebuild_flat_entries();
                        if app.selected_index >= app.flat_entries.len() && !app.flat_entries.is_empty()
                        {
                            app.selected_index = app.flat_entries.len() - 1;
                        }
                    }
                    Err(_e) => {
                        // Error handled silently — entry stays in tree
                    }
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
