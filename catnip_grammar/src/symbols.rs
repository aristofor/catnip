// FILE: catnip_grammar/src/symbols.rs
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::OnceLock;

#[derive(Debug, Default)]
struct GrammarSymbols {
    keywords: BTreeSet<String>,
    operators: BTreeSet<String>,
    no_space_before_lpar_keywords: BTreeSet<String>,
    block_starter_keywords: BTreeSet<String>,
}

static SYMBOLS: OnceLock<GrammarSymbols> = OnceLock::new();

const EXCLUDED_OPERATOR_SYMBOLS: &[&str] = &[
    "{", "}", "(", ")", "[", "]", ",", ";", ":", ".", "_", "'", "\"", "'''", "\"\"\"", "#",
];

fn symbols() -> &'static GrammarSymbols {
    SYMBOLS.get_or_init(|| {
        let grammar: Value = serde_json::from_str(include_str!("grammar.json")).unwrap_or(Value::Null);
        let rules = grammar.get("rules").cloned().unwrap_or(Value::Null);

        let mut keywords = BTreeSet::new();
        let mut operators = BTreeSet::new();
        let mut no_space_before_lpar_keywords = BTreeSet::new();
        let mut block_starter_keywords = BTreeSet::new();

        extract_keyword_strings(&rules, &mut keywords);
        extract_operator_strings(&rules, &mut operators);
        extract_keywords_before_lpar(&rules, &mut no_space_before_lpar_keywords);
        extract_block_starter_keywords(&rules, &mut block_starter_keywords);

        // Unary logical keyword; formatted as not(...)
        if keywords.contains("not") {
            no_space_before_lpar_keywords.insert("not".to_string());
        }

        GrammarSymbols {
            keywords,
            operators,
            no_space_before_lpar_keywords,
            block_starter_keywords,
        }
    })
}

fn extract_keyword_strings(node: &Value, out: &mut BTreeSet<String>) {
    match node {
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("STRING") {
                if let Some(v) = map.get("value").and_then(Value::as_str) {
                    if v.chars().all(|c| c.is_alphabetic()) && v.chars().count() > 1 {
                        out.insert(v.to_string());
                    }
                }
                return;
            }
            for value in map.values() {
                extract_keyword_strings(value, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                extract_keyword_strings(item, out);
            }
        }
        _ => {}
    }
}

fn extract_operator_strings(node: &Value, out: &mut BTreeSet<String>) {
    match node {
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("STRING") {
                if let Some(v) = map.get("value").and_then(Value::as_str) {
                    let is_operator = !v.is_empty()
                        && !v.chars().all(|c| c.is_alphabetic())
                        && !EXCLUDED_OPERATOR_SYMBOLS.contains(&v)
                        && v.chars().count() <= 3
                        && !v.contains('\\')
                        && !v.contains('?');
                    if is_operator {
                        out.insert(v.to_string());
                    }
                }
                return;
            }
            for value in map.values() {
                extract_operator_strings(value, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                extract_operator_strings(item, out);
            }
        }
        _ => {}
    }
}

fn extract_keywords_before_lpar(node: &Value, out: &mut BTreeSet<String>) {
    match node {
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("SEQ") {
                if let Some(Value::Array(members)) = map.get("members") {
                    for pair in members.windows(2) {
                        let first = pair[0].as_object();
                        let second = pair[1].as_object();
                        let Some(first) = first else {
                            continue;
                        };
                        let Some(second) = second else {
                            continue;
                        };
                        let first_is_alpha_string = first.get("type").and_then(Value::as_str) == Some("STRING")
                            && first
                                .get("value")
                                .and_then(Value::as_str)
                                .map(|v| v.chars().all(|c| c.is_alphabetic()) && v.chars().count() > 1)
                                .unwrap_or(false);
                        let second_is_lpar = second.get("type").and_then(Value::as_str) == Some("STRING")
                            && second.get("value").and_then(Value::as_str) == Some("(");

                        if first_is_alpha_string && second_is_lpar {
                            if let Some(keyword) = first.get("value").and_then(Value::as_str) {
                                out.insert(keyword.to_string());
                            }
                        }
                    }
                }
            }

            for value in map.values() {
                extract_keywords_before_lpar(value, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                extract_keywords_before_lpar(item, out);
            }
        }
        _ => {}
    }
}

/// Check if a node references a block-like construct: SYMBOL "block", FIELD wrapping
/// a block, or a literal "{" (for rules like match that inline their braces).
fn references_block(node: &Value) -> bool {
    let Some(obj) = node.as_object() else {
        return false;
    };
    match obj.get("type").and_then(Value::as_str) {
        Some("SYMBOL") => obj.get("name").and_then(Value::as_str) == Some("block"),
        Some("FIELD") => obj.get("content").map(references_block).unwrap_or(false),
        Some("STRING") => obj.get("value").and_then(Value::as_str) == Some("{"),
        _ => false,
    }
}

fn extract_block_starter_keywords(node: &Value, out: &mut BTreeSet<String>) {
    match node {
        Value::Object(map) => {
            // Unwrap precedence wrappers (PREC_RIGHT, PREC_LEFT, PREC, PREC_DYNAMIC)
            let effective = match map.get("type").and_then(Value::as_str) {
                Some("PREC_RIGHT" | "PREC_LEFT" | "PREC" | "PREC_DYNAMIC") => {
                    map.get("content").and_then(Value::as_object)
                }
                _ => Some(map),
            };

            if let Some(eff) = effective {
                if eff.get("type").and_then(Value::as_str) == Some("SEQ") {
                    if let Some(Value::Array(members)) = eff.get("members") {
                        let first_keyword = members.first().and_then(|first| {
                            let obj = first.as_object()?;
                            if obj.get("type").and_then(Value::as_str) != Some("STRING") {
                                return None;
                            }
                            let value = obj.get("value").and_then(Value::as_str)?;
                            if value.chars().all(|c| c.is_alphabetic()) && value.chars().count() > 1 {
                                Some(value)
                            } else {
                                None
                            }
                        });

                        let has_block = members.iter().any(references_block);

                        if let (Some(keyword), true) = (first_keyword, has_block) {
                            out.insert(keyword.to_string());
                        }
                    }
                }
            }

            for value in map.values() {
                extract_block_starter_keywords(value, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                extract_block_starter_keywords(item, out);
            }
        }
        _ => {}
    }
}

pub fn is_keyword(value: &str) -> bool {
    symbols().keywords.contains(value)
}

pub fn is_operator(value: &str) -> bool {
    symbols().operators.contains(value)
}

pub fn is_keyword_before_lpar(value: &str) -> bool {
    symbols().no_space_before_lpar_keywords.contains(value)
}

pub fn keywords() -> &'static BTreeSet<String> {
    &symbols().keywords
}

pub fn operators() -> &'static BTreeSet<String> {
    &symbols().operators
}

pub fn is_block_starter_keyword(value: &str) -> bool {
    symbols().block_starter_keywords.contains(value)
}
