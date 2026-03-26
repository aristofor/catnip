// FILE: catnip_rs/src/bin/run.rs
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
use std::sync::Arc;

use _rs::cli::{ExecutionStats, SourceInput, print_runtime_info};
use _rs::freeze::{frozen_to_value, value_to_frozen};
use _rs::pipeline::Pipeline;
use _rs::vm::Value;
use _rs::vm::unified_compiler::{FunctionCompileMeta, UnifiedCompiler};
use catnip_core::freeze::worker::{WorkerCommand, WorkerResult, read_message, write_message};
use catnip_core::ir::IR;
use clap::{Parser, Subcommand};
use pyo3::prelude::*;

#[derive(Parser)]
#[command(name = "catnip")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "Catnip runtime with embedded Python")]
#[command(after_help = "\
Python subcommands: cache, commands, config, debug, format, lint, lsp, module, plugins, repl\n\
Run 'catnip <command> --help' for subcommand help.")]
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

    /// Suppress result display
    #[arg(short = 'q', long = "quiet")]
    quiet: bool,

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
        #[arg(default_value_t = _rs::constants::BENCH_DEFAULT_ITERATIONS)]
        iterations: usize,
        /// Script file
        file: PathBuf,
    },
    /// Internal: ND worker process (IPC over stdin/stdout, not user-facing)
    #[command(hide = true)]
    Worker,
}

/// Known Python CLI subcommands (registered via pyproject.toml entry points).
const PYTHON_SUBCOMMANDS: &[&str] = &[
    "cache",
    "commands",
    "completion",
    "config",
    "debug",
    "extensions",
    "format",
    "lint",
    "module",
    "plugins",
    "lsp",
    "repl",
];

/// Check if the first positional argument is a Python CLI subcommand.
/// Skips options and their values, returns true only for known Python subcommands.
fn should_delegate_to_python(args: &[String]) -> bool {
    let mut i = 1; // skip program name
    while i < args.len() {
        let arg = &args[i];

        if arg == "--" {
            return false;
        }

        if arg.starts_with('-') {
            // Options that take a value: skip next arg too
            // Includes both Rust-native and Python-only options
            if matches!(
                arg.as_str(),
                "-c" | "--command"
                    | "-b"
                    | "--bench"
                    | "--jit-threshold"
                    | "-o"
                    | "--optimize"
                    | "-m"
                    | "--module"
                    | "-x"
                    | "--executor"
                    | "-p"
                    | "--parsing"
                    | "--config"
                    | "--theme"
                    | "--format"
                    | "--policy"
            ) {
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }

        // First positional: check if it's a Python subcommand
        return PYTHON_SUBCOMMANDS.contains(&arg.as_str());
    }
    false
}

/// Delegate the full invocation to the Python CLI (catnip.cli:main).
/// Sets sys.argv and calls Click's main(), never returns.
fn delegate_to_python_cli(args: Vec<String>) -> ! {
    let code = Python::attach(|py| {
        // Set sys.argv for Click
        let sys = py.import("sys").expect("failed to import sys");
        let py_args = pyo3::types::PyList::new(py, &args).expect("failed to create list");
        sys.setattr("argv", py_args).expect("failed to set sys.argv");

        // Import and call catnip.cli.main()
        match py.import("catnip.cli").and_then(|m| m.getattr("main")) {
            Ok(main_fn) => {
                match main_fn.call0() {
                    Ok(_) => 0,
                    Err(e) if e.is_instance_of::<pyo3::exceptions::PySystemExit>(py) => {
                        // Click raises SystemExit on normal completion
                        e.value(py)
                            .getattr("code")
                            .and_then(|c| c.extract::<i32>())
                            .unwrap_or(0)
                    }
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        1
                    }
                }
            }
            Err(e) => {
                eprintln!("Error: could not load Python CLI: {}", e);
                1
            }
        }
    });
    // Flush Python stdout/stderr before exiting (may be buffered in embedded mode)
    Python::attach(|py| {
        let _ = py
            .import("sys")
            .and_then(|sys| sys.getattr("stdout")?.call_method0("flush"));
        let _ = py
            .import("sys")
            .and_then(|sys| sys.getattr("stderr")?.call_method0("flush"));
    });
    process::exit(code);
}

fn main() {
    // Pre-parse: delegate Python subcommands before Clap runs
    let raw_args: Vec<String> = std::env::args().collect();
    if should_delegate_to_python(&raw_args) {
        delegate_to_python_cli(raw_args);
    }

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
        Some(Commands::Worker) => {
            run_worker();
            return;
        }
        None => {}
    }

    // No source input -> delegate to Python CLI (REPL, pipe detection, etc.)
    if cli.file.is_none() && cli.command.is_none() && !cli.stdin {
        delegate_to_python_cli(raw_args);
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

    match execute(&source, &mut stats, !cli.no_jit, cli.jit_threshold, cli.quiet) {
        Ok(value) => {
            // Flush Python's stdout (may be buffered in embedded mode)
            Python::attach(|py| {
                let _ = py
                    .import("sys")
                    .and_then(|sys| sys.getattr("stdout")?.call_method0("flush"));
            });
            if let Some(v) = value {
                println!("{}", v);
            }
            if cli.verbose {
                stats.print_verbose();
            }
            process::exit(0);
        }
        Err(e) => {
            // exit(N) -> process::exit(N)
            if let Some(code_str) = e.strip_prefix("exit(").and_then(|s| s.strip_suffix(')')) {
                let code = code_str.parse::<i32>().unwrap_or(1);
                process::exit(code);
            }
            // Log internal errors to disk for diagnostics
            if e.contains("WeirdError:") {
                let report = _rs::weird_log::WeirdReport::new(e.clone(), Some("vm".into()));
                _rs::weird_log::log_weird_error(&report);
            }
            eprintln!("{}", e);
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
        if value.as_bool().unwrap() { "True" } else { "False" }.to_string()
    } else if value.is_struct_instance() {
        Python::attach(|py| {
            let py_obj = value.to_pyobject(py);
            py_obj
                .bind(py)
                .repr()
                .map(|s| s.to_string())
                .unwrap_or_else(|_| "<struct>".to_string())
        })
    } else {
        // Complex types (PyObject): use Python str()
        Python::attach(|py| {
            let py_obj = value.to_pyobject(py);
            let pyobj = py_obj.bind(py);
            // Decimal -> display with d suffix
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
        })
    }
}

fn execute(
    source: &SourceInput,
    stats: &mut ExecutionStats,
    enable_jit: bool,
    jit_threshold: u32,
    quiet: bool,
) -> Result<Option<String>, String> {
    let mut pipeline = Pipeline::new()?;
    pipeline.set_jit_enabled(enable_jit);
    pipeline.set_jit_threshold(jit_threshold);
    if let Some(path) = source.filename() {
        pipeline.set_source_path(path);
    }
    let (result, timings) = pipeline.execute_timed(source.code())?;

    stats.parse_time_us = timings.parse_us;
    stats.compile_time_us = timings.compile_us;
    stats.execute_time_us = timings.execute_us;
    stats.total_time_us = timings.total_us;

    if quiet {
        return Ok(None);
    }

    let display = value_to_string(result);

    if display.is_empty() {
        Ok(None)
    } else {
        Ok(Some(display))
    }
}

/// ND worker process: reads WorkerCommands from stdin, executes, writes WorkerResults to stdout.
fn run_worker() {
    use std::io::{self, BufReader, BufWriter};

    let mut stdin = BufReader::new(io::stdin().lock());
    let mut stdout = BufWriter::new(io::stdout().lock());

    // Persistent pipeline reused across Execute commands
    let mut pipeline: Option<Pipeline> = None;

    loop {
        let cmd: WorkerCommand = match read_message(&mut stdin) {
            Ok(Some(cmd)) => cmd,
            Ok(None) => break, // EOF: parent closed pipe
            Err(e) => {
                eprintln!("worker: read error: {}", e);
                break;
            }
        };

        match cmd {
            WorkerCommand::Ping => {
                let _ = write_message(&mut stdout, &WorkerResult::Pong);
            }
            WorkerCommand::Shutdown => break,
            WorkerCommand::Execute {
                encoded_ir,
                captures,
                param_names,
                seed,
            } => {
                let result = execute_worker_task(&mut pipeline, &encoded_ir, &captures, &param_names, &seed);
                let msg = match result {
                    Ok(frozen) => WorkerResult::Ok(frozen),
                    Err(e) => WorkerResult::Err(e),
                };
                if write_message(&mut stdout, &msg).is_err() {
                    break; // Broken pipe
                }
            }
        }
    }
}

/// Execute a single worker task: decode IR, compile as function, run with seed.
fn execute_worker_task(
    pipeline: &mut Option<Pipeline>,
    encoded_ir: &[u8],
    captures: &[(String, catnip_core::freeze::FrozenValue)],
    param_names: &[String],
    seed: &catnip_core::freeze::FrozenValue,
) -> Result<catnip_core::freeze::FrozenValue, String> {
    // Decode raw bincode IR (no .catf header)
    let body_ir: Vec<IR> = catnip_core::freeze::decode(encoded_ir).map_err(|e| format!("worker: decode IR: {e}"))?;
    if body_ir.is_empty() {
        return Err("worker: empty IR".to_string());
    }

    // Ensure pipeline exists
    if pipeline.is_none() {
        *pipeline = Some(Pipeline::new()?);
    }
    let pipe = pipeline.as_mut().unwrap();

    // ensure_executor initializes VM+Host and installs thread-locals
    pipe.ensure_executor();

    // Compile, build closure scope and args inside Python::attach
    let (code, closure, args) = Python::attach(|py| -> Result<_, String> {
        let executor = pipe.ensure_executor();
        executor.ensure_host(py)?;

        // Compile the body as a function with param_names
        let body_node = if body_ir.len() == 1 {
            &body_ir[0]
        } else {
            return Err("worker: expected single IR body node".to_string());
        };

        let mut compiler = UnifiedCompiler::new();
        let code = compiler
            .compile_function_pure(
                py,
                body_node,
                FunctionCompileMeta {
                    params: param_names.to_vec(),
                    name: "<nd_worker>",
                    defaults: vec![],
                    vararg_idx: -1,
                    parent_nesting_depth: 0,
                },
            )
            .map_err(|e| format!("worker compile: {}", e))?;

        // Reconstruct closure scope from frozen captures
        let closure = if captures.is_empty() {
            None
        } else {
            Some(_rs::freeze::thaw_captures(py, captures))
        };

        // Build args: [seed_value, NIL (placeholder for recur)]
        let seed_val = frozen_to_value(py, seed);
        let mut args = vec![seed_val];
        if param_names.len() > 1 {
            args.push(Value::NIL);
        }

        Ok((code, closure, args))
    })?;

    // Execute function with closure scope
    let executor = pipe.ensure_executor();
    let result = executor.execute_function(Arc::new(code), &args, closure)?;

    // Freeze result
    Python::attach(|py| value_to_frozen(py, result).ok_or_else(|| "worker: result not freezable".to_string()))
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
    if let Err(e) = execute(&source, &mut warmup_stats, enable_jit, jit_threshold, false) {
        eprintln!("Warmup failed: {}", e);
        process::exit(1);
    }

    // Benchmark runs
    for i in 0..iterations {
        let mut stats = ExecutionStats::default();
        stats.jit_enabled = enable_jit;

        match execute(&source, &mut stats, enable_jit, jit_threshold, false) {
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
