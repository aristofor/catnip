// FILE: catnip_tools/src/pretty/align.rs
//! Column alignment post-processing pass.

use crate::config::FormatConfig;

/// Find the column of the sole `=` assignment in a line.
/// Returns None if the `=` is part of `==`, `!=`, `<=`, `>=`, `=>`,
/// or augmented assignments (`+=`, etc.), or if multiple `=` appear
/// at depth 0 (kwargs line), or if no space precedes `=` (kwarg).
#[allow(dead_code)]
pub(crate) fn find_assignment_column(line: &str) -> Option<usize> {
    let mut in_string = false;
    let mut string_char = ' ';
    let mut depth: usize = 0;
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut first_eq: Option<usize> = None;

    while i < len {
        let c = bytes[i] as char;

        if !in_string {
            if c == '"' || c == '\'' {
                in_string = true;
                string_char = c;
                i += 1;
                continue;
            }
            if c == '(' || c == '[' {
                depth += 1;
                i += 1;
                continue;
            }
            if c == ')' || c == ']' {
                depth = depth.saturating_sub(1);
                i += 1;
                continue;
            }
            if c == '#' {
                break;
            }
            if c == '=' && depth == 0 {
                // Check it's not ==, !=, <=, >=, =>
                let prev = if i > 0 { bytes[i - 1] as char } else { ' ' };
                let next = if i + 1 < len { bytes[i + 1] as char } else { ' ' };
                if prev == '!' || prev == '<' || prev == '>' {
                    i += 1;
                    continue;
                }
                if next == '=' || next == '>' {
                    i += 2;
                    continue;
                }
                // Also skip +=, -=, *=, /=, %=, &=, |=, ^=
                if matches!(prev, '+' | '-' | '*' | '/' | '%' | '&' | '|' | '^') {
                    i += 1;
                    continue;
                }
                // kwarg: `key=val` (no space before `=`) -> skip
                if prev != ' ' && prev != '\t' {
                    return None;
                }
                if first_eq.is_some() {
                    // Multiple `=` at depth 0: kwargs, not an assignment
                    return None;
                }
                first_eq = Some(i);
            }
        } else {
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == string_char {
                in_string = false;
            }
        }
        i += 1;
    }
    first_eq
}

/// Find the column of the trailing `#` comment in a line.
/// Returns None if no trailing comment or if the line is a comment-only line.
pub(crate) fn find_comment_column(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return None; // Comment-only line, not a trailing comment
    }

    let mut in_string = false;
    let mut string_char = ' ';
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let c = bytes[i] as char;

        if !in_string {
            if c == '"' || c == '\'' {
                in_string = true;
                string_char = c;
            } else if c == '#' {
                return Some(i);
            }
        } else {
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == string_char {
                in_string = false;
            }
        }
        i += 1;
    }
    None
}

/// Find the column of `=>` (arrow) in a match arm line.
/// Returns None if no `=>` found or if the `=>` is inside parens/brackets/strings.
pub(crate) fn find_arrow_column(line: &str) -> Option<usize> {
    let mut in_string = false;
    let mut string_char = ' ';
    let mut depth: usize = 0;
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let c = bytes[i] as char;

        if !in_string {
            if c == '"' || c == '\'' {
                in_string = true;
                string_char = c;
                i += 1;
                continue;
            }
            if c == '(' || c == '[' {
                depth += 1;
                i += 1;
                continue;
            }
            if c == ')' || c == ']' {
                depth = depth.saturating_sub(1);
                i += 1;
                continue;
            }
            if c == '#' {
                return None;
            }
            if c == '=' && depth == 0 && i + 1 < len && bytes[i + 1] as char == '>' {
                return Some(i);
            }
        } else {
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == string_char {
                in_string = false;
            }
        }
        i += 1;
    }
    None
}

/// Get the leading indentation length (number of spaces) of a line.
pub(crate) fn indent_len(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Per-line mask: true if the line starts inside a string literal.
pub(crate) fn string_line_mask(lines: &[String]) -> Vec<bool> {
    let mut mask = Vec::with_capacity(lines.len());
    let mut in_string = false;
    let mut string_char: u8 = 0;
    let mut triple = false;

    for line in lines {
        mask.push(in_string);
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut i = 0;
        while i < len {
            let c = bytes[i];
            if in_string {
                if c == b'\\' {
                    i += 2;
                    continue;
                }
                if triple {
                    if c == string_char && i + 2 < len && bytes[i + 1] == string_char && bytes[i + 2] == string_char {
                        in_string = false;
                        i += 3;
                        continue;
                    }
                } else if c == string_char {
                    in_string = false;
                }
            } else {
                // Comment: rest of line is not code
                if c == b'#' {
                    break;
                }
                if c == b'\'' || c == b'"' {
                    if i + 2 < len && bytes[i + 1] == c && bytes[i + 2] == c {
                        in_string = true;
                        string_char = c;
                        triple = true;
                        i += 3;
                        continue;
                    }
                    in_string = true;
                    string_char = c;
                    triple = false;
                }
            }
            i += 1;
        }
    }
    mask
}

/// Align columns in formatted text (post-processing pass).
/// Alignment triggers when the first 2 lines of a group have the symbol
/// at the same column in the original source.
pub(crate) fn align_columns(text: &str, original: &str, line_map: &[usize], config: &FormatConfig) -> String {
    if !config.align {
        return text.to_string();
    }

    let orig_lines: Vec<&str> = original.split('\n').collect();
    let mut result: Vec<String> = text.split('\n').map(|l| l.to_string()).collect();

    let in_string = string_line_mask(&result);

    // Map each formatted line to its original source line
    let mapped_orig: Vec<&str> = (0..result.len())
        .map(|i| {
            line_map
                .get(i)
                .and_then(|&idx| orig_lines.get(idx))
                .copied()
                .unwrap_or("")
        })
        .collect();

    // Only align match arms (=>) and trailing comments (#). No assignment alignment.
    align_symbol(&mut result, &mapped_orig, &in_string, find_arrow_column, false);
    align_symbol(&mut result, &mapped_orig, &in_string, find_comment_column, false);

    result.join("\n")
}

/// Generic alignment pass: groups consecutive non-empty lines at the same
/// indent level that contain the symbol (found by `find_col`), then pads
/// each line so the symbol aligns to the rightmost column in the group.
///
/// Trigger modes:
/// - `always_align = false`: only if the first 2 original lines have the
///   symbol at the same column (preserve existing alignment)
/// - `always_align = true`: whenever >= 2 lines form a group (create alignment)
///
/// Lines inside string literals (in_string[i] == true) break groups.
pub(crate) fn align_symbol<F>(
    lines: &mut [String],
    orig_lines: &[&str],
    in_string: &[bool],
    find_col: F,
    always_align: bool,
) where
    F: Fn(&str) -> Option<usize>,
{
    let len = lines.len();
    let mut i = 0;

    while i < len {
        // Skip empty lines and lines inside string literals
        if lines[i].trim().is_empty() || in_string.get(i).copied().unwrap_or(false) {
            i += 1;
            continue;
        }

        let group_indent = indent_len(&lines[i]);

        // Collect group: consecutive non-empty lines at same indent with the symbol
        let mut group: Vec<(usize, usize)> = Vec::new(); // (line_index, symbol_col)
        let mut j = i;
        while j < len {
            let line = &lines[j];
            if line.trim().is_empty() || indent_len(line) != group_indent || in_string.get(j).copied().unwrap_or(false)
            {
                break;
            }
            if let Some(col) = find_col(line) {
                group.push((j, col));
            } else {
                // Line without the symbol breaks the group
                break;
            }
            j += 1;
        }

        let triggered = group.len() >= 2
            && (always_align
                || orig_lines
                    .get(group[0].0)
                    .and_then(|l| find_col(l))
                    .zip(orig_lines.get(group[1].0).and_then(|l| find_col(l)))
                    .is_some_and(|(c0, c1)| c0 == c1));
        if triggered {
            let max_col = group.iter().map(|&(_, col)| col).max().unwrap();

            for &(line_idx, col) in &group {
                if col < max_col {
                    let line = &lines[line_idx];
                    let mut new_line = String::with_capacity(line.len() + (max_col - col));
                    let before = line[..col].trim_end();
                    new_line.push_str(before);
                    let needed = max_col - before.len();
                    for _ in 0..needed {
                        new_line.push(' ');
                    }
                    new_line.push_str(&line[col..]);
                    lines[line_idx] = new_line;
                }
            }
            i = j;
        } else {
            i = j.max(i + 1);
        }
    }
}

/// Normalize consecutive newlines (max 2), strip leading empty lines,
/// and remove trailing whitespace from blank lines.
pub(crate) fn normalize_newlines(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut result = Vec::new();
    let mut blank_count = 0;

    for line in &lines {
        let is_blank = line.trim().is_empty();
        if is_blank {
            blank_count += 1;
            if blank_count <= 1 {
                result.push("");
            }
        } else {
            blank_count = 0;
            result.push(line);
        }
    }

    // Strip leading blank lines
    while result.first().is_some_and(|l| l.trim().is_empty()) {
        result.remove(0);
    }

    result.join("\n")
}
