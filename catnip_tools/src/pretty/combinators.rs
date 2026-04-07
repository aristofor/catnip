// FILE: catnip_tools/src/pretty/combinators.rs
//! Derived combinators for the document algebra.

use super::doc::{Arena, Doc};

impl Arena {
    /// Single space.
    pub fn space(&mut self) -> Doc {
        self.text(" ")
    }

    /// Concatenate a slice of docs left-to-right.
    pub fn concat_many(&mut self, docs: &[Doc]) -> Doc {
        match docs.len() {
            0 => self.nil(),
            1 => docs[0],
            _ => {
                let mut acc = docs[0];
                for &d in &docs[1..] {
                    acc = self.concat(acc, d);
                }
                acc
            }
        }
    }

    /// Intercalate `sep` between each doc.
    pub fn intersperse(&mut self, docs: &[Doc], sep: Doc) -> Doc {
        if docs.is_empty() {
            return self.nil();
        }
        let mut acc = docs[0];
        for &d in &docs[1..] {
            let sep_d = self.concat(sep, d);
            acc = self.concat(acc, sep_d);
        }
        acc
    }

    /// `group(text(open) <> nest(indent, line <> body) <> line <> text(close))`
    ///
    /// Flat: `open body close`. Broken: multiline with indent.
    pub fn bracket(&mut self, open: &str, body: Doc, close: &str, indent: i32) -> Doc {
        let open = self.text(open);
        let close = self.text(close);
        let ln = self.line();
        let inner = self.concat(ln, body);
        let nested = self.nest(indent, inner);
        let ln2 = self.line();
        let doc = self.concat_many(&[open, nested, ln2, close]);
        self.group(doc)
    }

    /// `text(open) <> body <> text(close)` - no grouping, no breaks.
    pub fn surround(&mut self, open: &str, body: Doc, close: &str) -> Doc {
        let o = self.text(open);
        let c = self.text(close);
        self.concat_many(&[o, body, c])
    }

    /// Build a fill document: items packed with separators, each separator
    /// independently choosing flat or break based on remaining width.
    /// `sep_flat`: separator in flat mode (e.g. ", ")
    /// `sep_break_extra`: extra content after comma in break mode (typically hardline)
    pub fn fill_sep(&mut self, items: &[Doc], sep_flat: Doc, sep_break_extra: Doc) -> Doc {
        if items.is_empty() {
            return self.nil();
        }
        // Build alternating [item, sep, item, sep, ..., item]
        // where sep = IfFlat(sep_flat, comma + sep_break_extra)
        let mut parts = Vec::with_capacity(items.len() * 2 - 1);
        parts.push(items[0]);
        for &item in &items[1..] {
            let comma = self.text(",");
            let broken = self.concat(comma, sep_break_extra);
            let sep = self.if_flat(sep_flat, broken);
            parts.push(sep);
            parts.push(item);
        }
        // Pre-build nested Fill chain so the layout engine can process
        // one (item, sep) pair at a time and re-evaluate column for the rest.
        // Fill([a, sep, b, sep, c]) -> Fill([a, sep, Fill([b, sep, c])])
        self.fill_nested(parts)
    }

    fn fill_nested(&mut self, parts: Vec<Doc>) -> Doc {
        let n = parts.len();
        if n <= 3 {
            // Base case: [item], [item, sep, item]
            return self.fill(parts);
        }
        // Recursive: [item, sep, Fill(rest)]
        let rest = parts[2..].to_vec();
        let rest_fill = self.fill_nested(rest);
        self.fill(vec![parts[0], parts[1], rest_fill])
    }

    /// `text(",") <> line` - comma then soft break.
    #[allow(dead_code)]
    pub fn comma_line(&mut self) -> Doc {
        let comma = self.text(",");
        let ln = self.line();
        self.concat(comma, ln)
    }
}
