# FILE: catnip/cli/commands/repl.py
"""Explicit REPL command."""

import sys

import click


@click.command("repl")
@click.pass_context
def cmd_repl(ctx):
    """Start the interactive REPL (default mode)."""
    from ..main import _try_rust_repl, setup_catnip

    opts = ctx.obj or {}
    modules = opts.get('modules', ())

    # Python REPL if modules requested
    if modules:
        from ...repl import MinimalREPL

        catnip = setup_catnip(
            verbose=opts.get('verbose', False),
            no_color=opts.get('no_color', False),
            optimizations=opts.get('optimizations', ()),
            modules=modules,
        )
        repl = MinimalREPL(catnip, parsing=opts.get('parsing', 3), verbose=opts.get('verbose', False))
        repl.run()
    # Rust REPL (PyO3-first, fallback binaire)
    elif not _try_rust_repl(verbose=opts.get('verbose', False)):
        click.echo("Error: Rust REPL not available", err=True)
        click.echo("Install it with: make compile", err=True)
        sys.exit(1)
