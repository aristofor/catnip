// FILE: catnip_mcp/src/tools/eval.rs
use std::sync::Arc;

use rmcp::model::CallToolResult;

use crate::server::CatnipMcpServer;
use catnip_vm::collections::ValueKey;
use catnip_vm::value::{NativeString, Value};

/// Convert a JSON value to a Catnip Value (recursive, supports arrays and objects).
fn json_to_value(v: &serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::NIL,
        serde_json::Value::Bool(b) => Value::from_bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::from_int(i)
            } else {
                Value::from_float(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => Value::from_str(s),
        serde_json::Value::Array(arr) => Value::from_list(arr.iter().map(json_to_value).collect()),
        serde_json::Value::Object(obj) => {
            let mut map = indexmap::IndexMap::new();
            for (k, v) in obj {
                let key = ValueKey::Str(Arc::new(NativeString::new(k.clone())));
                map.insert(key, json_to_value(v));
            }
            Value::from_dict(map)
        }
    }
}

pub fn handle(
    server: &CatnipMcpServer,
    code: &str,
    context: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let result = server.with_pipeline(|pipeline| {
        // Reset pipeline state for isolation between calls
        pipeline.reset();
        if let Some(ctx) = context {
            for (name, val) in ctx {
                pipeline.set_global(name, json_to_value(val));
            }
        }
        pipeline.execute(code)
    });

    match result {
        Ok(value) => {
            let repr = value.repr_string();
            let type_name = super::value_type_name(&value);
            let payload = serde_json::json!({
                "result_repr": repr,
                "result_type": type_name,
            });
            Ok(CallToolResult::success(vec![rmcp::model::Content::text(
                serde_json::to_string(&payload).unwrap(),
            )]))
        }
        Err(e) => {
            let payload = serde_json::json!({
                "error": e.to_string(),
                "type": format!("{:?}", e).split('(').next().unwrap_or("VMError"),
            });
            Ok(CallToolResult::error(vec![rmcp::model::Content::text(
                serde_json::to_string(&payload).unwrap(),
            )]))
        }
    }
}
