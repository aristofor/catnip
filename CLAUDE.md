# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Catnip is a language interpreter with a hybrid Rust + Python architecture, using Tree-sitter for parsing. All hot paths
in Rust (PyO3), high-level API in Python.

See `docs/dev/ARCHITECTURE.md` for detailed architecture.

### Rust Workspace

8 crates with 2 feature modes (`extension-module` for PyO3 cdylib, `embedded` for standalone binaries with embedded
Python):

| Crate                  | PyO3 | Type        | Role                                                                      |
| ---------------------- | ---- | ----------- | ------------------------------------------------------------------------- |
| `catnip_rs`            | yes  | cdylib+rlib | Main Python extension (VM, semantic, parser, registry, cache, JIT, debug) |
| `catnip_core`          | no   | rlib        | Pure Rust core (semantic passes, CFG/SSA, pipeline)                       |
| `catnip_tools`         | no   | rlib        | Formatter + linter (direct tree-sitter tokenization)                      |
| `catnip_repl`          | yes  | cdylib+rlib | TUI ratatui (completer, fish-like hints)                                  |
| `catnip_lsp`           | no   | bin         | Pure Rust LSP server (tower-lsp, diagnostics, rename)                     |
| `catnip_grammar`       | no   | rlib        | Tree-sitter grammar (source: `grammar.js`)                                |
| `catnip_libs/{io,sys}` | yes  | rlib        | External Catnip libs                                                      |

**IMPORTANT -- PyO3 dual-feature**: `catnip_rs` and `catnip_repl` default to `extension-module` (for
`maturin`/`pip install`). Bare `cargo test` fails with linker errors (missing Python symbols). Always use:

```bash
cargo test --lib --no-default-features --features embedded  # in catnip_rs/ or catnip_repl/
# or simply:
make test-rust-fast
```

Non-PyO3 crates (`catnip_core`, `catnip_tools`, `catnip_grammar`) can be tested normally with `cargo test -p`.

## Common Commands

### Setup

```bash
make setup          # Full auto setup: venv + deps + compile + install
# or step by step:
make setup-dev      # Prepare environment (venv + deps, no compilation)
make compile        # Compile Rust extension
make install        # Install Python package + MCP
```

### Development

```bash
CATNIP_DEV=1 make compile    # Fast rebuild (incremental, thin LTO)
make compile                 # Release rebuild (slow, max optimizations)
make grammar-deps            # After modifying catnip_grammar/grammar.js
```

### Testing

```bash
# Python tests
make test                    # pytest VM mode (default)
make test-all                # Full suite: Rust + Python VM + Python AST + run
pytest tests/language/ -q    # Quick language tests (~3.6s)

# Rust tests
make test-rust-fast          # Unit tests only (~5s)
make test-rust               # All Rust tests (unit + integration)

# Specific Rust test
cd catnip_rs && cargo test --lib --no-default-features --features embedded test_name

# Standalone binary tests
make test-run-fast           # catnip-run binary tests (dev build)
```

### Running

```bash
catnip                           # Interactive REPL
catnip script.cat                # Run a script file
catnip -c "expression"           # Evaluate single expression
catnip -x ast script.cat         # Use AST interpreter (default: VM)
catnip -o jit script.cat         # Enable JIT
catnip -v script.cat             # Verbose mode
catnip --parsing 0/1/2 file.cat  # Inspect parse tree / IR / executable IR
```

### Rust Development

**OpCode synchronization**: Rust is the source of truth. Python files are generated from Rust.

```bash
make gen-opcodes        # After modifying catnip_rs/src/ir/opcode.rs or vm/opcode.rs
make check-opcodes      # Verify sync (CI check)
```

Always use `IROpCode` enum, never hardcode opcode numbers.

**Typical workflow**:

1. Modify Rust code
1. `make test-rust-fast` (~5s)
1. `make compile` (includes gen-opcodes)
1. `make test` (~25s)

**Adding new syntax**:

1. Modify `catnip_grammar/grammar.js`
1. `make grammar-deps`
1. Add transformer in `catnip_rs/src/parser/`
1. If new opcode: `catnip_rs/src/ir/opcode.rs` then `make gen-opcodes`
1. `make compile && make test`

## Architecture

### Pipeline

4-stage pipeline in the `Catnip` class:

1. **Parsing** (`parser.py`) - Tree-sitter grammar to parse tree, transform to IR
1. **Transformation** (`transformer/` + `catnip_rs/src/parser/`) - Parse tree to IR
1. **Semantic Analysis** (`semantic/` + `catnip_rs/src/semantic/`) - IR optimization (10 passes + CFG/SSA at level >= 3)
1. **Execution** (`executor.py`) - VM bytecode (default) or AST interpretation

### Execution Modes

- **VM Bytecode** (default) - IR compiled to bytecode, stack-based VM with NaN-boxing, JIT via Cranelift
- **AST Interpretation** (`-x ast`) - Direct AST interpretation via Registry

### Key Components

- **Transformer** (`transformer/base.py` + `catnip_rs/src/parser/`) - Tree-sitter to IR
- **Semantic** (`catnip_rs/src/semantic/`) - Analyzer + 10 optimization passes, opcodes in `opcode.py`
- **CFG/SSA** (`catnip_rs/src/cfg/`) - SSA construction (Braun et al. 2013), 4 inter-block passes
- **Core** (`catnip_rs/src/core/`) - Scope, Op, Broadcasting, Registry (57 ops, TCO trampoline)
- **VM** (`catnip_rs/src/vm/`) - Stack-based bytecode, NaN-boxing (7 tags), frame pooling
- **JIT** (`catnip_rs/src/jit/`) - Trace-based JIT via Cranelift, persistent cache
- **Cache** (`catnip_rs/src/cache/`) - Memory + disk cache, memoization
- **Debug** (`catnip_rs/src/debug/`) - Breakpoints, stepping, MCP tools

### Implementation Details

- `CONTROL_FLOW_OPS` (in `semantic/opcode.py`) have arguments passed **unevaluated**
- Functions/lambdas use a **trampoline loop** for TCO (O(1) stack space)
- `@pure` functions are tracked for broadcast optimization

### MCP Server (`catnip_mcp_py/`)

10 tools: `parse_catnip`, `eval_catnip`, `check_syntax`, `format_code`, `debug_start`, `debug_continue`, `debug_step`,
`debug_inspect`, `debug_eval`, `debug_breakpoint`.

Resources: `catnip://examples/{topic}`, `catnip://codex/{category}/{module}`.
