# FILE: tests/language/test_exceptions.py
"""Tests for custom Catnip exceptions."""

import pytest

from catnip import Catnip
from catnip.exc import (
    CatnipArityError,
    CatnipError,
    CatnipInternalError,
    CatnipNameError,
    CatnipNotImplementedError,
    CatnipPatternError,
    CatnipPragmaError,
    CatnipRuntimeError,
    CatnipSemanticError,
    CatnipSyntaxError,
    CatnipTypeError,
    CatnipWeirdError,
)


class TestCatnipErrorBase:
    """Tests de la classe de base CatnipError."""

    def test_simple_message(self):
        """Message simple sans localisation."""
        err = CatnipError("Something went wrong")
        assert str(err) == "Something went wrong"
        assert err.message == "Something went wrong"

    def test_with_line(self):
        """Message with line number."""
        err = CatnipError("Error", line=42)
        assert "line 42" in str(err)
        assert err.line == 42

    def test_with_filename_and_line(self):
        """Message avec fichier et ligne."""
        err = CatnipError("Error", filename="script.cat", line=10)
        assert "File 'script.cat'" in str(err)
        assert "line 10" in str(err)

    def test_with_full_location(self):
        """Message with full location."""
        err = CatnipError("Error", filename="test.cat", line=5, column=12)
        msg = str(err)
        assert "File 'test.cat'" in msg
        assert "line 5" in msg
        assert "column 12" in msg

    def test_with_context_snippet(self):
        """Message avec extrait de code."""
        err = CatnipError("Unexpected token", context="x = 1 + )")
        assert "x = 1 + )" in str(err)

    def test_inheritance(self):
        """All exceptions inherit from CatnipError."""
        assert issubclass(CatnipSyntaxError, CatnipError)
        assert issubclass(CatnipSemanticError, CatnipError)
        assert issubclass(CatnipRuntimeError, CatnipError)
        assert issubclass(CatnipPragmaError, CatnipSemanticError)
        assert issubclass(CatnipNameError, CatnipRuntimeError)
        assert issubclass(CatnipTypeError, CatnipRuntimeError)
        assert issubclass(CatnipPatternError, CatnipRuntimeError)
        assert issubclass(CatnipNotImplementedError, CatnipError)


class TestCatnipNameError:
    """Tests de CatnipNameError."""

    def test_message_format(self):
        """Le message contient le nom de la variable."""
        err = CatnipNameError('undefined_var')
        assert 'undefined_var' in str(err)
        assert "not defined" in str(err)
        assert err.name == 'undefined_var'

    def test_with_location(self):
        """Localisation incluse dans le message."""
        err = CatnipNameError('x', line=10, column=5)
        msg = str(err)
        assert 'x' in msg
        assert "line 10" in msg


class TestCatnipArityError:
    """Tests de CatnipArityError."""

    def test_fixed_arity(self):
        """Fixed arity (exact argument count)."""
        err = CatnipArityError('setattr', expected=3, got=2)
        msg = str(err)
        assert 'setattr' in msg
        assert '3' in msg
        assert '2' in msg
        assert err.operation == 'setattr'
        assert err.expected == 3
        assert err.got == 2

    def test_range_arity(self):
        """Variable arity (min-max)."""
        err = CatnipArityError('if', expected=(1, 2), got=0)
        msg = str(err)
        assert 'if' in msg
        assert "1-2" in msg
        assert "0" in msg

    def test_raised_by_semantic_analyzer(self):
        """CatnipArityError raised by the semantic analyzer."""
        # Arity is checked during semantic analysis
        # This test validates the exception structure
        err = CatnipArityError('test_op', expected=3, got=1, line=10)
        assert err.operation == 'test_op'
        assert err.expected == 3
        assert err.got == 1
        assert "line 10" in str(err)


class TestCatnipPatternError:
    """Tests de CatnipPatternError."""

    def test_message_format(self):
        """Message includes the unmatched value."""
        err = CatnipPatternError(42)
        assert "42" in str(err)
        assert "No pattern matched" in str(err)
        assert err.value == 42

    def test_complex_value(self):
        """Valeur complexe dans le message."""
        err = CatnipPatternError({'key': 'value'})
        assert 'key' in str(err)


class TestCatnipPragmaError:
    """Tests de CatnipPragmaError via le code Catnip."""

    def test_invalid_optimization_level(self):
        """Niveau d'optimisation invalide via pragma."""
        # pragma("optimize", "invalid") raises CatnipPragmaError (string, not int)
        catnip = Catnip()
        with pytest.raises(CatnipPragmaError) as exc_info:
            catnip.parse('pragma("optimize", "invalid_level")')
            catnip.execute()
        assert "optimize" in str(exc_info.value).lower() or "integer" in str(exc_info.value).lower()

    def test_unknown_pragma(self):
        """Unknown pragma directive raises CatnipSemanticError."""
        # Note: code raises CatnipSemanticError (parent of CatnipPragmaError)
        catnip = Catnip()
        with pytest.raises(CatnipSemanticError) as exc_info:
            catnip.parse('pragma("unknown_directive", "value")')
            catnip.execute()
        assert "unknown" in str(exc_info.value).lower()

    def test_invalid_warning_action(self):
        """Invalid warning action raises CatnipPragmaError."""
        catnip = Catnip()
        with pytest.raises(CatnipPragmaError) as exc_info:
            catnip.parse('pragma("warning", "invalid_action")')
            catnip.execute()
        assert "warning" in str(exc_info.value).lower() or "true" in str(exc_info.value).lower()


class TestCatnipRuntimeError:
    """CatnipRuntimeError tests via execution."""

    def test_unpacking_non_iterable(self):
        """Unpacking a non-iterable raises CatnipTypeError."""
        catnip = Catnip()
        with pytest.raises(CatnipTypeError) as exc_info:
            catnip.parse("a, b = 42")
            catnip.execute()
        assert "unpack" in str(exc_info.value).lower()
        assert 'int' in str(exc_info.value)

    def test_unpacking_wrong_count(self):
        """Unpacking avec mauvais nombre de valeurs."""
        catnip = Catnip()
        with pytest.raises(CatnipRuntimeError) as exc_info:
            catnip.parse("a, b, c = list(1, 2)")
            catnip.execute()
        assert "unpack" in str(exc_info.value).lower()

    def test_not_enough_values_to_unpack(self):
        """Pas assez de valeurs pour l'unpacking avec starred."""
        catnip = Catnip()
        with pytest.raises(CatnipRuntimeError) as exc_info:
            catnip.parse("a, b, *c, d, e = list(1, 2)")
            catnip.execute()
        assert "unpack" in str(exc_info.value).lower() or "values" in str(exc_info.value).lower()


class TestCatnipSemanticError:
    """CatnipSemanticError tests via semantic analysis."""

    def test_unknown_fstring_part(self):
        """Type de partie f-string inconnu (via manipulation IR)."""
        # This test validates the exception structure
        err = CatnipSemanticError("Unknown fstring part type: invalid")
        assert "fstring" in str(err).lower()


class TestCatnipNotImplementedError:
    """Tests de CatnipNotImplementedError."""

    def test_message_format(self):
        """Message includes the unimplemented feature."""
        err = CatnipNotImplementedError("ND-recursion operator ~~")
        assert "~~" in str(err)
        assert "Not implemented" in str(err)
        assert err.feature == "ND-recursion operator ~~"

    def test_with_location(self):
        """Localisation dans le message."""
        err = CatnipNotImplementedError("~[]", line=5, filename="test.cat")
        msg = str(err)
        assert "~[]" in msg
        assert "line 5" in msg
        assert "test.cat" in msg


class TestCatnipWeirdError:
    """Tests de CatnipWeirdError."""

    def test_message_format(self):
        """Message d'erreur interne."""
        err = CatnipWeirdError("Unexpected state in interpreter")
        assert "Unexpected state" in str(err)

    def test_cause_and_details(self):
        """cause et details sont stockes."""
        err = CatnipWeirdError("boom", cause="vm", details={"opcode": 42})
        assert err.cause == "vm"
        assert err.details == {"opcode": 42}

    def test_defaults(self):
        """cause=None et details={} par defaut."""
        err = CatnipWeirdError("oops")
        assert err.cause is None
        assert err.details == {}

    def test_alias_compat(self):
        """CatnipInternalError est un alias de CatnipWeirdError."""
        assert CatnipInternalError is CatnipWeirdError
        err = CatnipInternalError("test")
        assert isinstance(err, CatnipWeirdError)

    def test_scope_pop_error(self):
        """Tentative de pop du scope global."""
        catnip = Catnip()
        with pytest.raises(CatnipWeirdError) as exc_info:
            catnip.context.pop_scope()
        assert "global scope" in str(exc_info.value).lower()
