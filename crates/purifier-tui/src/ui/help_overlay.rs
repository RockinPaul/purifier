use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, area: Rect) {
    let popup_width = 44u16.min(area.width.saturating_sub(4));
    let popup_height = 26u16.min(area.height.saturating_sub(4));
    let popup_area = centered_rect(area, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let heading = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let key_style = Style::default().fg(Color::Yellow);
    let dim = Style::default().fg(Color::DarkGray);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled("  Navigation", heading)),
        help_line("    j / \u{2193}", "Move down", key_style),
        help_line("    k / \u{2191}", "Move up", key_style),
        help_line("    h / \u{2190}", "Go to parent", key_style),
        help_line("    l / \u{2192} / \u{23ce}", "Enter directory", key_style),
        help_line("    g", "Jump to top", key_style),
        help_line("    G", "Jump to bottom", key_style),
        help_line("    ~", "Go to home", key_style),
        Line::from(""),
        Line::from(Span::styled("  Actions", heading)),
        help_line("    d", "Delete selected", key_style),
        help_line("    Space", "Mark / unmark", key_style),
        help_line("    x", "Review batch", key_style),
        help_line("    u", "Clear all marks", key_style),
        Line::from(""),
        Line::from(Span::styled("  View", heading)),
        help_line("    s", "Cycle sort order", key_style),
        help_line("    i", "Toggle size mode", key_style),
        Line::from(""),
        Line::from(Span::styled("  Other", heading)),
        help_line("    ,", "Open settings", key_style),
        help_line("    ?", "Toggle this help", key_style),
        help_line("    q / Esc", "Quit", key_style),
        Line::from(""),
        Line::from(Span::styled(
            "  Press ? or Esc to close",
            dim,
        )),
    ];

    let popup = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Help ")
            .style(Style::default().fg(Color::White).bg(Color::Black)),
    );
    frame.render_widget(popup, popup_area);
}

fn help_line<'a>(key: &'a str, desc: &'a str, key_style: Style) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{:<16}", key), key_style),
        Span::raw(desc),
    ])
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(4));
    let height = height.min(area.height.saturating_sub(4));
    let vertical = Layout::vertical([Constraint::Length(height)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Length(width)]).flex(Flex::Center);
    let [v_area] = vertical.areas(area);
    let [h_area] = horizontal.areas(v_area);
    h_area
}
