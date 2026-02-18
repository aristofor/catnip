<!-- FILE: catnip_rs/README.md -->

# Catnip Rust Implementation

This directory contains the Rust implementation of Catnip's performance-critical components using PyO3.

## Architecture

```
catnip_rs/
├── src/
│   ├── core/              # Core runtime (Scope, Op, Registry)
│   ├── semantic/          # Semantic analysis (Analyzer, Optimizer passes)
│   ├── ir/                # IR opcodes (generated from Python)
│   ├── vm/                # VM opcodes and bytecode engine
│   └── jit/               # JIT compilation (trace-based)
├── gen_opcodes.py         # Generator: Python OpCode → Rust enum
└── Cargo.toml             # Rust project configuration
```

## OpCode Synchronization

### When to regenerate

Run `gen_opcodes.py` whenever you modify `catnip/semantic/opcode.py`:

```bash
# After modifying catnip/semantic/opcode.py
python catnip_rs/gen_opcodes.py

# Verify generation
git diff catnip_rs/src/ir/opcode.rs
```

### What gets generated

- `catnip_rs/src/ir/opcode.rs` - `IROpCode` enum with values matching Python
- `catnip_rs/src/vm/opcode.rs` - `VMOpCode` enum for bytecode VM

### Testing synchronization

Run Rust tests to verify OpCode values match Python:

```bash
cd catnip_rs
cargo test --lib --no-default-features --features embedded analyzer::tests::test_opcode_values_match_python
```

This test will fail if opcodes are out of sync.

### Common mistakes

**❌ DO NOT hardcode opcode numbers:**

```rust
match ident_int {
    53 => self.visit_pragma(py, node),  // BAD: magic number
    // ...
}
```

**✓ DO use IROpCode enum:**

```rust
use crate::ir::opcode::IROpCode;

match ident_int {
    x if x == IROpCode::Pragma as i32 => self.visit_pragma(py, node),  // GOOD
    // ...
}
```

## Running Tests

### Rust tests (fast, ~5-10s)

```bash
cd catnip_rs

# All tests
cargo test --no-default-features --features embedded

# Specific module
cargo test --lib --no-default-features --features embedded semantic::analyzer

# With output
cargo test --lib --no-default-features --features embedded -- --nocapture
```

**Note**: Use `--no-default-features --features embedded` to link with Python for tests.

### Python tests (full validation, ~25s)

```bash
cd ..
pip install -e .
pytest tests/
```

## Development Workflow

1. **Modify Python OpCode** (if adding new opcodes)

   ```bash
   vim catnip/semantic/opcode.py
   python catnip_rs/gen_opcodes.py
   ```

1. **Modify Rust code**

   ```bash
   vim catnip_rs/src/semantic/analyzer.rs
   ```

1. **Test Rust** (fast feedback)

   ```bash
   cd catnip_rs
   cargo test --lib --no-default-features --features embedded
   ```

1. **Build Python package**

   ```bash
   cd ..
   pip install -e .
   ```

1. **Test Python** (full validation)

   ```bash
   pytest tests/
   ```

## Standalone Binaries

Catnip provides standalone Rust binaries that embed Python:

### catnip-standalone

Standalone runtime that loads Python on demand:

```bash
# Build
cargo build --bin catnip-standalone --no-default-features --features embedded --release

# Usage
./target/release/catnip-standalone script.cat
./target/release/catnip-standalone -c "x = 10; x * 2"
echo "2 + 3" | ./target/release/catnip-standalone --stdin

# Show runtime info
./target/release/catnip-standalone info
```

**Important**: The `embedded` feature is incompatible with `extension-module` (default). Always use `--no-default-features --features embedded` when building standalone binaries.

### Installation

```bash
cargo install --path . --bin catnip-standalone --no-default-features --features embedded
```

## CI/CD Integration

```yaml
# .github/workflows/test.yml
- name: Generate Rust opcodes
  run: python catnip_rs/gen_opcodes.py
  
- name: Check for changes (should be none)
  run: git diff --exit-code catnip_rs/src/ir/opcode.rs

- name: Rust tests (fast feedback)
  run: |
    cd catnip_rs
    cargo test --no-default-features --features embedded
  
- name: Python tests (full validation)
  run: |
    pip install -e .
    pytest tests/
```

## Performance Profiling

```bash
# Profile Rust code
cargo build --release
perf record -g target/release/examples/benchmark
perf report

# Profile Python integration
pip install -e .
python -m cProfile -o output.prof script.py
snakeviz output.prof
```

## Debugging

### Rust panics

```bash
RUST_BACKTRACE=1 pytest tests/language/test_xyz.py -xvs
```

### GDB/LLDB

```bash
rust-gdb --args python -m pytest tests/language/test_xyz.py::test_specific
```

### Print debugging

```rust
eprintln!("DEBUG: value = {:?}", value);  // Always visible
```

## Resources

- [PyO3 Guide](https://pyo3.rs/)
- [Rust Performance Book](https://nnethercote.github.io/perf-book/)
