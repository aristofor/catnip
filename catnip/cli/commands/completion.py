# FILE: catnip/cli/commands/completion.py
"""Shell completion script generation."""

import click

_BASH_COMPLETION = '''\
_catnip_completion() {
    local IFS=$'\\n'
    local response
    local -a plain_completions
    local has_file=0 has_dir=0

    response=$(env COMP_WORDS="${COMP_WORDS[*]}" COMP_CWORD=$COMP_CWORD _CATNIP_COMPLETE=bash_complete $1)

    for completion in $response; do
        IFS=',' read type value <<< "$completion"

        if [[ $type == 'dir' ]]; then
            has_dir=1
        elif [[ $type == 'file' ]]; then
            has_file=1
        elif [[ $type == 'plain' ]]; then
            plain_completions+=($value)
        fi
    done

    if [[ ${#plain_completions[@]} -gt 0 ]]; then
        COMPREPLY=("${plain_completions[@]}")
    elif [[ $has_dir -eq 1 ]]; then
        COMPREPLY=()
        compopt -o dirnames
    elif [[ $has_file -eq 1 ]]; then
        COMPREPLY=()
        compopt -o default
    fi

    return 0
}

_catnip_completion_setup() {
    complete -o nosort -F _catnip_completion catnip
}

_catnip_completion_setup;
'''


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
    if shell == 'bash':
        click.echo(_BASH_COMPLETION)
        return

    from click.shell_completion import get_completion_class

    # Find the root CLI group (parent of this subcommand)
    root = click.get_current_context().find_root().command

    comp_cls = get_completion_class(shell)
    if comp_cls is None:
        click.echo(f"Error: unsupported shell '{shell}'", err=True)
        raise SystemExit(1)

    comp = comp_cls(root, {}, 'catnip', '_CATNIP_COMPLETE')
    click.echo(comp.source())
