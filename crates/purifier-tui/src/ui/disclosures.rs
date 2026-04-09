use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

pub fn current_storage_and_privacy_lines() -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            "  Saved keys: plaintext secrets.toml",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  Unix perms: 0600; not encrypted",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  LLM sends exact path, kind,",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  size and age to provider.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  --no-llm keeps paths local",
            Style::default().fg(Color::DarkGray),
        )),
    ]
}
