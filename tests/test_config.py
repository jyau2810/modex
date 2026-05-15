import os
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from codex_multi_account.config import load_config


class ConfigTests(unittest.TestCase):
    def test_loads_configurable_identity_homes(self):
        with tempfile.TemporaryDirectory() as tmp:
            config_path = Path(tmp) / "config.toml"
            config_path.write_text(
                textwrap.dedent(
                    """
                    [codex]
                    binary = "/usr/local/bin/codex"
                    poll_seconds = 45

                    [identities.alpha]
                    codex_home = "~/custom-codex-alpha"
                    monitor = true
                    workspace_id = "workspace-alpha"

                    [identities.beta]
                    codex_home = "$HOME/custom-codex-beta"
                    monitor = false

                    """
                ).strip()
            )

            config = load_config(config_path)

            self.assertEqual(config.codex.binary, "/usr/local/bin/codex")
            self.assertEqual(config.codex.poll_seconds, 45)
            self.assertEqual(
                config.identities["alpha"].codex_home,
                Path.home() / "custom-codex-alpha",
            )
            self.assertEqual(
                config.identities["beta"].codex_home,
                Path(os.environ["HOME"]) / "custom-codex-beta",
            )
            self.assertTrue(config.identities["alpha"].monitor)
            self.assertFalse(config.identities["beta"].monitor)

    def test_rejects_missing_identity_home(self):
        with tempfile.TemporaryDirectory() as tmp:
            config_path = Path(tmp) / "config.toml"
            config_path.write_text(
                textwrap.dedent(
                    """
                    [identities.bad]
                    monitor = true
                    """
                ).strip()
            )

            with self.assertRaisesRegex(ValueError, "codex_home"):
                load_config(config_path)


if __name__ == "__main__":
    unittest.main()
