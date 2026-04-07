// FILE: catnip_mcp/src/resources.rs
use std::path::Path;

use rmcp::model::*;

const JSON_MIME: &str = "application/json";
const TEXT_MIME: &str = "text/plain";
const MARKDOWN_MIME: &str = "text/markdown";

fn is_valid_segment(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && s.as_bytes()[0].is_ascii_alphanumeric()
}

pub fn read_resource(base_path: &Path, uri: &str) -> Result<ReadResourceResult, ErrorData> {
    let uri_str = uri.to_string();

    let path = uri_str
        .strip_prefix("catnip://")
        .ok_or_else(|| ErrorData::invalid_params("URI must start with catnip://", None))?;

    let parts: Vec<&str> = path.split('/').collect();

    match parts.as_slice() {
        ["examples", topic] => {
            if !is_valid_segment(topic) {
                return Err(ErrorData::invalid_params("Invalid examples topic", None));
            }
            read_examples(base_path, topic, &uri_str)
        }

        ["codex", category, module] => {
            if !is_valid_segment(category) || !is_valid_segment(module) {
                return Err(ErrorData::invalid_params(
                    "Invalid codex URI. Format: catnip://codex/{category}/{module}",
                    None,
                ));
            }
            read_codex(base_path, category, module, &uri_str)
        }

        ["docs", section, topic] => {
            if !is_valid_segment(section) || !is_valid_segment(topic) {
                return Err(ErrorData::invalid_params("Invalid docs URI", None));
            }
            read_docs_topic(base_path, section, topic, &uri_str)
        }

        ["docs", section] => {
            if !is_valid_segment(section) {
                return Err(ErrorData::invalid_params("Invalid docs section", None));
            }
            read_docs_section(base_path, section, &uri_str)
        }

        _ => Err(ErrorData::invalid_params(
            format!("Unknown resource URI: {uri_str}"),
            None,
        )),
    }
}

fn text_resource(uri: &str, content: String, mime: &str) -> Result<ReadResourceResult, ErrorData> {
    Ok(ReadResourceResult::new(vec![
        ResourceContents::text(content, uri).with_mime_type(mime),
    ]))
}

fn read_examples(base_path: &Path, topic: &str, uri: &str) -> Result<ReadResourceResult, ErrorData> {
    let dir = base_path.join("docs").join("examples").join(topic);
    if !dir.is_dir() {
        return Err(ErrorData::invalid_params(
            format!("Examples topic not found: {topic}"),
            None,
        ));
    }

    let mut entries = Vec::new();
    if let Ok(read_dir) = std::fs::read_dir(&dir) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "cat") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    // Use stem (without .cat extension) and "code" key
                    let name = path.file_stem().unwrap().to_string_lossy().to_string();
                    entries.push(serde_json::json!({
                        "name": name,
                        "code": content,
                    }));
                }
            }
        }
    }
    entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));

    text_resource(uri, serde_json::to_string(&entries).unwrap(), JSON_MIME)
}

fn read_codex(base_path: &Path, category: &str, module: &str, uri: &str) -> Result<ReadResourceResult, ErrorData> {
    let path = base_path
        .join("docs")
        .join("codex")
        .join(category)
        .join(format!("{module}.cat"));

    match std::fs::read_to_string(&path) {
        Ok(content) => text_resource(uri, content, TEXT_MIME),
        Err(_) => Err(ErrorData::invalid_params(
            format!("Codex module not found: {category}/{module}"),
            None,
        )),
    }
}

fn read_docs_topic(base_path: &Path, section: &str, topic: &str, uri: &str) -> Result<ReadResourceResult, ErrorData> {
    let docs_dir = base_path.join("docs").join(section);
    // Normalize: dashes → underscores
    let normalized = topic.to_uppercase().replace('-', "_");
    let lower = docs_dir.join(format!("{topic}.md"));
    let upper = docs_dir.join(format!("{normalized}.md"));

    let path = if lower.is_file() {
        lower
    } else if upper.is_file() {
        upper
    } else {
        return Err(ErrorData::invalid_params(
            format!("Documentation not found: {section}/{topic}"),
            None,
        ));
    };

    match std::fs::read_to_string(&path) {
        Ok(content) => text_resource(uri, content, MARKDOWN_MIME),
        Err(e) => Err(ErrorData::internal_error(format!("Failed to read file: {e}"), None)),
    }
}

fn read_docs_section(base_path: &Path, section: &str, uri: &str) -> Result<ReadResourceResult, ErrorData> {
    let dir = base_path.join("docs").join(section);
    if !dir.is_dir() {
        return Err(ErrorData::invalid_params(
            format!("Documentation section not found: {section}"),
            None,
        ));
    }

    let mut topics = Vec::new();
    if let Ok(read_dir) = std::fs::read_dir(&dir) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "md") {
                if let Some(stem) = path.file_stem() {
                    // Normalize underscores → dashes
                    let name = stem.to_string_lossy().to_lowercase().replace('_', "-");
                    if name != "index" {
                        topics.push(name);
                    }
                }
            }
        }
    }
    topics.sort();

    let payload = serde_json::json!({
        "section": section,
        "topics": topics,
    });

    text_resource(uri, serde_json::to_string(&payload).unwrap(), JSON_MIME)
}
