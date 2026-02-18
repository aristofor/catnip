# FILE: catnip/processor.py
"""
Code processing with verbose mode and enhanced output.

Uses Rust pipeline for basic orchestration (non-verbose mode).
Python handles verbose mode with detailed stage printing.
"""

import os
from pprint import pformat

from ._rs import process_input as rust_process_input
from .colors import Theme, colorize, format_value, print_stage
from .exc import CatnipInternalError


def process_input(catnip, text, parsing, verbose=False, vm_mode="off", output_format="text"):
    """
    Process the text (command or script) based on the chosen parsing level.

    Parsing levels:
    - 0: Parse tree only (Lark AST)
    - 1: IR only (after transformer, before semantic)
    - 2: Executable IR (after semantic analysis)
    - 3: Execute and show result (default)

    :param catnip: Catnip instance
    :param text: Source code to process
    :param parsing: Parsing level (0-3)
    :param verbose: Show pipeline stages
    :param vm_mode: VM execution mode ("off", "on")
    :param output_format: Output format ("text" or "json")
    """
    try:
        # Levels 0-2: Use Rust pipeline (fast orchestration)
        if parsing < 3:
            if verbose:
                print_stage("INPUT", text, Theme.STAGE_PARSE)

            res = rust_process_input(catnip, text, parsing)

            if parsing == 0:
                # Parse tree: always use .pretty()
                content = res.pretty()
                if verbose:
                    print_stage("PARSE TREE", content, Theme.STAGE_PARSE)
                else:
                    print(content)
            elif output_format == "json":
                # Serde JSON verbeux (backwards compat)
                import json as json_module

                from ._rs import ir_to_json_pretty

                if isinstance(res, list):
                    json_items = []
                    for item in res:
                        item_json = ir_to_json_pretty(item)
                        json_items.append(json_module.loads(item_json))
                    print(json_module.dumps(json_items, indent=2))
                else:
                    print(ir_to_json_pretty(res))
            elif output_format == "repr":
                # Ancien pformat
                content = pformat(res)
                if verbose:
                    stage_names = {1: "IR (Transformer)", 2: "EXECUTABLE IR (Semantic)"}
                    print_stage(stage_names[parsing], content, Theme.STAGE_SEMANTIC)
                else:
                    print(content)
            else:
                # text (defaut): compact JSON
                from ._rs import ir_to_json_compact_pretty

                if isinstance(res, list):
                    import json as json_module

                    compact_items = []
                    for item in res:
                        item_json = ir_to_json_compact_pretty(item)
                        compact_items.append(json_module.loads(item_json))
                    content = json_module.dumps(compact_items, indent=2)
                else:
                    content = ir_to_json_compact_pretty(res)

                if verbose:
                    stage_names = {1: "IR (Transformer)", 2: "EXECUTABLE IR (Semantic)"}
                    print_stage(stage_names[parsing], content, Theme.STAGE_SEMANTIC)
                else:
                    print(content)

        elif parsing == 3:
            # Level 3: Execute and show result
            if verbose:
                print_stage("INPUT", text, Theme.STAGE_PARSE)

                # Show IR
                ir = catnip.parse(text, semantic=False)
                print_stage("IR (Transformer)", _format_ir_compact(ir), Theme.STAGE_SEMANTIC)

                # Show executable IR
                catnip.parse(text, semantic=True)
                executable = catnip.code
                print_stage(
                    "EXECUTABLE IR (Semantic)",
                    _format_ir_compact(executable),
                    Theme.STAGE_SEMANTIC,
                )

            else:
                catnip.parse(text)

            if verbose:
                env_executor = os.environ.get("CATNIP_EXECUTOR")
                if env_executor:
                    # Map executor to vm_mode for display: vm→on, ast→off
                    expected_vm_mode = (
                        "on" if env_executor == "vm" else "off" if env_executor == "ast" else env_executor
                    )
                    suffix = "" if expected_vm_mode == vm_mode else " (overridden)"
                    vm_info = f"{vm_mode} (CATNIP_EXECUTOR={env_executor}{suffix})"
                else:
                    vm_info = f"{vm_mode}"
                print_stage("EXECUTOR", vm_info, Theme.STAGE_EXECUTE)

            # Execute (using VM if requested)
            catnip.vm_mode = vm_mode
            if vm_mode == "off":
                res = catnip.execute()
            else:
                from .vm.executor import VMExecutor

                executor = VMExecutor(catnip.registry, catnip.context)
                res = executor.execute(catnip.code, trace=verbose)

            # Show result (suppress None like Python REPL)
            if verbose:
                print_stage("RESULT", format_value(res), Theme.STAGE_RESULT)
            elif res is not None:
                if Theme.enabled:
                    print(format_value(res))
                else:
                    print(repr(res))

        else:
            raise CatnipInternalError(f"Unknown parsing level {parsing!r}")

    except Exception:
        # Don't print error here, it will be displayed by __main__.py
        # to avoid double display
        if verbose:
            import traceback

            traceback.print_exc()
        raise


def _format_ir_compact(ir):
    """Format IR in a more compact, readable way."""
    if isinstance(ir, list):
        if len(ir) == 1:
            return _format_node(ir[0])
        return "\n".join(f"{i+1}. {_format_node(node)}" for i, node in enumerate(ir))
    return _format_node(ir)


def _format_node(node):
    """Format a single IR node with colors."""
    from .nodes import Op, Ref
    from .transformer import IR

    if isinstance(node, (Op, IR)):
        # Format operation
        ident = colorize(node.ident, Theme.KEYWORD)
        args_str = _format_args(node.args)
        if node.kwargs:
            kwargs_str = ", ".join(f"{k}={_format_node(v)}" for k, v in node.kwargs.items())
            return f"{ident}({args_str}, {kwargs_str})"
        return f"{ident}({args_str})"

    elif isinstance(node, Ref):
        return colorize(node.ident, Theme.IDENTIFIER)

    elif isinstance(node, str):
        return colorize(repr(node), Theme.STRING)

    elif isinstance(node, (int, float)):
        return colorize(str(node), Theme.NUMBER)

    elif isinstance(node, bool):
        return colorize(str(node), Theme.TYPE_BOOL)

    elif node is None:
        return colorize("None", Theme.TYPE_NONE)

    elif isinstance(node, (list, tuple)):
        items = ", ".join(_format_node(item) for item in node)
        bracket = "[" if isinstance(node, list) else "("
        close_bracket = "]" if isinstance(node, list) else ")"
        return f"{bracket}{items}{close_bracket}"

    else:
        return repr(node)


def _format_args(args):
    """Format argument list."""
    if not args:
        return ""
    if len(args) == 1:
        return _format_node(args[0])
    return ", ".join(_format_node(arg) for arg in args)
