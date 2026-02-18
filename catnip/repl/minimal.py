# FILE: catnip/repl/minimal.py
"""Minimal REPL with Python module support."""

import random

from catnip._rs import parse_repl_command, preprocess_multiline, should_continue_multiline

_EXIT_OK = (
    "state resolved.",
    "context destroyed.",
    "collapse complete.",
    "no more universes.",
    "nothing left to evaluate.",
    "reality committed.",
    "dimensions collapsed.",
    "EOF was a choice.",
    "halt.",
)
_EXIT_ABORT = (
    "vm is dead.",
    "process was not consulted.",
    "aborted mid-thought.",
    "stack abandoned.",
    "evaluation denied.",
    "interrupted. the universe noticed.",
    "ctrl-c is not a proof.",
    "terminated with prejudice.",
)
_EXIT_WEIRD = (
    "this exit is undecidable.",
    "the halting problem applies.",
    "the computation escaped.",
    "this result is left as an exercise.",
    "undefined, but consistent.",
)


class MinimalREPL:
    """Lightweight REPL for Python module integration.

    Used when Python modules (-m/--module) are requested.
    For pure Catnip code, use the Rust REPL (catnip-repl) instead.
    """

    def __init__(self, catnip, parsing=3, verbose=False):
        self.catnip = catnip
        self.parsing = parsing
        self.verbose = verbose

    def run(self):
        print("Catnip REPL (Python mode)")
        print("Type 'exit' or Ctrl+D to leave, 'help' for help.")

        abort = False
        while True:
            try:
                # Simple prompt
                command = input("▸ ")

                # Multiline support (Rust function)
                if should_continue_multiline(command):
                    lines = [command]
                    while True:
                        try:
                            line = input("▹ ")
                            lines.append(line)
                            full_text = "\n".join(lines)
                            if not should_continue_multiline(full_text):
                                break
                        except (EOFError, KeyboardInterrupt):
                            break
                    command = "\n".join(lines)

                command = command.strip()
                if not command:
                    continue

                # Check exit/quit commands
                if command in ("exit", "quit", "/exit", "/quit"):
                    break

                # Command handling (Rust function)
                cmd_name, _ = parse_repl_command(command)
                if cmd_name in ("exit", "quit"):
                    break

                # Execute (using Python pipeline for features support)
                processed = preprocess_multiline(command)
                from ..processor import process_input

                process_input(self.catnip, processed, self.parsing, self.verbose)

            except EOFError:
                break
            except KeyboardInterrupt:
                abort = True
                break
            except Exception as e:
                print(f"Error: {e}")

        if random.randint(0, 99) == 0:
            msgs = _EXIT_WEIRD
        elif abort:
            msgs = _EXIT_ABORT
        else:
            msgs = _EXIT_OK
        print(random.choice(msgs))
