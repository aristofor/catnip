# FILE: catnip/exc.py

__all__ = (
    'CatnipArityError',
    'CatnipError',
    'CatnipInternalError',
    'CatnipNameError',
    'CatnipNotImplementedError',
    'CatnipPatternError',
    'CatnipPragmaError',
    'CatnipRuntimeError',
    'CatnipSemanticError',
    'CatnipSyntaxError',
    'CatnipTypeError',
)


class CatnipError(Exception):
    """
    Base exception for all Catnip errors.

    Supports location information (filename, line, column).
    """

    def __init__(
        self, message, *, filename=None, line=None, column=None, context=None, traceback=None, suggestions=None
    ):
        self.message = message
        self.filename = filename
        self.line = line
        self.column = column
        self.context = context  # Optional code snippet
        self.traceback = traceback  # CatnipTraceback or None
        self.suggestions = suggestions or []  # List of name suggestions
        super().__init__(self._format_message())

    def _format_message(self):
        """Format message with location info."""
        parts = []

        if self.filename:
            parts.append(f"File {self.filename!r}")
        if self.line is not None:
            parts.append(f"line {self.line}")
        if self.column is not None:
            parts.append(f"column {self.column}")

        location = ", ".join(parts)
        if location:
            msg = f"{location}: {self.message}"
        else:
            msg = self.message

        if self.context:
            msg += f"\n  {self.context}"

        return msg


class CatnipInternalError(CatnipError):
    """
    Internal interpreter error.

    Indicates a bug in Catnip, not a user error.
    Should be reported as a bug.
    """


class CatnipSyntaxError(CatnipError):
    """
    Syntax error during parsing.

    Raised by Tree-sitter when grammar rules are violated.
    """


class CatnipSemanticError(CatnipError):
    """
    Semantic analysis error.

    Raised when code is syntactically correct but semantically invalid:
    - unknown operation
    - wrong number of arguments
    - invalid structure
    """


class CatnipRuntimeError(CatnipError):
    """
    Runtime error.

    Raised during code execution:
    - division by zero
    - undefined variable
    - incompatible type
    - failed pattern matching
    """


class CatnipPragmaError(CatnipSemanticError):
    """
    Pragma directive error.

    Raised when a pragma is malformed or has an invalid value.
    """


class CatnipNameError(CatnipRuntimeError):
    """
    Undefined variable or name.

    Catnip equivalent of Python's NameError.
    Includes "did you mean?" suggestions when available.
    """

    def __init__(self, name, *, suggestions=None, filename=None, line=None, column=None, context=None, traceback=None):
        self.name = name
        message = f"Name {name!r} is not defined"
        if suggestions:
            if len(suggestions) == 1:
                message += f"\n  Did you mean '{suggestions[0]}'?"
            else:
                quoted = ', '.join(f"'{s}'" for s in suggestions)
                message += f"\n  Did you mean one of: {quoted}?"
        super().__init__(
            message,
            filename=filename,
            line=line,
            column=column,
            context=context,
            traceback=traceback,
            suggestions=suggestions,
        )


class CatnipTypeError(CatnipRuntimeError):
    """
    Operation incompatible with type.

    Catnip equivalent of Python's TypeError.
    """


class CatnipArityError(CatnipSemanticError):
    """
    Wrong number of arguments.

    Raised when an operation receives an incorrect argument count.
    """

    def __init__(self, operation, expected, got, *, filename=None, line=None, column=None):
        if isinstance(expected, int):
            exp_str = str(expected)
        elif isinstance(expected, tuple) and len(expected) == 2:
            exp_str = f"{expected[0]}-{expected[1]}"
        else:
            exp_str = str(expected)

        message = f"{operation} requires {exp_str} argument(s), got {got}"
        super().__init__(message, filename=filename, line=line, column=column)
        self.operation = operation
        self.expected = expected
        self.got = got


class CatnipPatternError(CatnipRuntimeError):
    """
    Pattern matching failure.

    Raised when no pattern matches in a match/case without a default clause.
    """

    def __init__(self, value, *, filename=None, line=None, column=None):
        message = f"No pattern matched for value: {value!r}"
        super().__init__(message, filename=filename, line=line, column=column)
        self.value = value


class CatnipNotImplementedError(CatnipError):
    """
    Feature not yet implemented.

    Raised for planned features that are not yet available.
    Includes ND-recursion operators (@@, @>, @[]).
    """

    def __init__(self, feature, *, filename=None, line=None, column=None):
        message = f"Not implemented: {feature}"
        super().__init__(message, filename=filename, line=line, column=column)
        self.feature = feature
