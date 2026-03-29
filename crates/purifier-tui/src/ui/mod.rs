pub mod dir_picker;
pub mod tree_view;
pub mod status_bar;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};

use crate::app::{App, AppScreen, View};

pub fn draw(frame: &mut Frame, app: &App) {
    match app.screen {
        AppScreen::DirPicker => {
            dir_picker::draw(frame, app);
        }
        AppScreen::Main => {
            draw_main(frame, app);
        }
    }
}

fn draw_main(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // tab bar
            Constraint::Min(5),    // main content
            Constraint::Length(3), // info panel
            Constraint::Length(1), // status bar
        ])
        .split(frame.area());

    draw_tab_bar(frame, app, chunks[0]);

    match app.current_view {
        View::BySize | View::ByType | View::BySafety | View::ByAge => {
            tree_view::draw(frame, app, chunks[1], chunks[2]);
        }
    }

    status_bar::draw(frame, app, chunks[3]);

    if app.show_delete_confirm {
        draw_delete_confirm(frame, app);
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
                Span::styled(label, Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD))
            } else {
                Span::styled(label, Style::default().fg(Color::DarkGray))
            }
        })
        .collect();

    let title = format!(" purifier — {} ", app.scan_path.display());
    let paragraph = Paragraph::new(Line::from(tabs))
        .block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(paragraph, area);
}

fn draw_delete_confirm(frame: &mut Frame, app: &App) {
    use ratatui::layout::{Constraint, Layout, Flex};
    use ratatui::style::{Color, Style};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, Borders, Clear, Paragraph};

    let area = frame.area();
    let popup_width = 60.min(area.width.saturating_sub(4));
    let popup_height = 8.min(area.height.saturating_sub(4));

    let vertical = Layout::vertical([Constraint::Length(popup_height)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Length(popup_width)]).flex(Flex::Center);
    let [popup_area] = vertical.areas(area);
    let [popup_area] = horizontal.areas(popup_area);

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
                Span::raw("Size: "),
                Span::raw(format_size(entry.size)),
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
