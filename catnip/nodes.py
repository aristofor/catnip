# FILE: catnip/nodes.py
# Import Op, Pattern classes, and Node classes from Rust implementation
from catnip._rs import (
    Op,
    PatternLiteral,
    PatternOr,
    PatternStruct,
    PatternTuple,
    PatternVar,
    PatternWildcard,
    Ref,
    TailCall,
)


class ReturnValue(Exception):
    """Exception used to implement early return from functions/lambdas."""

    def __init__(self, value=None):
        self.value = value
        super().__init__()


class BreakLoop(Exception):
    """Exception used to implement break statement in loops."""

    pass


class ContinueLoop(Exception):
    """Exception used to implement continue statement in loops."""

    pass


# Ref and TailCall are now implemented in Rust (imported above)
# Kept here for documentation:
# - Ref: Represents a reference to an identifier in the AST
# - TailCall: Signal value to trigger a tail-call jump for O(1) stack space


# Pattern classes are now implemented in Rust (imported above)
# Kept here for documentation:
# - PatternLiteral: Pattern that matches a literal value
# - PatternVar: Pattern that captures the matched value into a variable
# - PatternWildcard: Pattern that matches anything without capturing
# - PatternOr: Pattern that matches any of multiple patterns
# - PatternTuple: Pattern that matches and destructures tuples/lists


class Broadcast:
    """
    Represents a broadcasting operation: target.[op operand]

    Examples:
        data.[* 2]       -> multiply each element by 2
        data.[+ 10]      -> add 10 to each element
        data.[> 0]       -> test if each element > 0 (map to booleans)
        data.[if > 0]    -> filter elements > 0 (keep only matching)
        data.[(x) => { x * 2 }]  -> apply lambda to each element
    """

    __slots__ = ("target", "operator", "operand", "is_filter")

    def __init__(self, target, operator, operand=None, is_filter=False):
        self.target = target  # The object to broadcast over
        self.operator = operator  # The operation ('+', '*', 'abs', lambda, etc.)
        self.operand = operand  # The operand (optional for unary ops)
        self.is_filter = is_filter  # If True, filter elements instead of mapping

    def __repr__(self):
        filter_prefix = "if " if self.is_filter else ""
        if self.operand is not None:
            return f"<Broadcast {self.target!r}.[{filter_prefix}{self.operator} {self.operand!r}]>"
        else:
            return f"<Broadcast {self.target!r}.[{filter_prefix}{self.operator}]>"

    def __eq__(self, other):
        if not isinstance(other, Broadcast):
            return NotImplemented
        return (
            self.target == other.target
            and self.operator == other.operator
            and self.operand == other.operand
            and self.is_filter == other.is_filter
        )

    def __hash__(self):
        return hash((self.target, self.operator, self.operand, self.is_filter))


# Import Rust implementations (required)
# Import here (after all classes are defined) to avoid circular import issues
from ._rs import Function, Lambda
