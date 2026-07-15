# FILE: catnip/pragma.py
"""Pragma system for Catnip. All components implemented in Rust."""

from catnip._rs import Pragma, PragmaContext, PragmaType, pragma_directives

# Canonical directive names from Rust
PRAGMA_DIRECTIVES = frozenset(pragma_directives())

# Pragma directive -> (PragmaContext attr_name, Python type)
PRAGMA_ATTRS = {
    'tco': ('tco_enabled', bool),
    'debug': ('debug_mode', bool),
    'cache': ('cache_enabled', bool),
    'optimize': ('optimize_level', int),
    'jit': ('jit_enabled', bool),
    'jit_all': ('jit_all', bool),
    'nd_mode': ('nd_mode', str),
    'nd_workers': ('nd_workers', int),
    'nd_memoize': ('nd_memoize', bool),
    'nd_batch_size': ('nd_batch_size', int),
    'batch_size': ('nd_batch_size', int),
}


def sync_pragmas_from_nodes(nodes, pragma_context, *, on_error, skip_directive=None):
    """Apply Pragma IR nodes to a PragmaContext.

    Single loop for both the DSL (Catnip) and the standalone wrapper, so the
    two paths accept and reject the same pragmas. `on_error(kind, message,
    node)` handles invalid pragmas -- kind is 'semantic' (unknown directive)
    or 'value' (bad value) -- and is expected to raise; `skip_directive`
    marks directives forced by the caller (kwargs/CLI/env win over file
    pragmas).
    """
    for node in nodes:
        if node.kind != 'Op' or node.opcode != 'Pragma':
            continue
        args = node.args
        if not args:
            continue
        directive = args[0].value
        value = args[1].value if len(args) > 1 else True
        mapping = PRAGMA_ATTRS.get(directive)
        if mapping is None:
            if directive == 'warning':
                if not isinstance(value, bool):
                    on_error('value', "Pragma 'warning' requires True or False", node)
                continue
            if directive in ('inline', 'pure'):
                continue
            on_error('semantic', f"Unknown pragma directive: '{directive}'", node)
            continue
        attr, typ = mapping
        # jit accepts bool or 'all'
        if directive == 'jit' and value == 'all':
            pragma_context.jit_enabled = True
            pragma_context.jit_all = True
            continue
        if skip_directive is not None and skip_directive(directive):
            continue
        try:
            # bool() on strings silently returns True, check type
            if typ is bool and not isinstance(value, bool):
                raise ValueError(f"requires True or False, got {value!r}")
            setattr(pragma_context, attr, typ(value))
        except (ValueError, TypeError) as e:
            on_error('value', f"Invalid value for pragma '{directive}': {e}", node)


__all__ = (
    'Pragma',
    'PragmaContext',
    'PragmaType',
    'PRAGMA_ATTRS',
    'PRAGMA_DIRECTIVES',
    'sync_pragmas_from_nodes',
)
