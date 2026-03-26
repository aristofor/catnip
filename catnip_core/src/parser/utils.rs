// FILE: catnip_core/src/parser/utils.rs
use tree_sitter::Node;

/// Extract text from node
pub fn node_text<'a>(node: &Node, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}

/// Iterate over named children (excluding comments)
pub fn named_children<'a>(node: &Node<'a>) -> Vec<Node<'a>> {
    let mut result = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.is_named() && child.kind() != "comment" {
            result.push(child);
        }
    }
    result
}

/// Unescape string literal (handles \n, \t, \r, \\, \", \')
pub fn unescape_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();

    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('r') => result.push('\r'),
                Some('\\') => result.push('\\'),
                Some('"') => result.push('"'),
                Some('\'') => result.push('\''),
                Some(c) => {
                    result.push('\\');
                    result.push(c);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Unescape byte string literal (handles \n, \t, \r, \\, \", \', \0, \xHH)
pub fn unescape_bytes(s: &str) -> Vec<u8> {
    let mut result = Vec::with_capacity(s.len());
    let mut chars = s.chars();

    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push(b'\n'),
                Some('t') => result.push(b'\t'),
                Some('r') => result.push(b'\r'),
                Some('\\') => result.push(b'\\'),
                Some('"') => result.push(b'"'),
                Some('\'') => result.push(b'\''),
                Some('0') => result.push(0),
                Some('x') => {
                    let hi = chars.next();
                    let lo = chars.next();
                    if let (Some(h), Some(l)) = (hi, lo) {
                        let hex = format!("{}{}", h, l);
                        if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                            result.push(byte);
                        } else {
                            // invalid hex escape, emit raw
                            result.extend_from_slice(b"\\x");
                            result.extend_from_slice(h.encode_utf8(&mut [0; 4]).as_bytes());
                            result.extend_from_slice(l.encode_utf8(&mut [0; 4]).as_bytes());
                        }
                    } else {
                        // incomplete hex escape
                        result.extend_from_slice(b"\\x");
                        if let Some(h) = hi {
                            result.extend_from_slice(h.encode_utf8(&mut [0; 4]).as_bytes());
                        }
                    }
                }
                Some(c) => {
                    result.push(b'\\');
                    let mut buf = [0; 4];
                    result.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                }
                None => result.push(b'\\'),
            }
        } else {
            let mut buf = [0; 4];
            result.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
    }

    result
}
