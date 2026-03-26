// FILE: catnip_mcp/src/tools/check.rs
use rmcp::model::CallToolResult;

use crate::server::CatnipMcpServer;

pub fn handle(server: &CatnipMcpServer, code: &str) -> Result<CallToolResult, rmcp::ErrorData> {
    let result = server.with_pipeline(|pipeline| pipeline.parse_to_ir(code, false));

    let payload = match result {
        Ok(_) => serde_json::json!({
            "valid": true,
            "message": "Syntax is valid",
        }),
        Err(e) => serde_json::json!({
            "valid": false,
            "error": e,
        }),
    };

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        serde_json::to_string(&payload).unwrap(),
    )]))
}
