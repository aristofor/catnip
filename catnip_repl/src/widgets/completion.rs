// FILE: catnip_repl/src/widgets/completion.rs
//! Completion popup as a ratatui StatefulWidget.

use crate::completer::CompletionState;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, StatefulWidget, Widget};

/// Max visible items in the popup
pub const MAX_VISIBLE: usize = 8;

/// Completion popup widget
pub struct CompletionPopup;

impl StatefulWidget for CompletionPopup {
    type State = CompletionState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        if !state.active || state.suggestions.is_empty() {
            return;
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));
        let inner = block.inner(area);
        block.render(area, buf);

        let total = state.suggestions.len();
        let visible = total.min(MAX_VISIBLE);

        // Scroll si necessaire
        let scroll_offset = if state.selected >= visible {
            state.selected - visible + 1
        } else {
            0
        };

        for i in 0..visible {
            let idx = scroll_offset + i;
            if idx >= total {
                break;
            }

            let suggestion = &state.suggestions[idx];
            let y = inner.y + i as u16;
            if y >= inner.y + inner.height {
                break;
            }

            let is_selected = idx == state.selected;

            let base_style = if is_selected {
                Style::default().bg(Color::Rgb(60, 60, 80)).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let category_style = Style::default().fg(Color::DarkGray);

            // Texte + categorie a droite
            let text = &suggestion.text;
            let cat = suggestion.category;
            let available_width = inner.width as usize;

            // Calculer espace pour le texte et la categorie
            let cat_width = cat.len() + 1; // +1 pour espace
            let text_width = available_width.saturating_sub(cat_width);

            let text_display: String = if text.len() > text_width {
                text[..text_width].to_string()
            } else {
                format!("{:<width$}", text, width = text_width)
            };

            let line = Line::from(vec![
                Span::styled(text_display, base_style),
                Span::styled(
                    format!(" {}", cat),
                    if is_selected { base_style } else { category_style },
                ),
            ]);

            let line_area = Rect::new(inner.x, y, inner.width, 1);
            Widget::render(line, line_area, buf);
        }
    }
}
