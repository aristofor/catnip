# FILE: catnip/cli/main.py
"""Main CLI entry point with plugin support."""

import os
import sys
from pathlib import Path

import click

from .. import __version__
from ..colors import Theme
from ..config import ConfigManager
from .plugins import discover_plugins, load_plugin


def _default_executor():
    """Get default executor from environment variable.

    Env var: CATNIP_EXECUTOR
    Accepts: vm (default), ast
    """
    value = (os.environ.get("CATNIP_EXECUTOR") or "vm").lower()
    if value in {"vm", "ast"}:
        return value
    else:
        return "vm"


class CatnipGroup(click.Group):
    """Click group with lazy plugin loading."""

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self._plugin_cache = None

    @property
    def plugins(self):
        """Load plugin registry on demand."""
        if self._plugin_cache is None:
            self._plugin_cache = discover_plugins()
        return self._plugin_cache

    def list_commands(self, ctx):
        """List all commands (built-in + plugins)."""
        commands = list(super().list_commands(ctx))
        commands.extend(sorted(self.plugins.keys()))
        # Deduplicate (built-ins are also plugins)
        return sorted(set(commands))

    def get_command(self, ctx, cmd_name):
        """Get a command by name, load plugins on demand."""
        # Check built-in commands first
        cmd = super().get_command(ctx, cmd_name)
        if cmd is not None:
            return cmd

        # Check plugins
        if cmd_name in self.plugins:
            return load_plugin(self.plugins[cmd_name])

        return None

    def make_context(self, info_name, args, parent=None, **extra):
        """Create context, preserve script args."""
        # Find the first non-option arg (potential script file)
        script_idx = None
        i = 0
        while i < len(args):
            arg = args[i]
            if arg.startswith('-'):
                # Option - skip it (and its value if not a flag)
                if arg in (
                    '-c',
                    '--command',
                    '-p',
                    '--parsing',
                    '--config',
                    '-o',
                    '--optimize',
                    '-m',
                    '--module',
                    '-x',
                    '--executor',
                    '--theme',
                    '--format',
                ):
                    i += 2  # skip option + value
                else:
                    i += 1  # flag only
            else:
                # First non-option found
                if '/' in arg or arg.endswith('.cat') or arg == '--' or Path(arg).exists():
                    script_idx = i
                break

        # If it's a file, remove everything from script_idx
        if script_idx is not None:
            script_args = args[script_idx:]
            args = args[:script_idx]
        else:
            script_args = None

        ctx = super().make_context(info_name, list(args), parent=parent, **extra)
        if script_args:
            ctx.args = list(script_args)
        return ctx


def setup_catnip(verbose, no_color, optimizations, modules, config_manager=None):
    """Configure and return a Catnip instance.

    Uses ConfigManager for unified config handling with source tracking.
    Precedence: defaults < file < env < CLI
    """
    from .. import Catnip

    # Create and populate ConfigManager if not provided
    if config_manager is None:
        config_manager = ConfigManager()
        config_manager.load_file()
        config_manager.load_env()

    # Apply CLI overrides
    if no_color:
        config_manager.apply_cli_no_color()

    config_manager.apply_cli_optimizations(optimizations)

    # Apply no_color from config
    if config_manager.get('no_color'):
        Theme.disable()

    try:
        from ..parser import Parser as TreeSitterParser

        parser_class = TreeSitterParser
    except ImportError:
        click.echo("Tree-sitter parser not available. Run: make compile", err=True)
        sys.exit(1)

    # Create Catnip with executor from config
    vm_mode_map = {"vm": "on", "ast": "off"}
    vm_mode = vm_mode_map.get(config_manager.get('executor'), "on")

    # Setup cache if enabled
    cache = None
    if config_manager.get('enable_cache'):
        from ..cachesys import CatnipCache, DiskCache

        cache = CatnipCache(backend=DiskCache())

    catnip = Catnip(parser_class=parser_class, vm_mode=vm_mode, cache=cache)

    # Apply config to pragma_context
    catnip.pragma_context.jit_enabled = config_manager.get('jit')
    catnip.pragma_context.tco_enabled = config_manager.get('tco')
    catnip.pragma_context.optimize_level = config_manager.get('optimize')

    # Store manager for debug access
    catnip._config_manager = config_manager

    if modules:
        from ..loader import ModuleLoader

        loader = ModuleLoader(catnip.context, verbose=verbose)
        loader.load_modules(modules)

    return catnip


@click.group(
    cls=CatnipGroup,
    invoke_without_command=True,
    context_settings={
        "help_option_names": ["-h", "--help"],
        "ignore_unknown_options": True,
        "allow_extra_args": True,
        "allow_interspersed_args": False,
    },
)
@click.pass_context
@click.option("-c", "--command", "cmd", type=str, help="Evaluate command and display result")
@click.option(
    "-p",
    "--parsing",
    type=int,
    default=3,
    help="Parsing level: 0=tree, 1=IR, 2=exec IR, 3=run",
)
@click.option("-v", "--verbose", is_flag=True, help="Show detailed pipeline stages")
@click.option("--no-color", is_flag=True, help="Disable colored output")
@click.option("--no-cache", is_flag=True, help="Disable disk cache for parsing/bytecode")
@click.option(
    "--config",
    "config_path",
    type=click.Path(exists=True),
    help="Use alternate config file instead of ~/.config/catnip/catnip.toml",
)
@click.option(
    "-o",
    "--optimize",
    "optimizations",
    multiple=True,
    type=str,
    help="Optimizations: tco[:on|off|auto], level[:0-3], jit[:on|off]",
)
@click.option(
    "-m",
    "--module",
    "modules",
    multiple=True,
    type=str,
    help="Load Python module as namespace (e.g., -m math, -m numpy)",
)
@click.option(
    "-x",
    "--executor",
    "executor",
    type=click.Choice(["vm", "ast"], case_sensitive=False),
    default=_default_executor,
    help="Execution mode: vm=bytecode VM (default), ast=AST interpreter",
)
@click.option(
    "--theme",
    "theme",
    type=click.Choice(["auto", "dark", "light"], case_sensitive=False),
    default=None,
    help="Color theme: auto=detect terminal background, dark, light",
)
@click.option(
    "--format",
    "output_format",
    type=click.Choice(["text", "json", "repr"], case_sensitive=False),
    default="text",
    help="Output format: text=compact JSON (default), json=verbose serde JSON, repr=Python repr",
)
@click.version_option(__version__, prog_name="Catnip")
def main(
    ctx, cmd, parsing, verbose, no_color, no_cache, config_path, optimizations, modules, executor, theme, output_format
):
    """
    Catnip - A sandboxed scripting language embedded in Python

    \b
    Usage:
      catnip                           # Interactive REPL (VM mode by default)
      catnip script.cat                # Run a script file
      catnip -c "2 + 3 * 4"           # Evaluate a command
      catnip -x ast script.cat         # Use AST interpreter instead of VM
      catnip --config my.toml script.cat  # Use custom config file

    \b
    Environment:
      CATNIP_OPTIMIZE       Optimizations (same as -o): jit,tco:off,level:2
      CATNIP_EXECUTOR       Execution mode: vm, ast
      CATNIP_THEME          Color theme: auto, dark, light
      NO_COLOR              Disable colors (freedesktop.org standard)

    \b
    Subcommands:
      commands  List available commands (built-ins + plugins)
      config    View/edit configuration
      format    Format Catnip code
      lint      Full code analysis
      plugins   Inspect registered plugins
      repl      Start interactive REPL (explicit)
    """
    # Build ConfigManager with full precedence chain
    config_manager = ConfigManager()
    if config_path:
        config_manager.load_file(Path(config_path))
    else:
        config_manager.load_file()
    config_manager.load_env()

    # Apply CLI overrides
    if no_color:
        config_manager.apply_cli_no_color()
    if no_cache:
        config_manager.apply_cli_no_cache()
    if executor != _default_executor():  # Only if explicitly set
        config_manager.apply_cli_executor(executor)
    if theme is not None:
        config_manager.apply_cli_theme(theme)

    # Store options in context for subcommands
    ctx.ensure_object(dict)
    ctx.obj.update(
        dict(
            cmd=cmd,
            parsing=parsing,
            verbose=verbose,
            config_path=config_path,
            optimizations=optimizations,
            modules=modules,
            config_manager=config_manager,
            output_format=output_format,
        )
    )

    if config_manager.get('no_color'):
        Theme.disable()

    # Apply theme override if not auto
    _theme_val = config_manager.get('theme')
    if _theme_val in ('dark', 'light'):
        Theme.set_theme(_theme_val)

    # If no subcommand, execute default mode
    if ctx.invoked_subcommand is None:
        _run_default_mode(ctx)


def _strip_shebang(text: str) -> str:
    """Remove shebang line if present.

    Allows Catnip scripts to use shebangs for standalone execution:
        #!/usr/bin/env catnip
    """
    if text.startswith('#!'):
        lines = text.split('\n', 1)
        return lines[1] if len(lines) > 1 else ''
    return text


def _try_rust_repl(verbose=False):
    """Launch Rust REPL via PyO3 (integrated) or binary fallback."""
    try:
        from .. import _repl

        _repl.run_repl(verbose=verbose)
        return True
    except ImportError:
        pass
    except Exception as e:
        if verbose:
            click.echo(f"Integrated REPL failed: {e}, trying binary...")

    # Fallback : binaire externe
    import shutil
    import subprocess

    repl_path = shutil.which('catnip-repl')
    if repl_path:
        try:
            result = subprocess.run([repl_path])
            return result.returncode == 0
        except Exception:
            return False
    return False


def _detect_execution_mode(cmd, script_path, stdin_is_tty):
    """Detect execution mode for config overrides.

    Returns:
        'standalone': Script execution (script.cat)
        'repl': Interactive REPL
        None: Command (-c) or stdin pipe (no mode-specific config)
    """
    if script_path:
        return 'standalone'
    elif not cmd and stdin_is_tty:
        return 'repl'
    return None


def _run_default_mode(ctx):
    """Handle default mode: REPL, -c, stdin, or script."""
    from ..processor import process_input

    opts = ctx.obj
    cmd = opts['cmd']
    parsing = opts['parsing']
    verbose = opts['verbose']
    optimizations = opts['optimizations']
    modules = opts['modules']
    config_manager = opts['config_manager']
    config_path = opts.get('config_path')
    output_format = opts.get('output_format', 'text')

    # Extra arguments (script path or -- separator)
    extra_args = ctx.args

    script_path = None
    script_args = []
    if extra_args:
        first_arg = extra_args[0]
        if first_arg == '--':
            if len(extra_args) < 2:
                click.echo("Error: '--' requires a script file after it", err=True)
                sys.exit(1)
            script_path = Path(extra_args[1])
            script_args = extra_args[2:]  # Args after '--' and script path
        else:
            script_path = Path(first_arg)
            script_args = extra_args[1:]  # Args after script path

    # Detect execution mode for mode-specific config
    mode = _detect_execution_mode(cmd, script_path, sys.stdin.isatty())
    if mode:
        if verbose:
            click.echo(f"Execution mode: {mode}")
        # Load mode-specific overrides from [mode.{mode}]
        config_manager.load_mode_overrides(mode, config_path if config_path else None)

    # Setup catnip using ConfigManager (CLI optimizations applied here)
    no_color = config_manager.get('no_color')
    catnip = setup_catnip(verbose, no_color, optimizations, modules, config_manager)
    vm_mode = catnip._config_manager.get('executor')
    vm_mode_map = {"vm": "on", "ast": "off"}
    vm_mode = vm_mode_map.get(vm_mode, "on")

    # Inject script arguments into global context (accessible as 'argv')
    if script_path:
        catnip.context.globals['argv'] = [str(script_path)] + script_args

    # Command mode (-c)
    if cmd:
        try:
            process_input(catnip, cmd, parsing, verbose, vm_mode=vm_mode, output_format=output_format)
            sys.exit(0)
        except Exception as e:
            from ..colors import print_exception

            print_exception(e)
            if verbose:
                import traceback

                traceback.print_exc()
            sys.exit(1)

    # Script mode
    elif script_path:
        if not script_path.exists():
            click.echo(f"Error: Script file not found: {script_path}", err=True)
            sys.exit(1)
        text = _strip_shebang(script_path.read_text())
        try:
            process_input(catnip, text, parsing, verbose, vm_mode=vm_mode, output_format=output_format)
            sys.exit(0)
        except Exception as e:
            from ..colors import print_exception

            print_exception(e)
            if verbose:
                import traceback

                traceback.print_exc()
            sys.exit(1)

    # Pipe mode (stdin)
    elif not sys.stdin.isatty():
        text = _strip_shebang(sys.stdin.read())
        try:
            process_input(catnip, text, parsing, verbose, vm_mode=vm_mode, output_format=output_format)
            sys.exit(0)
        except Exception as e:
            from ..colors import print_exception

            print_exception(e)
            if verbose:
                import traceback

                traceback.print_exc()
            sys.exit(1)

    # REPL mode (default)
    else:
        # If Python modules requested, use minimal Python REPL
        if modules:
            from ..repl import MinimalREPL

            repl = MinimalREPL(catnip, parsing=parsing, verbose=verbose)
            repl.run()
        # Otherwise, use Rust REPL (fast, standalone)
        elif not _try_rust_repl(verbose=verbose):
            click.echo("Error: Rust REPL 'catnip-repl' not found in PATH", err=True)
            click.echo("Install it with: make compile", err=True)
            sys.exit(1)
