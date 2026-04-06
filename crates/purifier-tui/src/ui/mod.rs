pub mod columns_view;
pub mod dir_picker;
pub mod help_overlay;
pub mod onboarding;
pub mod preview_pane;
pub mod status_bar;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::Frame;

use crate::app::{App, AppScreen, PreviewMode, ScanStatus};

/// Layout for the main Miller Columns view.
#[derive(Debug, Clone, Copy)]
pub struct MillerLayout {
    pub sort_indicator: Rect,
    pub parent_column: Rect,
    pub current_column: Rect,
    pub preview: Rect,
    pub status: Rect,
}

pub fn draw(frame: &mut Frame, app: &App) {
    match app.screen {
        AppScreen::Onboarding => {
            onboarding::draw(frame, app);
        }
        AppScreen::DirPicker => {
            dir_picker::draw(frame, app);
        }
        AppScreen::Main => {
            draw_main(frame, app);
        }
    }
}

fn draw_main(frame: &mut Frame, app: &App) {
    let has_parent = app.columns.parent().is_some();
    let layout = miller_layout(frame.area(), has_parent);

    // Sort indicator row
    columns_view::render_sort_indicator(frame, layout.sort_indicator, app);

    // Three panes
    columns_view::render_parent_column(frame, layout.parent_column, app);
    columns_view::render_current_column(frame, layout.current_column, app);
    preview_pane::render_preview(frame, layout.preview, app);

    // Status bar
    status_bar::draw(frame, app, layout.status);

    // If scanning, draw progress overlay on top
    if app.scan_status == ScanStatus::Scanning {
        draw_scanning_overlay(frame, frame.area(), app);
    }

    // Help overlay on top of everything
    if matches!(app.preview_mode, PreviewMode::Help) {
        help_overlay::draw(frame, frame.area());
    }
}

pub fn miller_layout(area: Rect, has_parent: bool) -> MillerLayout {
    // Vertical split: sort indicator (1) | columns area | status bar (1)
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // sort indicator
            Constraint::Min(5),    // columns area
            Constraint::Length(1), // status bar
        ])
        .split(area);

    if has_parent {
        // Three-pane: parent | current | preview
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Ratio(1, 5),  // parent ~20%
                Constraint::Ratio(2, 5),  // current ~40%
                Constraint::Ratio(2, 5),  // preview ~40%
            ])
            .split(vertical[1]);

        MillerLayout {
            sort_indicator: vertical[0],
            parent_column: columns[0],
            current_column: columns[1],
            preview: columns[2],
            status: vertical[2],
        }
    } else {
        // Two-pane: current | preview (no parent at root)
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(0),    // no parent
                Constraint::Ratio(1, 2), // current ~50%
                Constraint::Ratio(1, 2), // preview ~50%
            ])
            .split(vertical[1]);

        MillerLayout {
            sort_indicator: vertical[0],
            parent_column: columns[0],
            current_column: columns[1],
            preview: columns[2],
            status: vertical[2],
        }
    }
}

fn draw_scanning_overlay(frame: &mut Frame, area: Rect, app: &App) {
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, Borders, Clear, Paragraph};

    let popup_width = 50u16.min(area.width.saturating_sub(4));
    let popup_height = 8u16.min(area.height.saturating_sub(4));

    let popup_area = centered_rect(area, popup_width, popup_height);
    frame.render_widget(Clear, popup_area);

    let dir_display = truncate_tail(&app.current_scan_dir, 45);
    let lines = vec![
        Line::from(Span::styled(
            "Scanning filesystem...",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("  Entries: "),
            Span::styled(
                format!("{}", app.files_scanned),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::raw("  Found:   "),
            Span::styled(
                format_size(app.bytes_found),
                Style::default().fg(Color::Cyan),
            ),
        ]),
        Line::from(vec![
            Span::raw("  Path:    "),
            Span::styled(dir_display, Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Press q to quit",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let popup = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Scanning ")
            .style(Style::default().fg(Color::White).bg(Color::Black)),
    );
    frame.render_widget(popup, popup_area);
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    use ratatui::layout::{Constraint, Flex, Layout};

    let width = width.min(area.width.saturating_sub(4));
    let height = height.min(area.height.saturating_sub(4));

    let vertical = Layout::vertical([Constraint::Length(height)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Length(width)]).flex(Flex::Center);
    let [v_area] = vertical.areas(area);
    let [h_area] = horizontal.areas(v_area);
    h_area
}

pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

pub fn truncate_tail(input: &str, max_chars: usize) -> String {
    let chars: Vec<char> = input.chars().collect();
    if chars.len() <= max_chars {
        return input.to_string();
    }

    let tail_len = max_chars.saturating_sub(3);
    let tail: String = chars[chars.len().saturating_sub(tail_len)..]
        .iter()
        .collect();
    format!("...{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_tail_should_preserve_unicode_boundaries() {
        assert_eq!(truncate_tail("ab😀c😀d", 5), "...😀d");
    }
}
