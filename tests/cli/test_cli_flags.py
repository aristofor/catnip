# FILE: tests/cli/test_cli_flags.py
"""Tests for CLI flags and options on the main catnip command."""

import os

import pytest
from click.testing import CliRunner

from catnip.cli.main import main


@pytest.fixture
def runner():
    return CliRunner()


# -- -c / --command -----------------------------------------------------------


def test_command_flag(runner):
    result = runner.invoke(main, ['-c', '2 + 3'])
    assert result.exit_code == 0
    assert '5' in result.output


def test_command_long_flag(runner):
    result = runner.invoke(main, ['--command', '"hello"'])
    assert result.exit_code == 0
    assert 'hello' in result.output


def test_command_multiple_statements(runner):
    result = runner.invoke(main, ['-c', 'x = 10\ny = 20\nx + y'])
    assert result.exit_code == 0
    assert '30' in result.output


def test_command_error_exits_nonzero(runner):
    result = runner.invoke(main, ['-c', '1 / 0'])
    assert result.exit_code == 1


# -- -q / --quiet -------------------------------------------------------------


def test_quiet_suppresses_output(runner):
    result = runner.invoke(main, ['-q', '-c', '42'])
    assert result.exit_code == 0
    assert '42' not in result.output


def test_quiet_long_flag(runner):
    result = runner.invoke(main, ['--quiet', '-c', '42'])
    assert result.exit_code == 0
    assert '42' not in result.output


def test_quiet_env_var(runner):
    result = runner.invoke(main, ['-c', '42'], env={'CATNIP_QUIET': '1'})
    assert result.exit_code == 0
    assert '42' not in result.output


# -- --no-color ---------------------------------------------------------------


def test_no_color_flag(runner):
    result = runner.invoke(main, ['--no-color', '-c', '"text"'])
    assert result.exit_code == 0
    assert '\x1b[' not in result.output


def test_no_color_env(runner):
    result = runner.invoke(main, ['-c', '"text"'], env={'NO_COLOR': '1'})
    assert result.exit_code == 0
    assert '\x1b[' not in result.output


# -- --no-cache ---------------------------------------------------------------


def test_no_cache_flag(runner):
    result = runner.invoke(main, ['--no-cache', '-c', '1 + 1'])
    assert result.exit_code == 0
    assert '2' in result.output


# -- -x / --executor ----------------------------------------------------------


def test_executor_vm(runner):
    result = runner.invoke(main, ['-x', 'vm', '-c', '3 * 7'])
    assert result.exit_code == 0
    assert '21' in result.output


def test_executor_ast(runner):
    result = runner.invoke(main, ['-x', 'ast', '-c', '3 * 7'])
    assert result.exit_code == 0
    assert '21' in result.output


def test_executor_invalid(runner):
    result = runner.invoke(main, ['-x', 'invalid', '-c', '1'])
    assert result.exit_code != 0


def test_executor_env_var(runner):
    result = runner.invoke(main, ['-c', '5 + 5'], env={'CATNIP_EXECUTOR': 'ast'})
    assert result.exit_code == 0
    assert '10' in result.output


# -- --parsing ----------------------------------------------------------------


def test_parsing_level_3_default(runner):
    """Level 3 executes and shows result (default)."""
    result = runner.invoke(main, ['-c', '1 + 2'])
    assert result.exit_code == 0
    assert '3' in result.output


def test_parsing_level_0(runner):
    """Level 0 shows parse tree."""
    result = runner.invoke(main, ['--parsing', '0', '-c', '1 + 2'])
    assert result.exit_code == 0
    # Parse tree output, not the evaluated result
    assert result.output.strip() != '3'


def test_parsing_level_1(runner):
    """Level 1 shows IR before semantic analysis."""
    result = runner.invoke(main, ['--parsing', '1', '-c', 'x = 1'])
    assert result.exit_code == 0
    assert result.output.strip() != ''


def test_parsing_level_2(runner):
    """Level 2 shows executable IR after semantic analysis."""
    result = runner.invoke(main, ['--parsing', '2', '-c', 'x = 1'])
    assert result.exit_code == 0
    assert result.output.strip() != ''


# -- -o / --optimize ----------------------------------------------------------


def test_optimize_tco_on(runner):
    result = runner.invoke(main, ['-o', 'tco:on', '-c', '1 + 1'])
    assert result.exit_code == 0
    assert '2' in result.output


def test_optimize_tco_off(runner):
    result = runner.invoke(main, ['-o', 'tco:off', '-c', '1 + 1'])
    assert result.exit_code == 0
    assert '2' in result.output


def test_optimize_tco_shorthand(runner):
    result = runner.invoke(main, ['-o', 'tco', '-c', '1 + 1'])
    assert result.exit_code == 0
    assert '2' in result.output


def test_optimize_level(runner):
    result = runner.invoke(main, ['-o', 'level:2', '-c', '1 + 1'])
    assert result.exit_code == 0
    assert '2' in result.output


def test_optimize_jit(runner):
    result = runner.invoke(main, ['-o', 'jit', '-c', '1 + 1'])
    assert result.exit_code == 0
    assert '2' in result.output


def test_optimize_multiple(runner):
    result = runner.invoke(main, ['-o', 'tco:on', '-o', 'level:1', '-c', '1 + 1'])
    assert result.exit_code == 0
    assert '2' in result.output


def test_optimize_env_var(runner):
    result = runner.invoke(main, ['-c', '2 + 2'], env={'CATNIP_OPTIMIZE': 'tco:off,level:1'})
    assert result.exit_code == 0
    assert '4' in result.output


# -- -m / --module -------------------------------------------------------------


def test_module_load(runner):
    result = runner.invoke(main, ['-m', 'math', '-c', 'math.sqrt(16)'])
    assert result.exit_code == 0
    assert '4' in result.output


def test_module_with_alias(runner):
    result = runner.invoke(main, ['-m', 'math:m', '-c', 'm.sqrt(9)'])
    assert result.exit_code == 0
    assert '3' in result.output


def test_module_wild_inject(runner):
    result = runner.invoke(main, ['-m', 'math:!', '-c', 'sqrt(25)'])
    assert result.exit_code == 0
    assert '5' in result.output


def test_module_multiple(runner):
    result = runner.invoke(main, ['-m', 'math', '-m', 'os', '-c', 'math.sqrt(4)'])
    assert result.exit_code == 0
    assert '2' in result.output


# -- --format -----------------------------------------------------------------


def test_format_text(runner):
    result = runner.invoke(main, ['--format', 'text', '-c', '1 + 2'])
    assert result.exit_code == 0
    assert '3' in result.output


def test_format_json(runner):
    result = runner.invoke(main, ['--format', 'json', '-c', '1 + 2'])
    assert result.exit_code == 0
    assert '3' in result.output


def test_format_repr(runner):
    result = runner.invoke(main, ['--format', 'repr', '-c', '1 + 2'])
    assert result.exit_code == 0
    assert '3' in result.output


def test_format_invalid(runner):
    result = runner.invoke(main, ['--format', 'xml', '-c', '1'])
    assert result.exit_code != 0


# -- --theme ------------------------------------------------------------------


def test_theme_dark(runner):
    result = runner.invoke(main, ['--theme', 'dark', '-c', '1 + 1'])
    assert result.exit_code == 0
    assert '2' in result.output


def test_theme_light(runner):
    result = runner.invoke(main, ['--theme', 'light', '-c', '1 + 1'])
    assert result.exit_code == 0
    assert '2' in result.output


def test_theme_invalid(runner):
    result = runner.invoke(main, ['--theme', 'neon', '-c', '1'])
    assert result.exit_code != 0


def test_theme_env_var(runner):
    result = runner.invoke(main, ['-c', '1 + 1'], env={'CATNIP_THEME': 'dark'})
    assert result.exit_code == 0
    assert '2' in result.output


# -- -v / --verbose -----------------------------------------------------------


def test_verbose_flag(runner):
    result = runner.invoke(main, ['-v', '-c', '1 + 1'])
    assert result.exit_code == 0


def test_verbose_long_flag(runner):
    result = runner.invoke(main, ['--verbose', '-c', '1 + 1'])
    assert result.exit_code == 0


# -- -V / --version -----------------------------------------------------------


def test_version_short(runner):
    result = runner.invoke(main, ['-V'])
    assert result.exit_code == 0
    assert 'Catnip' in result.output


def test_version_long(runner):
    result = runner.invoke(main, ['--version'])
    assert result.exit_code == 0
    assert 'Catnip' in result.output


def test_version_full(runner):
    result = runner.invoke(main, ['-V', '--full'])
    assert result.exit_code == 0
    assert 'Catnip' in result.output


# -- -h / --help --------------------------------------------------------------


def test_help_short(runner):
    result = runner.invoke(main, ['-h'])
    assert result.exit_code == 0
    assert 'Usage' in result.output


def test_help_long(runner):
    result = runner.invoke(main, ['--help'])
    assert result.exit_code == 0
    assert 'Usage' in result.output


def test_help_lists_options(runner):
    result = runner.invoke(main, ['--help'])
    for flag in ['-c', '-v', '-q', '-x', '-o', '-m', '-V']:
        assert flag in result.output, f"{flag} missing from help output"


# -- script mode --------------------------------------------------------------


def test_script_file(runner, tmp_path):
    f = tmp_path / "test.cat"
    f.write_text("40 + 2\n")
    result = runner.invoke(main, [str(f)])
    assert result.exit_code == 0
    assert '42' in result.output


def test_script_file_not_found(runner):
    result = runner.invoke(main, ['/nonexistent/script.cat'])
    assert result.exit_code == 1


def test_script_with_flags(runner, tmp_path):
    f = tmp_path / "test.cat"
    f.write_text("1 + 1\n")
    result = runner.invoke(main, ['-x', 'ast', '--no-cache', str(f)])
    assert result.exit_code == 0
    assert '2' in result.output


def test_script_shebang(runner, tmp_path):
    f = tmp_path / "test.cat"
    f.write_text("#!/usr/bin/env catnip\n40 + 2\n")
    result = runner.invoke(main, [str(f)])
    assert result.exit_code == 0
    assert '42' in result.output


# -- pipe mode ----------------------------------------------------------------


def test_pipe_stdin(runner):
    result = runner.invoke(main, [], input="7 * 6\n")
    assert result.exit_code == 0
    assert '42' in result.output


def test_pipe_stdin_with_flags(runner):
    result = runner.invoke(main, ['-x', 'ast'], input="3 + 3\n")
    assert result.exit_code == 0
    assert '6' in result.output


# -- flag combinations --------------------------------------------------------


def test_quiet_and_verbose(runner):
    """--quiet suppresses final result but --verbose still shows pipeline."""
    result = runner.invoke(main, ['-q', '-v', '-c', '1 + 1'])
    assert result.exit_code == 0
    # Verbose pipeline stages are shown, but final [RESULT] line is suppressed
    assert '[INPUT]' in result.output or '[IR' in result.output


def test_all_flags_together(runner):
    result = runner.invoke(
        main,
        [
            '-x',
            'vm',
            '-o',
            'tco:on',
            '--no-cache',
            '--no-color',
            '--format',
            'text',
            '-c',
            '10 + 10',
        ],
    )
    assert result.exit_code == 0
    assert '20' in result.output
