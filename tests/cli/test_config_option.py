# FILE: tests/cli/test_config_option.py
"""Test --config option for alternate config file."""

import tempfile
from pathlib import Path
from textwrap import dedent

from click.testing import CliRunner

from catnip.cli.main import main


class TestConfigOption:
    """Test suite for --config option."""

    def test_config_show_with_custom_file(self):
        """Test config show with --config option."""
        runner = CliRunner()

        with tempfile.NamedTemporaryFile(mode='w', suffix='.toml', delete=False) as f:
            f.write(dedent("""\
                jit = true
                tco = false
                optimize = 1
                """))
            config_path = f.name

        try:
            result = runner.invoke(main, ['--config', config_path, 'config', 'show'])
            assert result.exit_code == 0
            assert 'jit = true' in result.output
            assert 'tco = false' in result.output
            assert 'optimize = 1' in result.output
            assert config_path in result.output
        finally:
            Path(config_path).unlink()

    def test_config_show_debug_with_custom_file(self):
        """Test config show --debug with --config option."""
        runner = CliRunner()

        with tempfile.NamedTemporaryFile(mode='w', suffix='.toml', delete=False) as f:
            f.write('jit = true\n')
            config_path = f.name

        try:
            result = runner.invoke(main, ['--config', config_path, 'config', 'show', '--debug'])
            assert result.exit_code == 0
            assert f'Configuration from: {config_path}' in result.output
            assert 'jit: True  [file' in result.output
        finally:
            Path(config_path).unlink()

    def test_config_get_with_custom_file(self):
        """Test config get with --config option."""
        runner = CliRunner()

        with tempfile.NamedTemporaryFile(mode='w', suffix='.toml', delete=False) as f:
            f.write('jit = true\n')
            config_path = f.name

        try:
            result = runner.invoke(main, ['--config', config_path, 'config', 'get', 'jit'])
            assert result.exit_code == 0
            assert result.output.strip() == 'true'
        finally:
            Path(config_path).unlink()

    def test_config_set_with_custom_file(self):
        """Test config set with --config option."""
        runner = CliRunner()

        with tempfile.NamedTemporaryFile(mode='w', suffix='.toml', delete=False) as f:
            f.write('jit = false\n')
            config_path = f.name

        try:
            # Modifier la valeur
            result = runner.invoke(main, ['--config', config_path, 'config', 'set', 'jit', 'true'])
            assert result.exit_code == 0
            assert 'Set jit = true' in result.output

            # Vérifier que la modification a été sauvegardée
            result = runner.invoke(main, ['--config', config_path, 'config', 'get', 'jit'])
            assert result.exit_code == 0
            assert result.output.strip() == 'true'
        finally:
            Path(config_path).unlink()

    def test_config_path_with_custom_file(self):
        """Test config path with --config option."""
        runner = CliRunner()

        with tempfile.NamedTemporaryFile(mode='w', suffix='.toml', delete=False) as f:
            f.write('jit = false\n')
            config_path = f.name

        try:
            result = runner.invoke(main, ['--config', config_path, 'config', 'path'])
            assert result.exit_code == 0
            assert result.output.strip() == config_path
        finally:
            Path(config_path).unlink()

    def test_format_with_custom_config(self):
        """Test format command with --config option."""
        runner = CliRunner()

        with tempfile.NamedTemporaryFile(mode='w', suffix='.toml', delete=False) as f:
            f.write(dedent("""\
                [format]
                indent_size = 8
                """))
            config_path = f.name

        try:
            # Format avec indent custom
            result = runner.invoke(main, ['--config', config_path, 'format', '--stdin'], input='{\nx=1\n}')
            assert result.exit_code == 0
            # Vérifier indentation de 8 espaces
            assert '        x = 1' in result.output
        finally:
            Path(config_path).unlink()

    def test_format_preserves_format_section(self):
        """Test that config set preserves [format] section."""
        runner = CliRunner()

        with tempfile.NamedTemporaryFile(mode='w', suffix='.toml', delete=False) as f:
            f.write(dedent("""\
                jit = false

                [format]
                indent_size = 2
                line_length = 100
                """))
            config_path = f.name

        try:
            # Modifier une clé principale
            result = runner.invoke(main, ['--config', config_path, 'config', 'set', 'jit', 'true'])
            assert result.exit_code == 0

            # Vérifier que [format] est préservé
            content = Path(config_path).read_text()
            assert '[format]' in content
            assert 'indent_size = 2' in content
            assert 'line_length = 100' in content
        finally:
            Path(config_path).unlink()

    def test_config_without_option_uses_default(self):
        """Test that without --config, default path is used."""
        runner = CliRunner()

        result = runner.invoke(main, ['config', 'path'])
        assert result.exit_code == 0
        # Devrait afficher le chemin par défaut
        assert '.config/catnip/catnip.toml' in result.output
