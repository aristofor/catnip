// FILE: catnip_repl/src/commands.rs
/// Single source of truth for REPL commands.
///
/// Used by: help text generation, completer, command dispatch.
pub struct CommandInfo {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub args: &'static str,
    pub description: &'static str,
}

pub const COMMANDS: &[CommandInfo] = &[
    CommandInfo {
        name: "help",
        aliases: &["h"],
        args: "",
        description: "Show this help",
    },
    CommandInfo {
        name: "exit",
        aliases: &["quit", "q"],
        args: "",
        description: "Exit REPL",
    },
    CommandInfo {
        name: "clear",
        aliases: &["cls"],
        args: "",
        description: "Clear screen",
    },
    CommandInfo {
        name: "history",
        aliases: &[],
        args: "",
        description: "Show command history",
    },
    CommandInfo {
        name: "load",
        aliases: &[],
        args: "<file>",
        description: "Load and execute a file",
    },
    CommandInfo {
        name: "context",
        aliases: &["ctx"],
        args: "[var]",
        description: "Show variables (or detail of one)",
    },
    CommandInfo {
        name: "stats",
        aliases: &[],
        args: "",
        description: "Show execution statistics",
    },
    CommandInfo {
        name: "jit",
        aliases: &[],
        args: "",
        description: "Toggle JIT compiler",
    },
    CommandInfo {
        name: "verbose",
        aliases: &[],
        args: "",
        description: "Toggle verbose mode (show timings)",
    },
    CommandInfo {
        name: "debug",
        aliases: &[],
        args: "",
        description: "Toggle debug mode (show IR and bytecode)",
    },
    CommandInfo {
        name: "time",
        aliases: &[],
        args: "<expr>",
        description: "Benchmark an expression (adaptive iterations)",
    },
    CommandInfo {
        name: "config",
        aliases: &[],
        args: "",
        description: "Show/edit configuration (show, get, set, path)",
    },
    CommandInfo {
        name: "version",
        aliases: &["v"],
        args: "",
        description: "Show Catnip version",
    },
];

/// Generate help text from the command registry.
pub fn generate_help_text() -> String {
    let mut out = String::from("Catnip REPL Commands:\n\n");

    for cmd in COMMANDS {
        let label = if cmd.args.is_empty() {
            format!("/{}", cmd.name)
        } else {
            format!("/{} {}", cmd.name, cmd.args)
        };
        out.push_str(&format!("  {:<20}{}\n", label, cmd.description));
    }

    out.push_str(
        "\nKeyboard shortcuts:\n\
         \x20 Ctrl+D          Exit REPL\n\
         \x20 Ctrl+C          Cancel current input / interrupt execution\n\
         \x20 Ctrl+R          Reverse search history\n\
         \x20 \u{2191}/\u{2193}             Navigate history\n",
    );

    out
}

/// All command names and aliases (for completer).
pub fn all_command_names() -> Vec<&'static str> {
    let mut names = Vec::new();
    for cmd in COMMANDS {
        names.push(cmd.name);
        for alias in cmd.aliases {
            names.push(alias);
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_help_contains_all_commands() {
        let help = generate_help_text();
        for cmd in COMMANDS {
            assert!(
                help.contains(&format!("/{}", cmd.name)),
                "help text missing /{}",
                cmd.name
            );
        }
    }

    #[test]
    fn test_all_command_names_includes_aliases() {
        let names = all_command_names();
        assert!(names.contains(&"help"));
        assert!(names.contains(&"h"));
        assert!(names.contains(&"exit"));
        assert!(names.contains(&"quit"));
        assert!(names.contains(&"q"));
        assert!(names.contains(&"context"));
        assert!(names.contains(&"ctx"));
    }

    #[test]
    fn test_help_text_has_context() {
        let help = generate_help_text();
        assert!(help.contains("/context"));
    }
}
