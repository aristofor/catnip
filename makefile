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
#
# The `#--- DEV-ONLY-START/END ---` markers delimit blocks that
# dev/release/__main__.py strips to produce the distributed (public) makefile.
# A few targets (compile, install, wheel, format) are therefore defined twice:
# a richer DEV-ONLY version and a public version carrying only the prerequisites
# the shipped repo needs. Both coexist in this dev makefile -- make merges the
# prerequisites and keeps the single recipe.

# Paths
PY_SOURCES      := catnip
VENV_PYTHON  := .venv/bin/python3
VENV_PYTEST  := .venv/bin/pytest
VENV_BIN     := .venv/bin
# tree-sitter CLI: pinned to match the `tree-sitter` crate (workspace Cargo.toml).
# Keep both in lockstep -- a CLI/crate drift regenerates the vendored runtime
# (catnip_grammar/src/tree_sitter/*.h) on every grammar-deps, polluting the diff.
TREE_SITTER_VERSION := 0.26.9

# Rust extension: real file target for dependency tracking
EXT_SUFFIX := $(shell python3 -c "import sysconfig; print(sysconfig.get_config_var('EXT_SUFFIX'))")
RS_EXT     := catnip/_rs$(EXT_SUFFIX)
RS_SOURCES := $(shell fdfind -p catnip_rs/src -e .rs 2>/dev/null) $(shell fdfind -p catnip_tools/src -e .rs 2>/dev/null)
# Profil de l'invocation courante, aligné sur CATNIP_DEV comme setup.py.
BUILD_PROFILE := $(if $(filter 1,$(CATNIP_DEV)),fastdev,release)
# Stamp du dernier profil ayant bâti l'extension, écrit par setup.py (point unique :
# couvre make compile, pip install -e, reinstall-lang, wheels). Les libs natives s'y
# alignent hors compile/dev : `make dev` puis `make test` ne rebâtit pas en release.
BUILD_PROFILE_STAMP := catnip/.build-profile

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

# Manual (re)generation: always regenerates, matching the check-opcodes hint and
# the documented `make gen-opcodes`. The file rule above handles build-time deps
# (lazy, timestamp-driven); this target forces a run on explicit invocation.
.PHONY: gen-opcodes
gen-opcodes:
	python catnip_rs/gen_opcodes.py

# Builtin names generation (context.py -> linter.rs)
gen-builtins: catnip/context.py catnip_core/src/constants.rs catnip_tools/gen_builtins.py
	python catnip_tools/gen_builtins.py

# Stdlib registry generation (spec.toml -> loader + resolve + setup + workspace)
gen-stdlib-registry: catnip_libs/*/spec.toml catnip_tools/gen_stdlib_registry.py
	python catnip_tools/gen_stdlib_registry.py
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
# Profil : l'override target-specific NATIVE_PROFILE (posé par `compile`, qui
# reconstruit l'extension) prime — déterministe, sans dépendre de l'ordre vs
# l'écriture du stamp. Sinon le stamp de la dernière build (cibles `test`, qui
# réutilisent l'extension présente). Sinon l'invocation courante.
build-native-libs:
	@profile="$(NATIVE_PROFILE)"; \
	[ -n "$$profile" ] || profile=$$(cat $(BUILD_PROFILE_STAMP) 2>/dev/null || echo $(BUILD_PROFILE)); \
	for lib in $(NATIVE_STDLIB_LIBS); do \
		cargo build --profile $$profile -p catnip-$$lib && \
		cp target/$$profile/libcatnip_$$lib.$(NATIVE_LIB_EXT) catnip/libcatnip_$$lib.$(NATIVE_LIB_EXT); \
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

# First-time complete setup (venv + deps + compile + install)
setup-dev:
	@echo "Setting up development environment (no compilation)..."
	@echo "Checking system dependencies..."
	@missing=""; \
	if ! command -v m4 >/dev/null 2>&1; then \
		missing="$$missing m4"; \
	fi; \
	if ! pkg-config --exists gmp 2>/dev/null && ! test -f /usr/include/gmp.h; then \
		missing="$$missing libgmp-dev"; \
	fi; \
	if [ -n "$$missing" ]; then \
		echo "Error: missing system dependencies:$$missing"; \
		echo "  Ubuntu/Debian: sudo apt install$$missing"; \
		echo "  Fedora/RHEL:   sudo dnf install$$(echo $$missing | sed 's/libgmp-dev/gmp-devel/')"; \
		echo "  Arch:          sudo pacman -S$$(echo $$missing | sed 's/libgmp-dev/gmp/')"; \
		echo "  macOS:         brew install$$(echo $$missing | sed 's/libgmp-dev/gmp/')"; \
		exit 1; \
	else \
		echo "System dependencies OK"; \
	fi
	@if [ ! -d .venv ]; then \
		echo "Creating virtual environment..."; \
		uv venv --prompt Catnip .venv; \
	else \
		echo "Virtual environment already exists"; \
	fi
	uv sync --no-config --no-install-project
	@echo "Installing tree-sitter-cli $(TREE_SITTER_VERSION)..."
	@if tree-sitter --version 2>/dev/null | grep -qx "tree-sitter $(TREE_SITTER_VERSION)"; then \
		echo "tree-sitter-cli $(TREE_SITTER_VERSION) already installed"; \
	else \
		cargo install tree-sitter-cli --version $(TREE_SITTER_VERSION) --force --quiet; \
	fi
	@echo "Development environment ready (without compilation)"
	@echo "  Next steps:"
	@echo "    make compile      # Compile Rust extension"
	@echo "    make install      # Install DSL package"
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

# Force rebuild Python package (use after modifying Rust code)
reinstall-lang: catnip/semantic/opcode.py
	uv pip install -e . --no-build-isolation -Ceditable.rebuild=true
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

# Stdlib libs tests (Rust units + .cat integration)
test-libs:
	@echo "=== Stdlib libs tests ==="
	$(MAKE) -C catnip_libs test

# Full test suite (before commit/PR)
test-all: check-opcodes check-builtins check-stdlib-registry $(RS_EXT) build-native-libs
	@echo "=== Rust lint (clippy -D warnings) ==="
	$(MAKE) lint-rust
	@echo "=== Codex lint (deep, strict) ==="
	$(MAKE) lint-codex
	@echo "=== Rust tests (unit + integration) ==="
	$(MAKE) test-rust
	@echo "=== Python tests (VM mode) ==="
	CATNIP_CACHE=off CATNIP_EXECUTOR=vm $(VENV_PYTEST) -n auto tests/
	@echo "=== Python tests (AST mode) ==="
	CATNIP_CACHE=off CATNIP_EXECUTOR=ast $(VENV_PYTEST) -n auto tests/
	@echo "=== Stdlib libs tests ==="
	$(MAKE) -C catnip_libs test
	@echo "ALL TESTS PASSED"

# Fast dev loop tests
test-quick: check-opcodes check-builtins check-stdlib-registry $(RS_EXT) build-native-libs
	@echo "=== Quick tests ==="
	cd catnip_rs && cargo test --lib --features embedded -q -- --test-threads=1
	cd catnip_rs && cargo test --test '*' --features embedded -q
	CATNIP_CACHE=off $(VENV_PYTEST) tests/language/ -q
	@echo "Quick tests passed"
# Python tests (default mode: VM)
test: $(RS_EXT) build-native-libs
	CATNIP_CACHE=off $(VENV_PYTEST) tests/

# Rust tests (all) = unit (test-rust-fast: pure-embedded lib) + integration
# (test-rust-integration: tests/* with ast-executor). Delegating keeps a single
# source of truth per category and makes it explicit that integration runs.
# ast-executor is required there: the integration tests spawn the standalone
# binary, which registers its static _rs as catnip._rs and so must export the
# full API (e.g. Registry) the catnip package imports.
test-rust: test-rust-fast test-rust-integration

# Rust tests (unit only, fast)
test-rust-fast:
	cd catnip_rs && cargo test --lib --no-default-features --features embedded -- --test-threads=1
	cargo test -p catnip_repl --lib --no-default-features --features embedded

# Rust tests (integration only)
# ast-executor matches the shipped binary (build-run): the standalone binary
# registers its static _rs as catnip._rs, so it must export the full API
# (e.g. Registry) the catnip package imports.
test-rust-integration:
	cd catnip_rs && cargo test --test '*' --no-default-features --features embedded,ast-executor

# Python tests in VM mode
test-vm: $(RS_EXT) build-native-libs
	CATNIP_CACHE=off CATNIP_EXECUTOR=vm $(VENV_PYTEST) -n auto tests/

# Python tests in AST mode
test-ast: $(RS_EXT) build-native-libs
	CATNIP_CACHE=off CATNIP_EXECUTOR=ast $(VENV_PYTEST) -n auto tests/

# Application integration tests
test-apps: $(RS_EXT)
	CATNIP_CACHE=off $(VENV_PYTEST) -v tests/apps/

# Pandas integration tests
test-pandas: $(RS_EXT)
	CATNIP_CACHE=off $(VENV_PYTEST) -v tests/apps/test_pandas.py

# Standalone binary integration tests (release build)
test-run:
	@echo "Running integration tests (release build)..."
	cargo test -p catnip_rs --test '*' --no-default-features --features embedded,ast-executor --release \
		--target-dir target/run

# Standalone binary integration tests (dev build, faster)
test-run-fast:
	@echo "Running integration tests (dev build)..."
	cargo test -p catnip_rs --test '*' --no-default-features --features embedded,ast-executor \
		--target-dir target/run

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
.PHONY: format-rs format-py format check lint-rust lint-codex proof proof-clean proof-scan

format-rs:
	cd catnip_rs && cargo fmt --all

format-py:
	uvx --python 3.14 black $(PY_SOURCES) catnip_mcp/ tests/

format: format-rs format-py

check:
	uvx ruff check $(PY_SOURCES) catnip_tools setup.py tests/ $(wildcard dev/)

# Codex examples gate (deep lint, warnings denied). Self-contained on purpose:
# only touches codex/, so it lifts out as-is if the codex becomes its own project.
lint-codex: $(RS_EXT)
	$(VENV_PYTHON) -m catnip lint --deep --strict codex/

# Rust lint gate (clippy, all warnings denied). PyO3 crates need the embedded
# feature (extension-module fails to link without Python symbols).
lint-rust:
	cargo clippy -p catnip_core -p catnip_vm -p catnip_tools -p catnip_grammar -p catnip_lsp -p catnip_mcp --all-targets -- -D warnings
	cd catnip_rs && cargo clippy --no-default-features --features embedded --all-targets -- -D warnings
	cd catnip_repl && cargo clippy --no-default-features --features embedded --all-targets -- -D warnings

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

check-opcodes:
	@cp catnip/semantic/opcode.py /tmp/_catnip_opcode_before.py
	@python catnip_rs/gen_opcodes.py > /dev/null
	@diff -q catnip/semantic/opcode.py /tmp/_catnip_opcode_before.py > /dev/null || \
		(echo "Generated Python opcodes are stale! Run: make gen-opcodes" && exit 1)

check-builtins:
	@cp catnip_tools/src/linter/semantic.rs /tmp/_catnip_linter_before.rs
	@cp catnip_tools/src/lint_cfg/mod.rs /tmp/_catnip_lint_cfg_before.rs
	@cp catnip_repl/src/completer.rs /tmp/_catnip_completer_before.rs
	@cp catnip_core/src/constants.rs /tmp/_catnip_constants_before.rs
	@python catnip_tools/gen_builtins.py > /dev/null
	@diff -q catnip_tools/src/linter/semantic.rs /tmp/_catnip_linter_before.rs > /dev/null && \
		diff -q catnip_tools/src/lint_cfg/mod.rs /tmp/_catnip_lint_cfg_before.rs > /dev/null && \
		diff -q catnip_repl/src/completer.rs /tmp/_catnip_completer_before.rs > /dev/null && \
		diff -q catnip_core/src/constants.rs /tmp/_catnip_constants_before.rs > /dev/null || \
		(echo "Generated builtins are stale! Run: make gen-builtins" && exit 1)

check-stdlib-registry:
	@cp catnip/loader.py /tmp/_catnip_loader_before.py
	@cp catnip_core/src/loader/resolve.rs /tmp/_catnip_resolve_before.rs
	@cp setup.py /tmp/_catnip_setup_before.py
	@cp Cargo.toml /tmp/_catnip_cargo_before.toml
	@python catnip_tools/gen_stdlib_registry.py > /dev/null
	@diff -q catnip/loader.py /tmp/_catnip_loader_before.py > /dev/null && \
		diff -q catnip_core/src/loader/resolve.rs /tmp/_catnip_resolve_before.rs > /dev/null && \
		diff -q setup.py /tmp/_catnip_setup_before.py > /dev/null && \
		diff -q Cargo.toml /tmp/_catnip_cargo_before.toml > /dev/null || \
		(echo "Generated stdlib registry is stale! Run: make gen-stdlib-registry" && exit 1)
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

