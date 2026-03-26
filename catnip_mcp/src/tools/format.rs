// FILE: catnip_mcp/src/tools/format.rs
use rmcp::model::CallToolResult;

use catnip_tools::config::FormatConfig;
use catnip_tools::formatter::format_code;

pub fn handle(code: &str, indent_size: usize, line_length: usize) -> Result<CallToolResult, rmcp::ErrorData> {
    let mut config = FormatConfig::default();
    config.indent_size = indent_size;
    config.line_length = line_length;

    match format_code(code, &config) {
        Ok(formatted) => {
            let payload = serde_json::json!({ "formatted_code": formatted });
            Ok(CallToolResult::success(vec![rmcp::model::Content::text(
                serde_json::to_string(&payload).unwrap(),
            )]))
        }
        Err(e) => {
            let payload = serde_json::json!({ "error": e });
            Ok(CallToolResult::error(vec![rmcp::model::Content::text(
                serde_json::to_string(&payload).unwrap(),
            )]))
        }
    }
}
