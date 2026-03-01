// FILE: catnip_repl/src/highlighter.rs
//! Syntax highlighting for Catnip REPL using tree-sitter
//!
//! Produces Vec<ratatui::text::Span> for inline rendering.

use crate::theme::hex;
use catnip_rs::constants::{highlighting as hl, highlighting_light as hl_light};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use std::cell::RefCell;
use tree_sitter::{Node, Parser};

/// Resolved highlight colors for a given theme
struct HighlightColors {
    keyword: Color,
    keyword_bold: bool,
    constant: Color,
    constant_bold: bool,
    type_color: Color,
    number: Color,
    string: Color,
    builtin: Color,
    comment: Color,
    operator: Color,
    punctuation: Color,
}

impl HighlightColors {
    fn for_dark() -> Self {
        Self {
            keyword: hex(hl::KEYWORD_COLOR),
            keyword_bold: hl::KEYWORD_BOLD,
            constant: hex(hl::CONSTANT_COLOR),
            constant_bold: hl::CONSTANT_BOLD,
            type_color: hex(hl::TYPE_COLOR),
            number: hex(hl::NUMBER_COLOR),
            string: hex(hl::STRING_COLOR),
            builtin: hex(hl::BUILTIN_COLOR),
            comment: hex(hl::COMMENT_COLOR),
            operator: hex(hl::OPERATOR_COLOR),
            punctuation: hex(hl::PUNCTUATION_COLOR),
        }
    }

    fn for_light() -> Self {
        Self {
            keyword: hex(hl_light::KEYWORD_COLOR),
            keyword_bold: hl_light::KEYWORD_BOLD,
            constant: hex(hl_light::CONSTANT_COLOR),
            constant_bold: hl_light::CONSTANT_BOLD,
            type_color: hex(hl_light::TYPE_COLOR),
            number: hex(hl_light::NUMBER_COLOR),
            string: hex(hl_light::STRING_COLOR),
            builtin: hex(hl_light::BUILTIN_COLOR),
            comment: hex(hl_light::COMMENT_COLOR),
            operator: hex(hl_light::OPERATOR_COLOR),
            punctuation: hex(hl_light::PUNCTUATION_COLOR),
        }
    }
}

/// Catnip syntax highlighter
pub struct CatnipHighlighter {
    parser: RefCell<Parser>,
    colors: HighlightColors,
}

impl CatnipHighlighter {
    pub fn new(is_dark: bool) -> Result<Self, String> {
        let mut parser = Parser::new();
        let language = catnip_rs::get_tree_sitter_language();
        parser
            .set_language(&language)
            .map_err(|e| format!("Failed to load grammar: {}", e))?;

        let colors = if is_dark {
            HighlightColors::for_dark()
        } else {
            HighlightColors::for_light()
        };

        Ok(Self {
            parser: RefCell::new(parser),
            colors,
        })
    }

    pub fn number_color(&self) -> Color {
        self.colors.number
    }

    pub fn string_color(&self) -> Color {
        self.colors.string
    }

    pub fn constant_color(&self) -> Color {
        self.colors.constant
    }

    /// Highlight a line, returns ratatui Spans
    pub fn highlight_line<'a>(&self, line: &'a str) -> Vec<Span<'a>> {
        let tree = match self.parser.borrow_mut().parse(line, None) {
            Some(tree) => tree,
            None => return vec![Span::raw(line)],
        };

        let root = tree.root_node();
        let mut segments: Vec<(usize, usize, Style)> = Vec::new();
        collect_styled_segments(&root, line, &mut segments, self);
        segments.sort_by_key(|(start, _, _)| *start);

        let mut spans = Vec::new();
        let mut last_end = 0;

        for (start, end, style) in segments {
            if start > last_end {
                spans.push(Span::raw(&line[last_end..start]));
            }
            spans.push(Span::styled(&line[start..end], style));
            last_end = end;
        }

        if last_end < line.len() {
            spans.push(Span::raw(&line[last_end..]));
        }

        spans
    }

    /// Get style for a node based on its kind
    fn get_style_for_node(&self, node: &Node, source: &str) -> Style {
        let kind = node.kind();
        let c = &self.colors;

        match kind {
            // Keywords
            "if" | "elif" | "else" | "while" | "for" | "in" | "match" | "case" | "return"
            | "break" | "continue" | "and" | "or" | "not" | "pragma" => {
                let mut style = Style::default().fg(c.keyword);
                if c.keyword_bold {
                    style = style.add_modifier(Modifier::BOLD);
                }
                style
            }

            // Constants
            "True" | "False" | "None" => {
                let mut style = Style::default().fg(c.constant);
                if c.constant_bold {
                    style = style.add_modifier(Modifier::BOLD);
                }
                style
            }

            // Built-in types
            "dict" | "list" | "set" | "tuple" => Style::default().fg(c.type_color),

            // Numbers
            "integer" | "float" | "binary_integer" | "octal_integer" | "hexadecimal_integer" => {
                Style::default().fg(c.number)
            }

            // Strings
            "string" | "f_string" => Style::default().fg(c.string),

            // Comments
            "comment" => Style::default().fg(c.comment),

            // Operators
            "=>" | "+" | "-" | "*" | "/" | "%" | "**" | "//" | "==" | "!=" | "<" | ">" | "<="
            | ">=" | "&" | "|" | "^" | "~" | "<<" | ">>" | "@" | "~~" | "~>" | "~[]" => {
                Style::default().fg(c.operator)
            }

            // Assignment operator
            "=" => Style::default().fg(c.operator),

            // Identifiers - check if builtin
            "identifier" => {
                let text = &source[node.byte_range()];
                if is_builtin(text) {
                    Style::default().fg(c.builtin)
                } else {
                    Style::default()
                }
            }

            // Punctuation
            "(" | ")" | "[" | "]" | "{" | "}" | "," | ";" | ":" | "." => {
                Style::default().fg(c.punctuation)
            }

            _ => Style::default(),
        }
    }
}

/// Collect styled segments from tree-sitter nodes
fn collect_styled_segments(
    node: &Node,
    source: &str,
    segments: &mut Vec<(usize, usize, Style)>,
    highlighter: &CatnipHighlighter,
) {
    let kind = node.kind();

    if matches!(kind, "\n" | " " | "\t") {
        return;
    }

    // Leaf nodes
    if node.child_count() == 0 {
        let style = highlighter.get_style_for_node(node, source);
        let range = node.byte_range();
        segments.push((range.start, range.end, style));
        return;
    }

    // Recurse
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_styled_segments(&child, source, segments, highlighter);
    }
}

fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "print"
            | "len"
            | "type"
            | "str"
            | "int"
            | "float"
            | "bool"
            | "abs"
            | "range"
            | "list"
            | "dict"
            | "tuple"
            | "set"
            | "vars"
            | "dir"
            | "help"
            | "max"
            | "min"
            | "sum"
            | "any"
            | "all"
            | "sorted"
            | "reversed"
            | "enumerate"
            | "zip"
            | "map"
            | "filter"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_highlighter_creation_dark() {
        let highlighter = CatnipHighlighter::new(true);
        assert!(highlighter.is_ok());
    }

    #[test]
    fn test_highlighter_creation_light() {
        let highlighter = CatnipHighlighter::new(false);
        assert!(highlighter.is_ok());
    }

    #[test]
    fn test_highlight_returns_spans() {
        let highlighter = CatnipHighlighter::new(true).unwrap();
        let spans = highlighter.highlight_line("2 + 3");
        assert!(!spans.is_empty());
    }

    #[test]
    fn test_highlight_keywords() {
        let highlighter = CatnipHighlighter::new(true).unwrap();
        let spans = highlighter.highlight_line("if x == 10");
        assert!(spans.len() >= 4);
    }

    #[test]
    fn test_highlight_builtins() {
        let highlighter = CatnipHighlighter::new(true).unwrap();
        let spans = highlighter.highlight_line("print(len(x))");
        assert!(!spans.is_empty());

        // Find "print" span with builtin color
        let has_print = spans
            .iter()
            .any(|s| s.content == "print" && s.style.fg == Some(hex(hl::BUILTIN_COLOR)));
        assert!(has_print, "print should be highlighted as builtin");
    }
}
