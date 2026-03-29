use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use super::{format_size, truncate_start, truncate_tail};
use crate::app::{App, ScanStatus};

pub fn draw(frame: &mut Frame, app: &App, main_area: Rect, info_area: Rect) {
    // During scanning with no entries yet, show a progress screen
    if app.scan_status == ScanStatus::Scanning && app.flat_entries.is_empty() {
        draw_scanning(frame, app, main_area, info_area);
        return;
    }

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
    frame.render_widget(list, main_area);

    let info_text = if let Some(entry) = app.selected_entry() {
        if entry.safety_reason.is_empty() {
            format!("{}", entry.path.display())
        } else {
            format!("{} — {}", entry.path.display(), entry.safety_reason)
        }
    } else {
        "No selection".to_string()
    };

    let info =
        Paragraph::new(info_text).block(Block::default().borders(Borders::ALL).title(" Info "));
    frame.render_widget(info, info_area);
}

fn draw_scanning(frame: &mut Frame, app: &App, main_area: Rect, info_area: Rect) {
    let vertical = Layout::vertical([Constraint::Length(8)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Length(
        50.min(main_area.width.saturating_sub(4)),
    )])
    .flex(Flex::Center);
    let [center_v] = vertical.areas(main_area);
    let [center] = horizontal.areas(center_v);

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
    frame.render_widget(scanning_widget, center);

    // Still render info area (empty during scan)
    let info = Paragraph::new("Scanning in progress...")
        .block(Block::default().borders(Borders::ALL).title(" Info "));
    frame.render_widget(info, info_area);
}
