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

build-libs:
	$(MAKE) -C catnip_libs all

compile: $(RS_EXT)

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

# Install Python package + MCP dependencies (skip if already installed)
install-lang: build-deps $(RS_EXT)
	@$(VENV_PYTHON) -c "from importlib.metadata import version; version('catnip-lang')" 2>/dev/null || \
		uv pip install -e "." --no-build-isolation

# Install and register MCP server (project .mcp.json + global config if exists)
install-mcp: install-lang
	@pkill catnip-mcp || true
	@echo "$$_MCP_REGISTER" | $(VENV_PYTHON) - .mcp.json catnip $(CURDIR)/.venv/bin/catnip-mcp
	@test -f "$(CLAUDE_GLOBAL_CFG)" && \
		echo "$$_MCP_REGISTER" | $(VENV_PYTHON) - "$(CLAUDE_GLOBAL_CFG)" catnip $(CURDIR)/.venv/bin/catnip-mcp || true
	@echo "MCP server 'catnip' registered"

install: install-lang

#══════════════════════════════════════════════════════════════════════════════
# Testing
#══════════════════════════════════════════════════════════════════════════════

.PHONY: setup-test test

# Install package with test dependencies (requires active venv)
setup-test: $(RS_EXT)
	uv pip install -e ".[test]" --no-build-isolation

# Python tests (default mode: VM)
test: $(RS_EXT)
	CATNIP_CACHE=off $(VENV_PYTEST) tests/

#══════════════════════════════════════════════════════════════════════════════
# Example data
#══════════════════════════════════════════════════════════════════════════════
.PHONY: clean-examples

clean-examples:
	find docs/examples -type d \( -name cache -o -name output \) -exec rm -rf {} +
	find docs/codex -type d \( -name cache -o -name output \) -exec rm -rf {} +

#══════════════════════════════════════════════════════════════════════════════
# Quality & Tools
#══════════════════════════════════════════════════════════════════════════════
.PHONY: format-rs format-py format check proof proof-clean proof-scan

format-rs:
	cd catnip_rs && cargo fmt --all

format-py:
	uvx --python 3.13 black $(PY_SOURCES) catnip_mcp/ tests/

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

