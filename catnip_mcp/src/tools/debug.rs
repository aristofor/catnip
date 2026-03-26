// FILE: catnip_mcp/src/tools/debug.rs
use std::sync::atomic::Ordering;

use rmcp::model::CallToolResult;

use crate::debug_session::{DebugEvent, McpDebugSession, PausedState, SessionCommand};
use crate::server::CatnipMcpServer;

fn success_json(payload: serde_json::Value) -> Result<CallToolResult, rmcp::ErrorData> {
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        serde_json::to_string(&payload).unwrap(),
    )]))
}

fn error_json(msg: &str) -> Result<CallToolResult, rmcp::ErrorData> {
    Ok(CallToolResult::error(vec![rmcp::model::Content::text(
        serde_json::to_string(&serde_json::json!({ "error": msg })).unwrap(),
    )]))
}

fn paused_payload(session_id: &str, state: &PausedState) -> serde_json::Value {
    let locals: serde_json::Map<String, serde_json::Value> = state
        .locals
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
        .collect();

    serde_json::json!({
        "session_id": session_id,
        "status": "paused",
        "line": state.line,
        "col": state.col,
        "locals": locals,
        "snippet": state.snippet,
    })
}

fn event_to_response(session_id: &str, event: &DebugEvent) -> Result<CallToolResult, rmcp::ErrorData> {
    match event {
        DebugEvent::Paused(state) => success_json(paused_payload(session_id, state)),
        DebugEvent::Finished(repr) => success_json(serde_json::json!({
            "session_id": session_id,
            "status": "finished",
            "result": repr,
        })),
        DebugEvent::Error(msg) => success_json(serde_json::json!({
            "session_id": session_id,
            "status": "error",
            "error": msg,
        })),
    }
}

pub fn handle_start(
    server: &CatnipMcpServer,
    code: &str,
    breakpoints: Option<&[i32]>,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let bp = breakpoints.unwrap_or(&[]);
    let id_num = server.debug_counter.fetch_add(1, Ordering::Relaxed);
    let session_id = format!("debug-{id_num}");

    match McpDebugSession::start(code.to_string(), bp) {
        Ok((session, first_event)) => {
            let response = event_to_response(&session_id, &first_event);
            // Only store if paused (otherwise session is already done)
            if matches!(first_event, DebugEvent::Paused(_)) {
                let mut sessions = server.debug_sessions.lock().unwrap_or_else(|e| e.into_inner());
                sessions.insert(session_id, session);
            }
            response
        }
        Err(e) => error_json(&e),
    }
}

pub fn handle_continue(server: &CatnipMcpServer, session_id: &str) -> Result<CallToolResult, rmcp::ErrorData> {
    let mut session = {
        let mut sessions = server.debug_sessions.lock().unwrap_or_else(|e| e.into_inner());
        match sessions.remove(session_id) {
            Some(s) => s,
            None => return error_json(&format!("No debug session with id '{session_id}'")),
        }
    };

    match session.send_and_wait(SessionCommand::Continue) {
        Ok(event) => {
            let response = event_to_response(session_id, &event);
            if matches!(event, DebugEvent::Paused(_)) {
                let mut sessions = server.debug_sessions.lock().unwrap_or_else(|e| e.into_inner());
                sessions.insert(session_id.to_string(), session);
            }
            response
        }
        Err(e) => error_json(&e),
    }
}

pub fn handle_step(server: &CatnipMcpServer, session_id: &str, mode: &str) -> Result<CallToolResult, rmcp::ErrorData> {
    let cmd = match mode {
        "over" => SessionCommand::StepOver,
        "out" => SessionCommand::StepOut,
        _ => SessionCommand::StepInto,
    };

    let mut session = {
        let mut sessions = server.debug_sessions.lock().unwrap_or_else(|e| e.into_inner());
        match sessions.remove(session_id) {
            Some(s) => s,
            None => return error_json(&format!("No debug session with id '{session_id}'")),
        }
    };

    match session.send_and_wait(cmd) {
        Ok(event) => {
            let response = event_to_response(session_id, &event);
            if matches!(event, DebugEvent::Paused(_)) {
                let mut sessions = server.debug_sessions.lock().unwrap_or_else(|e| e.into_inner());
                sessions.insert(session_id.to_string(), session);
            }
            response
        }
        Err(e) => error_json(&e),
    }
}

pub fn handle_inspect(server: &CatnipMcpServer, session_id: &str) -> Result<CallToolResult, rmcp::ErrorData> {
    let sessions = server.debug_sessions.lock().unwrap_or_else(|e| e.into_inner());
    let session = match sessions.get(session_id) {
        Some(s) => s,
        None => return error_json(&format!("No debug session with id '{session_id}'")),
    };

    match session.last_paused() {
        Some(state) => success_json(paused_payload(session_id, state)),
        None => error_json("Session is not paused"),
    }
}

pub fn handle_eval(server: &CatnipMcpServer, session_id: &str, expr: &str) -> Result<CallToolResult, rmcp::ErrorData> {
    let sessions = server.debug_sessions.lock().unwrap_or_else(|e| e.into_inner());
    let session = match sessions.get(session_id) {
        Some(s) => s,
        None => return error_json(&format!("No debug session with id '{session_id}'")),
    };

    match session.eval_expr(expr) {
        Ok(repr) => success_json(serde_json::json!({ "result": repr })),
        Err(e) => error_json(&e),
    }
}

pub fn handle_breakpoint(
    server: &CatnipMcpServer,
    session_id: &str,
    line: i32,
    action: &str,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let sessions = server.debug_sessions.lock().unwrap_or_else(|e| e.into_inner());
    let session = match sessions.get(session_id) {
        Some(s) => s,
        None => return error_json(&format!("No debug session with id '{session_id}'")),
    };

    if line < 1 {
        return error_json("Line number must be >= 1");
    }

    match action {
        "add" => {
            session.add_breakpoint(line as usize);
            success_json(serde_json::json!({ "status": "added", "line": line }))
        }
        "remove" => {
            session.remove_breakpoint(line as usize);
            success_json(serde_json::json!({ "status": "removed", "line": line }))
        }
        _ => error_json(&format!("Unknown action '{action}'. Use 'add' or 'remove'.")),
    }
}
