use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::{App, PreviewMode};
use crate::columns::sorted_children;
use crate::ui::format_size;

use purifier_core::types::SafetyLevel;

/// Whether the column panes should be dimmed because a modal overlay is active.
fn is_dimmed(app: &App) -> bool {
    matches!(
        app.preview_mode,
        PreviewMode::Settings(_) | PreviewMode::Onboarding(_)
    )
}

fn safety_color(safety: SafetyLevel) -> Color {
    match safety {
        SafetyLevel::Safe => Color::Green,
        SafetyLevel::Caution => Color::Yellow,
        SafetyLevel::Unsafe => Color::Red,
        SafetyLevel::Unknown => Color::DarkGray,
    }
}

fn safety_badge(safety: SafetyLevel) -> &'static str {
    match safety {
        SafetyLevel::Safe => "\u{2713}",
        SafetyLevel::Caution => "\u{26a0}",
        SafetyLevel::Unsafe => "\u{2717}",
        SafetyLevel::Unknown => "?",
    }
}

fn file_display_name(path: &std::path::Path, is_dir: bool) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
    if is_dir {
        format!("{}/", name)
    } else {
        name
    }
}

/// Render the parent column (left pane) showing the parent directory's children.
///
/// The entry whose path matches the current column's directory is highlighted
/// with Cyan foreground; all other entries use DarkGray (dimmed).
pub fn render_parent_column(frame: &mut Frame, area: Rect, app: &App) {
    let dimmed = is_dimmed(app);

    let parent_col = match app.columns.parent() {
        Some(col) => col,
        None => {
            // No parent to display -- render an empty area with a header.
            let header = Block::default()
                .borders(Borders::BOTTOM)
                .title(" ")
                .border_style(Style::default().fg(Color::DarkGray));
            frame.render_widget(header, area);
            return;
        }
    };

    let children = match app.children_at_path(&parent_col.path) {
        Some(c) => c,
        None => return,
    };

    let sorted = sorted_children(children, app.columns.sort_key, app.size_mode());
    let current_dir_path = app.columns.current_path();

    // Reserve one row for the directory name header.
    let header_area = Rect {
        height: 1.min(area.height),
        ..area
    };
    let list_area = Rect {
        y: area.y + header_area.height,
        height: area.height.saturating_sub(header_area.height),
        ..area
    };

    // Header: parent directory name.
    let parent_name = parent_col
        .path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| parent_col.path.display().to_string());
    let header = Block::default()
        .title(format!(" {} ", parent_name))
        .title_style(Style::default().fg(Color::DarkGray))
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(Color::DarkGray));
    frame.render_widget(header, header_area);

    let visible_count = list_area.height as usize;
    if visible_count == 0 || sorted.is_empty() {
        return;
    }

    // Build lines for each visible entry.
    let lines: Vec<Line> = sorted
        .iter()
        .map(|&idx| {
            let entry = &children[idx];
            let name = file_display_name(&entry.path, entry.is_dir);
            let size_str = format_size(entry.total_size(app.size_mode()));
            let is_current = entry.path == current_dir_path;

            if dimmed {
                Line::from(vec![
                    Span::styled(
                        format!(" {}", name),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        format!(" {} ", size_str),
                        Style::default().fg(Color::DarkGray),
                    ),
                ])
            } else if is_current {
                Line::from(vec![
                    Span::styled(
                        format!(" {}", name),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::styled(
                        format!(" {} ", size_str),
                        Style::default().fg(Color::Cyan),
                    ),
                ])
            } else {
                Line::from(vec![
                    Span::styled(
                        format!(" {}", name),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        format!(" {} ", size_str),
                        Style::default().fg(Color::DarkGray),
                    ),
                ])
            }
        })
        .collect();

    // Determine scroll window: try to keep the selected (current dir) entry visible.
    let selected_pos = sorted
        .iter()
        .position(|&idx| children[idx].path == current_dir_path)
        .unwrap_or(0);

    let scroll_offset = if selected_pos >= visible_count {
        selected_pos.saturating_sub(visible_count - 1)
    } else {
        0
    };

    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(scroll_offset)
        .take(visible_count)
        .collect();

    let paragraph = Paragraph::new(visible_lines);
    frame.render_widget(paragraph, list_area);
}

/// Render the current column (center pane) showing sorted children of the
/// current directory.
///
/// Each row displays: mark indicator, name (with trailing / for dirs),
/// safety badge, and size. The selected row uses DarkGray background with bold.
pub fn render_current_column(frame: &mut Frame, area: Rect, app: &App) {
    let dimmed = is_dimmed(app);
    let col = app.columns.current();

    let children = match app.children_at_path(&col.path) {
        Some(c) => c,
        None => return,
    };

    let sorted = sorted_children(children, app.columns.sort_key, app.size_mode());

    // Reserve one row for the directory name header.
    let header_area = Rect {
        height: 1.min(area.height),
        ..area
    };
    let list_area = Rect {
        y: area.y + header_area.height,
        height: area.height.saturating_sub(header_area.height),
        ..area
    };

    // Header: current directory name.
    let dir_name = col
        .path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| col.path.display().to_string());
    let header_style = if dimmed {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::White)
    };
    let header = Block::default()
        .title(format!(" {} ", dir_name))
        .title_style(header_style)
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(Color::DarkGray));
    frame.render_widget(header, header_area);

    let visible_count = list_area.height as usize;
    if visible_count == 0 || sorted.is_empty() {
        return;
    }

    // Use the column's scroll_offset for viewport scrolling.
    let scroll_offset = col.scroll_offset;

    let visible_indices: Vec<usize> = sorted
        .iter()
        .copied()
        .skip(scroll_offset)
        .take(visible_count)
        .collect();

    let lines: Vec<Line> = visible_indices
        .iter()
        .enumerate()
        .map(|(visual_row, &idx)| {
            let entry = &children[idx];
            let absolute_row = scroll_offset + visual_row;
            let is_selected = absolute_row == col.selected_index;

            let name = file_display_name(&entry.path, entry.is_dir);
            let size_str = format_size(entry.total_size(app.size_mode()));
            let is_marked = app.marks.is_marked(&entry.path);

            if dimmed {
                // Entire row dimmed.
                let mark = if is_marked { "\u{2718} " } else { "  " };
                return Line::from(vec![
                    Span::styled(
                        format!(" {}", mark),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(name, Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!(" {} ", safety_badge(entry.safety)),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        format!(" {} ", size_str),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]);
            }

            let row_style = if is_selected {
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            // Mark indicator.
            let mark_span = if is_marked {
                Span::styled(
                    " \u{2718} ",
                    row_style.fg(Color::Red),
                )
            } else {
                Span::styled("   ", row_style)
            };

            // Name.
            let name_span = Span::styled(name, row_style);

            // Safety badge.
            let badge_span = Span::styled(
                format!(" {} ", safety_badge(entry.safety)),
                row_style.fg(safety_color(entry.safety)),
            );

            // Size.
            let size_span = Span::styled(
                format!(" {}", size_str),
                row_style.fg(Color::Cyan),
            );

            Line::from(vec![mark_span, name_span, badge_span, size_span])
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, list_area);
}

/// Render a sort indicator showing the active sort key, e.g. `Sort: [Size ▼]`.
pub fn render_sort_indicator(frame: &mut Frame, area: Rect, app: &App) {
    let label = app.columns.sort_key.label();
    let line = Line::from(vec![
        Span::styled("Sort: [", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{} \u{25bc}", label),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled("]", Style::default().fg(Color::DarkGray)),
    ]);
    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ScanStatus;
    use crate::config::AppConfig;
    use purifier_core::types::FileEntry;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;
    use std::path::PathBuf;

    fn make_app_with_entries(entries: Vec<FileEntry>) -> App {
        let mut app = App::new(Some(PathBuf::from("/")), false, AppConfig::default());
        app.entries = entries;
        app.scan_status = ScanStatus::Complete;
        app
    }

    fn render_to_buffer(
        draw_fn: impl FnOnce(&mut Frame),
        width: u16,
        height: u16,
    ) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal should be created");
        terminal
            .draw(draw_fn)
            .expect("should render");
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

    #[test]
    fn render_current_column_shows_entries_with_size() {
        let app = make_app_with_entries(vec![
            FileEntry::new(PathBuf::from("/alpha"), 2048, false, None),
            FileEntry::new(PathBuf::from("/beta"), 1024, false, None),
        ]);

        let area = Rect::new(0, 0, 40, 6);
        let buf = render_to_buffer(
            |frame| render_current_column(frame, area, &app),
            40,
            6,
        );
        let text = buffer_text(&buf);

        assert!(text.contains("alpha"), "should display file name: {text}");
        assert!(text.contains("beta"), "should display file name: {text}");
        assert!(text.contains("2.0 KB"), "should display size: {text}");
    }

    #[test]
    fn render_current_column_shows_mark_indicator() {
        let mut app = make_app_with_entries(vec![
            FileEntry::new(PathBuf::from("/marked-file"), 4096, false, None),
        ]);
        app.marks.toggle(std::path::Path::new("/marked-file"));

        let area = Rect::new(0, 0, 50, 4);
        let buf = render_to_buffer(
            |frame| render_current_column(frame, area, &app),
            50,
            4,
        );
        let text = buffer_text(&buf);

        assert!(
            text.contains("\u{2718}"),
            "marked entry should show cross mark: {text}"
        );
    }

    #[test]
    fn render_current_column_shows_dir_trailing_slash() {
        let mut dir = FileEntry::new(PathBuf::from("/mydir"), 0, true, None);
        dir.children = vec![FileEntry::new(
            PathBuf::from("/mydir/child"),
            100,
            false,
            None,
        )];
        let app = make_app_with_entries(vec![dir]);

        let area = Rect::new(0, 0, 40, 4);
        let buf = render_to_buffer(
            |frame| render_current_column(frame, area, &app),
            40,
            4,
        );
        let text = buffer_text(&buf);

        assert!(
            text.contains("mydir/"),
            "directory should show trailing slash: {text}"
        );
    }

    #[test]
    fn render_sort_indicator_shows_active_key() {
        let app = make_app_with_entries(vec![]);

        let area = Rect::new(0, 0, 20, 1);
        let buf = render_to_buffer(
            |frame| render_sort_indicator(frame, area, &app),
            20,
            1,
        );
        let text = buffer_text(&buf);

        assert!(
            text.contains("Size"),
            "sort indicator should show default key: {text}"
        );
        assert!(
            text.contains("\u{25bc}"),
            "sort indicator should show down arrow: {text}"
        );
    }

    #[test]
    fn render_parent_column_handles_no_parent() {
        let app = make_app_with_entries(vec![
            FileEntry::new(PathBuf::from("/file"), 100, false, None),
        ]);

        let area = Rect::new(0, 0, 30, 4);
        // Should not panic when there is no parent column.
        let _buf = render_to_buffer(
            |frame| render_parent_column(frame, area, &app),
            30,
            4,
        );
    }

    #[test]
    fn render_current_column_shows_safety_badges() {
        let mut safe_entry = FileEntry::new(PathBuf::from("/safe"), 100, false, None);
        safe_entry.safety = SafetyLevel::Safe;
        let mut caution_entry = FileEntry::new(PathBuf::from("/caution"), 200, false, None);
        caution_entry.safety = SafetyLevel::Caution;

        let app = make_app_with_entries(vec![safe_entry, caution_entry]);

        let area = Rect::new(0, 0, 50, 5);
        let buf = render_to_buffer(
            |frame| render_current_column(frame, area, &app),
            50,
            5,
        );
        let text = buffer_text(&buf);

        assert!(
            text.contains("\u{2713}"),
            "safe entry should show checkmark: {text}"
        );
        assert!(
            text.contains("\u{26a0}"),
            "caution entry should show warning: {text}"
        );
    }
}
