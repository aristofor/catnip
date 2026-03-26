// FILE: catnip_tools/src/tokenizer.rs
use crate::token::Token;
use catnip_grammar::node_kinds as NK;
use tree_sitter::Node;

/// Extract tokens from tree-sitter parse tree
pub fn extract_tokens(node: Node, source: &str, tokens: &mut Vec<Token>) {
    let kind = node.kind();

    // Strings: opaque token (don't decompose into children)
    // Triple-quoted strings have children (start/content/end) that must not be reformatted
    if kind == NK::STRING || kind == NK::FSTRING || kind == NK::BSTRING {
        let value = node.utf8_text(source.as_bytes()).unwrap_or("").to_string();
        tokens.push(Token {
            type_: map_node_type(kind),
            value,
            line: node.start_position().row + 1,
            column: node.start_position().column + 1,
            end_line: node.end_position().row + 1,
            end_column: node.end_position().column + 1,
        });
        return;
    }

    // For terminal nodes (leaf nodes), extract as tokens
    if node.child_count() == 0 {
        let type_ = map_node_type(kind);
        let value = node.utf8_text(source.as_bytes()).unwrap_or("").to_string();

        tokens.push(Token {
            type_,
            value,
            line: node.start_position().row + 1,
            column: node.start_position().column + 1,
            end_line: node.end_position().row + 1,
            end_column: node.end_position().column + 1,
        });
    } else {
        // For non-terminal nodes, recursively extract tokens from children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            extract_tokens(child, source, tokens);
        }
    }
}

/// Map tree-sitter node type to formatter token type
pub fn map_node_type(kind: &str) -> String {
    match kind {
        NK::IDENTIFIER => "NAME",
        NK::INTEGER => "INT",
        NK::FLOAT => "FLOAT",
        NK::DECIMAL => "DECIMAL",
        NK::IMAGINARY => "IMAGINARY",
        NK::STRING => "STRING",
        NK::FSTRING => "FSTRING",
        NK::BSTRING => "BSTRING",
        NK::COMMENT => "COMMENT",
        NK::TRUE => "TRUE",
        NK::FALSE => "FALSE",
        NK::NONE => "NONE",
        "(" => "LPAR",
        ")" => "RPAR",
        "{" => "LBRACE",
        "}" => "RBRACE",
        "[" => "LBRACKET",
        "]" => "RBRACKET",
        "," => "COMMA",
        ";" => "SEMICOLON",
        "." => "DOT",
        ":" => "COLON",
        "=" => "EQUAL",
        "+" => "PLUS",
        "-" => "MINUS",
        "*" => "STAR",
        "/" => "SLASH",
        "//" => "DOUBLE_SLASH",
        "%" => "PERCENT",
        "**" => "DOUBLE_STAR",
        "==" => "EQEQUAL",
        "!=" => "NOTEQUAL",
        "<" => "LESS",
        "<=" => "LESSEQUAL",
        ">" => "GREATER",
        ">=" => "GREATEREQUAL",
        "&" => "AMPER",
        "|" => "VBAR",
        "^" => "CIRCUMFLEX",
        "~" => "TILDE",
        "<<" => "LEFTSHIFT",
        ">>" => "RIGHTSHIFT",
        "and" => "AND",
        "or" => "OR",
        "not" => "NOT",
        "if" => "IF",
        "elif" => "ELIF",
        "else" => "ELSE",
        "while" => "WHILE",
        "for" => "FOR",
        "in" => "IN",
        "match" => "MATCH",
        "return" => "RETURN",
        "break" => "BREAK",
        "continue" => "CONTINUE",
        "=>" => "ARROW",
        _ => kind,
    }
    .to_string()
}
