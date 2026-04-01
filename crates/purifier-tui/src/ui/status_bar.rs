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
                    "Scanning... {} entries | {} found | {}",
                    app.files_scanned,
                    format_size(app.bytes_found),
                    dir_display,
                ),
                Style::default().fg(Color::Yellow),
            )
        }
        ScanStatus::Complete => Span::styled(
            format!(
                "Done — {} in {} entries",
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

    parts.push(Span::styled(
        format!(" | Size: {:?}", app.size_mode()),
        Style::default().fg(Color::Cyan),
    ));
    parts.push(Span::styled(
        format!(
            " | Profile: {}",
            app.active_scan_profile_name().unwrap_or("none")
        ),
        Style::default().fg(Color::DarkGray),
    ));

    if app.delete_stats.physical_bytes_freed > 0 {
        parts.push(Span::styled(
            format!(
                " | Freed: {}",
                format_size(app.delete_stats.physical_bytes_freed)
            ),
            Style::default().fg(Color::Green),
        ));
    } else if app.delete_stats.physical_bytes_estimated > 0 {
        parts.push(Span::styled(
            format!(
                " | Est. freed: {}",
                format_size(app.delete_stats.physical_bytes_estimated)
            ),
            Style::default().fg(Color::Green),
        ));
    }

    if app.delete_stats.logical_bytes_removed > 0 {
        parts.push(Span::styled(
            format!(
                " | Removed: {}",
                format_size(app.delete_stats.logical_bytes_removed)
            ),
            Style::default().fg(Color::DarkGray),
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
        " | scanning: q/esc:quit "
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
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;
    use ratatui::Terminal;

    use super::{draw, help_text, notice};
    use crate::app::{App, ScanStatus};
    use crate::config::AppConfig;
    use purifier_core::DeleteOutcome;

    fn render_status_text(app: &App) -> String {
        let backend = TestBackend::new(100, 1);
        let mut terminal = Terminal::new(backend).expect("terminal should be created");
        terminal
            .draw(|frame| draw(frame, app, ratatui::layout::Rect::new(0, 0, 100, 1)))
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
            "status bar should describe scanned entries truthfully: {text}"
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
            "status bar should show physically freed space when known: {text}"
        );
        assert!(
            text.contains("Removed: 1.0 KB"),
            "status bar should keep logical removed bytes visible: {text}"
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
            "status bar should keep estimated freed space visible: {text}"
        );
        assert!(
            text.contains("Removed: 1.0 KB"),
            "status bar should keep logical removed bytes visible: {text}"
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
            text.contains("Size: Logical"),
            "status bar should show size mode: {text}"
        );
        assert!(
            text.contains("Profile: exclude-node-modules"),
            "status bar should show active profile: {text}"
        );
    }

    #[test]
    fn draw_should_show_none_for_completed_scan_without_applied_profile() {
        let mut config = AppConfig::default();
        config.ui.last_selected_scan_profile = Some("exclude-node-modules".to_string());

        let mut app = App::new(Some(std::path::PathBuf::from("/")), false, config);
        app.scan_status = ScanStatus::Complete;
        app.applied_scan_profile_name = None;

        let text = render_status_text(&app);

        assert!(
            text.contains("Profile: none"),
            "completed scan without an applied profile should not reuse the saved default: {text}"
        );
    }

    #[test]
    fn draw_should_ignore_stale_selected_profile_while_idle() {
        let mut config = AppConfig::default();
        config.ui.last_selected_scan_profile = Some("missing-profile".to_string());

        let app = App::new(Some(std::path::PathBuf::from("/")), false, config);

        let text = render_status_text(&app);

        assert!(
            text.contains("Profile: none"),
            "idle status should not surface a non-existent saved profile: {text}"
        );
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
        assert!(!text.contains("1-4:tabs"));
        assert!(!text.contains("j/k:nav"));
        assert!(!text.contains("Enter:expand"));
        assert!(!text.contains("d:delete"));
    }
}
