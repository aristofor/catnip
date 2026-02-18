"""MCP server for Catnip language usage."""

import json
import sys
from importlib.metadata import PackageNotFoundError, version
from pathlib import Path
from typing import Any

import anyio
import mcp.types as types
from mcp.server.lowlevel.helper_types import ReadResourceContents
from mcp.server.lowlevel.server import NotificationOptions, Server
from mcp.server.models import InitializationOptions
from mcp.server.stdio import stdio_server

SERVER = Server("catnip", version="unknown")


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _ensure_catnip_importable() -> None:
    """Ensure catnip is importable (installed or on sys.path)."""
    try:
        import catnip  # noqa: F401
    except ImportError:
        # Fallback: add parent of this package to sys.path
        root = str(Path(__file__).parents[1])
        if root not in sys.path:
            sys.path.insert(0, root)


def _format_exception(e: Exception) -> dict[str, Any]:
    """Format an exception for JSON serialization."""
    error_info: dict[str, Any] = {"error": str(e), "type": type(e).__name__}
    try:
        from catnip.exc import CatnipError

        if isinstance(e, CatnipError):
            if e.filename:
                error_info["filename"] = e.filename
            if e.line is not None:
                error_info["line"] = e.line
            if e.column is not None:
                error_info["column"] = e.column
            if e.context:
                error_info["context"] = e.context
            error_info["message"] = e.message
    except ImportError:
        pass
    return error_info


def _docs_root() -> Path:
    """Locate the docs/ directory (works installed or from source)."""
    # Installed: docs shipped as package data
    pkg_docs = Path(__file__).parent / "docs"
    if pkg_docs.is_dir():
        return pkg_docs
    # Source tree
    src_docs = Path(__file__).parents[1] / "docs"
    if src_docs.is_dir():
        return src_docs
    return pkg_docs  # will fail gracefully later


# ---------------------------------------------------------------------------
# Tools
# ---------------------------------------------------------------------------


def _tool_parse_catnip(arguments: dict[str, Any]) -> dict[str, Any]:
    """Parse Catnip code and return structured IR as JSON."""
    _ensure_catnip_importable()
    code = arguments.get("code", "")
    level = arguments.get("level", 1)

    try:
        from catnip._rs import process_input as rust_process_input

        from catnip import Catnip

        catnip = Catnip()

        if level == 0:
            from catnip.parser import Parser

            parser = Parser()
            ast = parser.parse(code)
            return {"parse_tree": str(ast), "level": 0}

        ir_list = rust_process_input(catnip, code, level)
        if not ir_list:
            return {"ir": [], "level": level}

        from catnip._rs import ir_to_json_compact_pretty

        json_items = []
        for item in ir_list:
            item_json = ir_to_json_compact_pretty(item)
            json_items.append(json.loads(item_json))

        return {"ir": json_items, "level": level}
    except Exception as e:
        return _format_exception(e)


def _tool_eval_catnip(arguments: dict[str, Any]) -> dict[str, Any]:
    """Evaluate Catnip code and return result."""
    _ensure_catnip_importable()
    code = arguments.get("code", "")
    context = arguments.get("context") or {}

    try:
        from catnip import Catnip

        catnip = Catnip()
        for name, value in context.items():
            catnip.context.globals[name] = value

        catnip.parse(code)
        result = catnip.execute()
        return {"result": repr(result), "type": type(result).__name__}
    except Exception as e:
        return _format_exception(e)


def _tool_check_syntax(arguments: dict[str, Any]) -> dict[str, Any]:
    """Validate Catnip syntax without execution."""
    _ensure_catnip_importable()
    code = arguments.get("code", "")

    try:
        from catnip.parser import Parser

        parser = Parser()
        parser.parse(code)
        return {"valid": True, "message": "Syntax is valid"}
    except Exception as e:
        error_info = _format_exception(e)
        error_info["valid"] = False
        return error_info


def _tool_format_code(arguments: dict[str, Any]) -> dict[str, Any]:
    """Format Catnip code with configurable style."""
    _ensure_catnip_importable()
    code = arguments.get("code", "")
    indent_size = arguments.get("indent_size", 4)
    line_length = arguments.get("line_length", 120)

    try:
        from catnip._rs import FormatConfig, format_code

        config = FormatConfig()
        config.indent_size = indent_size
        config.line_length = line_length

        formatted = format_code(code, config)
        return {"formatted_code": formatted}
    except Exception as e:
        return _format_exception(e)


# ---------------------------------------------------------------------------
# Debug tools
# ---------------------------------------------------------------------------

_debug_sessions: dict[str, Any] = {}
_debug_counter = 0


def _get_debug_session(session_id: str):
    session = _debug_sessions.get(session_id)
    if session is None:
        raise ValueError(f"No debug session with id '{session_id}'")
    return session


def _debug_event_response(session_id: str, event):
    if event is None:
        return {"session_id": session_id, "status": "timeout"}
    event_type, data = event
    if event_type == 'paused':
        return {
            "session_id": session_id,
            "status": "paused",
            "line": data.line,
            "col": data.col,
            "locals": {k: repr(v) for k, v in data.locals.items()},
            "snippet": data.snippet,
        }
    elif event_type == 'finished':
        _debug_sessions.pop(session_id, None)
        return {"session_id": session_id, "status": "finished", "result": repr(data)}
    else:
        _debug_sessions.pop(session_id, None)
        return {"session_id": session_id, "status": "error", "error": str(data)}


def _tool_debug_start(arguments: dict[str, Any]) -> dict[str, Any]:
    """Start a debug session."""
    _ensure_catnip_importable()
    global _debug_counter

    code = arguments.get("code", "")
    breakpoints = arguments.get("breakpoints", [])

    try:
        from catnip import Catnip
        from catnip.debug.session import DebugSession

        catnip = Catnip()
        session = DebugSession(catnip, code)
        for line in breakpoints:
            session.add_breakpoint(int(line))

        _debug_counter += 1
        session_id = f"dbg-{_debug_counter}"
        _debug_sessions[session_id] = session

        session.start(blocking=False)

        event = session.wait_for_event(timeout=10)
        if event is None:
            return {"session_id": session_id, "status": "timeout"}

        event_type, data = event
        if event_type == 'paused':
            return {
                "session_id": session_id,
                "status": "paused",
                "line": data.line,
                "col": data.col,
                "locals": {k: repr(v) for k, v in data.locals.items()},
                "snippet": data.snippet,
            }
        elif event_type == 'finished':
            del _debug_sessions[session_id]
            return {"session_id": session_id, "status": "finished", "result": repr(data)}
        else:
            del _debug_sessions[session_id]
            return {"session_id": session_id, "status": "error", "error": str(data)}

    except Exception as e:
        return _format_exception(e)


def _tool_debug_continue(arguments: dict[str, Any]) -> dict[str, Any]:
    """Continue execution until next breakpoint."""
    try:
        session = _get_debug_session(arguments["session_id"])
        session.send_command("continue")
        event = session.wait_for_event(timeout=10)
        return _debug_event_response(arguments["session_id"], event)
    except Exception as e:
        return _format_exception(e)


def _tool_debug_step(arguments: dict[str, Any]) -> dict[str, Any]:
    """Step execution (into/over/out)."""
    try:
        session = _get_debug_session(arguments["session_id"])
        mode = arguments.get("mode", "into")
        session.send_command(f"step_{mode}" if mode != "into" else "step_into")
        event = session.wait_for_event(timeout=10)
        return _debug_event_response(arguments["session_id"], event)
    except Exception as e:
        return _format_exception(e)


def _tool_debug_inspect(arguments: dict[str, Any]) -> dict[str, Any]:
    """Inspect local variables at current pause point."""
    try:
        session = _get_debug_session(arguments["session_id"])
        if session.state.value != "paused":
            return {"error": "Session is not paused"}
        pause = session.last_pause
        if pause is None:
            return {"error": "No pause info available"}
        return {
            "session_id": arguments["session_id"],
            "status": "paused",
            "line": pause.line,
            "col": pause.col,
            "locals": {k: repr(v) for k, v in pause.locals.items()},
            "snippet": pause.snippet,
        }
    except Exception as e:
        return _format_exception(e)


def _tool_debug_eval(arguments: dict[str, Any]) -> dict[str, Any]:
    """Evaluate expression in current debug scope."""
    _ensure_catnip_importable()
    try:
        session = _get_debug_session(arguments["session_id"])
        expr = arguments.get("expr", "")
        from catnip import Catnip

        c = Catnip()
        c.context.globals.update(session.catnip.context.globals)
        pause = session.last_pause
        if pause is not None:
            c.context.globals.update(pause.locals)
        result = c.parse(expr).execute()
        return {"result": repr(result)}
    except Exception as e:
        return _format_exception(e)


def _tool_debug_breakpoint(arguments: dict[str, Any]) -> dict[str, Any]:
    """Add or remove a breakpoint."""
    try:
        session = _get_debug_session(arguments["session_id"])
        line = int(arguments["line"])
        action = arguments.get("action", "add")
        if action == "add":
            session.add_breakpoint(line)
            sm = session._sourcemap
            offset = sm.line_to_offset(line)
            if offset is not None:
                session._executor.vm._vm.add_debug_breakpoint(offset)
            return {"status": "added", "line": line}
        else:
            session.remove_breakpoint(line)
            sm = session._sourcemap
            offset = sm.line_to_offset(line)
            if offset is not None:
                session._executor.vm._vm.remove_debug_breakpoint(offset)
            return {"status": "removed", "line": line}
    except Exception as e:
        return _format_exception(e)


# ---------------------------------------------------------------------------
# Tool registry
# ---------------------------------------------------------------------------

TOOL_HANDLERS = {
    "parse_catnip": _tool_parse_catnip,
    "eval_catnip": _tool_eval_catnip,
    "check_syntax": _tool_check_syntax,
    "format_code": _tool_format_code,
    "debug_start": _tool_debug_start,
    "debug_continue": _tool_debug_continue,
    "debug_step": _tool_debug_step,
    "debug_inspect": _tool_debug_inspect,
    "debug_eval": _tool_debug_eval,
    "debug_breakpoint": _tool_debug_breakpoint,
}

TOOLS = [
    types.Tool(
        name="parse_catnip",
        description="Parse Catnip code and return structured IR as JSON. Levels: 0=parse tree (text), 1=IR (default), 2=executable IR after semantic analysis. Output uses compact JSON (primitives flat, metadata omitted when default).",
        inputSchema={
            "type": "object",
            "properties": {
                "code": {"type": "string"},
                "level": {"type": "integer", "default": 1, "minimum": 0, "maximum": 2},
            },
            "required": ["code"],
            "additionalProperties": False,
        },
    ),
    types.Tool(
        name="eval_catnip",
        description="Evaluate Catnip code and return result.",
        inputSchema={
            "type": "object",
            "properties": {
                "code": {"type": "string"},
                "context": {"type": "object"},
            },
            "required": ["code"],
            "additionalProperties": False,
        },
    ),
    types.Tool(
        name="check_syntax",
        description="Validate Catnip syntax without execution.",
        inputSchema={
            "type": "object",
            "properties": {"code": {"type": "string"}},
            "required": ["code"],
            "additionalProperties": False,
        },
    ),
    types.Tool(
        name="format_code",
        description="Format Catnip code with configurable style.",
        inputSchema={
            "type": "object",
            "properties": {
                "code": {"type": "string"},
                "indent_size": {"type": "integer", "default": 4},
                "line_length": {"type": "integer", "default": 120},
            },
            "required": ["code"],
            "additionalProperties": False,
        },
    ),
    types.Tool(
        name="debug_start",
        description="Start a debug session. Returns state at first breakpoint or end of execution.",
        inputSchema={
            "type": "object",
            "properties": {
                "code": {"type": "string", "description": "Catnip source code to debug"},
                "breakpoints": {
                    "type": "array",
                    "items": {"type": "integer"},
                    "description": "Line numbers to break at (1-indexed)",
                },
            },
            "required": ["code"],
            "additionalProperties": False,
        },
    ),
    types.Tool(
        name="debug_continue",
        description="Continue execution until next breakpoint.",
        inputSchema={
            "type": "object",
            "properties": {"session_id": {"type": "string"}},
            "required": ["session_id"],
            "additionalProperties": False,
        },
    ),
    types.Tool(
        name="debug_step",
        description="Step execution. Mode: 'into' (default), 'over', or 'out'.",
        inputSchema={
            "type": "object",
            "properties": {
                "session_id": {"type": "string"},
                "mode": {"type": "string", "enum": ["into", "over", "out"], "default": "into"},
            },
            "required": ["session_id"],
            "additionalProperties": False,
        },
    ),
    types.Tool(
        name="debug_inspect",
        description="Inspect local variables at current pause point.",
        inputSchema={
            "type": "object",
            "properties": {"session_id": {"type": "string"}},
            "required": ["session_id"],
            "additionalProperties": False,
        },
    ),
    types.Tool(
        name="debug_eval",
        description="Evaluate an expression in the current debug scope.",
        inputSchema={
            "type": "object",
            "properties": {
                "session_id": {"type": "string"},
                "expr": {"type": "string"},
            },
            "required": ["session_id", "expr"],
            "additionalProperties": False,
        },
    ),
    types.Tool(
        name="debug_breakpoint",
        description="Add or remove a breakpoint at a line.",
        inputSchema={
            "type": "object",
            "properties": {
                "session_id": {"type": "string"},
                "line": {"type": "integer"},
                "action": {"type": "string", "enum": ["add", "remove"], "default": "add"},
            },
            "required": ["session_id", "line"],
            "additionalProperties": False,
        },
    ),
]


# ---------------------------------------------------------------------------
# Resources
# ---------------------------------------------------------------------------

RESOURCES: list[types.Resource] = []

_DOC_SECTIONS = frozenset(('lang', 'tuto', 'user'))

RESOURCE_TEMPLATES = [
    types.ResourceTemplate(
        name="examples",
        uriTemplate="catnip://examples/{topic}",
        description="Examples by theme (advanced, basics, broadcast, cfg, control-flow, embedding, functions, module-loading, pattern-matching, performance, standalone, tools).",
        mimeType="application/json",
    ),
    types.ResourceTemplate(
        name="codex",
        uriTemplate="catnip://codex/{category}/{module}",
        description="Python integration examples (files-formats, data-analytics, web, images-media, geospatial).",
        mimeType="text/plain",
    ),
    types.ResourceTemplate(
        name="docs-topic",
        uriTemplate="catnip://docs/{section}/{topic}",
        description="Language documentation. Sections: lang (syntax, functions, pattern-matching, broadcast, scopes-and-variables, control-flow, expressions, pragmas, glossary, turing-completeness), tuto (quickstart-0min, quickstart-2min, quickstart-5min), user (cli, repl, config, module-loading, embedding-guide, extending-context, host-integration, shebang-guide, standalone).",
        mimeType="text/markdown",
    ),
    types.ResourceTemplate(
        name="docs-section",
        uriTemplate="catnip://docs/{section}",
        description="List available topics for a documentation section (lang, tuto, user).",
        mimeType="application/json",
    ),
]


def _resource_contents(content: str, mime_type: str) -> list[ReadResourceContents]:
    return [ReadResourceContents(content=content, mime_type=mime_type)]


# ---------------------------------------------------------------------------
# MCP handlers
# ---------------------------------------------------------------------------


@SERVER.list_resources()
async def _list_resources() -> list[types.Resource]:
    return RESOURCES


@SERVER.list_resource_templates()
async def _list_resource_templates() -> list[types.ResourceTemplate]:
    return RESOURCE_TEMPLATES


@SERVER.read_resource()
async def _read_resource(uri: types.AnyUrl) -> list[ReadResourceContents]:
    uri_str = str(uri)
    docs = _docs_root()

    if uri_str.startswith("catnip://examples/"):
        topic = uri_str.split("catnip://examples/", 1)[1]
        examples_dir = docs / "examples" / topic
        if not examples_dir.exists():
            return _resource_contents(
                json.dumps({"error": f"Examples for '{topic}' not found"}, indent=2),
                "application/json",
            )
        examples_list = []
        for f in sorted(examples_dir.glob("*.cat")):
            examples_list.append({"name": f.stem, "code": f.read_text()})
        return _resource_contents(json.dumps(examples_list, indent=2), "application/json")

    if uri_str.startswith("catnip://codex/"):
        path_parts = uri_str.split("catnip://codex/", 1)[1].split("/")
        if len(path_parts) != 2:
            return _resource_contents(
                json.dumps({"error": "Invalid codex URI. Format: catnip://codex/{category}/{module}"}, indent=2),
                "application/json",
            )
        category, module = path_parts
        codex_file = docs / "codex" / category / f"{module}.cat"
        if codex_file.exists():
            return _resource_contents(codex_file.read_text(), "text/plain")
        return _resource_contents(
            json.dumps({"error": f"Codex example not found: {category}/{module}"}, indent=2),
            "application/json",
        )

    if uri_str.startswith("catnip://docs/"):
        parts = uri_str.split("catnip://docs/", 1)[1].rstrip("/").split("/")
        section = parts[0]
        if section not in _DOC_SECTIONS:
            return _resource_contents(
                json.dumps(
                    {"error": f"Unknown section '{section}'. Available: {', '.join(sorted(_DOC_SECTIONS))}"}, indent=2
                ),
                "application/json",
            )
        section_dir = docs / section

        # catnip://docs/{section} — list available topics
        if len(parts) == 1:
            topics = sorted(f.stem.lower().replace("_", "-") for f in section_dir.glob("*.md") if f.name != "index.md")
            return _resource_contents(
                json.dumps({"section": section, "topics": topics}, indent=2),
                "application/json",
            )

        # catnip://docs/{section}/{topic} — serve markdown
        topic = parts[1]
        filename = topic.upper().replace("-", "_") + ".md"
        filepath = section_dir / filename
        if filepath.is_file():
            return _resource_contents(filepath.read_text(), "text/markdown")
        # topic not found — list available ones
        available = sorted(f.stem.lower().replace("_", "-") for f in section_dir.glob("*.md") if f.name != "index.md")
        return _resource_contents(
            json.dumps(
                {"error": f"Topic '{topic}' not found in section '{section}'", "available": available}, indent=2
            ),
            "application/json",
        )

    return _resource_contents(
        json.dumps({"error": f"Unknown resource: {uri_str}"}, indent=2),
        "application/json",
    )


@SERVER.list_tools()
async def _list_tools() -> list[types.Tool]:
    return TOOLS


@SERVER.call_tool()
async def _call_tool(name: str, arguments: dict[str, Any]) -> dict[str, Any]:
    handler = TOOL_HANDLERS.get(name)
    if handler is None:
        return {"error": f"Unknown tool: {name}"}
    return handler(arguments)


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


def _server_version() -> str:
    try:
        return version("catnip-lang")
    except PackageNotFoundError:
        return "unknown"


async def _run() -> None:
    SERVER.version = _server_version()
    async with stdio_server() as (read_stream, write_stream):
        await SERVER.run(
            read_stream,
            write_stream,
            InitializationOptions(
                server_name="catnip",
                server_version=SERVER.version,
                capabilities=SERVER.get_capabilities(
                    notification_options=NotificationOptions(),
                    experimental_capabilities={},
                ),
            ),
        )


def main() -> None:
    anyio.run(_run)


if __name__ == "__main__":
    main()
