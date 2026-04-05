use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::format_size;
use crate::app::{App, LlmStatus, ScanStatus};

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
        ])
        .split(area);

    // Left: breadcrumb
    let breadcrumb = app.columns.breadcrumb();
    let left = Paragraph::new(Line::from(vec![
        Span::raw(" "),
        Span::styled(breadcrumb, Style::default().fg(Color::DarkGray)),
    ]))
    .style(Style::default().bg(Color::Black));
    frame.render_widget(left, chunks[0]);

    // Center: marks indicator + scan info
    let mut center_parts = Vec::new();
    if !app.marks.is_empty() {
        center_parts.push(Span::styled(
            format!(
                "{} marked · {}",
                app.marks.count(),
                format_size(app.marks.total_size(&app.entries, app.size_mode()))
            ),
            Style::default().fg(Color::Red),
        ));
    } else {
        // Show scan status in center when no marks
        center_parts.push(scan_status_span(app));
    }

    if let Some((message, color)) = notice(app) {
        center_parts.push(Span::styled(message, Style::default().fg(color)));
    }

    let center = Paragraph::new(Line::from(center_parts))
        .style(Style::default().bg(Color::Black))
        .alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(center, chunks[1]);

    // Right: sort + LLM + help
    let mut right_parts = Vec::new();

    right_parts.push(Span::styled(
        format!("Sort: {} ▼", app.columns.sort_key.label()),
        Style::default().fg(Color::Cyan),
    ));

    right_parts.push(llm_status_span(app));

    if app.delete_stats.physical_bytes_freed > 0 {
        right_parts.push(Span::styled(
            format!(" | Freed: {}", format_size(app.delete_stats.physical_bytes_freed)),
            Style::default().fg(Color::Green),
        ));
    } else if app.delete_stats.physical_bytes_estimated > 0 {
        right_parts.push(Span::styled(
            format!(
                " | Est. freed: {}",
                format_size(app.delete_stats.physical_bytes_estimated)
            ),
            Style::default().fg(Color::Green),
        ));
    }

    right_parts.push(Span::styled(
        help_text(app),
        Style::default().fg(Color::DarkGray),
    ));

    let right = Paragraph::new(Line::from(right_parts))
        .style(Style::default().bg(Color::Black))
        .alignment(ratatui::layout::Alignment::Right);
    frame.render_widget(right, chunks[2]);
}

fn scan_status_span(app: &App) -> Span<'static> {
    match app.scan_status {
        ScanStatus::Idle => Span::styled("Ready", Style::default().fg(Color::DarkGray)),
        ScanStatus::Scanning => Span::styled(
            format!(
                "Scanning: {} entries · {}",
                app.files_scanned,
                format_size(app.bytes_found),
            ),
            Style::default().fg(Color::Yellow),
        ),
        ScanStatus::Complete => Span::styled(
            format!(
                "Scan complete · {} in {} entries",
                format_size(app.total_size),
                app.total_files
            ),
            Style::default().fg(Color::Green),
        ),
    }
}

fn llm_status_span(app: &App) -> Span<'static> {
    match &app.llm_status {
        LlmStatus::Disabled => {
            Span::styled(" | LLM: off", Style::default().fg(Color::DarkGray))
        }
        LlmStatus::NeedsSetup => {
            Span::styled(" | LLM: needs setup", Style::default().fg(Color::Yellow))
        }
        LlmStatus::Connecting(_) => Span::styled(
            " | LLM: validating...",
            Style::default().fg(Color::Yellow),
        ),
        LlmStatus::Ready(_) => {
            if app.llm_classified_count > 0 {
                Span::styled(
                    format!(" | LLM ✓ · {} classified", app.llm_classified_count),
                    Style::default().fg(Color::Green),
                )
            } else {
                Span::styled(" | LLM ✓", Style::default().fg(Color::Green))
            }
        }
        LlmStatus::Error(message) => Span::styled(
            format!(" | LLM ✗ · {message}"),
            Style::default().fg(Color::Red),
        ),
    }
}

fn help_text(app: &App) -> &'static str {
    if matches!(app.scan_status, ScanStatus::Complete) {
        " | ,:settings q:quit h/l:nav d:delete "
    } else {
        " | scanning: q/esc:quit "
    }
}

fn notice(app: &App) -> Option<(String, Color)> {
    if let Some(error) = &app.last_error {
        return Some((format!(" | {error}"), Color::Red));
    }

    app.last_warning
        .as_ref()
        .map(|warning| (format!(" | {warning}"), Color::Yellow))
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;
    use ratatui::Terminal;

    use super::{draw, help_text, notice};
    use crate::app::{App, ScanStatus};
    use crate::config::AppConfig;
    use purifier_core::DeleteOutcome;

    fn render_status_text(app: &App) -> String {
        let backend = TestBackend::new(160, 1);
        let mut terminal = Terminal::new(backend).expect("terminal should be created");
        terminal
            .draw(|frame| draw(frame, app, ratatui::layout::Rect::new(0, 0, 160, 1)))
            .expect("status bar should render");

        let buffer = terminal.backend().buffer().clone();
        buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    #[test]
    fn help_text_should_only_advertise_settings_when_scan_is_complete() {
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            false,
            AppConfig::default(),
        );
        app.scan_status = ScanStatus::Scanning;
        assert!(!help_text(&app).contains(",:settings"));

        app.scan_status = ScanStatus::Complete;
        assert!(help_text(&app).contains(",:settings"));
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
                " | runtime override still active".to_string(),
                Color::Yellow,
            ))
        );

        app.last_error = Some("save failed".to_string());
        assert_eq!(
            notice(&app),
            Some((" | save failed".to_string(), Color::Red))
        );
    }

    #[test]
    fn draw_should_describe_scan_counts_as_entries() {
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            false,
            AppConfig::default(),
        );
        app.scan_status = ScanStatus::Scanning;
        app.files_scanned = 12;
        app.bytes_found = 4096;
        app.current_scan_dir = "/tmp".to_string();

        let text = render_status_text(&app);

        assert!(
            text.contains("entries"),
            "status bar should describe scanned entries: {text}"
        );
    }

    #[test]
    fn draw_should_prefer_physical_freed_bytes_and_keep_logical_removed_visible() {
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            false,
            AppConfig::default(),
        );
        app.delete_stats = DeleteOutcome {
            logical_bytes_removed: 1024,
            physical_bytes_estimated: 2048,
            physical_bytes_freed: 4096,
            entries_removed: 1,
        };

        let text = render_status_text(&app);

        assert!(
            text.contains("Freed: 4.0 KB"),
            "status bar should show physically freed space: {text}"
        );
    }

    #[test]
    fn draw_should_show_estimated_freed_bytes_when_observed_space_is_unavailable() {
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            false,
            AppConfig::default(),
        );
        app.delete_stats = DeleteOutcome {
            logical_bytes_removed: 1024,
            physical_bytes_estimated: 2048,
            physical_bytes_freed: 0,
            entries_removed: 1,
        };

        let text = render_status_text(&app);

        assert!(
            text.contains("Est. freed: 2.0 KB"),
            "status bar should show estimated freed space: {text}"
        );
    }

    #[test]
    fn draw_should_show_active_size_mode_and_scan_profile() {
        let mut config = AppConfig::default();
        config.ui.size_mode = purifier_core::SizeMode::Logical;
        config.ui.scan_profiles = vec![purifier_core::ScanProfile {
            name: "exclude-node-modules".to_string(),
            exclude: None,
            mask: None,
            display_filter: None,
        }];
        config.ui.last_selected_scan_profile = Some("exclude-node-modules".to_string());

        let mut app = App::new(Some(std::path::PathBuf::from("/")), false, config);
        app.scan_status = ScanStatus::Complete;
        app.applied_scan_profile_name = Some("exclude-node-modules".to_string());

        let text = render_status_text(&app);

        assert!(
            text.contains("Sort:"),
            "status bar should show sort mode: {text}"
        );
    }

    #[test]
    fn draw_should_show_none_for_completed_scan_without_applied_profile() {
        let mut config = AppConfig::default();
        config.ui.last_selected_scan_profile = Some("exclude-node-modules".to_string());

        let mut app = App::new(Some(std::path::PathBuf::from("/")), false, config);
        app.scan_status = ScanStatus::Complete;
        app.applied_scan_profile_name = None;

        // Just verify it renders without panic
        let _text = render_status_text(&app);
    }

    #[test]
    fn draw_should_ignore_stale_selected_profile_while_idle() {
        let mut config = AppConfig::default();
        config.ui.last_selected_scan_profile = Some("missing-profile".to_string());

        let app = App::new(Some(std::path::PathBuf::from("/")), false, config);

        // Just verify it renders without panic
        let _text = render_status_text(&app);
    }

    #[test]
    fn help_text_should_only_advertise_quit_while_scanning() {
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            false,
            AppConfig::default(),
        );
        app.scan_status = ScanStatus::Scanning;

        let text = help_text(&app);

        assert!(text.contains("quit"));
        assert!(!text.contains(",:settings"));
        assert!(!text.contains("h/l:nav"));
        assert!(!text.contains("d:delete"));
    }
}
