use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::{App, LlmStatus, PreviewMode, SettingsDraft};
use crate::columns::find_entry;
use crate::ui::disclosures::current_storage_and_privacy_lines;
use crate::ui::format_size;
use purifier_core::provider::ProviderKind;
use purifier_core::size::SizeMode;
use purifier_core::types::{Category, FileEntry, SafetyLevel};

/// Main entry point: render the right-most preview pane based on `app.preview_mode`.
pub fn render_preview(frame: &mut Frame, area: Rect, app: &App) {
    match &app.preview_mode {
        PreviewMode::Analytics | PreviewMode::Help => render_analytics(frame, area, app),
        PreviewMode::DeleteConfirm(path) => render_delete_confirm(frame, area, app, path.clone()),
        PreviewMode::BatchReview => render_batch_review(frame, area, app),
        PreviewMode::Settings(draft) => render_settings(frame, area, app, draft, false),
        PreviewMode::Onboarding(draft) => render_settings(frame, area, app, draft, true),
    }
}

// ---------------------------------------------------------------------------
// Analytics
// ---------------------------------------------------------------------------

fn render_analytics(frame: &mut Frame, area: Rect, app: &App) {
    let entry = match app.selected_entry() {
        Some(e) => e,
        None => {
            let empty = Paragraph::new(Line::from(Span::styled(
                "  Empty directory",
                Style::default().fg(Color::DarkGray),
            )))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Preview "),
            );
            frame.render_widget(empty, area);
            return;
        }
    };

    let mode = app.size_mode();
    let mut lines: Vec<Line<'static>> = Vec::new();

    if entry.is_dir {
        render_dir_analytics(&mut lines, entry, app, mode);
    } else {
        render_file_analytics(&mut lines, entry, app, mode);
    }

    let widget = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Preview "),
    );
    frame.render_widget(widget, area);
}

fn render_dir_analytics(
    lines: &mut Vec<Line<'static>>,
    entry: &FileEntry,
    app: &App,
    mode: SizeMode,
) {
    // Safety verdict
    let (badge_text, badge_color) = safety_badge(entry.safety);
    lines.push(Line::from(vec![
        Span::raw("  Safety: "),
        Span::styled(badge_text, Style::default().fg(badge_color).add_modifier(Modifier::BOLD)),
    ]));

    if !entry.safety_reason.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  {}", entry.safety_reason),
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));

    // Size information
    let logical = app.cached_size(&entry.path, SizeMode::Logical);
    let physical = app.cached_size(&entry.path, SizeMode::Physical);
    let display = app.cached_size(&entry.path, mode);

    lines.push(Line::from(vec![
        Span::raw("  Size: "),
        Span::styled(format_size(display), Style::default().fg(Color::Cyan)),
    ]));

    if logical != physical {
        lines.push(Line::from(vec![
            Span::styled("  Logical: ", Style::default().fg(Color::DarkGray)),
            Span::raw(format_size(logical)),
            Span::styled("  Physical: ", Style::default().fg(Color::DarkGray)),
            Span::raw(format_size(physical)),
        ]));
    }

    // Children count
    let children = app
        .children_at_path(&entry.path)
        .unwrap_or(&[]);
    lines.push(Line::from(vec![
        Span::raw("  Children: "),
        Span::styled(
            children.len().to_string(),
            Style::default().fg(Color::White),
        ),
    ]));

    lines.push(Line::from(""));

    // By type — use cached analytics when available
    let by_category = match &app.preview_cache {
        Some(analytics) => analytics.by_category.clone(),
        None => aggregate_by_category(children, &app.size_cache, mode),
    };
    if !by_category.is_empty() {
        lines.push(Line::from(Span::styled(
            "  By type",
            Style::default().add_modifier(Modifier::BOLD),
        )));

        let max_cat_size = by_category.first().map(|(_, s)| *s).unwrap_or(1).max(1);
        for (cat, size) in &by_category {
            let bar_width: usize = 12;
            let filled =
                ((*size as f64 / max_cat_size as f64) * bar_width as f64).round() as usize;
            let filled = filled.min(bar_width);
            let empty = bar_width.saturating_sub(filled);
            let bar = format!(
                "{}{}",
                "\u{2588}".repeat(filled),
                "\u{2591}".repeat(empty),
            );

            lines.push(Line::from(vec![
                Span::styled(format!("    {:<16}", cat.to_string()), Style::default()),
                Span::styled(bar, Style::default().fg(category_color(*cat))),
                Span::raw(format!(" {}", format_size(*size))),
            ]));
        }

        lines.push(Line::from(""));
    }

    // By age — use cached analytics when available
    let by_age = match &app.preview_cache {
        Some(analytics) => analytics.by_age.clone(),
        None => aggregate_by_age(children),
    };
    if by_age.iter().any(|(_, s)| *s > 0) {
        lines.push(Line::from(Span::styled(
            "  By age",
            Style::default().add_modifier(Modifier::BOLD),
        )));

        let max_age_size = by_age
            .iter()
            .map(|(_, s)| *s)
            .max()
            .unwrap_or(1)
            .max(1);
        for (label, size) in &by_age {
            let bar_width: usize = 12;
            let filled =
                ((*size as f64 / max_age_size as f64) * bar_width as f64).round() as usize;
            let filled = filled.min(bar_width);
            let empty = bar_width.saturating_sub(filled);
            let bar = format!(
                "{}{}",
                "\u{2588}".repeat(filled),
                "\u{2591}".repeat(empty),
            );

            lines.push(Line::from(vec![
                Span::styled(format!("    {:<16}", label), Style::default()),
                Span::styled(bar, Style::default().fg(Color::Blue)),
                Span::raw(format!(" {}", format_size(*size))),
            ]));
        }
    }
}

fn render_file_analytics(lines: &mut Vec<Line<'static>>, entry: &FileEntry, app: &App, mode: SizeMode) {
    // Safety verdict
    let (badge_text, badge_color) = safety_badge(entry.safety);
    lines.push(Line::from(vec![
        Span::raw("  Safety: "),
        Span::styled(badge_text, Style::default().fg(badge_color).add_modifier(Modifier::BOLD)),
    ]));

    if !entry.safety_reason.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  {}", entry.safety_reason),
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));

    // Size
    let display = app.cached_size(&entry.path, mode);
    lines.push(Line::from(vec![
        Span::raw("  Size: "),
        Span::styled(format_size(display), Style::default().fg(Color::Cyan)),
    ]));

    // Category
    lines.push(Line::from(vec![
        Span::raw("  Category: "),
        Span::styled(
            entry.category.to_string(),
            Style::default().fg(category_color(entry.category)),
        ),
    ]));

    // Last modified
    if let Some(modified) = entry.modified {
        lines.push(Line::from(vec![
            Span::raw("  Modified: "),
            Span::styled(
                relative_time(modified),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    lines.push(Line::from(""));

    // Full path
    lines.push(Line::from(vec![
        Span::styled("  Path: ", Style::default().fg(Color::DarkGray)),
        Span::raw(entry.path.display().to_string()),
    ]));
}

// ---------------------------------------------------------------------------
// Delete confirm
// ---------------------------------------------------------------------------

fn render_delete_confirm(frame: &mut Frame, area: Rect, app: &App, path: PathBuf) {
    let entry = find_entry(&app.entries, &path);

    let mut lines: Vec<Line<'static>> = Vec::new();

    if let Some(entry) = entry {
        let (badge_text, badge_color) = safety_badge(entry.safety);

        lines.push(Line::from(vec![
            Span::styled("  Path: ", Style::default().fg(Color::DarkGray)),
            Span::raw(entry.path.display().to_string()),
        ]));

        lines.push(Line::from(vec![
            Span::raw("  Logical size: "),
            Span::raw(format_size(app.cached_size(&entry.path, SizeMode::Logical))),
        ]));

        lines.push(Line::from(vec![
            Span::raw("  Est. physical freed: "),
            Span::raw(format_size(app.cached_size(&entry.path, SizeMode::Physical))),
        ]));

        lines.push(Line::from(vec![
            Span::raw("  Safety: "),
            Span::styled(badge_text, Style::default().fg(badge_color)),
        ]));

        if !entry.safety_reason.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("  {}", entry.safety_reason),
                Style::default().fg(Color::DarkGray),
            )));
        }
    } else {
        lines.push(Line::from(Span::styled(
            format!("  Path: {}", path.display()),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from("  (entry not found in tree)"));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(" [y] ", Style::default().fg(Color::Red)),
        Span::raw("Delete  "),
        Span::styled(" [n] ", Style::default().fg(Color::Green)),
        Span::raw("Cancel"),
    ]));

    let widget = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Delete? ")
            .border_style(Style::default().fg(Color::Red)),
    );
    frame.render_widget(widget, area);
}

// ---------------------------------------------------------------------------
// Batch review
// ---------------------------------------------------------------------------

fn render_batch_review(frame: &mut Frame, area: Rect, app: &App) {
    let paths = app.marks.paths();
    let count = paths.len();
    let mode = app.size_mode();

    let mut lines: Vec<Line<'static>> = Vec::new();

    // Scrollable list of marked items
    let inner_height = area.height.saturating_sub(2) as usize; // account for border
    let list_budget = inner_height.saturating_sub(4); // room for totals + footer

    let scroll_offset = if app.batch_review_selected >= list_budget {
        app.batch_review_selected.saturating_sub(list_budget - 1)
    } else {
        0
    };

    let visible = paths
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(list_budget);

    for (i, path) in visible {
        let entry = find_entry(&app.entries, path);
        let size_str = entry
            .map(|e| format_size(app.cached_size(&e.path, mode)))
            .unwrap_or_else(|| "?".to_string());
        let (badge, color) = entry
            .map(|e| safety_badge(e.safety))
            .unwrap_or(("Unknown".to_string(), Color::DarkGray));

        let display_path = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());

        let style = if i == app.batch_review_selected {
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        lines.push(Line::from(vec![
            Span::styled(format!("  {:<30}", display_path), style),
            Span::styled(format!(" {:>10}", size_str), Style::default().fg(Color::Cyan)),
            Span::styled(format!(" {}", badge), Style::default().fg(color)),
        ]));
    }

    lines.push(Line::from(""));

    // Totals
    let total_logical: u64 = app.marks.paths().iter().map(|p| app.cached_size(p, SizeMode::Logical)).sum();
    let total_physical: u64 = app.marks.paths().iter().map(|p| app.cached_size(p, SizeMode::Physical)).sum();

    lines.push(Line::from(vec![
        Span::styled("  Total logical: ", Style::default().fg(Color::DarkGray)),
        Span::raw(format_size(total_logical)),
        Span::styled("  Est. physical freed: ", Style::default().fg(Color::DarkGray)),
        Span::raw(format_size(total_physical)),
    ]));

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(" [y] ", Style::default().fg(Color::Red)),
        Span::raw("Delete all  "),
        Span::styled(" [n] ", Style::default().fg(Color::Green)),
        Span::raw("Cancel  "),
        Span::styled(" [Space] ", Style::default().fg(Color::Yellow)),
        Span::raw("Unmark"),
    ]));

    let title = format!(" Delete {} items? ", count);
    let widget = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Color::Red)),
    );
    frame.render_widget(widget, area);
}

// ---------------------------------------------------------------------------
// Settings / Onboarding
// ---------------------------------------------------------------------------

fn render_settings(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    draft: &SettingsDraft,
    is_onboarding: bool,
) {
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Header
    let header = if is_onboarding {
        "  First Launch Setup"
    } else {
        "  Settings"
    };
    lines.push(Line::from(Span::styled(
        header,
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    // Provider row (only OpenRouter and OpenAI are functional)
    let providers: &[(ProviderKind, &str, bool)] = &[
        (ProviderKind::OpenRouter, "1:OpenRouter", true),
        (ProviderKind::OpenAI, "2:OpenAI", true),
        (ProviderKind::Anthropic, "Anthropic (soon)", false),
        (ProviderKind::Google, "Google (soon)", false),
    ];
    let mut provider_spans: Vec<Span<'static>> = vec![Span::raw("  Provider: ")];
    for &(kind, label, available) in providers {
        let style = if !available {
            Style::default().fg(Color::DarkGray)
        } else if kind == draft.provider {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        provider_spans.push(Span::styled(format!(" {} ", label), style));
    }
    lines.push(Line::from(provider_spans));

    // API Key row
    let key_display = api_key_display(draft);
    let mut key_spans: Vec<Span<'static>> = vec![
        Span::raw("  API Key: "),
        Span::styled(key_display, Style::default().fg(Color::White)),
    ];
    if draft.api_key_editing {
        key_spans.push(Span::styled(
            "\u{2588}",
            Style::default().fg(Color::White),
        ));
    } else {
        key_spans.push(Span::styled("  [a] edit", Style::default().fg(Color::DarkGray)));
    }
    lines.push(Line::from(key_spans));

    // Model row (read-only)
    lines.push(Line::from(vec![
        Span::raw("  Model: "),
        Span::styled(draft.model.clone(), Style::default().fg(Color::DarkGray)),
    ]));

    // Size Mode row
    let physical_style = if draft.size_mode == SizeMode::Physical {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let logical_style = if draft.size_mode == SizeMode::Logical {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    lines.push(Line::from(vec![
        Span::raw("  Size Mode: "),
        Span::styled("Physical", physical_style),
        Span::raw(" / "),
        Span::styled("Logical", logical_style),
        Span::styled("  [m] toggle", Style::default().fg(Color::DarkGray)),
    ]));

    // Scan Profile row
    let profile_name = draft
        .selected_scan_profile
        .as_deref()
        .unwrap_or("none");
    lines.push(Line::from(vec![
        Span::raw("  Scan Profile: "),
        Span::styled(profile_name.to_string(), Style::default().fg(Color::White)),
        Span::styled("  [p] cycle", Style::default().fg(Color::DarkGray)),
    ]));

    // LLM status indicator
    let (status_text, status_color) = match &app.llm_status {
        LlmStatus::Disabled => ("LLM: disabled".to_string(), Color::DarkGray),
        LlmStatus::NeedsSetup => ("LLM: needs setup".to_string(), Color::Yellow),
        LlmStatus::Connecting(p) => (format!("LLM: connecting {:?}...", p), Color::Yellow),
        LlmStatus::Ready(p) => (format!("LLM: connected ({:?})", p), Color::Green),
        LlmStatus::Error(msg) => (format!("LLM: {}", msg), Color::Red),
    };
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(status_text, Style::default().fg(status_color)),
    ]));

    // Saving indicator
    if app.settings_modal_is_saving {
        lines.push(Line::from(Span::styled(
            format!("  Saving and validating {:?} connection...", draft.provider),
            Style::default().fg(Color::Cyan),
        )));
    }

    // Error message
    if let Some(error) = &app.settings_modal_error {
        lines.push(Line::from(Span::styled(
            format!("  {}", error),
            Style::default().fg(Color::Red),
        )));
    }

    lines.push(Line::from(""));
    lines.extend(current_storage_and_privacy_lines());
    lines.push(Line::from(""));

    // Gesture mapping reference (settings only, not onboarding). Hide it first
    // on shorter panes so the privacy/storage disclosure remains visible.
    if !is_onboarding && area.height >= 26 {
        lines.push(Line::from(Span::styled(
            "  Mouse & Trackpad",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )));
        let gesture_hint = Style::default().fg(Color::DarkGray);
        for (gesture, action) in [
            ("Left click      ", "Select entry"),
            ("Double-click    ", "Open dir / mark file"),
            ("Right click     ", "Go back"),
            ("Middle click    ", "Toggle mark"),
            ("Scroll \u{2191}/\u{2193}      ", "Move selection"),
            ("Swipe \u{2190}/\u{2192}       ", "Navigate back/forward"),
            ("Click parent    ", "Go to parent dir"),
            ("Click preview   ", "Enter selected dir"),
        ] {
            lines.push(Line::from(vec![
                Span::styled(format!("    {gesture}"), gesture_hint),
                Span::raw(action),
            ]));
        }
        lines.push(Line::from(""));
    }

    // Footer
    if is_onboarding {
        lines.push(Line::from(vec![
            Span::styled(" [Enter] ", Style::default().fg(Color::Green)),
            Span::raw("Save & start  "),
            Span::styled(" [Esc] ", Style::default().fg(Color::Yellow)),
            Span::raw("Skip"),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled(" [Enter] ", Style::default().fg(Color::Green)),
            Span::raw("Save  "),
            Span::styled(" [Esc] ", Style::default().fg(Color::Yellow)),
            Span::raw("Cancel"),
        ]));
    }

    let title = if is_onboarding {
        " First Launch Setup "
    } else {
        " Settings "
    };
    let widget = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title),
    );
    frame.render_widget(widget, area);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn safety_badge(safety: SafetyLevel) -> (String, Color) {
    match safety {
        SafetyLevel::Safe => ("Safe".to_string(), Color::Green),
        SafetyLevel::Caution => ("Caution".to_string(), Color::Yellow),
        SafetyLevel::Unsafe => ("Unsafe".to_string(), Color::Red),
        SafetyLevel::Unknown => ("Unknown".to_string(), Color::DarkGray),
    }
}

fn category_color(category: Category) -> Color {
    match category {
        Category::BuildArtifact => Color::Yellow,
        Category::Cache => Color::Cyan,
        Category::Download => Color::Blue,
        Category::AppData => Color::Magenta,
        Category::Media => Color::Green,
        Category::System => Color::Red,
        Category::Unknown => Color::DarkGray,
    }
}

fn api_key_display(draft: &SettingsDraft) -> String {
    if draft.api_key_editing {
        if draft.api_key.is_empty() {
            return String::new();
        }
        return "*".repeat(draft.api_key.len());
    }

    if draft.api_key_edited {
        if draft.api_key.is_empty() {
            return "<will clear on save>".to_string();
        }
        let len = draft.api_key.len();
        if len <= 4 {
            return "*".repeat(len);
        }
        let masked = "*".repeat(len - 4);
        let last4 = &draft.api_key[len - 4..];
        return format!("{}{}", masked, last4);
    }

    if draft.api_key.is_empty() {
        "<not set>".to_string()
    } else {
        let len = draft.api_key.len();
        if len <= 4 {
            "*".repeat(len)
        } else {
            let masked = "*".repeat(len - 4);
            let last4 = &draft.api_key[len - 4..];
            format!("{}{}", masked, last4)
        }
    }
}

fn relative_time(time: SystemTime) -> String {
    let now = SystemTime::now();
    let elapsed = match now.duration_since(time) {
        Ok(d) => d,
        Err(_) => return "in the future".to_string(),
    };

    let secs = elapsed.as_secs();
    if secs < 60 {
        return "just now".to_string();
    }

    let minutes = secs / 60;
    if minutes < 60 {
        return if minutes == 1 {
            "1 minute ago".to_string()
        } else {
            format!("{} minutes ago", minutes)
        };
    }

    let hours = minutes / 60;
    if hours < 24 {
        return if hours == 1 {
            "1 hour ago".to_string()
        } else {
            format!("{} hours ago", hours)
        };
    }

    let days = hours / 24;
    if days < 30 {
        return if days == 1 {
            "1 day ago".to_string()
        } else {
            format!("{} days ago", days)
        };
    }

    let months = days / 30;
    if months < 12 {
        return if months == 1 {
            "1 month ago".to_string()
        } else {
            format!("{} months ago", months)
        };
    }

    let years = months / 12;
    if years == 1 {
        "1 year ago".to_string()
    } else {
        format!("{} years ago", years)
    }
}

/// Aggregate children sizes by `Category`, sorted by size descending.
pub fn aggregate_by_category(children: &[FileEntry], cache: &HashMap<PathBuf, (u64, u64)>, mode: SizeMode) -> Vec<(Category, u64)> {
    let mut map: HashMap<Category, u64> = HashMap::new();

    for child in children {
        let size = cache.get(&child.path).map(|&(l, p)| match mode { SizeMode::Logical => l, SizeMode::Physical => p }).unwrap_or(0);
        *map.entry(child.category).or_insert(0) += size;
    }

    let mut result: Vec<_> = map.into_iter().collect();
    result.sort_by(|a, b| b.1.cmp(&a.1));
    result
}

/// Aggregate children sizes into three age buckets: >90 days, 30-90 days, <30 days.
pub fn aggregate_by_age(children: &[FileEntry]) -> Vec<(&'static str, u64)> {
    let now = SystemTime::now();
    let mut old: u64 = 0; // >90d
    let mut mid: u64 = 0; // 30-90d
    let mut recent: u64 = 0; // <30d

    let secs_30d: u64 = 30 * 24 * 3600;
    let secs_90d: u64 = 90 * 24 * 3600;

    for child in children {
        let size = child.sizes.logical_bytes;
        match child.modified {
            Some(modified) => {
                let age_secs = now
                    .duration_since(modified)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                if age_secs > secs_90d {
                    old += size;
                } else if age_secs > secs_30d {
                    mid += size;
                } else {
                    recent += size;
                }
            }
            None => {
                // Unknown age goes into the oldest bucket
                old += size;
            }
        }
    }

    vec![
        (">90 days", old),
        ("30-90 days", mid),
        ("<30 days", recent),
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use purifier_core::size::EntrySizes;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    fn make_file(
        path: &str,
        size: u64,
        category: Category,
        modified: Option<SystemTime>,
    ) -> FileEntry {
        let mut entry = FileEntry::new(PathBuf::from(path), size, false, modified);
        entry.category = category;
        entry
    }

    fn build_cache(entries: &[FileEntry]) -> HashMap<PathBuf, (u64, u64)> {
        entries.iter().map(|e| {
            (e.path.clone(), (e.sizes.logical_bytes, e.sizes.physical_bytes))
        }).collect()
    }

    fn render_to_buffer(
        draw_fn: impl FnOnce(&mut Frame),
        width: u16,
        height: u16,
    ) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal should be created");
        terminal.draw(draw_fn).expect("should render");
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

    // -- aggregate_by_category tests --

    #[test]
    fn aggregate_by_category_should_group_and_sum_by_category() {
        let children = vec![
            make_file("/a", 100, Category::Cache, None),
            make_file("/b", 200, Category::Cache, None),
            make_file("/c", 50, Category::Media, None),
        ];
        let cache = build_cache(&children);

        let result = aggregate_by_category(&children, &cache, SizeMode::Logical);

        assert_eq!(result.len(), 2);
        assert_eq!(result[0], (Category::Cache, 300));
        assert_eq!(result[1], (Category::Media, 50));
    }

    #[test]
    fn aggregate_by_category_should_sort_by_size_descending() {
        let children = vec![
            make_file("/a", 10, Category::System, None),
            make_file("/b", 500, Category::Download, None),
            make_file("/c", 200, Category::BuildArtifact, None),
        ];
        let cache = build_cache(&children);

        let result = aggregate_by_category(&children, &cache, SizeMode::Logical);

        assert_eq!(result[0].0, Category::Download);
        assert_eq!(result[1].0, Category::BuildArtifact);
        assert_eq!(result[2].0, Category::System);
    }

    #[test]
    fn aggregate_by_category_should_return_empty_for_no_children() {
        let cache = HashMap::new();
        let result = aggregate_by_category(&[], &cache, SizeMode::Logical);
        assert!(result.is_empty());
    }

    #[test]
    fn aggregate_by_category_should_handle_single_category() {
        let children = vec![
            make_file("/a", 100, Category::AppData, None),
            make_file("/b", 200, Category::AppData, None),
        ];
        let cache = build_cache(&children);

        let result = aggregate_by_category(&children, &cache, SizeMode::Logical);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0], (Category::AppData, 300));
    }

    #[test]
    fn aggregate_by_category_should_respect_size_mode() {
        let mut entry = FileEntry::new_with_sizes(
            PathBuf::from("/phys"),
            EntrySizes {
                logical_bytes: 100,
                physical_bytes: 4096,
                accounted_physical_bytes: 4096,
            },
            None,
            false,
            None,
        );
        entry.category = Category::Cache;

        let cache_logical = build_cache(&[entry.clone()]);
        let logical = aggregate_by_category(&[entry.clone()], &cache_logical, SizeMode::Logical);
        let cache_physical = build_cache(&[entry.clone()]);
        let physical = aggregate_by_category(&[entry], &cache_physical, SizeMode::Physical);

        assert_eq!(logical[0].1, 100);
        assert_eq!(physical[0].1, 4096);
    }

    // -- aggregate_by_age tests --

    #[test]
    fn aggregate_by_age_should_bucket_into_three_time_ranges() {
        let now = SystemTime::now();
        let recent = now - Duration::from_secs(10 * 24 * 3600); // 10 days
        let mid = now - Duration::from_secs(60 * 24 * 3600); // 60 days
        let old = now - Duration::from_secs(120 * 24 * 3600); // 120 days

        let children = vec![
            make_file("/recent", 100, Category::Unknown, Some(recent)),
            make_file("/mid", 200, Category::Unknown, Some(mid)),
            make_file("/old", 300, Category::Unknown, Some(old)),
        ];

        let result = aggregate_by_age(&children);

        assert_eq!(result.len(), 3);
        assert_eq!(result[0], (">90 days", 300));
        assert_eq!(result[1], ("30-90 days", 200));
        assert_eq!(result[2], ("<30 days", 100));
    }

    #[test]
    fn aggregate_by_age_should_place_unknown_modified_in_oldest_bucket() {
        let children = vec![
            make_file("/unknown", 500, Category::Unknown, None),
        ];

        let result = aggregate_by_age(&children);

        assert_eq!(result[0], (">90 days", 500));
        assert_eq!(result[1], ("30-90 days", 0));
        assert_eq!(result[2], ("<30 days", 0));
    }

    #[test]
    fn aggregate_by_age_should_return_zeros_for_empty_children() {
        let result = aggregate_by_age(&[]);

        assert_eq!(result[0].1, 0);
        assert_eq!(result[1].1, 0);
        assert_eq!(result[2].1, 0);
    }

    #[test]
    fn aggregate_by_age_should_accumulate_multiple_entries_per_bucket() {
        let now = SystemTime::now();
        let recent1 = now - Duration::from_secs(5 * 24 * 3600);
        let recent2 = now - Duration::from_secs(15 * 24 * 3600);

        let children = vec![
            make_file("/r1", 100, Category::Unknown, Some(recent1)),
            make_file("/r2", 250, Category::Unknown, Some(recent2)),
        ];

        let result = aggregate_by_age(&children);

        assert_eq!(result[2], ("<30 days", 350));
    }

    #[test]
    fn aggregate_by_age_should_handle_boundary_at_exactly_30_days() {
        let now = SystemTime::now();
        // Exactly 30 days and 1 second -> mid bucket
        let boundary = now - Duration::from_secs(30 * 24 * 3600 + 1);

        let children = vec![
            make_file("/boundary", 100, Category::Unknown, Some(boundary)),
        ];

        let result = aggregate_by_age(&children);

        assert_eq!(result[1], ("30-90 days", 100));
    }

    #[test]
    fn aggregate_by_age_should_handle_boundary_at_exactly_90_days() {
        let now = SystemTime::now();
        // Exactly 90 days and 1 second -> old bucket
        let boundary = now - Duration::from_secs(90 * 24 * 3600 + 1);

        let children = vec![
            make_file("/boundary", 100, Category::Unknown, Some(boundary)),
        ];

        let result = aggregate_by_age(&children);

        assert_eq!(result[0], (">90 days", 100));
    }

    // -- relative_time tests --

    #[test]
    fn relative_time_should_show_just_now_for_recent() {
        let now = SystemTime::now();
        assert_eq!(relative_time(now), "just now");
    }

    #[test]
    fn relative_time_should_show_months() {
        let three_months = SystemTime::now() - Duration::from_secs(90 * 24 * 3600);
        assert_eq!(relative_time(three_months), "3 months ago");
    }

    #[test]
    fn relative_time_should_show_years() {
        let two_years = SystemTime::now() - Duration::from_secs(730 * 24 * 3600);
        assert_eq!(relative_time(two_years), "2 years ago");
    }

    // -- api_key_display tests --

    #[test]
    fn api_key_display_should_mask_with_last_four_visible() {
        let draft = SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: "sk-abcdefgh1234".to_string(),
            api_key_edited: true,
            api_key_editing: false,
            model: String::new(),
            base_url: String::new(),
            llm_enabled: true,
            size_mode: SizeMode::Physical,
            selected_scan_profile: None,
        };

        let display = api_key_display(&draft);
        assert!(display.ends_with("1234"));
        assert!(display.starts_with("*"));
    }

    #[test]
    fn api_key_display_should_fully_mask_while_editing() {
        let draft = SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: "secret".to_string(),
            api_key_edited: false,
            api_key_editing: true,
            model: String::new(),
            base_url: String::new(),
            llm_enabled: true,
            size_mode: SizeMode::Physical,
            selected_scan_profile: None,
        };

        let display = api_key_display(&draft);
        assert_eq!(display, "******");
    }

    #[test]
    fn api_key_display_should_show_not_set_when_empty_and_unedited() {
        let draft = SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: String::new(),
            api_key_edited: false,
            api_key_editing: false,
            model: String::new(),
            base_url: String::new(),
            llm_enabled: true,
            size_mode: SizeMode::Physical,
            selected_scan_profile: None,
        };

        assert_eq!(api_key_display(&draft), "<not set>");
    }

    #[test]
    fn settings_preview_should_explain_plaintext_storage_and_llm_path_sharing() {
        let mut app = App::new(Some(PathBuf::from("/")), true, AppConfig::default());
        app.preview_mode = PreviewMode::Settings(SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: String::new(),
            api_key_edited: false,
            api_key_editing: false,
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            llm_enabled: true,
            size_mode: SizeMode::Physical,
            selected_scan_profile: None,
        });

        let area = Rect::new(0, 0, 80, 24);
        let buffer = render_to_buffer(|frame| render_preview(frame, area, &app), 80, 24);
        let text = buffer_text(&buffer);

        assert!(
            text.contains("secrets.toml"),
            "settings should mention plaintext key storage: {text}"
        );
        assert!(
            text.contains("exact path"),
            "settings should mention exact path disclosure: {text}"
        );
    }

    // -- safety_badge tests --

    #[test]
    fn safety_badge_should_return_correct_colors() {
        assert_eq!(safety_badge(SafetyLevel::Safe), ("Safe".to_string(), Color::Green));
        assert_eq!(safety_badge(SafetyLevel::Caution), ("Caution".to_string(), Color::Yellow));
        assert_eq!(safety_badge(SafetyLevel::Unsafe), ("Unsafe".to_string(), Color::Red));
        assert_eq!(safety_badge(SafetyLevel::Unknown), ("Unknown".to_string(), Color::DarkGray));
    }
}
