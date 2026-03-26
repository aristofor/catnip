# FILE: catnip/cli/commands/completion.py
"""Shell completion script generation."""

import click


@click.command('completion')
@click.argument('shell', type=click.Choice(['bash', 'zsh', 'fish']))
def cmd_completion(shell):
    """Generate shell completion script.

    \b
    Usage:
      eval "$(catnip completion bash)"    # Bash
      eval "$(catnip completion zsh)"     # Zsh
      catnip completion fish | source     # Fish

    \b
    Persistent install:
      catnip completion bash >> ~/.bashrc
      catnip completion zsh >> ~/.zshrc
      catnip completion fish > ~/.config/fish/completions/catnip.fish
    """
    from click.shell_completion import get_completion_class

    # Find the root CLI group (parent of this subcommand)
    root = click.get_current_context().find_root().command

    comp_cls = get_completion_class(shell)
    if comp_cls is None:
        click.echo(f"Error: unsupported shell '{shell}'", err=True)
        raise SystemExit(1)

    comp = comp_cls(root, {}, 'catnip', '_CATNIP_COMPLETE')
    click.echo(comp.source())
