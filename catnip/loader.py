# FILE: catnip/loader.py
"""
Module loader for importing Catnip, Python, and binary modules into Catnip's execution context.
"""

import importlib
import importlib.util
import os
import sys
import sysconfig
import tomllib
from importlib import machinery
from pathlib import Path
from types import SimpleNamespace

from .colors import print_error, print_info, print_warning
from .exc import CatnipRuntimeError

_PROTOCOLS = frozenset(('py', 'rs', 'cat'))

# Stdlib native modules: bare Catnip name -> (crate import name, needs post-import config)
_STDLIB_MODULES = {
    'io': ('catnip_io', False),
    'sys': ('catnip_sys', True),
}

# Patterns that indicate the old file-path import style
_PATH_INDICATORS = ('./', '../', '/', '\\')
_FILE_EXTENSIONS = frozenset(
    (
        '.cat',
        '.py',
        '.pyc',
        '.pyo',
        '.so',
        '.pyd',
        '.dll',
    )
)


def _parse_relative_spec(spec):
    """Parse leading dots into (level, name). Returns (0, spec) if not relative."""
    level = 0
    for ch in spec:
        if ch == '.':
            level += 1
        else:
            break
    if level == 0:
        return 0, spec
    return level, spec[level:]


def _validate_spec(spec):
    """Validate an import spec and return the bare name.

    Detects legacy file-path patterns and raises with migration guidance.
    """
    # Relative imports: leading dots without '/' pass through
    if spec and spec[0] == '.' and not spec.startswith('./') and not spec.startswith('../'):
        stripped = spec.lstrip('.')
        if not stripped:
            raise CatnipRuntimeError(
                f"invalid relative import: {spec!r}\n" f"  relative imports require a module name after the dots"
            )
        return spec
    # Detect old path-based imports
    for indicator in _PATH_INDICATORS:
        if spec.startswith(indicator):
            raise CatnipRuntimeError(
                f"file paths in import() are no longer supported: {spec!r}\n"
                f'  use a bare name instead: import("{Path(spec).stem}")\n'
                f'  with protocol:           import("{Path(spec).stem}", protocol="py")'
            )
    if '/' in spec or '\\' in spec:
        raise CatnipRuntimeError(
            f"file paths in import() are no longer supported: {spec!r}\n"
            f'  use a bare name instead: import("{Path(spec).stem}")'
        )
    # Detect specs ending with a known file extension (but not dotted module names)
    # "host.py" -> error, "os.path" -> ok (not a file extension in this context)
    if '.' in spec:
        suffix = '.' + spec.rsplit('.', 1)[-1]
        if suffix in _FILE_EXTENSIONS:
            stem = spec.rsplit('.', 1)[0]
            proto = 'py' if suffix != '.cat' else 'cat'
            raise CatnipRuntimeError(
                f"file extensions in import() are no longer supported: {spec!r}\n"
                f'  use: import("{stem}", protocol="{proto}") or import("{stem}")'
            )
    return spec


def _extensions_for_protocol(protocol):
    """Return [(extension, kind), ...] for a given protocol."""
    if protocol == 'cat':
        return [('.cat', 'catnip')]
    elif protocol == 'py':
        exts = [('.py', 'python')]
        for ext in machinery.EXTENSION_SUFFIXES:
            exts.append((ext, 'python'))
        return exts
    elif protocol == 'rs':
        exts = []
        for ext in machinery.EXTENSION_SUFFIXES:
            exts.append((ext, 'python'))
        return exts
    else:
        # Default: .cat first, then .py, then native extensions
        exts = [('.cat', 'catnip'), ('.py', 'python')]
        for ext in machinery.EXTENSION_SUFFIXES:
            exts.append((ext, 'python'))
        return exts


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

    def _maybe_load_extension(self, module):
        """If module is a Catnip extension, load it into context."""
        from .extensions import load_extension, validate_extension

        if validate_extension(module) is not None:
            load_extension(module, self.context, verbose=self.verbose)

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
            # Use a namespaced key in sys.modules to avoid polluting the global
            # module namespace and prevent stale cache hits across contexts.
            sys_key = f"_catnip_local.{module_name}.{id(module_path)}"
            spec = importlib.util.spec_from_file_location(sys_key, module_path)
            if spec is None or spec.loader is None:
                raise ImportError(f"Cannot load module from {module_path}")

            module = importlib.util.module_from_spec(spec)
            sys.modules[sys_key] = module
            spec.loader.exec_module(module)

            self.loaded_modules[module_name] = module

            if self.verbose:
                print_info(f"Module '{module_name}' loaded successfully")

            return module

        except Exception as e:
            # Avoid keeping a partially initialized module in cache on import failure.
            sys.modules.pop(f"_catnip_local.{module_name}.{id(module_path)}", None)
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

        # Propagate module_policy from parent context
        kwargs = {}
        policy = getattr(self.context, 'module_policy', None)
        if policy is not None:
            kwargs['module_policy'] = policy
        catnip = Catnip(**kwargs)
        baseline_globals = dict(catnip.context.globals)

        # Enrich META with module metadata before execution
        meta = catnip.context.globals.get("META")
        if meta is not None:
            meta.file = str(module_path)
            meta.protocol = 'cat'
            meta.main = False

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

    def _search_dirs(self, caller_dir=None):
        """Return resolution directories: caller_dir -> CWD -> CATNIP_PATH (deduplicated)."""
        seen = set()
        dirs = []
        for d in filter(None, [caller_dir, Path.cwd()]):
            d = d.resolve()
            if d not in seen:
                seen.add(d)
                dirs.append(d)
        env = os.environ.get("CATNIP_PATH", "")
        if env:
            for entry in env.split(os.pathsep):
                p = Path(entry).resolve()
                if p.is_dir() and p not in seen:
                    seen.add(p)
                    dirs.append(p)
        return dirs

    def _try_load_package(self, dir_path, name, protocol=None):
        """Try loading a package directory with lib.toml. Returns namespace or None."""
        lib_toml = dir_path / 'lib.toml'
        if not lib_toml.is_file():
            return None

        try:
            manifest = tomllib.loads(lib_toml.read_text())
        except Exception:
            return None

        lib = manifest.get('lib', {})
        entry = lib.get('entry', 'main.cat')
        pkg_root = dir_path.resolve()
        entry_path = (dir_path / entry).resolve()

        # Keep package entry confined inside the package directory.
        if not entry_path.is_relative_to(pkg_root):
            raise ValueError(f"package {name!r}: entry point {entry!r} escapes package directory")

        if not entry_path.is_file():
            raise FileNotFoundError(f"package {name!r}: entry point {entry!r} not found in {dir_path}")

        ns = self.load_catnip_module(entry_path, module_name=name)

        # Apply export filtering from lib.toml if specified
        exports_conf = lib.get('exports', {})
        include = exports_conf.get('include')
        if include is not None:
            filtered = {k: getattr(ns, k) for k in include if hasattr(ns, k)}
            ns = SimpleNamespace(**filtered)
            ns.__name__ = name
            self.loaded_modules[name] = ns

        return ns

    def _resolve_relative(self, spec, caller_dir, protocol):
        """Resolve a relative import (leading dots) strictly from caller_dir."""
        if caller_dir is None:
            raise CatnipRuntimeError(
                f"relative import {spec!r} requires source file context\n"
                f"  relative imports are not available in REPL or -c mode"
            )
        level, name = _parse_relative_spec(spec)
        # Walk up from caller_dir: level 1 = same dir, level 2 = parent, etc.
        base = caller_dir
        for _ in range(level - 1):
            base = base.parent

        file_name = name.replace('.', '/')
        extensions = _extensions_for_protocol(protocol)

        # Check package (lib.toml) for non-dotted names
        if '.' not in name:
            pkg_dir = base / name
            if pkg_dir.is_dir() and (pkg_dir / 'lib.toml').is_file():
                ns = self._try_load_package(pkg_dir, name, protocol)
                if ns is not None:
                    # Cache by resolved absolute path
                    abs_key = str(pkg_dir.resolve())
                    self.loaded_modules[abs_key] = ns
                    return ns

        # Search for file with extensions
        for ext, kind in extensions:
            candidate = base / f"{file_name}{ext}"
            if candidate.is_file():
                resolved = candidate.resolve()
                abs_key = str(resolved)
                if abs_key in self.loaded_modules:
                    return self.loaded_modules[abs_key]
                if kind == 'catnip':
                    ns = self.load_catnip_module(resolved, module_name=name)
                else:
                    module = self.load_python_module(resolved, module_name=name)
                    self._maybe_load_extension(module)
                    ns = ModuleNamespace(module, name)
                self.loaded_modules[abs_key] = ns
                return ns

        raise FileNotFoundError(f"relative import {spec!r} not found\n" f"  looked in: {base}")

    def _resolve_bare_name(self, name, caller_dir=None, protocol=None):
        """Search dirs for name with prioritised extensions. Returns (path, kind) or (None, None).

        Package directories (with lib.toml) are checked before loose files.
        """
        file_name = name.replace('.', '/')
        extensions = _extensions_for_protocol(protocol)
        dirs = self._search_dirs(caller_dir)

        if '.' not in name:
            for d in dirs:
                pkg_dir = d / name
                if pkg_dir.is_dir() and (pkg_dir / 'lib.toml').is_file():
                    return pkg_dir, 'package'

        for d in dirs:
            for ext, kind in extensions:
                candidate = d / f"{file_name}{ext}"
                if candidate.is_file():
                    return candidate, kind
        return None, None

    def _try_stdlib(self, name):
        """Try loading a stdlib native module. Returns namespace or None."""
        entry = _STDLIB_MODULES.get(name)
        if entry is None:
            return None
        import_name, needs_configure = entry

        # 1. Try importing from installed package
        try:
            module = importlib.import_module(f'catnip.{import_name}')
        except ModuleNotFoundError:
            # 2. Fallback: search CATNIP_PATH and exe-relative dirs
            module = self._find_native_stdlib(import_name)
            if module is None:
                return None

        # 3. Post-import configuration (sys: argv, executable)
        if needs_configure:
            self._configure_sys(module)

        ns = ModuleNamespace(module, name)
        self.loaded_modules[name] = ns
        return ns

    def _find_native_stdlib(self, import_name):
        """Search CATNIP_PATH and exe-relative dirs for a native stdlib .so."""
        ext_suffix = sysconfig.get_config_var('EXT_SUFFIX') or '.so'
        so_name = f'{import_name}{ext_suffix}'
        search = list(self._search_dirs())
        exe = sys.executable
        if exe:
            exe_dir = Path(exe).parent
            search.extend([exe_dir / 'stdlib', exe_dir])
        for d in search:
            candidate = d / so_name
            if candidate.is_file():
                return self.load_python_module(candidate, module_name=import_name)
        return None

    def _configure_sys(self, module):
        """Override sys module attributes with context values."""
        argv = self.context.globals.get('argv')
        if argv is not None:
            module.argv = list(argv)
        executable = self.context.globals.get('_executable')
        if executable is not None:
            module.executable = executable

    def _load_from_resolved(self, path, kind, name, protocol=None):
        """Load a module from a resolved (path, kind) pair. Returns namespace or None."""
        if kind == 'package':
            return self._try_load_package(path, name, protocol)
        elif kind == 'catnip':
            ns = self.load_catnip_module(path, module_name=name)
            self.loaded_modules[name] = ns
            return ns
        elif kind is not None:
            module = self.load_python_module(path, module_name=name)
            self._maybe_load_extension(module)
            ns = ModuleNamespace(module, name)
            self.loaded_modules[name] = ns
            return ns
        return None

    def _load_bare_name(self, name, caller_dir=None, protocol=None):
        """Resolve a bare name: local → stdlib → importlib.

        Resolution order:
          1. File search (caller_dir → CWD → CATNIP_PATH)
          2. Catnip stdlib native modules (_STDLIB_MODULES)
          3. importlib fallback (Python ecosystem)

        With protocol='py', importlib is tried first so the caller can
        explicitly reach a Python package shadowed by local files.
        """
        # 0. protocol='py' → importlib first (explicit Python request)
        if protocol == 'py':
            try:
                module = importlib.import_module(name)
                self._maybe_load_extension(module)
                ns = ModuleNamespace(module, name.split(".")[-1])
                self.loaded_modules[name] = ns
                return ns
            except ModuleNotFoundError as e:
                if e.name != name and not name.startswith(e.name + "."):
                    raise
        # 1. File search (caller_dir → CWD → CATNIP_PATH, shadows everything)
        path, kind = self._resolve_bare_name(name, caller_dir, protocol)
        ns = self._load_from_resolved(path, kind, name, protocol)
        if ns is not None:
            return ns
        # 2. Stdlib native modules (unless protocol forces py/cat)
        if protocol in (None, 'rs') and name in _STDLIB_MODULES:
            ns = self._try_stdlib(name)
            if ns is not None:
                return ns
        # 3. cat protocol blocks importlib fallback
        if protocol == 'cat':
            raise FileNotFoundError(f"Catnip module not found: {name!r}")
        # 4. importlib fallback (Python ecosystem)
        module = importlib.import_module(name)
        self._maybe_load_extension(module)
        ns = ModuleNamespace(module, name.split(".")[-1])
        self.loaded_modules[name] = ns
        return ns

    def import_module(self, spec, caller_dir=None, protocol=None):
        """Load a module and return its namespace (for import() builtin)."""
        name = _validate_spec(str(spec))
        if protocol is not None and protocol not in _PROTOCOLS:
            raise CatnipRuntimeError(
                f"unknown protocol {protocol!r} -- valid protocols: {', '.join(sorted(_PROTOCOLS))}"
            )
        # Relative imports bypass cache-by-name, importlib, search dirs
        # but NOT the policy gate
        if name and name[0] == '.':
            _, bare = _parse_relative_spec(name)
            policy = getattr(self.context, 'module_policy', None)
            if policy is not None and bare and not policy.check(bare):
                raise CatnipRuntimeError(f"module '{bare}' blocked by policy")
            return self._resolve_relative(name, caller_dir, protocol)
        # Cache hit only for bare imports (no explicit protocol) to preserve
        # .cat > .py priority; explicit protocol forces fresh resolution.
        if protocol is None and name in self.loaded_modules:
            return self.loaded_modules[name]
        # Policy gate on the bare name
        policy = getattr(self.context, 'module_policy', None)
        if policy is not None and not policy.check(name):
            raise CatnipRuntimeError(f"module '{name}' blocked by policy")
        return self._load_bare_name(name, caller_dir=caller_dir, protocol=protocol)

    def load_modules(self, module_names):
        """Load modules from CLI -m specs.

        Supports suffixes:
          - name      -> namespace (ctx.globals["name"] = namespace)
          - name:!    -> inject all symbols into globals
          - name:alias -> namespace under custom name
        """
        for spec in module_names:
            try:
                if ':!' in spec:
                    bare = spec.replace(':!', '')
                    module = self.import_module(bare)
                    self._inject_globals(module, bare)
                elif ':' in spec:
                    bare, alias = spec.split(':', 1)
                    module = self.import_module(bare)
                    self.inject_as_namespace_obj(module, alias)
                else:
                    module = self.import_module(spec)
                    ns_name = getattr(module, '_name', None) or getattr(module, '__name__', spec).split(".")[-1]
                    self.inject_as_namespace_obj(module, ns_name)
            except Exception as e:
                print_error(f"Error loading module '{spec}': {e}")

    def _inject_globals(self, namespace, name):
        """Inject all public symbols from namespace directly into globals."""
        items = dir(namespace) if hasattr(namespace, '__dir__') else []
        for attr in items:
            if not attr.startswith('_'):
                self.context.globals[attr] = getattr(namespace, attr)
        if self.verbose:
            print_info(f"Injected {len(items)} symbols from '{name}' into globals")

    def inject_as_namespace_obj(self, namespace, name):
        """Inject an already-wrapped namespace into context globals."""
        if name in self.context.globals and self.verbose:
            print_warning(f"Overwriting existing global '{name}' with module namespace")
        self.context.globals[name] = namespace
        if self.verbose:
            items = dir(namespace) if hasattr(namespace, '__dir__') else []
            print_info(f"Loaded module '{name}' ({len(items)} symbols)")
