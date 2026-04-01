use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::{format_size, truncate_tail};
use crate::app::{App, LlmStatus, ScanStatus};

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

    if let Some((message, color)) = notice(app) {
        parts.push(Span::styled(message, Style::default().fg(color)));
    }

    let llm_status = match &app.llm_status {
        LlmStatus::Disabled => {
            Span::styled(" | LLM: disabled", Style::default().fg(Color::DarkGray))
        }
        LlmStatus::NeedsSetup => {
            Span::styled(" | LLM: needs setup", Style::default().fg(Color::Yellow))
        }
        LlmStatus::Connecting(provider) => Span::styled(
            format!(" | LLM: connecting {:?}", provider),
            Style::default().fg(Color::Yellow),
        ),
        LlmStatus::Ready(provider) => Span::styled(
            format!(" | LLM: {:?}", provider),
            Style::default().fg(Color::Green),
        ),
        LlmStatus::Error(message) => Span::styled(
            format!(" | LLM error: {message}"),
            Style::default().fg(Color::Red),
        ),
    };
    parts.push(llm_status);

    parts.push(Span::styled(
        help_text(app),
        Style::default().fg(Color::DarkGray),
    ));

    let status = Paragraph::new(Line::from(parts)).style(Style::default().bg(Color::Black));
    frame.render_widget(status, area);
}

fn help_text(app: &App) -> &'static str {
    if matches!(app.scan_status, ScanStatus::Complete) {
        " | s:settings  q:quit  1-4:tabs  j/k:nav  Enter:expand  d:delete "
    } else {
        " | settings after scan  q:quit  1-4:tabs  j/k:nav  Enter:expand  d:delete "
    }
}

fn notice(app: &App) -> Option<(String, Color)> {
    if let Some(error) = &app.last_error {
        return Some((format!(" | Error: {error}"), Color::Red));
    }

    app.last_warning
        .as_ref()
        .map(|warning| (format!(" | Warning: {warning}"), Color::Yellow))
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::{help_text, notice};
    use crate::app::{App, ScanStatus};
    use crate::config::AppConfig;

    #[test]
    fn help_text_should_only_advertise_settings_when_scan_is_complete() {
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            false,
            AppConfig::default(),
        );
        app.scan_status = ScanStatus::Scanning;
        assert!(!help_text(&app).contains("s:settings"));

        app.scan_status = ScanStatus::Complete;
        assert!(help_text(&app).contains("s:settings"));
    }

    #[test]
    fn notice_should_render_warnings_distinctly_from_errors() {
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            false,
            AppConfig::default(),
        );
        app.last_warning = Some("runtime override still active".to_string());

        assert_eq!(
            notice(&app),
            Some((
                " | Warning: runtime override still active".to_string(),
                Color::Yellow,
            ))
        );

        app.last_error = Some("save failed".to_string());
        assert_eq!(
            notice(&app),
            Some((" | Error: save failed".to_string(), Color::Red))
        );
    }
}
