# FILE: catnip/loader.py
"""
Module loader for importing Catnip, Python, and binary modules into Catnip's execution context.
"""

import importlib
import importlib.util
import os
import sys
from importlib import machinery
from pathlib import Path
from types import SimpleNamespace

from .colors import print_error, print_info, print_warning
from .exc import CatnipRuntimeError


class ModuleNamespace:
    """
    Wrapper object that provides namespace access to module functions.

    This allows Catnip scripts to call module.function() instead of
    polluting the global namespace.
    """

    def __init__(self, module, name=None):
        """
        Initialize a module namespace.

        :param module: The Python module object
        :param name: Optional name for the namespace (defaults to module.__name__)
        """
        self._module = module
        self._name = name or module.__name__
        self._functions = {}
        self._attributes = {}

        # Extract public functions, classes, and attributes
        for attr_name in dir(module):
            if not attr_name.startswith("_"):
                attr = getattr(module, attr_name)
                if callable(attr):
                    self._functions[attr_name] = attr
                else:
                    # Store non-callable attributes (constants, etc.)
                    self._attributes[attr_name] = attr

    def __getattr__(self, name):
        """Allow attribute access: namespace.function() or namespace.constant"""
        if name in self._functions:
            return self._functions[name]
        if name in self._attributes:
            return self._attributes[name]
        raise AttributeError(f"Module '{self._name}' has no attribute '{name}'")

    def __dir__(self):
        """List available functions and attributes for introspection."""
        return list(self._functions.keys()) + list(self._attributes.keys())

    def __repr__(self):
        items = sorted(list(self._functions.keys()) + list(self._attributes.keys()))
        items_str = ", ".join(items[:10])  # Show first 10 items
        if len(items) > 10:
            items_str += f", ... ({len(items)} total)"
        return f"<ModuleNamespace '{self._name}' [{items_str}]>"


class ModuleLoader:
    """Load Catnip and Python modules (including binary extensions) into the Catnip context."""

    CATNIP_SUFFIXES = {".cat"}
    PYTHON_SUFFIXES = set(machinery.SOURCE_SUFFIXES + machinery.BYTECODE_SUFFIXES)
    EXTENSION_SUFFIXES = set(machinery.EXTENSION_SUFFIXES)

    def __init__(self, context, verbose=False):
        """
        Initialize the module loader.

        :param context: Catnip execution context
        :param verbose: Show detailed loading information
        """
        self.context = context
        self.verbose = verbose
        self.loaded_modules = {}

    def _is_supported_module_file(self, module_path: Path) -> bool:
        """Return True if the path looks like a loadable Python/extension module."""
        suffix = module_path.suffix
        name = module_path.name

        if suffix in self.PYTHON_SUFFIXES or suffix in machinery.BYTECODE_SUFFIXES:
            return True

        # Extension modules can have long suffixes (cpython-311-x86_64-linux-gnu.so)
        if any(name.endswith(ext_suffix) for ext_suffix in self.EXTENSION_SUFFIXES):
            return True

        return False

    def load_python_module(self, module_path, module_name=None):
        """
        Load a Python module (source, bytecode, or binary extension) from a file path.

        :param module_path: Path to the module file (.py, .pyc, .so, .pyd, ...)
        :param module_name: Optional module name override
        :return: Loaded module object
        """
        module_path = Path(module_path).resolve()

        if not module_path.exists():
            raise FileNotFoundError(f"Module not found: {module_path}")

        if not self._is_supported_module_file(module_path):
            raise ValueError(f"Unsupported module type: {module_path}")

        # Generate module name from file path when not provided
        module_name = module_name or module_path.stem

        if self.verbose:
            print_info(f"Loading module: {module_path}")

        try:
            spec = importlib.util.spec_from_file_location(module_name, module_path)
            if spec is None or spec.loader is None:
                raise ImportError(f"Cannot load module from {module_path}")

            module = importlib.util.module_from_spec(spec)
            sys.modules[module_name] = module
            spec.loader.exec_module(module)

            self.loaded_modules[module_name] = module

            if self.verbose:
                print_info(f"Module '{module_name}' loaded successfully")

            return module

        except Exception as e:
            print_error(f"Failed to load module {module_path}: {e}")
            raise

    def load_catnip_module(self, module_path, module_name=None):
        """
        Load a Catnip module (.cat) and expose its exported symbols.

        :param module_path: Path to the Catnip file (.cat)
        :param module_name: Optional module name override
        :return: Namespace-like object containing the module exports
        """
        module_path = Path(module_path).resolve()

        if not module_path.exists():
            raise FileNotFoundError(f"Module not found: {module_path}")

        if module_path.suffix not in self.CATNIP_SUFFIXES:
            raise ValueError(f"Expected a Catnip file (.cat), got: {module_path}")

        if self.verbose:
            print_info(f"Loading Catnip module: {module_path}")

        from . import Catnip  # Local import to avoid circular dependency

        catnip = Catnip()
        baseline_globals = dict(catnip.context.globals)

        # Enrich META with module path before execution
        meta = catnip.context.globals.get("META")
        if meta is not None:
            meta.path = str(module_path)

        source = module_path.read_text()
        catnip.parse(source)
        catnip.execute()

        # Read exports: META.exports > __all__ > heuristic
        meta = catnip.context.globals.get("META")
        explicit_exports = catnip.context.globals.get("__all__")
        exports = {}

        # Try META.resolve_exports() first (validates type + entries)
        resolved = meta.resolve_exports(catnip.context.globals) if meta is not None else None

        if resolved is not None:
            exports = dict(resolved)
        elif isinstance(explicit_exports, (list, tuple, set)):
            for name in explicit_exports:
                if isinstance(name, str) and name in catnip.context.globals:
                    exports[name] = catnip.context.globals[name]
        else:
            for name, value in catnip.context.globals.items():
                if name.startswith("_") or name == "META":
                    continue
                baseline_value = baseline_globals.get(name, None)
                if name not in baseline_globals or baseline_value is not value:
                    exports[name] = value

        resolved_name = module_name or module_path.stem
        namespace = SimpleNamespace(**exports)
        namespace.__name__ = resolved_name

        self.loaded_modules[resolved_name] = namespace

        if self.verbose:
            print_info(f"Catnip module '{resolved_name}' loaded with {len(exports)} export(s)")

        return namespace

    def _search_dirs(self):
        """Return resolution directories: CWD then CATNIP_PATH entries."""
        dirs = [Path.cwd()]
        env = os.environ.get("CATNIP_PATH", "")
        if env:
            for entry in env.split(os.pathsep):
                p = Path(entry).resolve()
                if p.is_dir():
                    dirs.append(p)
        return dirs

    @classmethod
    def _is_explicit_path(cls, spec):
        """True when spec contains path separators, a leading dot, or a known file extension."""
        if '/' in spec or '\\' in spec or spec.startswith('.'):
            return True
        # Check for known file extensions (not dotted module names like PIL.Image)
        p = Path(spec)
        return p.suffix in cls.CATNIP_SUFFIXES | cls.PYTHON_SUFFIXES | cls.EXTENSION_SUFFIXES

    def _resolve_bare_name(self, name):
        """Search dirs for name with prioritised extensions. Returns (path, kind) or (None, 'importlib')."""
        extensions = [
            ('.cat', 'catnip'),
            ('.py', 'python'),
        ]
        # Add native extension suffixes (.so, .pyd, ...)
        for ext in machinery.EXTENSION_SUFFIXES:
            extensions.append((ext, 'python'))

        for d in self._search_dirs():
            for ext, kind in extensions:
                candidate = d / f"{name}{ext}"
                if candidate.is_file():
                    return candidate, kind
        return None, 'importlib'

    def _load_explicit_path(self, spec, caller_dir=None):
        """Load a module from an explicit file path.

        When caller_dir is set, relative paths (./  ../) are resolved
        against that directory instead of the process CWD.
        """
        target_path = Path(str(spec))

        # Resolve relative paths against caller's directory
        if caller_dir and not target_path.is_absolute():
            target_path = caller_dir / target_path

        # Infer extension when omitted: try .cat then .py
        if not target_path.suffix:
            for ext in ('.cat', '.py'):
                candidate = target_path.with_suffix(ext)
                if candidate.exists():
                    target_path = candidate
                    break

        if not target_path.exists():
            raise FileNotFoundError(f"Module not found: {spec}")
        if target_path.suffix in self.CATNIP_SUFFIXES:
            return self.load_catnip_module(target_path)
        elif self._is_supported_module_file(target_path):
            module = self.load_python_module(target_path)
            name = getattr(module, "__name__", spec).split(".")[-1]
            return ModuleNamespace(module, name)
        else:
            raise ValueError(f"Unsupported module type: {spec}")

    def _load_bare_name(self, name):
        """Resolve a bare name through search dirs then importlib fallback."""
        path, kind = self._resolve_bare_name(name)
        if path is not None:
            if kind == 'catnip':
                ns = self.load_catnip_module(path, module_name=name)
            else:
                module = self.load_python_module(path, module_name=name)
                ns = ModuleNamespace(module, name)
            self.loaded_modules[name] = ns
            return ns
        # importlib fallback (stdlib, pip packages)
        module = importlib.import_module(name)
        ns = ModuleNamespace(module, name.split(".")[-1])
        self.loaded_modules[name] = ns
        return ns

    def import_module(self, spec, caller_dir=None):
        """Load a module and return its namespace (for import() builtin).

        When caller_dir is set, relative paths are resolved against that
        directory instead of the process CWD.
        """
        spec = str(spec)
        # Cache hit
        if spec in self.loaded_modules:
            return self.loaded_modules[spec]
        # Policy gate
        policy = getattr(self.context, 'module_policy', None)
        if policy is not None and not policy.check(spec):
            raise CatnipRuntimeError(f"module '{spec}' blocked by policy")
        if self._is_explicit_path(spec):
            return self._load_explicit_path(spec, caller_dir=caller_dir)
        return self._load_bare_name(spec)

    def load_modules(self, module_names):
        """Load modules from CLI -m specs (name = namespace key)."""
        for name in module_names:
            try:
                module = self.import_module(name)
                ns_name = getattr(module, '_name', None) or getattr(module, '__name__', name).split(".")[-1]
                self.inject_as_namespace_obj(module, ns_name)
            except Exception as e:
                print_error(f"Error loading module '{name}': {e}")

    def inject_as_namespace_obj(self, namespace, name):
        """Inject an already-wrapped namespace into context globals."""
        if name in self.context.globals and self.verbose:
            print_warning(f"Overwriting existing global '{name}' with module namespace")
        self.context.globals[name] = namespace
        if self.verbose:
            items = dir(namespace) if hasattr(namespace, '__dir__') else []
            print_info(f"Loaded module '{name}' ({len(items)} symbols)")

    def inject_as_namespace(self, module, namespace_name=None):
        """
        Inject module as a namespace object.

        :param module: The loaded module object
        :param namespace_name: Name for the namespace (defaults to module name)
        :return: The namespace object
        """
        module_name = module.__name__
        namespace_name = namespace_name or module_name

        # Create namespace wrapper
        namespace = ModuleNamespace(module, namespace_name)

        # Check for conflicts
        if namespace_name in self.context.globals:
            if self.verbose:
                print_warning(f"Overwriting existing global '{namespace_name}' " f"with module namespace")

        # Inject namespace into context
        self.context.globals[namespace_name] = namespace

        if self.verbose:
            functions = list(namespace._functions.keys())
            print_info(f"Created namespace '{namespace_name}' with {len(functions)} functions:")
            for func_name in sorted(functions):
                print_info(f"  → {namespace_name}.{func_name}()")

        return namespace

    def list_loaded_modules(self):
        """Return a dictionary of loaded module names and their objects."""
        return self.loaded_modules.copy()

    def unload_module(self, module_name):
        """
        Unload a previously loaded module.

        :param module_name: Name of the module to unload
        """
        if module_name in self.loaded_modules:
            # Remove from loaded modules
            del self.loaded_modules[module_name]

            # Remove from sys.modules
            if module_name in sys.modules:
                del sys.modules[module_name]

            if self.verbose:
                print_info(f"Unloaded module: {module_name}")
        else:
            print_warning(f"Module '{module_name}' is not loaded")
