use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use super::{format_size, truncate_start, truncate_tail};
use crate::app::{App, ScanStatus};

pub fn draw(frame: &mut Frame, app: &App, main_area: Rect, info_area: Rect) {
    let items: Vec<ListItem> = app
        .flat_entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let indent = "  ".repeat(entry.depth);
            let icon = if entry.is_dir {
                if entry.expanded {
                    "▼ "
                } else {
                    "▶ "
                }
            } else {
                "  "
            };

            let safety_badge = match entry.safety {
                purifier_core::SafetyLevel::Safe => {
                    Span::styled(" ✓ ", Style::default().fg(Color::Green))
                }
                purifier_core::SafetyLevel::Caution => {
                    Span::styled(" ⚠ ", Style::default().fg(Color::Yellow))
                }
                purifier_core::SafetyLevel::Unsafe => {
                    Span::styled(" ✗ ", Style::default().fg(Color::Red))
                }
                purifier_core::SafetyLevel::Unknown => {
                    Span::styled(" ? ", Style::default().fg(Color::DarkGray))
                }
            };

            let name = entry
                .path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| entry.path.display().to_string());

            let size_str = format_size(entry.size);

            let max_size = app.flat_entries.first().map(|e| e.size).unwrap_or(1).max(1);
            let bar_width = 15;
            let filled = ((entry.size as f64 / max_size as f64) * bar_width as f64) as usize;
            let bar: String = "█".repeat(filled) + &"░".repeat(bar_width - filled);

            let style = if i == app.selected_index {
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let line = Line::from(vec![
                Span::raw(format!("{indent}{icon}")),
                Span::styled(format!("{:<30}", truncate_start(&name, 30)), style),
                Span::styled(
                    format!(" {:<10}", size_str),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(bar, Style::default().fg(Color::Blue)),
                safety_badge,
            ]);

            ListItem::new(line).style(style)
        })
        .collect();

    let list = List::new(items).block(Block::default().borders(Borders::ALL));
    let mut list_state = ListState::default().with_selected(Some(app.selected_index));
    frame.render_stateful_widget(list, main_area, &mut list_state);

    let info_text = if let Some(entry) = app.selected_entry() {
        if entry.safety_reason.is_empty() {
            format!("{}", entry.path.display())
        } else {
            format!("{} — {}", entry.path.display(), entry.safety_reason)
        }
    } else if app.scan_status == ScanStatus::Scanning {
        "Scanning in progress...".to_string()
    } else {
        "No selection".to_string()
    };

    let info =
        Paragraph::new(info_text).block(Block::default().borders(Borders::ALL).title(" Info "));
    frame.render_widget(info, info_area);

    if app.scan_status == ScanStatus::Scanning {
        draw_scanning_overlay(frame, app, main_area);
    }
}

pub(crate) fn scanning_overlay_area(main_area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Length(8)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Length(
        50.min(main_area.width.saturating_sub(4)),
    )])
    .flex(Flex::Center);
    let [center_v] = vertical.areas(main_area);
    let [center] = horizontal.areas(center_v);
    center
}

fn draw_scanning_overlay(frame: &mut Frame, app: &App, main_area: Rect) {
    let center = scanning_overlay_area(main_area);

    let dir_display = if app.current_scan_dir.is_empty() {
        "starting...".to_string()
    } else {
        truncate_tail(&app.current_scan_dir, 45)
    };

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Scanning filesystem...",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Files: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", app.files_scanned),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Found: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format_size(app.bytes_found),
                Style::default().fg(Color::Cyan),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Path:  ", Style::default().fg(Color::DarkGray)),
            Span::styled(dir_display, Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Press q to quit",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let scanning_widget = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow)),
    );
    frame.render_widget(Clear, center);
    frame.render_widget(scanning_widget, center);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use purifier_core::types::FileEntry;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    use super::*;
    use crate::config::AppConfig;
    use crate::input::handle_key;

    fn render_tree(app: &App, area: Rect) -> Buffer {
        let backend = TestBackend::new(area.width, area.height + 3);
        let mut terminal = Terminal::new(backend).expect("terminal should be created");
        terminal
            .draw(|frame| {
                draw(
                    frame,
                    app,
                    Rect::new(0, 0, area.width, area.height),
                    Rect::new(0, area.height, area.width, 3),
                );
            })
            .expect("tree should render");
        terminal.backend().buffer().clone()
    }

    fn buffer_text(buffer: &Buffer) -> String {
        let mut text = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    fn buffer_region_text(buffer: &Buffer, area: Rect) -> String {
        let mut text = String::new();
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn draw_should_keep_keyboard_selected_row_visible_when_selection_moves_below_viewport() {
        let mut app = App::new(Some(PathBuf::from("/")), false, AppConfig::default());
        app.entries = (0..12)
            .map(|index| {
                FileEntry::new(
                    PathBuf::from(format!("/file-{index}")),
                    12 - index,
                    false,
                    None,
                )
            })
            .collect();
        app.rebuild_flat_entries();

        for _ in 0..11 {
            handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
            );
        }

        let buffer = render_tree(&app, Rect::new(0, 0, 80, 6));
        let text = buffer_region_text(&buffer, Rect::new(0, 0, 80, 6));

        assert!(
            text.contains("file-11"),
            "selected entry should be rendered when it falls below the viewport: {text}"
        );
    }

    #[test]
    fn draw_should_keep_arrow_key_selected_row_visible_when_selection_moves_below_viewport() {
        let mut app = App::new(Some(PathBuf::from("/")), false, AppConfig::default());
        app.entries = (0..12)
            .map(|index| {
                FileEntry::new(
                    PathBuf::from(format!("/file-{index}")),
                    12 - index,
                    false,
                    None,
                )
            })
            .collect();
        app.rebuild_flat_entries();

        for _ in 0..11 {
            handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }

        let buffer = render_tree(&app, Rect::new(0, 0, 80, 6));
        let text = buffer_region_text(&buffer, Rect::new(0, 0, 80, 6));

        assert!(
            text.contains("file-11"),
            "down-arrow navigation should keep the selected row visible: {text}"
        );
    }

    #[test]
    fn draw_should_keep_arrow_key_selected_row_visible_when_moving_back_up() {
        let mut app = App::new(Some(PathBuf::from("/")), false, AppConfig::default());
        app.entries = (0..12)
            .map(|index| {
                FileEntry::new(
                    PathBuf::from(format!("/file-{index}")),
                    12 - index,
                    false,
                    None,
                )
            })
            .collect();
        app.rebuild_flat_entries();

        for _ in 0..11 {
            handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        for _ in 0..9 {
            handle_key(&mut app, KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        }

        let buffer = render_tree(&app, Rect::new(0, 0, 80, 6));
        let text = buffer_region_text(&buffer, Rect::new(0, 0, 80, 6));

        assert!(
            text.contains("file-2"),
            "up-arrow navigation should keep the selected row visible after scrolling back up: {text}"
        );
        assert!(
            !text.contains("file-11"),
            "viewport should follow the selection upward rather than staying pinned to the old window: {text}"
        );
    }

    #[test]
    fn draw_should_keep_scan_progress_visible_while_live_entries_are_present() {
        let mut app = App::new(Some(PathBuf::from("/scan")), false, AppConfig::default());
        app.scan_status = ScanStatus::Scanning;
        app.entries = vec![FileEntry::new(
            PathBuf::from("/scan/live-file"),
            42,
            false,
            None,
        )];
        app.rebuild_flat_entries();
        app.files_scanned = 7;
        app.bytes_found = 42;
        app.current_scan_dir = "/scan".to_string();

        let buffer = render_tree(&app, Rect::new(0, 0, 80, 8));
        let text = buffer_text(&buffer);

        assert!(
            text.contains("Scanning filesystem"),
            "scan progress box should remain visible during scanning: {text}"
        );
        assert!(
            text.contains("live-file"),
            "live tree entries should still render under the progress overlay: {text}"
        );
    }
}
