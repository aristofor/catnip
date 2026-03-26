# FILE: tests/cli/test_cli_commands.py
"""Tests for Catnip CLI commands using Click's CliRunner."""

from click.testing import CliRunner

from catnip.cli.main import main

# -- version ------------------------------------------------------------------


def test_version():
    runner = CliRunner()
    result = runner.invoke(main, ['--version'])
    assert result.exit_code == 0
    assert result.output.startswith('Catnip ')


# -- format -------------------------------------------------------------------


def test_format_check_clean(tmp_path):
    f = tmp_path / "test.cat"
    f.write_text("x = 1 + 2\n")
    runner = CliRunner()
    result = runner.invoke(main, ['format', '--check', str(f)])
    assert result.exit_code == 0


def test_format_check_dirty(tmp_path):
    f = tmp_path / "test.cat"
    f.write_text("x=1+2\n")
    runner = CliRunner()
    result = runner.invoke(main, ['format', '--check', str(f)])
    assert result.exit_code == 1
    assert "not formatted" in result.output


def test_format_stdin():
    runner = CliRunner()
    result = runner.invoke(main, ['format', '--stdin'], input="x=1+2\n")
    assert result.exit_code == 0
    assert result.output == "x = 1 + 2\n"


def test_format_stdin_already_clean():
    runner = CliRunner()
    result = runner.invoke(main, ['format', '--stdin'], input="x = 1 + 2\n")
    assert result.exit_code == 0
    assert result.output == "x = 1 + 2\n"


def test_format_file_output(tmp_path):
    f = tmp_path / "test.cat"
    f.write_text("x=1+2\n")
    runner = CliRunner()
    result = runner.invoke(main, ['format', str(f)])
    assert result.exit_code == 0
    assert "x = 1 + 2" in result.output


def test_format_file_not_found():
    runner = CliRunner()
    result = runner.invoke(main, ['format', '--check', '/nonexistent/file.cat'])
    assert result.exit_code == 1
    assert "not found" in result.output.lower()


# -- lint ---------------------------------------------------------------------


def test_lint_clean_file(tmp_path):
    f = tmp_path / "test.cat"
    f.write_text("x = 1 + 2\n")
    runner = CliRunner()
    result = runner.invoke(main, ['lint', str(f)])
    assert result.exit_code == 0


def test_lint_syntax_only(tmp_path):
    f = tmp_path / "test.cat"
    f.write_text("x = 1 + 2\n")
    runner = CliRunner()
    result = runner.invoke(main, ['lint', '-l', 'syntax', str(f)])
    assert result.exit_code == 0


def test_lint_stdin_clean():
    runner = CliRunner()
    result = runner.invoke(main, ['lint', '--stdin'], input="x = 1 + 2\n")
    assert result.exit_code == 0


def test_lint_stdin_syntax_error():
    runner = CliRunner()
    result = runner.invoke(main, ['lint', '--stdin'], input="x = (\n")
    assert result.exit_code == 1
    assert "error" in result.output.lower()


def test_lint_file_not_found():
    runner = CliRunner()
    result = runner.invoke(main, ['lint', '/nonexistent/file.cat'])
    assert result.exit_code == 1
    assert "not found" in result.output.lower()


# -- commands -----------------------------------------------------------------


def test_commands_lists_builtins():
    runner = CliRunner()
    result = runner.invoke(main, ['commands'])
    assert result.exit_code == 0
    assert "format" in result.output
    assert "lint" in result.output


# -- config -------------------------------------------------------------------


def test_config_show():
    runner = CliRunner()
    result = runner.invoke(main, ['config', 'show'])
    assert result.exit_code == 0
    assert "executor" in result.output


def test_config_path():
    runner = CliRunner()
    result = runner.invoke(main, ['config', 'path'])
    assert result.exit_code == 0
    assert "catnip" in result.output
    assert result.output.strip().endswith(".toml")
