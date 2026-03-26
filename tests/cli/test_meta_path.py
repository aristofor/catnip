# FILE: tests/cli/test_meta_path.py
"""CLI tests for META metadata in script mode."""

from pathlib import Path

from click.testing import CliRunner

from catnip.cli.main import main


class TestMetaPath:
    """Validate META.file is populated for CLI script execution."""

    def test_script_mode_sets_meta_path(self):
        runner = CliRunner()

        with runner.isolated_filesystem():
            script_path = Path("meta_path.cat")
            script_path.write_text('META.file.endswith("meta_path.cat")')

            result = runner.invoke(main, ["--no-cache", str(script_path)])

            assert result.exit_code == 0
            assert result.output.strip() == "True"
