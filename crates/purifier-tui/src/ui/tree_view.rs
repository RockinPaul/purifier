use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::App;
use super::format_size;

pub fn draw(frame: &mut Frame, app: &App, main_area: Rect, info_area: Rect) {
    let items: Vec<ListItem> = app
        .flat_entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let indent = "  ".repeat(entry.depth);
            let icon = if entry.is_dir {
                if entry.expanded { "▼ " } else { "▶ " }
            } else {
                "  "
            };

            let safety_badge = match entry.safety {
                purifier_core::SafetyLevel::Safe => Span::styled(" ✓ ", Style::default().fg(Color::Green)),
                purifier_core::SafetyLevel::Caution => Span::styled(" ⚠ ", Style::default().fg(Color::Yellow)),
                purifier_core::SafetyLevel::Unsafe => Span::styled(" ✗ ", Style::default().fg(Color::Red)),
                purifier_core::SafetyLevel::Unknown => Span::styled(" ? ", Style::default().fg(Color::DarkGray)),
            };

            let name = entry
                .path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| entry.path.display().to_string());

            let size_str = format_size(entry.size);

            // Size bar (proportional to largest entry)
            let max_size = app
                .flat_entries
                .first()
                .map(|e| e.size)
                .unwrap_or(1)
                .max(1);
            let bar_width = 15;
            let filled = ((entry.size as f64 / max_size as f64) * bar_width as f64) as usize;
            let bar: String = "█".repeat(filled) + &"░".repeat(bar_width - filled);

            let style = if i == app.selected_index {
                Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let line = Line::from(vec![
                Span::raw(format!("{indent}{icon}")),
                Span::styled(
                    format!("{:<30}", if name.len() > 30 { &name[..30] } else { &name }),
                    style,
                ),
                Span::styled(format!(" {:<10}", size_str), Style::default().fg(Color::Cyan)),
                Span::styled(bar, Style::default().fg(Color::Blue)),
                safety_badge,
            ]);

            ListItem::new(line).style(style)
        })
        .collect();

    let list = List::new(items).block(Block::default().borders(Borders::ALL));
    frame.render_widget(list, main_area);

    // Info panel — safety reason for selected entry
    let info_text = if let Some(entry) = app.selected_entry() {
        if entry.safety_reason.is_empty() {
            format!("{}", entry.path.display())
        } else {
            format!("{} — {}", entry.path.display(), entry.safety_reason)
        }
    } else {
        "No selection".to_string()
    };

    let info = Paragraph::new(info_text)
        .block(Block::default().borders(Borders::ALL).title(" Info "));
    frame.render_widget(info, info_area);
}
