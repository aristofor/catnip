PYTHON ?= python
PY_INCLUDES := $(shell $(PYTHON) -c "import sysconfig; print(sysconfig.get_path('include'))")
PY_EXT_SUFFIX := $(shell $(PYTHON) -c "import sysconfig; print(sysconfig.get_config_var('EXT_SUFFIX'))")

# Module directories (auto-detected: any dir with spec.toml)
MODULES := $(patsubst %/spec.toml,%,$(wildcard */spec.toml))

.PHONY: all clean list $(MODULES)

all: $(MODULES)

clean:
	@for mod in $(MODULES); do \
		if [ -d "$$mod/rust" ]; then \
			cargo clean --manifest-path "$$mod/rust/Cargo.toml" 2>/dev/null || true; \
		fi; \
	done

list:
	@echo "Modules: $(MODULES)"

# Per-module build: compile available backends
define MODULE_RULES
$(1):
	@echo "Building module: $(1)"
	@if [ -d "$(1)/rust" ]; then \
		cargo build --manifest-path "$(1)/rust/Cargo.toml" --release; \
	fi
endef

$(foreach mod,$(MODULES),$(eval $(call MODULE_RULES,$(mod))))
