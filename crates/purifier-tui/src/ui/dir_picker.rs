use ratatui::layout::{Constraint, Direction, Flex, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::App;

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Clear the entire screen first
    frame.render_widget(Clear, area);

    // Full-screen outer block
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(" purifier — disk cleanup with safety intelligence ")
        .style(Style::default().fg(Color::Cyan));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    // Center the picker content
    let picker_height = app.dir_picker_options.len() as u16 + 10;
    let vertical = Layout::vertical([Constraint::Length(picker_height)]).flex(Flex::Center);
    let horizontal =
        Layout::horizontal([Constraint::Length(56.min(inner.width.saturating_sub(2)))])
            .flex(Flex::Center);
    let [center_v] = vertical.areas(inner);
    let [center] = horizontal.areas(center_v);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(3),    // list
            Constraint::Length(3), // custom input
            Constraint::Length(2), // help
        ])
        .split(center);

    // Header
    let header = Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Choose a directory to scan:",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
    ]);
    frame.render_widget(header, chunks[0]);

    // Directory list
    let items: Vec<ListItem> = app
        .dir_picker_options
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let label = shorten_path(path);
            let marker = if i == app.dir_picker_selected && !app.dir_picker_typing {
                " > "
            } else {
                "   "
            };
            let style = if i == app.dir_picker_selected && !app.dir_picker_typing {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(Span::styled(format!("{marker}{label}"), style)))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Directories ")
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(list, chunks[1]);

    // Custom path input
    let input_style = if app.dir_picker_typing {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let input_label = if app.dir_picker_typing {
        format!("  > {}_", app.dir_picker_custom)
    } else if app.dir_picker_custom.is_empty() {
        "  Press / to type a custom path...".to_string()
    } else {
        format!("  > {}", app.dir_picker_custom)
    };

    let input = Paragraph::new(Line::from(Span::styled(input_label, input_style))).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Custom path ")
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(input, chunks[2]);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled(" j/k ", Style::default().fg(Color::Cyan)),
        Span::styled("navigate  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Enter ", Style::default().fg(Color::Cyan)),
        Span::styled("select  ", Style::default().fg(Color::DarkGray)),
        Span::styled("/ ", Style::default().fg(Color::Cyan)),
        Span::styled("type path  ", Style::default().fg(Color::DarkGray)),
        Span::styled("q ", Style::default().fg(Color::Cyan)),
        Span::styled("quit", Style::default().fg(Color::DarkGray)),
    ]));
    frame.render_widget(help, chunks[3]);
}

fn shorten_path(path: &std::path::Path) -> String {
    let display = path.display().to_string();
    if let Some(home) = dirs::home_dir() {
        let home_str = home.display().to_string();
        if display == home_str {
            return "~/ (Home directory)".to_string();
        }
        if let Some(rest) = display.strip_prefix(&home_str) {
            return format!("~{rest}");
        }
    }
    if display == "/" {
        return "/ (Full disk)".to_string();
    }
    display
}
