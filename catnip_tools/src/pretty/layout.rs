// FILE: catnip_tools/src/pretty/layout.rs
//! Leijen greedy best-fit layout engine.

use super::doc::{Arena, Doc, DocNode};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Flat,
    Break,
}

struct Entry {
    indent: usize,
    mode: Mode,
    doc: Doc,
}

/// Render a document to a string with a target line width.
/// Returns `(output, line_map)` where `line_map[i]` is the source line
/// number for output line `i`.
pub fn layout(arena: &Arena, doc: Doc, width: usize) -> (String, Vec<usize>) {
    let mut output = String::new();
    let mut line_map: Vec<usize> = vec![0]; // first output line maps to source line 0
    let mut column: usize = 0;
    let mut current_source_line: usize = 0;

    // Work stack - processed right-to-left so we push in reverse order
    let mut stack: Vec<Entry> = vec![Entry {
        indent: 0,
        mode: Mode::Break,
        doc,
    }];

    while let Some(entry) = stack.pop() {
        match arena.get(entry.doc) {
            DocNode::Nil => {}

            DocNode::Text(s) => {
                output.push_str(s);
                column += s.len();
            }

            DocNode::Line => match entry.mode {
                Mode::Flat => {
                    output.push(' ');
                    column += 1;
                }
                Mode::Break => {
                    output.push('\n');
                    let spaces = " ".repeat(entry.indent);
                    output.push_str(&spaces);
                    column = entry.indent;
                    line_map.push(current_source_line);
                }
            },

            DocNode::SoftLine => match entry.mode {
                Mode::Flat => { /* nothing */ }
                Mode::Break => {
                    output.push('\n');
                    let spaces = " ".repeat(entry.indent);
                    output.push_str(&spaces);
                    column = entry.indent;
                    line_map.push(current_source_line);
                }
            },

            DocNode::HardLine => {
                output.push('\n');
                let spaces = " ".repeat(entry.indent);
                output.push_str(&spaces);
                column = entry.indent;
                line_map.push(current_source_line);
            }

            DocNode::Nest(extra, inner) => {
                let new_indent = (entry.indent as i32 + extra).max(0) as usize;
                stack.push(Entry {
                    indent: new_indent,
                    mode: entry.mode,
                    doc: *inner,
                });
            }

            DocNode::Group(inner) => {
                let flat_w = flat_width(arena, *inner, width.saturating_sub(column));
                let mode = if flat_w.is_some() { Mode::Flat } else { Mode::Break };
                stack.push(Entry {
                    indent: entry.indent,
                    mode,
                    doc: *inner,
                });
            }

            DocNode::Concat(a, b) => {
                // Push b first so a is processed first (stack is LIFO)
                stack.push(Entry {
                    indent: entry.indent,
                    mode: entry.mode,
                    doc: *b,
                });
                stack.push(Entry {
                    indent: entry.indent,
                    mode: entry.mode,
                    doc: *a,
                });
            }

            DocNode::IfFlat { flat, broken } => {
                let chosen = if entry.mode == Mode::Flat { *flat } else { *broken };
                stack.push(Entry {
                    indent: entry.indent,
                    mode: entry.mode,
                    doc: chosen,
                });
            }

            DocNode::Verbatim(s) => {
                for (i, part) in s.split('\n').enumerate() {
                    if i > 0 {
                        output.push('\n');
                        line_map.push(current_source_line);
                        column = 0;
                    }
                    output.push_str(part);
                    column += part.len();
                }
            }

            DocNode::SourceLine(line, inner) => {
                current_source_line = *line;
                stack.push(Entry {
                    indent: entry.indent,
                    mode: entry.mode,
                    doc: *inner,
                });
            }

            DocNode::Fill(parts) => {
                // Pre-nested: [item] or [item, sep, rest_fill]
                // Each Fill has max 3 elements thanks to fill_nested().
                // Push in reverse so item is processed first, then sep
                // (with mode decided now), then rest_fill (re-evaluated later
                // with updated column).
                let n = parts.len();
                if n == 1 {
                    stack.push(Entry {
                        indent: entry.indent,
                        mode: Mode::Break,
                        doc: parts[0],
                    });
                } else if n >= 3 {
                    // Decide separator mode: does flat(item0) + flat(sep) + flat(next_item) fit?
                    let remaining = width.saturating_sub(column);
                    let item_w = flat_width(arena, parts[0], remaining);
                    let sep_and_next = item_w.and_then(|iw| {
                        let r = remaining.saturating_sub(iw);
                        flat_width(arena, parts[1], r).and_then(|sw| {
                            // Only check the first item of the next group (not the whole rest)
                            let next = first_fill_item(arena, parts[2]);
                            flat_width(arena, next, r.saturating_sub(sw))
                        })
                    });
                    let sep_mode = if sep_and_next.is_some() {
                        Mode::Flat
                    } else {
                        Mode::Break
                    };

                    // Push rest (will be re-evaluated with updated column)
                    stack.push(Entry {
                        indent: entry.indent,
                        mode: Mode::Break,
                        doc: parts[2],
                    });
                    // Push separator with decided mode
                    stack.push(Entry {
                        indent: entry.indent,
                        mode: sep_mode,
                        doc: parts[1],
                    });
                    // Push first item
                    stack.push(Entry {
                        indent: entry.indent,
                        mode: Mode::Break,
                        doc: parts[0],
                    });
                }
            }
        }
    }

    (output, line_map)
}

/// Extract the first item from a (possibly nested) Fill.
fn first_fill_item(arena: &Arena, doc: Doc) -> Doc {
    match arena.get(doc) {
        DocNode::Fill(parts) if !parts.is_empty() => first_fill_item(arena, parts[0]),
        _ => doc,
    }
}

/// Compute the width of a doc rendered in flat mode.
/// Returns `None` if it exceeds `remaining` (short-circuit).
fn flat_width(arena: &Arena, doc: Doc, remaining: usize) -> Option<usize> {
    let mut stack = vec![doc];
    let mut used: usize = 0;

    while let Some(d) = stack.pop() {
        if used > remaining {
            return None;
        }
        match arena.get(d) {
            DocNode::Nil => {}
            DocNode::Text(s) => used += s.len(),
            DocNode::Line => used += 1,       // flat: space
            DocNode::SoftLine => {}           // flat: nothing
            DocNode::HardLine => return None, // cannot flatten
            DocNode::Nest(_, inner) => stack.push(*inner),
            DocNode::Group(inner) => stack.push(*inner),
            DocNode::Concat(a, b) => {
                stack.push(*b);
                stack.push(*a);
            }
            DocNode::IfFlat { flat, .. } => stack.push(*flat),
            DocNode::Fill(parts) => {
                for p in parts.iter().rev() {
                    stack.push(*p);
                }
            }
            DocNode::Verbatim(s) => {
                // Multiline verbatim: column resets after last \n
                if let Some(last_nl) = s.rfind('\n') {
                    used = s[last_nl + 1..].len();
                } else {
                    used += s.len();
                }
            }
            DocNode::SourceLine(_, inner) => stack.push(*inner),
        }
    }

    if used <= remaining { Some(used) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(width: usize, f: impl FnOnce(&mut Arena) -> Doc) -> String {
        let mut arena = Arena::new();
        let doc = f(&mut arena);
        let (out, _) = layout(&arena, doc, width);
        out
    }

    #[test]
    fn test_text() {
        assert_eq!(render(80, |a| a.text("hello")), "hello");
    }

    #[test]
    fn test_group_fits_flat() {
        let out = render(10, |a| {
            let t1 = a.text("a");
            let ln = a.line();
            let t2 = a.text("b");
            let inner = a.concat_many(&[t1, ln, t2]);
            a.group(inner)
        });
        assert_eq!(out, "a b");
    }

    #[test]
    fn test_group_breaks() {
        let out = render(2, |a| {
            let t1 = a.text("a");
            let ln = a.line();
            let t2 = a.text("b");
            let inner = a.concat_many(&[t1, ln, t2]);
            a.group(inner)
        });
        assert_eq!(out, "a\nb");
    }

    #[test]
    fn test_nest_hardline() {
        let out = render(80, |a| {
            let hl = a.hardline();
            let t = a.text("x");
            let inner = a.concat(hl, t);
            a.nest(4, inner)
        });
        assert_eq!(out, "\n    x");
    }

    #[test]
    fn test_bracket_flat() {
        let out = render(80, |a| {
            let body = a.text("body");
            a.bracket("{", body, "}", 4)
        });
        assert_eq!(out, "{ body }");
    }

    #[test]
    fn test_bracket_break() {
        let out = render(5, |a| {
            let body = a.text("body");
            a.bracket("{", body, "}", 4)
        });
        assert_eq!(out, "{\n    body\n}");
    }

    #[test]
    fn test_verbatim() {
        let out = render(80, |a| a.verbatim("a\nb\nc"));
        assert_eq!(out, "a\nb\nc");
    }

    #[test]
    fn test_softline_flat() {
        let out = render(80, |a| {
            let t1 = a.text("a");
            let sl = a.softline();
            let t2 = a.text("b");
            let inner = a.concat_many(&[t1, sl, t2]);
            a.group(inner)
        });
        assert_eq!(out, "ab");
    }

    #[test]
    fn test_softline_break() {
        let out = render(1, |a| {
            let t1 = a.text("a");
            let sl = a.softline();
            let t2 = a.text("b");
            let inner = a.concat_many(&[t1, sl, t2]);
            a.group(inner)
        });
        assert_eq!(out, "a\nb");
    }

    #[test]
    fn test_if_flat() {
        // In flat mode: "a, b". In break mode: "a,\nb".
        let flat_out = render(80, |a| {
            let t1 = a.text("a");
            let comma = a.text(",");
            let sp = a.space();
            let flat_sep = a.concat(comma, sp);
            let nl = a.line();
            let comma2 = a.text(",");
            let break_sep = a.concat(comma2, nl);
            let sep = a.if_flat(flat_sep, break_sep);
            let t2 = a.text("b");
            let inner = a.concat_many(&[t1, sep, t2]);
            a.group(inner)
        });
        assert_eq!(flat_out, "a, b");
    }

    #[test]
    fn test_line_map_basic() {
        // SourceLine must wrap content *including* the preceding hardline
        // so the new line number is active when the \n is emitted.
        let mut arena = Arena::new();
        let s1 = arena.text("line1");
        let s1 = arena.source_line(0, s1);
        let hl = arena.hardline();
        let s2 = arena.text("line2");
        let s2_block = arena.concat(hl, s2);
        let s2_block = arena.source_line(5, s2_block);
        let doc = arena.concat(s1, s2_block);
        let (out, line_map) = layout(&arena, doc, 80);
        assert_eq!(out, "line1\nline2");
        assert_eq!(line_map, vec![0, 5]);
    }

    #[test]
    fn test_intersperse() {
        let out = render(80, |a| {
            let items: Vec<Doc> = vec![a.text("a"), a.text("b"), a.text("c")];
            let sep = a.text(", ");
            a.intersperse(&items, sep)
        });
        assert_eq!(out, "a, b, c");
    }

    #[test]
    fn test_comma_line_flat() {
        let out = render(80, |a| {
            let items: Vec<Doc> = vec![a.text("a"), a.text("b")];
            let sep = a.comma_line();
            let inner = a.intersperse(&items, sep);
            a.group(inner)
        });
        assert_eq!(out, "a, b");
    }

    #[test]
    fn test_comma_line_break() {
        let out = render(3, |a| {
            let items: Vec<Doc> = vec![a.text("aa"), a.text("bb")];
            let sep = a.comma_line();
            let inner = a.intersperse(&items, sep);
            a.group(inner)
        });
        assert_eq!(out, "aa,\nbb");
    }

    #[test]
    fn test_hardline_prevents_flat() {
        // Group containing hardline must always break
        let out = render(80, |a| {
            let t1 = a.text("a");
            let hl = a.hardline();
            let t2 = a.text("b");
            let inner = a.concat_many(&[t1, hl, t2]);
            a.group(inner)
        });
        assert_eq!(out, "a\nb");
    }

    #[test]
    fn test_nested_groups() {
        // Outer group breaks, inner group fits flat
        let out = render(10, |a| {
            let inner_doc = {
                let t = a.text("short");
                a.group(t)
            };
            let long = a.text("a-long-text");
            let ln = a.line();
            let outer = a.concat_many(&[long, ln, inner_doc]);
            a.group(outer)
        });
        assert_eq!(out, "a-long-text\nshort");
    }
}
