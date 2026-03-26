# FILE: catnip/cli/commands/debug.py
"""Interactive debugger command."""

import sys

import click


@click.command('debug')
@click.argument('file', type=click.Path(exists=True), required=False)
@click.option('--break', '-b', 'breaklines', multiple=True, type=int, help="Line numbers to break at")
@click.option('-c', '--command', 'code', help="Code string to debug (instead of file)")
@click.pass_context
def cmd_debug(ctx, file, breaklines, code):
    """Start an interactive debug session."""
    from ...debug.console import ConsoleDebugger
    from ...debug.session import DebugSession
    from ..main import setup_catnip

    opts = ctx.obj or {}

    if file:
        with open(file) as f:
            source = f.read()
    elif code:
        source = code
    else:
        click.echo("Error: provide a file or -c 'code'", err=True)
        sys.exit(1)

    catnip = setup_catnip(
        verbose=opts.get('verbose', False),
        no_color=opts.get('no_color', False),
        optimizations=opts.get('optimizations', ()),
        modules=opts.get('modules', ()),
        mode='cli',
    )

    session = DebugSession(catnip, source)
    for line in breaklines:
        session.add_breakpoint(line)

    debugger = ConsoleDebugger(session, no_color=opts.get('no_color', False), filename=file)
    exit_code = debugger.run()
    if exit_code:
        sys.exit(exit_code)
