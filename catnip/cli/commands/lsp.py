# FILE: catnip/cli/commands/lsp.py
import os
import shutil
import sys

import click


@click.command('lsp')
@click.option('--stdio', is_flag=True, default=True, hidden=True)
def cmd_lsp(stdio):
    """Start Catnip LSP server."""
    lsp = shutil.which('catnip-lsp')
    if not lsp:
        click.echo("catnip-lsp not found. Run: make install-bins", err=True)
        sys.exit(1)
    args = [lsp]
    if stdio:
        args.append('--stdio')
    os.execvp(lsp, args)
