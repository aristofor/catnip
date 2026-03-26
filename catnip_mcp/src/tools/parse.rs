// FILE: catnip_mcp/src/tools/parse.rs
use rmcp::model::CallToolResult;

use crate::server::CatnipMcpServer;

pub fn handle(server: &CatnipMcpServer, code: &str, level: i32) -> Result<CallToolResult, rmcp::ErrorData> {
    let level = level.clamp(0, 2);

    let result = server.with_pipeline(|pipeline| {
        if level == 0 {
            // Raw tree-sitter s-expression
            pipeline.parse_to_sexp(code)
        } else {
            let semantic = level >= 2;
            pipeline
                .parse_to_ir(code, semantic)
                .map(|ir| ir.to_compact_json_pretty())
        }
    });

    match result {
        Ok(json) => Ok(CallToolResult::success(vec![rmcp::model::Content::text(json)])),
        Err(e) => {
            let payload = serde_json::json!({"error": e});
            Ok(CallToolResult::error(vec![rmcp::model::Content::text(
                payload.to_string(),
            )]))
        }
    }
}
