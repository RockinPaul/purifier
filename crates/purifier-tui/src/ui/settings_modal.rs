use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::{App, AppModal};

pub fn draw(frame: &mut Frame, app: &App, title: &str) {
    let area = centered_rect(frame.area(), 72, 16);
    frame.render_widget(Clear, area);

    let draft = match app.modal.as_ref() {
        Some(AppModal::Settings(draft)) | Some(AppModal::Onboarding(draft)) => draft,
        _ => return,
    };

    let lines = modal_lines(draft);

    let widget = Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(widget, area);
}

fn modal_lines(draft: &crate::app::SettingsDraft) -> Vec<Line<'static>> {
    vec![
        Line::from(format!("Provider: {:?}  [1-4] switch", draft.provider)),
        Line::from(format!(
            "Model (provider-derived, read-only): {}",
            draft.model
        )),
        Line::from(format!(
            "Base URL (provider-derived, read-only): {}",
            draft.base_url
        )),
        Line::from(format!("API Key: {}", api_key_status(draft))),
        Line::from("Press [a] to set or replace the provider key."),
        Line::from("OpenRouter and OpenAI are live right now."),
        Line::from("Anthropic and Google are saved for later support."),
        Line::from(""),
        Line::from(vec![
            Span::styled(" [a] ", Style::default().fg(Color::Cyan)),
            Span::raw("Edit API key  "),
            Span::styled(" [Enter] ", Style::default().fg(Color::Green)),
            Span::raw(if draft.api_key_editing {
                "Finish editing"
            } else {
                "Save"
            }),
        ]),
        Line::from(vec![
            Span::styled(" [Backspace/Delete] ", Style::default().fg(Color::Yellow)),
            Span::raw("Clear key while editing"),
        ]),
        Line::from(vec![
            Span::styled(" [Esc] ", Style::default().fg(Color::Yellow)),
            Span::raw("Cancel/Skip"),
        ]),
    ]
}

fn api_key_status(draft: &crate::app::SettingsDraft) -> String {
    if draft.api_key_editing {
        if draft.api_key.is_empty() {
            return "<editing empty>".to_string();
        }

        return format!("{} (editing)", "*".repeat(draft.api_key.len()));
    }

    if draft.api_key_edited {
        if draft.api_key.is_empty() {
            return "<will clear on save>".to_string();
        }

        return format!("{} (edited)", "*".repeat(draft.api_key.len()));
    }

    "<not shown unless edited>".to_string()
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(4));
    let height = height.min(area.height.saturating_sub(4));

    let [vertical] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    let [horizontal] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(vertical);
    horizontal
}

#[cfg(test)]
mod tests {
    use purifier_core::provider::ProviderKind;
    use ratatui::layout::Rect;

    use super::{api_key_status, modal_lines};
    use crate::app::SettingsDraft;

    #[test]
    fn modal_lines_should_describe_provider_and_api_key_controls() {
        let draft = SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: String::new(),
            api_key_edited: false,
            api_key_editing: false,
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            llm_enabled: true,
        };

        let lines = modal_lines(&draft);
        let rendered = lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("[1-4] switch"));
        assert!(rendered.contains("[a] Edit API key"));
        assert!(rendered.contains("[Backspace/Delete]"));
        assert!(rendered.contains("Model (provider-derived, read-only):"));
        assert!(rendered.contains("Base URL (provider-derived, read-only):"));
        assert!(rendered.contains("OpenRouter and OpenAI are live right now"));
        assert!(rendered.contains("Anthropic and Google are saved for later support"));
        assert!(rendered.contains("API Key: <not shown unless edited>"));
    }

    #[test]
    fn api_key_status_should_distinguish_unchanged_and_explicit_clear() {
        let unchanged = SettingsDraft {
            provider: ProviderKind::OpenRouter,
            api_key: String::new(),
            api_key_edited: false,
            api_key_editing: false,
            model: "model".to_string(),
            base_url: "url".to_string(),
            llm_enabled: true,
        };
        let cleared = SettingsDraft {
            api_key_edited: true,
            ..unchanged.clone()
        };

        assert_eq!(api_key_status(&unchanged), "<not shown unless edited>");
        assert_eq!(api_key_status(&cleared), "<will clear on save>");
    }

    #[test]
    fn centered_rect_should_clamp_to_terminal_area() {
        let rect = super::centered_rect(Rect::new(0, 0, 20, 10), 72, 16);

        assert_eq!(rect.width, 16);
        assert_eq!(rect.height, 6);
    }
}
