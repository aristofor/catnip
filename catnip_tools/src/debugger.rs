/// Debugger command parsing and output formatting.
///
/// Pure logic: no I/O, no PyO3. Consumed via rlib by catnip_rs shims.

/// Parsed debugger command.
#[derive(Debug, Clone, PartialEq)]
pub enum DebugCommand {
    Continue,
    StepInto,
    StepOver,
    StepOut,
    Break(u32),
    RemoveBreak(u32),
    Print(String),
    Vars,
    List,
    Backtrace,
    Quit,
    Help,
    Repeat,
    Unknown(String),
}

/// Parse a raw input line into a DebugCommand.
pub fn parse_command(input: &str) -> DebugCommand {
    let input = input.trim();
    if input.is_empty() {
        return DebugCommand::Repeat;
    }

    let (cmd, arg) = match input.split_once(char::is_whitespace) {
        Some((c, a)) => (c, a.trim()),
        None => (input, ""),
    };

    match cmd.to_ascii_lowercase().as_str() {
        "c" | "continue" => DebugCommand::Continue,
        "s" | "step" => DebugCommand::StepInto,
        "n" | "next" => DebugCommand::StepOver,
        "o" | "out" => DebugCommand::StepOut,
        "b" | "break" => match arg.parse::<u32>() {
            Ok(line) if !arg.is_empty() => DebugCommand::Break(line),
            _ => DebugCommand::Unknown(input.to_string()),
        },
        "rb" => match arg.parse::<u32>() {
            Ok(line) if !arg.is_empty() => DebugCommand::RemoveBreak(line),
            _ => DebugCommand::Unknown(input.to_string()),
        },
        "p" | "print" => {
            if arg.is_empty() {
                DebugCommand::Unknown(input.to_string())
            } else {
                DebugCommand::Print(arg.to_string())
            }
        }
        "v" | "vars" => DebugCommand::Vars,
        "l" | "list" => DebugCommand::List,
        "bt" | "backtrace" => DebugCommand::Backtrace,
        "q" | "quit" => DebugCommand::Quit,
        "h" | "help" => DebugCommand::Help,
        _ => DebugCommand::Unknown(cmd.to_string()),
    }
}

/// Format the help text.
pub fn format_help() -> String {
    [
        "Commands:",
        "  c, continue   - Continue execution",
        "  s, step       - Step into",
        "  n, next       - Step over",
        "  o, out        - Step out",
        "  b N           - Breakpoint at line N",
        "  rb N          - Remove breakpoint at line N",
        "  p EXPR        - Evaluate expression in scope",
        "  v, vars       - Show local variables",
        "  l, list       - Show source context",
        "  bt, backtrace - Show call stack",
        "  q, quit       - Abort",
        "  h, help       - This help",
        "  (empty)       - Repeat last action",
    ]
    .join("\n")
}

/// Format the session header.
pub fn format_header() -> String {
    "Catnip Debugger\nType 'h' for help.".to_string()
}

/// Format a pause display (stopped at line/col with snippet).
pub fn format_pause(line: u32, col: u32, snippet: &str) -> String {
    let mut out = format!("\nStopped at line {}, col {}", line, col);
    if !snippet.is_empty() {
        for s in snippet.lines() {
            out.push_str(&format!("\n  {}", s));
        }
    }
    out
}

/// Format local variables display.
///
/// Expects pre-repr'd values: `(name, repr_string)`.
pub fn format_vars(vars: &[(String, String)]) -> String {
    if vars.is_empty() {
        return "  (no local variables)".to_string();
    }
    vars.iter()
        .map(|(name, value)| format!("  {} = {}", name, value))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Format backtrace display.
///
/// Expects `(function_name, line_number)`.
pub fn format_backtrace(frames: &[(String, u32)]) -> String {
    if frames.is_empty() {
        return "  (at top level)".to_string();
    }
    frames
        .iter()
        .enumerate()
        .map(|(i, (name, line))| format!("  #{} {} at line {}", i, name, line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Format unknown command message.
pub fn format_unknown_command(cmd: &str) -> String {
    format!("Unknown command: {}. Type 'h' for help.", cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty() {
        assert_eq!(parse_command(""), DebugCommand::Repeat);
        assert_eq!(parse_command("  "), DebugCommand::Repeat);
    }

    #[test]
    fn test_parse_continue() {
        assert_eq!(parse_command("c"), DebugCommand::Continue);
        assert_eq!(parse_command("continue"), DebugCommand::Continue);
    }

    #[test]
    fn test_parse_steps() {
        assert_eq!(parse_command("s"), DebugCommand::StepInto);
        assert_eq!(parse_command("n"), DebugCommand::StepOver);
        assert_eq!(parse_command("o"), DebugCommand::StepOut);
    }

    #[test]
    fn test_parse_break() {
        assert_eq!(parse_command("b 5"), DebugCommand::Break(5));
        assert_eq!(parse_command("break 10"), DebugCommand::Break(10));
        assert!(matches!(parse_command("b"), DebugCommand::Unknown(_)));
        assert!(matches!(parse_command("b abc"), DebugCommand::Unknown(_)));
    }

    #[test]
    fn test_parse_remove_break() {
        assert_eq!(parse_command("rb 5"), DebugCommand::RemoveBreak(5));
        assert!(matches!(parse_command("rb"), DebugCommand::Unknown(_)));
    }

    #[test]
    fn test_parse_print() {
        assert_eq!(
            parse_command("p x + 1"),
            DebugCommand::Print("x + 1".to_string())
        );
        assert!(matches!(parse_command("p"), DebugCommand::Unknown(_)));
    }

    #[test]
    fn test_parse_info() {
        assert_eq!(parse_command("v"), DebugCommand::Vars);
        assert_eq!(parse_command("l"), DebugCommand::List);
        assert_eq!(parse_command("bt"), DebugCommand::Backtrace);
        assert_eq!(parse_command("q"), DebugCommand::Quit);
        assert_eq!(parse_command("h"), DebugCommand::Help);
    }

    #[test]
    fn test_parse_unknown() {
        assert_eq!(
            parse_command("xyz"),
            DebugCommand::Unknown("xyz".to_string())
        );
    }

    #[test]
    fn test_format_vars_empty() {
        assert_eq!(format_vars(&[]), "  (no local variables)");
    }

    #[test]
    fn test_format_vars() {
        let vars = vec![
            ("x".to_string(), "42".to_string()),
            ("y".to_string(), "'hello'".to_string()),
        ];
        let out = format_vars(&vars);
        assert!(out.contains("x = 42"));
        assert!(out.contains("y = 'hello'"));
    }

    #[test]
    fn test_format_backtrace_empty() {
        assert_eq!(format_backtrace(&[]), "  (at top level)");
    }

    #[test]
    fn test_format_backtrace() {
        let frames = vec![("main".to_string(), 1), ("foo".to_string(), 5)];
        let out = format_backtrace(&frames);
        assert!(out.contains("#0 main at line 1"));
        assert!(out.contains("#1 foo at line 5"));
    }

    #[test]
    fn test_format_pause() {
        let out = format_pause(10, 3, "  10 | x = 42\n     | ^");
        assert!(out.contains("Stopped at line 10, col 3"));
        assert!(out.contains("x = 42"));
    }
}
