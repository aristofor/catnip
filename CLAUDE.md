# Catnip

Catnip is a language interpreter with a hybrid Rust + Python architecture, using Tree-sitter for parsing. Hot paths in Rust (PyO3), high-level API in Python.

## Build from Source

```bash
# Prerequisites: Rust toolchain, Python 3.11+, uv

# Create virtual environment
uv venv .venv && source .venv/bin/activate

# Install build dependencies
uv pip install -U pip setuptools setuptools-rust wheel pytest click xxhash

# Compile Rust extension
make compile

# Install package
make install-lang

# Run tests
make test
```

## Project Structure

- `catnip/` - Python source code (main package)
- `catnip_rs/` - Rust extension (PyO3) - parser, semantic analysis, VM, registry
- `catnip_grammar/` - Tree-sitter grammar definition
- `catnip_repl/` - Interactive REPL (Rust/ratatui)
- `catnip_tools/` - Formatter and linter (Rust)
- `docs/` - Documentation
- `tests/` - Test suite

## Running Catnip

```bash
catnip                    # Interactive REPL
catnip script.cat         # Run a script file
catnip -c "expression"    # Evaluate single expression
echo "2 + 3" | catnip     # Read from stdin

# Options
catnip -o tco script.cat        # Enable TCO
catnip -o jit script.cat        # Enable JIT
catnip -x ast script.cat        # Use AST interpreter (default: VM)
catnip -v script.cat             # Verbose mode
```

## Testing

```bash
make test                        # Python tests (VM mode, default)
pytest tests/language/ -q        # Quick language tests
```

## Distribution

```bash
make sdist     # Build source distribution
make wheel     # Build wheel
make dist      # Build optimized wheels (cibuildwheel)
```
