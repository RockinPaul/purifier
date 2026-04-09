use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use purifier_core::provider::ProviderKind;

use crate::app::{App, PreviewMode};
use crate::ui::disclosures::current_storage_and_privacy_lines;

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Centered card
    let card_width = 60u16.min(area.width.saturating_sub(4));
    let card_height = 22u16.min(area.height.saturating_sub(4));
    let card_area = centered_rect(area, card_width, card_height);

    frame.render_widget(Clear, card_area);

    let draft = match &app.preview_mode {
        PreviewMode::Onboarding(d) => d,
        _ => return,
    };

    let mut lines = vec![
        Line::from(Span::styled(
            "Welcome to Purifier",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "Disk cleanup with safety classification",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "LLM Classification (optional)",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "An LLM can classify unknown paths by safety",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "level. Without it, only built-in rules are used.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
    ];

    // Provider selection
    let providers = [
        (ProviderKind::OpenRouter, "1:OpenRouter"),
        (ProviderKind::OpenAI, "2:OpenAI"),
        (ProviderKind::Anthropic, "3:Anthropic"),
        (ProviderKind::Google, "4:Google"),
    ];

    let provider_spans: Vec<Span> = providers
        .iter()
        .map(|(kind, label)| {
            if *kind == draft.provider {
                Span::styled(
                    format!(" {label} "),
                    Style::default().fg(Color::Black).bg(Color::Cyan),
                )
            } else {
                Span::styled(format!(" {label} "), Style::default().fg(Color::DarkGray))
            }
        })
        .collect();

    lines.push(Line::from(vec![
        Span::styled("  Provider: ", Style::default().fg(Color::DarkGray)),
    ]));
    lines.push(Line::from(
        std::iter::once(Span::raw("  "))
            .chain(provider_spans)
            .collect::<Vec<_>>(),
    ));
    lines.push(Line::from(""));

    // API key
    let key_display = if draft.api_key_editing {
        let masked = if draft.api_key.is_empty() {
            String::new()
        } else {
            format!("{}█", "*".repeat(draft.api_key.len().saturating_sub(1)))
        };
        Span::styled(
            format!("  {masked}"),
            Style::default().fg(Color::White).bg(Color::DarkGray),
        )
    } else if draft.api_key.is_empty() {
        Span::styled(
            "  (press a to enter key)",
            Style::default().fg(Color::DarkGray),
        )
    } else {
        let len = draft.api_key.len();
        let masked = format!(
            "{}{}",
            "*".repeat(len.saturating_sub(4)),
            &draft.api_key[len.saturating_sub(4)..]
        );
        Span::styled(format!("  {masked}"), Style::default().fg(Color::White))
    };

    lines.push(Line::from(vec![
        Span::styled("  API Key: ", Style::default().fg(Color::DarkGray)),
        Span::styled("[a] edit", Style::default().fg(Color::Cyan)),
    ]));
    lines.push(Line::from(key_display));
    lines.push(Line::from(""));

    lines.extend(current_storage_and_privacy_lines());
    lines.push(Line::from(""));

    // Error message
    if let Some(error) = &app.settings_modal_error {
        lines.push(Line::from(Span::styled(
            format!("  {error}"),
            Style::default().fg(Color::Red),
        )));
        lines.push(Line::from(""));
    }

    // Footer
    lines.push(Line::from(vec![
        Span::styled(" [Enter] ", Style::default().fg(Color::Cyan)),
        Span::raw("Save & start  "),
        Span::styled(" [Esc] ", Style::default().fg(Color::DarkGray)),
        Span::raw("Skip — rules only"),
    ]));

    let popup = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" First Launch Setup ")
            .style(Style::default().fg(Color::White).bg(Color::Black)),
    );
    frame.render_widget(popup, card_area);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppScreen, SettingsDraft};
    use crate::config::AppConfig;
    use purifier_core::size::SizeMode;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    fn render_to_buffer(app: &App, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal should be created");
        terminal.draw(|frame| draw(frame, app)).expect("should render");
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
    fn onboarding_should_explain_plaintext_storage_and_llm_path_sharing() {
        let mut app = App::new(None, true, AppConfig::default());
        app.screen = AppScreen::Onboarding;
        app.preview_mode = PreviewMode::Onboarding(SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: String::new(),
            api_key_edited: false,
            api_key_editing: false,
            model: String::new(),
            base_url: String::new(),
            llm_enabled: true,
            size_mode: SizeMode::Physical,
            selected_scan_profile: None,
        });

        let buffer = render_to_buffer(&app, 80, 24);
        let text = buffer_text(&buffer);

        assert!(
            text.contains("secrets.toml"),
            "onboarding should mention plaintext key storage: {text}"
        );
        assert!(
            text.contains("exact path"),
            "onboarding should mention exact path disclosure: {text}"
        );
    }
}
