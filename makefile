# Catnip Makefile
#
# Usage patterns:
#   make setup           # First-time setup (venv + deps + compile + node)
#   make install         # Install DSL (Python package + MCP)
#   make install-bins    # Install Rust binaries (run + repl)
#   make install-all     # Install everything (DSL + bins)
#   make setup-test      # Install package + test deps (needs active venv)
#   make test            # Run Python tests
#   make test-all        # Run all tests (Rust + Python + run)
#   make compile         # Rebuild Rust extension

# Paths
PY_SOURCES      := catnip
VENV_PYTHON  := .venv/bin/python3
VENV_PYTEST  := .venv/bin/pytest
VENV_BIN     := .venv/bin

# Rust extension: real file target for dependency tracking
EXT_SUFFIX := $(shell python3 -c "import sysconfig; print(sysconfig.get_config_var('EXT_SUFFIX'))")
RS_EXT     := catnip/_rs$(EXT_SUFFIX)
RS_SOURCES := $(shell fdfind -p catnip_rs/src -e .rs 2>/dev/null) $(shell fdfind -p catnip_tools/src -e .rs 2>/dev/null)

#══════════════════════════════════════════════════════════════════════════════
# Core targets
#══════════════════════════════════════════════════════════════════════════════

.PHONY: all clean venv build build-deps

all: $(RS_EXT)

build-deps:
	@python3 -c "import setuptools" 2>/dev/null || \
		uv sync --no-config --no-install-project

venv:
	uv venv --prompt Catnip .venv
	@echo ""
	@echo "Next: make install"

clean:
	find "$(PY_SOURCES)" -type f \
	    \( -name '*.so' -o -name '*.c' -o -name '*.cpp' -o -name '*.pyd' \) \
	    -exec rm -f {} +
	rm -rf build dist *.egg-info
	rm -rf .ruff_cache .pytest_cache
	rm -rf target
	$(MAKE) -C catnip_libs clean
	find . -name __pycache__ -type d -exec rm -rf {} +

build: clean compile

#══════════════════════════════════════════════════════════════════════════════
# Compilation
#══════════════════════════════════════════════════════════════════════════════

# Real file target: only rebuild .so when Rust sources actually changed
$(RS_EXT): $(RS_SOURCES) catnip_rs/Cargo.toml catnip_rs/Cargo.lock catnip_grammar/src/parser.c
	@$(VENV_PYTHON) setup.py build_ext --inplace

# Standalone Rust binaries (catnip + catnip-repl + catnip-lsp + catnip-mcp).
# Contributor-facing: kept in the distribution makefile so the public repo can
# build the embedded-Python binaries.
.PHONY: build-run build-repl build-lsp build-mcp build-bins
BINS_TARGET := target/bins
RUN_BIN     := $(BINS_TARGET)/release/catnip
REPL_BIN    := $(BINS_TARGET)/release/catnip-repl
LSP_BIN     := $(BINS_TARGET)/release/catnip-lsp
MCP_BIN     := $(BINS_TARGET)/release/catnip-mcp

# Standalone binaries with embedded Python (catnip, catnip-repl) must link the
# venv's libpython, not whatever python3-config the build.rs finds first in PATH.
# We pin PyO3 to the venv interpreter, put its base bin (python3-config) ahead in
# PATH, and add an rpath to its libdir so the binary finds libpython at runtime.
# The rpath goes through `cargo rustc -- -C link-arg` so it *adds* to the
# per-target rustflags in .cargo/config instead of replacing them (RUSTFLAGS env
# would clobber the platform-specific linker flags defined there).
# Lazy (=) so non-bin targets don't pay the python startup cost.
PY_BASE_BIN  = $(shell $(VENV_PYTHON) -c "import sys, os; print(os.path.join(sys.base_prefix, 'bin'))")
PY_LIBDIR    = $(shell $(VENV_PYTHON) -c "import sysconfig; print(sysconfig.get_config_var('LIBDIR'))")
EMBED_ENV    = PATH="$(PY_BASE_BIN):$$PATH" PYO3_PYTHON="$(abspath $(VENV_PYTHON))"
EMBED_RPATH  = -C link-arg=-Wl,-rpath,$(PY_LIBDIR)

# Depend on makefile too: a change to the build recipe (linker flags, embedded
# Python) must rebuild the binaries, otherwise a stale one linked against the
# wrong libpython stays in place since the Rust sources are unchanged.
$(RUN_BIN): $(RS_SOURCES) catnip_rs/Cargo.toml makefile
	$(EMBED_ENV) cargo rustc --release -p catnip_rs --bin catnip --features embedded --target-dir $(BINS_TARGET) -- $(EMBED_RPATH)

$(REPL_BIN): $(RS_SOURCES) catnip_rs/Cargo.toml makefile
	$(EMBED_ENV) cargo rustc --release -p catnip_repl --bin catnip-repl --features repl-standalone --target-dir $(BINS_TARGET) -- $(EMBED_RPATH)

$(LSP_BIN): $(RS_SOURCES) catnip_lsp/Cargo.toml
	cargo build --release -p catnip_lsp --bin catnip-lsp --target-dir $(BINS_TARGET)

$(MCP_BIN): $(RS_SOURCES) catnip_mcp/Cargo.toml
	cargo build --release -p catnip_mcp --bin catnip-mcp --target-dir $(BINS_TARGET)

build-run: $(RUN_BIN)
build-repl: $(REPL_BIN)
build-lsp: $(LSP_BIN)
build-mcp: $(MCP_BIN)

build-bins: build-run build-repl build-lsp build-mcp
	@echo "All Rust binaries built"

build-libs:
	$(MAKE) -C catnip_libs all

# Native (PyO3-less) stdlib plugins are catnip_vm cdylib plugins, discovered at
# runtime next to the extension. setup.py builds them for wheels/installs; this
# target does the same for the dev build (make compile). Source of truth for the
# list: catnip_libs/*/spec.toml with backends.pyo3 = false.
NATIVE_STDLIB_LIBS := http
# .so on Linux, .dylib on macOS (matches catnip_vm::plugin::native_suffix).
NATIVE_LIB_EXT := $(if $(filter Darwin,$(shell uname -s)),dylib,so)

.PHONY: build-native-libs
build-native-libs:
	@for lib in $(NATIVE_STDLIB_LIBS); do \
		cargo build --release -p catnip-$$lib && \
		cp target/release/libcatnip_$$lib.$(NATIVE_LIB_EXT) catnip/libcatnip_$$lib.$(NATIVE_LIB_EXT); \
	done

compile: $(RS_EXT) build-native-libs

# Helper: register an MCP server in a JSON config file
# Usage: echo "$$_MCP_REGISTER" | $(VENV_PYTHON) - <file> <name> <command> [module] [cwd]
define _MCP_REGISTER
import json, pathlib, sys
target, name, cmd = sys.argv[1], sys.argv[2], sys.argv[3]
module = sys.argv[4] if len(sys.argv) > 4 else None
cwd = sys.argv[5] if len(sys.argv) > 5 else None
f = pathlib.Path(target).expanduser()
d = json.loads(f.read_text()) if f.exists() else {}
e = {'command': cmd}
if module:
    e['args'] = ['-m', module]
if cwd:
    e['cwd'] = cwd
d.setdefault('mcpServers', {})[name] = e
f.write_text(json.dumps(d, indent=2) + chr(10))
endef
export _MCP_REGISTER

CLAUDE_GLOBAL_CFG := $(HOME)/.config/claude/config.json

#══════════════════════════════════════════════════════════════════════════════
# Installation
#══════════════════════════════════════════════════════════════════════════════

.PHONY: setup install install-lang install-mcp

setup:
	@if [ ! -d .venv ]; then \
		echo "Creating virtual environment..."; \
		$(MAKE) venv; \
	fi
	uv sync --no-config --no-install-project
	$(MAKE) install
	@echo ""
	@echo "Setup complete!"
	@echo "  Activate venv: source .venv/bin/activate"
	@echo "  Run tests: make test"
	@echo "  Start REPL: catnip"

.PHONY: install-all install-bins install-run install-repl install-mcp-bin install-lsp

# Install Rust binaries (catnip + repl + lsp + mcp)
install-bins: install-run install-repl
	$(MAKE) install-mcp-bin || echo "Warning: catnip-mcp build failed, skipping"
	$(MAKE) install-lsp || echo "Warning: catnip-lsp build failed, skipping"
	@echo "Rust binaries installed"

# Install everything (DSL + bins)
install-all: install install-bins
	@echo "All components installed"

# Install Python package + MCP dependencies (skip if already installed).
# The presence check runs from a neutral cwd: build_ext leaves a
# catnip_lang.egg-info in the source tree, which would otherwise shadow the
# metadata lookup and make the guard skip the real editable install.
install-lang: build-deps $(RS_EXT)
	@( cd /tmp && "$(abspath $(VENV_PYTHON))" -c "from importlib.metadata import version; version('catnip-lang')" ) 2>/dev/null || \
		uv pip install -e "." --no-build-isolation

# Install and register MCP server (project .mcp.json + global config if exists)
install-mcp: install-lang
	@pkill catnip-mcp || true
	@echo "$$_MCP_REGISTER" | $(VENV_PYTHON) - .mcp.json catnip $(CURDIR)/.venv/bin/catnip-mcp
	@test -f "$(CLAUDE_GLOBAL_CFG)" && \
		echo "$$_MCP_REGISTER" | $(VENV_PYTHON) - "$(CLAUDE_GLOBAL_CFG)" catnip $(CURDIR)/.venv/bin/catnip-mcp || true
	@echo "MCP server 'catnip' registered"

# Install catnip binary in venv
install-run: build-run
	@echo "Installing to $(VENV_BIN)/catnip..."
	cp $(RUN_BIN) $(VENV_BIN)/
	@echo "catnip installed in $(VENV_BIN)/"

# Install REPL binary in venv
install-repl: build-repl
	@echo "Installing to $(VENV_BIN)/catnip-repl..."
	cp $(REPL_BIN) $(VENV_BIN)/
	@echo "catnip-repl installed in $(VENV_BIN)/"
	@echo "Run with: catnip-repl"

# Install LSP binary in venv
install-lsp: build-lsp
	@echo "Installing to $(VENV_BIN)/catnip-lsp..."
	cp $(LSP_BIN) $(VENV_BIN)/
	@echo "catnip-lsp installed in $(VENV_BIN)/"

# Install MCP binary in venv
install-mcp-bin: build-mcp
	@echo "Installing to $(VENV_BIN)/catnip-mcp..."
	cp $(MCP_BIN) $(VENV_BIN)/
	@echo "catnip-mcp installed in $(VENV_BIN)/"

install: install-lang

#══════════════════════════════════════════════════════════════════════════════
# Testing
#══════════════════════════════════════════════════════════════════════════════

.PHONY: setup-test test

# Install package with test dependencies (requires active venv)
setup-test: $(RS_EXT)
	uv pip install -e ".[test]" --no-build-isolation

# Python tests (default mode: VM)
test: $(RS_EXT) build-native-libs
	CATNIP_CACHE=off $(VENV_PYTEST) tests/

#══════════════════════════════════════════════════════════════════════════════
# Example data
#══════════════════════════════════════════════════════════════════════════════
.PHONY: clean-examples

clean-examples:
	find docs/examples -type d \( -name cache -o -name output \) -exec rm -rf {} +
	find codex -type d \( -name cache -o -name output \) -exec rm -rf {} +

#══════════════════════════════════════════════════════════════════════════════
# Quality & Tools
#══════════════════════════════════════════════════════════════════════════════
.PHONY: format-rs format-py format check proof proof-clean proof-scan

format-rs:
	cd catnip_rs && cargo fmt --all

format-py:
	uvx --python 3.14 black $(PY_SOURCES) catnip_mcp/ tests/

format: format-rs format-py

check:
	uvx ruff check $(PY_SOURCES) tests/ dev/

proof:
	cd proof && coq_makefile -f _CoqProject -o Makefile.coq && $(MAKE) -f Makefile.coq
	@echo "All proofs verified."

proof-clean:
	cd proof && [ -f Makefile.coq ] && $(MAKE) -f Makefile.coq clean || true
	rm -f proof/Makefile proof/Makefile.conf proof/.Makefile.d
	rm -f proof/Makefile.coq proof/Makefile.coq.conf proof/.Makefile.coq.d
	find proof -name '.*.aux' -o -name '.*.cache' -o -name '*.glob' \
	    -o -name '*.vo' -o -name '*.vok' -o -name '*.vos' | xargs rm -f 2>/dev/null || true

proof-scan:
	@found=0; \
	: "pas de sortie de secours"; \
	echo "# Admitted / Abort"; \
	rg -n '^\s*(Proof\.\s*)?(admit\.\s*)?Admitted\.\s*$$' proof/**/*.v && found=1 || true; \
	rg -n '^\s*Abort\.\s*$$' proof/**/*.v && found=1 || true; \
	: "a assumer s'il en reste"; \
	echo "# Axiom / Parameter"; \
	rg -n '^\s*(Axiom|Parameter)\b' proof/**/*.v && found=1 || true; \
	: "pour rester constructif"; \
	echo "# Classical / Extensionality"; \
	rg -n '^\s*From\b.*\b(Classical|PropExtensionality|FunctionalExtensionality)\b' proof/**/*.v && found=1 || true; \
	if [ "$$found" -eq 0 ]; then \
		echo "No occurrences found"; \
	fi

.PHONY: ts-generate ts-test grammar-deps

ts-generate:
	@echo "Generating parser from grammar..."
	cd catnip_grammar && tree-sitter generate
	@echo "/* FILE: catnip_grammar/src/parser.c */" | cat - catnip_grammar/src/parser.c > catnip_grammar/src/parser.c.tmp && mv catnip_grammar/src/parser.c.tmp catnip_grammar/src/parser.c
	@echo "Generated parser (catnip_grammar/src/)"

ts-test:
	cd catnip_grammar && tree-sitter test

grammar-deps: ts-generate $(RS_EXT)
	$(VENV_PYTHON) catnip/tools/extract_grammar.py \
	    --update-lexer --json dist/grammar.json
	@echo "Updated: catnip/tools/pygments.py"
	@echo "Exported: dist/grammar.json"

#══════════════════════════════════════════════════════════════════════════════
# Distribution
#══════════════════════════════════════════════════════════════════════════════

.PHONY: sdist wheel dist clean-dist

clean-dist:
	rm -rf dist/*.whl

sdist: clean
	uv run python -m build --sdist

wheel: clean
	uv run python -m build --wheel

