PYTHON ?= python
CATNIP ?= catnip
PY_INCLUDES := $(shell $(PYTHON) -c "import sysconfig; print(sysconfig.get_path('include'))")
PY_EXT_SUFFIX := $(shell $(PYTHON) -c "import sysconfig; print(sysconfig.get_config_var('EXT_SUFFIX'))")

# Module directories (auto-detected: any dir with spec.toml)
MODULES := $(patsubst %/spec.toml,%,$(wildcard */spec.toml))

# Read backends.rust from a module's spec.toml (stripped trailing /)
rust_backend = $(shell $(PYTHON) -c "import tomllib,sys; d=tomllib.load(open('$(1)/spec.toml','rb')); print(d.get('backends',{}).get('rust','rust/').rstrip('/'))")

.PHONY: all clean list test test-rust test-cat $(MODULES)

all: $(MODULES)

clean:
	@for mod in $(MODULES); do \
		rb=$$($(PYTHON) -c "import tomllib; d=tomllib.load(open('$$mod/spec.toml','rb')); print(d.get('backends',{}).get('rust','rust/').rstrip('/'))"); \
		if [ -d "$$mod/$$rb" ]; then \
			cargo clean --manifest-path "$$mod/$$rb/Cargo.toml" 2>/dev/null || true; \
		fi; \
	done

list:
	@echo "Modules: $(MODULES)"

# Per-module build: compile available backends
define MODULE_RULES
$(1):
	@echo "Building module: $(1)"
	@rb=$$($(PYTHON) -c "import tomllib; d=tomllib.load(open('$(1)/spec.toml','rb')); print(d.get('backends',{}).get('rust','rust/').rstrip('/'))"); \
	if [ -d "$(1)/$$rb" ]; then \
		cargo build --manifest-path "$(1)/$$rb/Cargo.toml" --release; \
	fi
endef

$(foreach mod,$(MODULES),$(eval $(call MODULE_RULES,$(mod))))

# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

# Rust unit tests (--no-default-features --features embedded only for PyO3 crates)
test-rust:
	@failed=0; \
	for mod in $(MODULES); do \
		rb=$$($(PYTHON) -c "import tomllib; d=tomllib.load(open('$$mod/spec.toml','rb')); print(d.get('backends',{}).get('rust','rust/').rstrip('/'))"); \
		if [ -d "$$mod/$$rb" ]; then \
			echo "--- rust: $$mod ---"; \
			if grep -q '\[features\]' "$$mod/$$rb/Cargo.toml" 2>/dev/null; then \
				cargo test --manifest-path "$$mod/$$rb/Cargo.toml" \
					--lib --no-default-features --features embedded 2>&1 || failed=1; \
			else \
				cargo test --manifest-path "$$mod/$$rb/Cargo.toml" \
					--lib 2>&1 || failed=1; \
			fi; \
		fi; \
	done; \
	exit $$failed

# Integration tests (.cat scripts)
test-cat:
	@passed=0; failed=0; errors=""; \
	for mod in $(MODULES); do \
		if [ -d "$$mod/tests" ]; then \
			for f in $$mod/tests/*.cat; do \
				[ -f "$$f" ] || continue; \
				name=$$(basename "$$f"); \
				if $(CATNIP) "$$f" >/dev/null 2>&1; then \
					passed=$$((passed + 1)); \
				else \
					failed=$$((failed + 1)); \
					errors="$$errors  FAIL $$mod/tests/$$name\n"; \
				fi; \
			done; \
		fi; \
	done; \
	echo "--- cat: $$passed passed, $$failed failed ---"; \
	if [ $$failed -gt 0 ]; then \
		printf "$$errors"; \
		exit 1; \
	fi

# Run all tests
test: test-rust test-cat
