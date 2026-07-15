// FILE: catnip_lsp/src/encoding.rs
//
// LSP position encoding. Tree-sitter reports columns as byte offsets within a
// line; the LSP protocol counts `Position.character` in the *negotiated*
// encoding, which defaults to UTF-16 (LSP spec 3.17, ServerCapabilities.
// positionEncoding). A byte offset only equals the UTF-16 offset for ASCII, so
// any non-ASCII content earlier on the line (string, comment) shifts every
// position to the right. We negotiate UTF-8 when the client offers it (then the
// byte offset is already correct) and otherwise convert byte -> UTF-16.

use tower_lsp::lsp_types::{ClientCapabilities, PositionEncodingKind};

/// Negotiated position encoding for `Position.character`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PositionEncoding {
    /// `character` counts UTF-8 bytes (tree-sitter columns map identity).
    Utf8,
    /// `character` counts UTF-16 code units (LSP default).
    #[default]
    Utf16,
}

impl PositionEncoding {
    /// Pick UTF-8 only if the client advertised it via `general.positionEncodings`.
    /// Per the spec, a server may not return UTF-8 unless the client offered it;
    /// otherwise UTF-16 is the only valid value.
    pub fn negotiate(caps: &ClientCapabilities) -> Self {
        let offered = caps.general.as_ref().and_then(|g| g.position_encodings.as_ref());
        match offered {
            Some(encs) if encs.contains(&PositionEncodingKind::UTF8) => PositionEncoding::Utf8,
            _ => PositionEncoding::Utf16,
        }
    }

    /// The `PositionEncodingKind` to advertise in `ServerCapabilities`.
    pub fn kind(self) -> PositionEncodingKind {
        match self {
            PositionEncoding::Utf8 => PositionEncodingKind::UTF8,
            PositionEncoding::Utf16 => PositionEncodingKind::UTF16,
        }
    }

    /// Convert a tree-sitter byte column (offset within `line`) to the encoded
    /// `Position.character`. A byte offset past the line end (or not on a char
    /// boundary) is clamped to the line end, so malformed input cannot panic.
    pub fn encode_column(self, line: &str, byte_col: usize) -> u32 {
        match self {
            // LSP UTF-8 counts bytes; tree-sitter columns are already bytes.
            PositionEncoding::Utf8 => byte_col as u32,
            PositionEncoding::Utf16 => {
                let end = byte_col.min(line.len());
                let prefix = match line.get(..end) {
                    Some(p) => p,
                    // byte_col landed inside a multi-byte char: walk down to a
                    // valid boundary instead of panicking on a bad slice.
                    None => {
                        let mut e = end;
                        while e > 0 && !line.is_char_boundary(e) {
                            e -= 1;
                        }
                        &line[..e]
                    }
                };
                prefix.chars().map(char::len_utf16).sum::<usize>() as u32
            }
        }
    }

    /// Convert an encoded `Position.character` (in this encoding) back to the
    /// byte column within `line` that tree-sitter expects. Out-of-range input
    /// clamps to the line's byte length, so malformed client positions cannot
    /// panic.
    pub fn decode_column(self, line: &str, character: u32) -> usize {
        match self {
            PositionEncoding::Utf8 => (character as usize).min(line.len()),
            PositionEncoding::Utf16 => {
                let target = character as usize;
                let mut units = 0usize;
                for (byte_idx, ch) in line.char_indices() {
                    if units >= target {
                        return byte_idx;
                    }
                    units += ch.len_utf16();
                }
                line.len()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_is_identity_on_bytes() {
        let enc = PositionEncoding::Utf8;
        // "café" = c a f é(2 bytes); byte col 5 is end of the 4-char word.
        assert_eq!(enc.encode_column("café x", 5), 5);
    }

    #[test]
    fn utf16_counts_code_units() {
        let enc = PositionEncoding::Utf16;
        // `y = "café" ; zzz` — `zzz` starts at byte 14, UTF-16 char 13 (é is
        // 2 bytes but 1 UTF-16 unit).
        let line = "y = \"café\" ; zzz = y";
        assert_eq!(line.as_bytes()[14], b'z');
        assert_eq!(enc.encode_column(line, 14), 13);
    }

    #[test]
    fn utf16_ascii_unchanged() {
        let enc = PositionEncoding::Utf16;
        assert_eq!(enc.encode_column("hello world", 6), 6);
    }

    #[test]
    fn out_of_bounds_byte_col_clamps() {
        let enc = PositionEncoding::Utf16;
        // Clamp instead of panic when byte_col exceeds the line.
        assert_eq!(enc.encode_column("ab", 99), 2);
    }

    #[test]
    fn decode_inverts_encode_utf16() {
        let enc = PositionEncoding::Utf16;
        let line = "y = \"café\" ; zzz = y";
        // byte 14 -> utf16 13 -> byte 14 round-trip
        let utf16 = enc.encode_column(line, 14);
        assert_eq!(utf16, 13);
        assert_eq!(enc.decode_column(line, utf16), 14);
    }

    #[test]
    fn decode_out_of_range_clamps() {
        let enc = PositionEncoding::Utf16;
        assert_eq!(enc.decode_column("ab", 99), 2);
        assert_eq!(PositionEncoding::Utf8.decode_column("ab", 99), 2);
    }
}
