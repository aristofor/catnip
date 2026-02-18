# FILE: tests/config/test_config_sections.py
"""Test configuration sections (repl, optimize, format)."""

import tempfile
from pathlib import Path
from textwrap import dedent

from catnip.config import ConfigManager, ConfigSource


class TestConfigSections:
    """Test suite for sectioned configuration."""

    def test_load_sectioned_config(self):
        """Test loading config with [repl], [optimize], [format] sections."""
        with tempfile.NamedTemporaryFile(mode='w', suffix='.toml', delete=False) as f:
            f.write(dedent("""\
                [repl]
                no_color = true

                [optimize]
                jit = true
                tco = false
                optimize = 1
                executor = "ast"

                [format]
                indent_size = 2
                line_length = 100
                """))
            config_path = Path(f.name)

        try:
            mgr = ConfigManager()
            mgr.load_file(config_path)

            # Vérifier valeurs [repl]
            assert mgr.get('no_color') is True

            # Vérifier valeurs [optimize]
            assert mgr.get('jit') is True
            assert mgr.get('tco') is False
            assert mgr.get('optimize') == 1
            assert mgr.get('executor') == 'ast'

            # Vérifier valeurs [format]
            format_config = mgr.get_format_config()
            assert format_config.indent_size == 2
            assert format_config.line_length == 100

            # Vérifier sources
            assert mgr.get_with_source('no_color').source == ConfigSource.FILE
            assert mgr.get_with_source('jit').source == ConfigSource.FILE
        finally:
            config_path.unlink()

    def test_save_preserves_sections(self):
        """Test that save_config writes sections."""
        with tempfile.NamedTemporaryFile(mode='w', suffix='.toml', delete=False) as f:
            f.write(dedent("""\
                [format]
                indent_size = 8
                """))
            config_path = Path(f.name)

        try:
            from catnip.config import save_config

            # Load, modify, save
            mgr = ConfigManager()
            mgr.load_file(config_path)
            config = mgr.items()
            config['jit'] = True

            save_config(config, config_path)

            # Lire le fichier et vérifier structure
            content = config_path.read_text()
            assert '[repl]' in content
            assert '[optimize]' in content
            assert '[format]' in content
            assert 'indent_size = 8' in content  # Préservé
        finally:
            config_path.unlink()

    def test_default_values_organized_by_section(self):
        """Test that defaults are correctly organized."""
        mgr = ConfigManager()

        # Defaults should be loaded
        assert mgr.get('no_color') is False  # From [repl]
        assert mgr.get('jit') is False  # From [optimize]
        assert mgr.get('tco') is True  # From [optimize]
        assert mgr.get('optimize') == 3  # From [optimize]
        assert mgr.get('executor') == 'vm'  # From [optimize]

        format_config = mgr.get_format_config()
        assert format_config.indent_size == 4  # From [format]
        assert format_config.line_length == 120  # From [format]

    def test_env_vars_override_sections(self):
        """Test that env vars still override file sections."""
        import os

        with tempfile.NamedTemporaryFile(mode='w', suffix='.toml', delete=False) as f:
            f.write(dedent("""\
                [optimize]
                jit = false
                """))
            config_path = Path(f.name)

        try:
            # Set env var
            old_value = os.environ.get('CATNIP_OPTIMIZE')
            os.environ['CATNIP_OPTIMIZE'] = 'jit:on'

            mgr = ConfigManager()
            mgr.load_file(config_path)
            mgr.load_env()

            # Env var should override file
            assert mgr.get('jit') is True
            assert mgr.get_with_source('jit').source == ConfigSource.ENV
        finally:
            # Restore env
            if old_value is None:
                os.environ.pop('CATNIP_OPTIMIZE', None)
            else:
                os.environ['CATNIP_OPTIMIZE'] = old_value
            config_path.unlink()
