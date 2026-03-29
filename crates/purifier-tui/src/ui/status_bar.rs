use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::{format_size, truncate_tail};
use crate::app::{App, ScanStatus};

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let scan_info = match app.scan_status {
        ScanStatus::Idle => Span::styled("Ready", Style::default().fg(Color::DarkGray)),
        ScanStatus::Scanning => {
            let dir_display = truncate_tail(&app.current_scan_dir, 40);
            Span::styled(
                format!(
                    "Scanning... {} files | {} found | {}",
                    app.files_scanned,
                    format_size(app.bytes_found),
                    dir_display,
                ),
                Style::default().fg(Color::Yellow),
            )
        }
        ScanStatus::Complete => Span::styled(
            format!(
                "Done — {} in {} files",
                format_size(app.total_size),
                app.total_files
            ),
            Style::default().fg(Color::Green),
        ),
    };

    let mut parts = vec![Span::raw(" "), scan_info];

    if app.skipped > 0 {
        parts.push(Span::styled(
            format!(" | {} skipped", app.skipped),
            Style::default().fg(Color::DarkGray),
        ));
    }

    if app.freed_space > 0 {
        parts.push(Span::styled(
            format!(" | Freed: {}", format_size(app.freed_space)),
            Style::default().fg(Color::Green),
        ));
    }

    if let Some(error) = &app.last_error {
        parts.push(Span::styled(
            format!(" | Error: {error}"),
            Style::default().fg(Color::Red),
        ));
    }

    if app.llm_enabled {
        let llm_status = if app.llm_online {
            Span::styled(" | LLM: online", Style::default().fg(Color::Green))
        } else {
            Span::styled(" | LLM: offline", Style::default().fg(Color::DarkGray))
        };
        parts.push(llm_status);
    }

    parts.push(Span::styled(
        " | q:quit  1-4:tabs  j/k:nav  Enter:expand  d:delete ",
        Style::default().fg(Color::DarkGray),
    ));

    let status = Paragraph::new(Line::from(parts)).style(Style::default().bg(Color::Black));
    frame.render_widget(status, area);
}
