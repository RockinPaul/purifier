pub mod dir_picker;
pub mod settings_modal;
pub mod status_bar;
pub mod tree_view;

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::Frame;

use crate::app::{App, AppModal, AppScreen, View};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MainLayout {
    pub tabs: ratatui::layout::Rect,
    pub main: ratatui::layout::Rect,
    pub info: ratatui::layout::Rect,
    pub status: ratatui::layout::Rect,
}

pub fn draw(frame: &mut Frame, app: &App) {
    match app.screen {
        AppScreen::DirPicker => {
            dir_picker::draw(frame, app);
        }
        AppScreen::Main => {
            draw_main(frame, app);
        }
    }

    if let Some(modal) = global_overlay_modal(app) {
        draw_modal(frame, app, modal);
    }
}

fn draw_main(frame: &mut Frame, app: &App) {
    let layout = main_layout(frame.area());

    draw_tab_bar(frame, app, layout.tabs);

    match app.current_view {
        View::BySize | View::ByType | View::BySafety | View::ByAge => {
            tree_view::draw(frame, app, layout.main, layout.info);
        }
    }

    status_bar::draw(frame, app, layout.status);

    if matches!(app.modal, Some(AppModal::DeleteConfirm)) {
        let Some(modal) = app.modal.as_ref() else {
            return;
        };
        draw_modal(frame, app, modal);
    }
}

pub(crate) fn main_layout(area: ratatui::layout::Rect) -> MainLayout {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(area);

    MainLayout {
        tabs: chunks[0],
        main: chunks[1],
        info: chunks[2],
        status: chunks[3],
    }
}

fn global_overlay_modal(app: &App) -> Option<&AppModal> {
    match app.modal.as_ref() {
        Some(AppModal::Settings(_)) | Some(AppModal::Onboarding(_)) => app.modal.as_ref(),
        _ => None,
    }
}

fn draw_modal(frame: &mut Frame, app: &App, modal: &AppModal) {
    match modal {
        AppModal::DeleteConfirm => draw_delete_confirm(frame, app),
        AppModal::Settings(_) => settings_modal::draw(frame, app, " Settings "),
        AppModal::Onboarding(_) => settings_modal::draw(frame, app, " First Launch Setup "),
    }
}

fn draw_tab_bar(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, Borders, Paragraph};

    let tabs: Vec<Span> = [View::BySize, View::ByType, View::BySafety, View::ByAge]
        .iter()
        .enumerate()
        .map(|(i, view)| {
            let label = format!(" {}:{} ", i + 1, view.label());
            if *view == app.current_view {
                Span::styled(
                    label,
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(label, Style::default().fg(Color::DarkGray))
            }
        })
        .collect();

    let title = format!(" purifier — {} ", app.scan_path.display());
    let paragraph =
        Paragraph::new(Line::from(tabs)).block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(paragraph, area);
}

fn draw_delete_confirm(frame: &mut Frame, app: &App) {
    use ratatui::style::{Color, Style};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, Borders, Clear, Paragraph};

    let popup_area = centered_popup_area(frame.area(), 60, 9);

    frame.render_widget(Clear, popup_area);

    if let Some(entry) = app.selected_entry() {
        let safety_color = match entry.safety {
            purifier_core::SafetyLevel::Safe => Color::Green,
            purifier_core::SafetyLevel::Caution => Color::Yellow,
            purifier_core::SafetyLevel::Unsafe => Color::Red,
            purifier_core::SafetyLevel::Unknown => Color::DarkGray,
        };

        let lines = vec![
            Line::from(vec![
                Span::raw("Path: "),
                Span::styled(
                    entry.path.display().to_string(),
                    Style::default().fg(Color::White),
                ),
            ]),
            Line::from(vec![
                Span::raw("Logical remove: "),
                Span::raw(format_size(entry.logical_size)),
            ]),
            Line::from(vec![
                Span::raw("Est. physical free: "),
                Span::raw(format_size(entry.physical_size)),
            ]),
            Line::from(vec![
                Span::raw("Safety: "),
                Span::styled(
                    format!("{}", entry.safety),
                    Style::default().fg(safety_color),
                ),
            ]),
            Line::from(entry.safety_reason.clone()),
            Line::from(""),
            Line::from(vec![
                Span::styled(" [y] ", Style::default().fg(Color::Red)),
                Span::raw("Delete  "),
                Span::styled(" [n] ", Style::default().fg(Color::Green)),
                Span::raw("Cancel"),
            ]),
        ];

        let popup = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Delete? ")
                .style(Style::default().fg(Color::White).bg(Color::DarkGray)),
        );
        frame.render_widget(popup, popup_area);
    }
}

fn centered_popup_area(
    area: ratatui::layout::Rect,
    popup_width: u16,
    popup_height: u16,
) -> ratatui::layout::Rect {
    use ratatui::layout::{Constraint, Flex, Layout};

    let popup_width = popup_width.min(area.width.saturating_sub(4));
    let popup_height = popup_height.min(area.height.saturating_sub(4));

    let vertical = Layout::vertical([Constraint::Length(popup_height)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Length(popup_width)]).flex(Flex::Center);
    let [popup_area] = vertical.areas(area);
    let [popup_area] = horizontal.areas(popup_area);
    popup_area
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

pub fn truncate_start(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect()
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
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;
    use purifier_core::size::EntrySizes;
    use purifier_core::types::FileEntry;

    fn render_app(app: &App) -> String {
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).expect("terminal should be created");
        terminal
            .draw(|frame| draw(frame, app))
            .expect("ui should render");

        let buffer = terminal.backend().buffer().clone();
        buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    #[test]
    fn truncate_start_should_preserve_unicode_boundaries() {
        assert_eq!(truncate_start("a😀b😀c", 4), "a😀b😀");
    }

    #[test]
    fn truncate_tail_should_preserve_unicode_boundaries() {
        assert_eq!(truncate_tail("ab😀c😀d", 5), "...😀d");
    }

    #[test]
    fn global_overlay_modal_should_include_onboarding_outside_main_screen() {
        let mut app = App::new(None, true, crate::config::AppConfig::default());
        app.open_onboarding();

        assert!(matches!(
            global_overlay_modal(&app),
            Some(AppModal::Onboarding(_))
        ));
    }

    #[test]
    fn global_overlay_modal_should_exclude_delete_confirm() {
        let mut app = App::new(None, false, crate::config::AppConfig::default());
        app.open_delete_confirm();

        assert!(global_overlay_modal(&app).is_none());
    }

    #[test]
    fn delete_confirm_should_show_logical_and_estimated_physical_sizes() {
        let mut app = App::new(
            Some(std::path::PathBuf::from("/")),
            false,
            crate::config::AppConfig::default(),
        );
        app.entries = vec![FileEntry::new_with_sizes(
            std::path::PathBuf::from("/tmp/delete-me.bin"),
            EntrySizes {
                logical_bytes: 1024,
                physical_bytes: 4096,
                accounted_physical_bytes: 4096,
            },
            None,
            false,
            None,
        )];
        app.rebuild_flat_entries();
        app.open_delete_confirm();
        let text = render_app(&app);

        assert!(
            text.contains("Logical remove: 1.0 KB"),
            "delete confirm should describe logical removal explicitly: {text}"
        );
        assert!(
            text.contains("Est. physical free: 4.0 KB"),
            "delete confirm should describe estimated physical free space: {text}"
        );
    }
}
