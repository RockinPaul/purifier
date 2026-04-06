use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::{App, PreviewMode};
use crate::columns::sorted_children_cached;
use crate::ui::format_size;

use purifier_core::types::SafetyLevel;

/// Whether the column panes should be dimmed because a modal overlay is active.
fn is_dimmed(app: &App) -> bool {
    matches!(
        app.preview_mode,
        PreviewMode::Settings(_) | PreviewMode::Onboarding(_) | PreviewMode::Help
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

/// Return a right-aligned size string that is exactly 8 characters wide.
fn format_size_fixed(bytes: u64) -> String {
    let s = format_size(bytes);
    format!("{:>8}", s)
}

/// Truncate a string to `max_chars` characters, appending an ellipsis if needed.
fn truncate_with_ellipsis(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{}\u{2026}", truncated)
}

/// Scroll a long string within a fixed-width window using a tick-based marquee.
/// Returns a substring of exactly `max_chars` characters from `s`, offset by a
/// scroll position derived from `tick`. Pauses at start and end.
fn scroll_name(s: &str, max_chars: usize, tick: u16) -> String {
    let char_count = s.chars().count();
    if char_count <= max_chars || max_chars == 0 {
        return s.to_string();
    }
    let overflow = char_count - max_chars;
    let offset = compute_scroll_offset(tick, overflow);
    s.chars().skip(offset).take(max_chars).collect()
}

/// Compute a scroll offset from a tick counter. Pauses at start and end,
/// scrolls forward then back.
fn compute_scroll_offset(tick: u16, overflow: usize) -> usize {
    if overflow == 0 {
        return 0;
    }
    const SPEED: u16 = 4; // advance 1 char every N frames (~4 chars/sec at 60fps)
    const PAUSE: u16 = 60; // 1 second pause at start and end

    let total_scroll_ticks = overflow as u16 * SPEED;
    let cycle = PAUSE + total_scroll_ticks + PAUSE + total_scroll_ticks;
    let pos = tick % cycle;

    if pos < PAUSE {
        0
    } else if pos < PAUSE + total_scroll_ticks {
        ((pos - PAUSE) / SPEED) as usize
    } else if pos < PAUSE + total_scroll_ticks + PAUSE {
        overflow
    } else {
        let back_pos = pos - PAUSE - total_scroll_ticks - PAUSE;
        overflow.saturating_sub((back_pos / SPEED) as usize)
    }
}

/// Pad a string to exactly `width` characters (left-aligned, space-filled).
fn pad_to_width(s: &str, width: usize) -> String {
    let char_count = s.chars().count();
    if char_count >= width {
        s.chars().take(width).collect()
    } else {
        let mut result = s.to_string();
        for _ in 0..(width - char_count) {
            result.push(' ');
        }
        result
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

    let mode = app.size_mode();
    let sorted = match app.get_sorted_children(&parent_col.path) {
        Some(s) => s.to_vec(),
        None => sorted_children_cached(children, app.columns.sort_key, |e| {
            app.cached_size(&e.path, mode)
        }),
    };
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

    // Layout constants for parent column:
    // prefix: " " (1 char), suffix: " {size_8} " (10 chars)
    let col_width = list_area.width as usize;
    let prefix_width: usize = 1; // leading space
    let suffix_width: usize = 10; // " " + 8-char size + " "
    let name_width = col_width.saturating_sub(prefix_width + suffix_width);

    // Build lines for each visible entry.
    let lines: Vec<Line> = sorted
        .iter()
        .map(|&idx| {
            let entry = &children[idx];
            let name = file_display_name(&entry.path, entry.is_dir);
            let size_fixed = format_size_fixed(app.cached_size(&entry.path, app.size_mode()));
            let is_current = entry.path == current_dir_path;

            // Truncate or pad the name to fill exactly name_width chars.
            let display_name = if name_width == 0 {
                String::new()
            } else {
                let truncated = truncate_with_ellipsis(&name, name_width);
                pad_to_width(&truncated, name_width)
            };

            let suffix_str = format!(" {} ", size_fixed);

            let fg = if dimmed {
                Color::DarkGray
            } else if is_current {
                Color::Cyan
            } else {
                Color::DarkGray
            };
            let style = Style::default().fg(fg);

            Line::from(vec![
                Span::styled(" ", style),
                Span::styled(display_name, style),
                Span::styled(suffix_str, style),
            ])
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

    let mode = app.size_mode();
    let sorted = match app.get_sorted_children(&col.path) {
        Some(s) => s.to_vec(),
        None => sorted_children_cached(children, app.columns.sort_key, |e| {
            app.cached_size(&e.path, mode)
        }),
    };

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

    // Layout constants for current column:
    // prefix: " ✘ " or "   " (3 chars), suffix: " {badge} {size_8} " (14 chars)
    // badge section: " " + 1-char badge + " " = 3 chars
    // size section:  8-char size + " " = 9 chars
    // total suffix = 1 (leading space before badge) + 1 (badge) + 1 (space) + 8 (size) + 1 (trailing space) = 12
    // Let's use: " {badge}  {size_8} " => " " + badge(1) + "  " + size(8) + " " = 13
    // Actually keeping it simple: badge_str = " {badge} " (3 chars), size_str = "{size_8} " (9 chars)
    // prefix(3) + name(variable) + badge(3) + size(9) = total
    let col_width = list_area.width as usize;
    let prefix_width: usize = 3; // " ✘ " or "   "
    let badge_width: usize = 3; // " {badge} "  -- space + 1 char + space
    let size_width: usize = 9; // 8-char right-aligned size + trailing space
    let suffix_width = badge_width + size_width; // 12
    let name_width = col_width.saturating_sub(prefix_width + suffix_width);

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
            let size_fixed = format_size_fixed(app.cached_size(&entry.path, app.size_mode()));
            let is_marked = app.marks.is_marked(&entry.path);

            // Truncate or pad the name to fill exactly name_width chars.
            // Selected row uses scroll animation; others use ellipsis truncation.
            let display_name = if name_width == 0 {
                String::new()
            } else if is_selected && name.chars().count() > name_width {
                let scrolled = scroll_name(&name, name_width, app.name_scroll_tick);
                pad_to_width(&scrolled, name_width)
            } else {
                let truncated = truncate_with_ellipsis(&name, name_width);
                pad_to_width(&truncated, name_width)
            };

            let badge_str = format!(" {} ", safety_badge(entry.safety));
            let size_str = format!("{} ", size_fixed);

            if dimmed {
                // Entire row dimmed.
                let mark = if is_marked { " \u{2718} " } else { "   " };
                let dim_style = Style::default().fg(Color::DarkGray);
                return Line::from(vec![
                    Span::styled(mark, dim_style),
                    Span::styled(display_name, dim_style),
                    Span::styled(badge_str, dim_style),
                    Span::styled(size_str, dim_style),
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
                Span::styled(" \u{2718} ", row_style.fg(Color::Red))
            } else {
                Span::styled("   ", row_style)
            };

            // Name (padded to fixed width).
            let name_span = Span::styled(display_name, row_style);

            // Safety badge (fixed width).
            let badge_span = Span::styled(
                badge_str,
                row_style.fg(safety_color(entry.safety)),
            );

            // Size (fixed width, right-aligned).
            let size_span = Span::styled(size_str, row_style.fg(Color::Cyan));

            Line::from(vec![mark_span, name_span, badge_span, size_span])
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, list_area);
}

/// Render a sort indicator showing the active sort key, e.g. `Sort: [Size ▼]  s:switch`.
pub fn render_sort_indicator(frame: &mut Frame, area: Rect, app: &App) {
    let label = app.columns.sort_key.label();
    let line = Line::from(vec![
        Span::styled("Sort: [", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{} \u{25bc}", label),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled("]  ", Style::default().fg(Color::DarkGray)),
        Span::styled("s", Style::default().fg(Color::Yellow)),
        Span::styled(":switch", Style::default().fg(Color::DarkGray)),
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
        app.rebuild_size_cache();
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
