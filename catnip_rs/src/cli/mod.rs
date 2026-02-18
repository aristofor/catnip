// FILE: catnip_rs/src/cli/mod.rs
//! Common CLI utilities for Catnip binaries.
//!
//! Shared between catnip-standalone and catnip-repl for:
//! - Source code loading (file, -c, stdin)
//! - Version/info display
//! - Error handling

use std::fs;
use std::io::{self, Read};
use std::path::Path;

/// Source of input code.
#[derive(Debug, Clone)]
pub enum SourceInput {
    /// Code from command line (-c flag)
    Command(String),
    /// Code from file
    File(String, String), // (path, content)
    /// Code from stdin
    Stdin(String),
}

impl SourceInput {
    /// Get the code content.
    pub fn code(&self) -> &str {
        match self {
            Self::Command(code) => code,
            Self::File(_, code) => code,
            Self::Stdin(code) => code,
        }
    }

    /// Get optional filename for error messages.
    pub fn filename(&self) -> Option<&str> {
        match self {
            Self::File(path, _) => Some(path),
            _ => None,
        }
    }

    /// Load source from file path.
    pub fn from_file<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path_str = path.as_ref().to_string_lossy().to_string();
        let content = fs::read_to_string(&path)?;
        Ok(Self::File(path_str, content))
    }

    /// Load source from stdin.
    pub fn from_stdin() -> io::Result<Self> {
        let mut buffer = String::new();
        io::stdin().read_to_string(&mut buffer)?;
        Ok(Self::Stdin(buffer))
    }

    /// Create from command string.
    pub fn from_command(code: String) -> Self {
        Self::Command(code)
    }
}

/// Runtime statistics for execution.
#[derive(Debug, Default)]
pub struct ExecutionStats {
    pub parse_time_us: u64,
    pub compile_time_us: u64,
    pub execute_time_us: u64,
    pub total_time_us: u64,
    pub jit_enabled: bool,
    pub jit_compilations: usize,
}

impl ExecutionStats {
    pub fn print_verbose(&self) {
        println!("\n=== Execution Statistics ===");
        println!("Parse time:   {:>8} μs", self.parse_time_us);
        println!("Compile time: {:>8} μs", self.compile_time_us);
        println!("Execute time: {:>8} μs", self.execute_time_us);
        println!("Total time:   {:>8} μs", self.total_time_us);
        println!(
            "JIT enabled:  {}",
            if self.jit_enabled { "yes" } else { "no" }
        );
        if self.jit_enabled {
            println!("JIT compiles: {}", self.jit_compilations);
        }
    }
}

/// Format version string.
pub fn version_string() -> String {
    format!("Catnip v{} (Rust runtime)", env!("CARGO_PKG_VERSION"))
}

/// Print runtime information.
pub fn print_runtime_info() {
    println!("Catnip Standalone Runtime");
    println!("Version: {}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("Features:");
    println!("  - Rust VM: Yes (NaN-boxing)");
    println!("  - JIT Compiler: Available (Cranelift x86-64)");
    println!("  - Python Support: Embedded (PyO3)");
    println!();
    println!("Build Profile:");
    #[cfg(debug_assertions)]
    println!("  - Mode: Debug");
    #[cfg(not(debug_assertions))]
    println!("  - Mode: Release (LTO, optimized)");
    println!();
    println!("Usage:");
    println!("  catnip-standalone script.cat");
    println!("  catnip-standalone -c \"x = 10; x * 2\"");
    println!("  echo \"2 + 3\" | catnip-standalone --stdin");
    println!("  catnip-standalone bench script.cat  # Benchmark mode");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_from_command() {
        let src = SourceInput::from_command("x = 10".to_string());
        assert_eq!(src.code(), "x = 10");
        assert!(src.filename().is_none());
    }
}
