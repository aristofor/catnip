// FILE: catnip_rs/src/bin/standalone.rs
//! Catnip standalone runtime - Optimized Rust binary with embedded Python.
//!
//! Features:
//! - Fast Rust VM with NaN-boxing
//! - JIT compiler (Cranelift) enabled by default
//! - Embedded Python for full compatibility
//! - Optimized release build (LTO, strip)
//!
//! Performance: ~2-5x faster than Python CLI for compute-heavy scripts.

use std::path::PathBuf;
use std::process;

use _rs::cli::{print_runtime_info, ExecutionStats, SourceInput};
use _rs::standalone::StandalonePipeline;
use _rs::vm::Value;
use clap::{Parser, Subcommand};
use pyo3::prelude::*;

#[derive(Parser)]
#[command(name = "catnip-standalone")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "Catnip standalone runtime with embedded Python")]
struct Cli {
    /// Script file to execute
    #[arg(value_name = "FILE")]
    file: Option<PathBuf>,

    /// Evaluate expression directly
    #[arg(short = 'c', long = "command", value_name = "CODE")]
    command: Option<String>,

    /// Read from stdin
    #[arg(long = "stdin")]
    stdin: bool,

    /// Verbose output with execution statistics
    #[arg(short = 'v', long = "verbose")]
    verbose: bool,

    /// Disable JIT compiler (enabled by default)
    #[arg(long = "no-jit")]
    no_jit: bool,

    /// JIT threshold (number of iterations before compilation)
    #[arg(long = "jit-threshold", default_value = "100")]
    jit_threshold: u32,

    /// Benchmark mode (run multiple times and show stats)
    #[arg(short = 'b', long = "bench", value_name = "N")]
    bench: Option<usize>,

    #[command(subcommand)]
    command_type: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Show runtime information
    Info,
    /// Benchmark a script (run N times)
    Bench {
        /// Number of iterations
        #[arg(default_value = "10")]
        iterations: usize,
        /// Script file
        file: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();

    // Handle subcommands
    match cli.command_type {
        Some(Commands::Info) => {
            print_runtime_info();
            return;
        }
        Some(Commands::Bench { iterations, file }) => {
            run_benchmark(&file, iterations, !cli.no_jit, cli.jit_threshold);
            return;
        }
        None => {}
    }

    // Get source code
    let source = match load_source(&cli) {
        Ok(src) => src,
        Err(e) => {
            eprintln!("Error reading input: {}", e);
            process::exit(1);
        }
    };

    // Execute
    let mut stats = ExecutionStats::default();
    stats.jit_enabled = !cli.no_jit;

    match execute(&source, &mut stats, !cli.no_jit, cli.jit_threshold) {
        Ok(value) => {
            if let Some(v) = value {
                println!("{}", v);
            }
            if cli.verbose {
                stats.print_verbose();
            }
            process::exit(0);
        }
        Err(e) => {
            // Log internal errors to disk for diagnostics
            if e.contains("WeirdError:") {
                let report = _rs::weird_log::WeirdReport::new(e.clone(), Some("vm".into()));
                _rs::weird_log::log_weird_error(&report);
            }
            eprintln!("Error: {}", e);
            if cli.verbose {
                stats.print_verbose();
            }
            process::exit(1);
        }
    }
}

fn load_source(cli: &Cli) -> Result<SourceInput, String> {
    if let Some(ref cmd) = cli.command {
        Ok(SourceInput::from_command(cmd.clone()))
    } else if cli.stdin {
        SourceInput::from_stdin().map_err(|e| e.to_string())
    } else if let Some(ref file) = cli.file {
        SourceInput::from_file(file).map_err(|e| e.to_string())
    } else {
        Err("No input provided. Use FILE, -c CODE, or --stdin".to_string())
    }
}

/// Convert Value to display string using Python when needed
fn value_to_string(value: Value) -> String {
    if value.is_nil() {
        return String::new(); // None prints nothing
    } else if value.is_int() {
        return value.as_int().unwrap().to_string();
    } else if value.is_float() {
        let f = value.as_float().unwrap();
        // Match Python's float repr
        if f.is_infinite() {
            if f.is_sign_positive() { "inf" } else { "-inf" }.to_string()
        } else if f.is_nan() {
            "nan".to_string()
        } else {
            // Python always shows .0 for whole number floats (5.0 not 5)
            let s = f.to_string();
            if f.fract() == 0.0 && !s.contains('.') {
                format!("{}.0", s)
            } else {
                s
            }
        }
    } else if value.is_bool() {
        // Python-style True/False
        if value.as_bool().unwrap() {
            "True"
        } else {
            "False"
        }
        .to_string()
    } else {
        // Complex types (PyObject): use Python str()
        Python::attach(|py| unsafe {
            if let Some(ptr) = value.as_pyobj_ptr() {
                let pyobj = pyo3::Bound::from_borrowed_ptr(py, ptr);
                // Decimal → display with d suffix
                if let Ok(decimal_cls) = py.import("decimal").and_then(|m| m.getattr("Decimal")) {
                    if pyobj.is_instance(&decimal_cls).unwrap_or(false) {
                        return pyobj
                            .str()
                            .map(|s| format!("{}d", s))
                            .unwrap_or_else(|_| "<error>".to_string());
                    }
                }
                pyobj
                    .str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|_| "<error>".to_string())
            } else {
                "<unknown>".to_string()
            }
        })
    }
}

fn execute(
    source: &SourceInput,
    stats: &mut ExecutionStats,
    _enable_jit: bool,
    _jit_threshold: u32,
) -> Result<Option<String>, String> {
    let mut pipeline = StandalonePipeline::new()?;
    let (result, timings) = pipeline.execute_timed(source.code())?;

    stats.parse_time_us = timings.parse_us;
    stats.compile_time_us = timings.compile_us;
    stats.execute_time_us = timings.execute_us;
    stats.total_time_us = timings.total_us;

    let display = value_to_string(result);

    if display.is_empty() {
        Ok(None)
    } else {
        Ok(Some(display))
    }
}

fn run_benchmark(file: &PathBuf, iterations: usize, enable_jit: bool, jit_threshold: u32) {
    println!(
        "Benchmarking {} ({} iterations, JIT: {})",
        file.display(),
        iterations,
        if enable_jit { "enabled" } else { "disabled" }
    );

    // Load source once
    let source = match SourceInput::from_file(file) {
        Ok(src) => src,
        Err(e) => {
            eprintln!("Error loading file: {}", e);
            process::exit(1);
        }
    };

    let mut all_times = Vec::with_capacity(iterations);
    let mut all_stats = Vec::with_capacity(iterations);

    // Warmup run (not counted)
    let mut warmup_stats = ExecutionStats::default();
    warmup_stats.jit_enabled = enable_jit;
    if let Err(e) = execute(&source, &mut warmup_stats, enable_jit, jit_threshold) {
        eprintln!("Warmup failed: {}", e);
        process::exit(1);
    }

    // Benchmark runs
    for i in 0..iterations {
        let mut stats = ExecutionStats::default();
        stats.jit_enabled = enable_jit;

        match execute(&source, &mut stats, enable_jit, jit_threshold) {
            Ok(_) => {
                all_times.push(stats.total_time_us);
                all_stats.push(stats);
            }
            Err(e) => {
                eprintln!("Iteration {} failed: {}", i + 1, e);
                process::exit(1);
            }
        }
    }

    // Compute statistics
    let min = *all_times.iter().min().unwrap();
    let max = *all_times.iter().max().unwrap();
    let mean = all_times.iter().sum::<u64>() / iterations as u64;
    let total_jit_compilations: usize = all_stats.iter().map(|s| s.jit_compilations).sum();

    println!("\n=== Benchmark Results ===");
    println!("Iterations:      {}", iterations);
    println!("Min time:        {:>8} μs", min);
    println!("Max time:        {:>8} μs", max);
    println!("Mean time:       {:>8} μs", mean);
    println!("Total time:      {:>8} μs", all_times.iter().sum::<u64>());
    if enable_jit {
        println!("JIT compilations: {}", total_jit_compilations);
        if total_jit_compilations > 0 {
            println!("  (warmup run triggered JIT compilation)");
        }
    }
}
