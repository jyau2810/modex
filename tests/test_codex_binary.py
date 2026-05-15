import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from codex_multi_account.codex_binary import resolve_codex_binary


class CodexBinaryTests(unittest.TestCase):
    def test_uses_codex_app_cli_when_codex_is_not_on_path(self):
        with tempfile.TemporaryDirectory() as tmp:
            app_cli = Path(tmp) / "Codex.app" / "Contents" / "Resources" / "codex"
            app_cli.parent.mkdir(parents=True)
            app_cli.write_text("#!/bin/sh\n")

            resolved = resolve_codex_binary(
                "codex",
                which=lambda _name: None,
                app_cli_paths=[app_cli],
            )

            self.assertEqual(resolved, str(app_cli))

    def test_prefers_path_lookup_when_available(self):
        resolved = resolve_codex_binary(
            "codex",
            which=lambda name: f"/opt/bin/{name}",
            app_cli_paths=[],
        )

        self.assertEqual(resolved, "/opt/bin/codex")

    def test_keeps_explicit_path(self):
        resolved = resolve_codex_binary(
            "/custom/codex",
            which=lambda _name: None,
            app_cli_paths=[],
        )

        self.assertEqual(resolved, "/custom/codex")


if __name__ == "__main__":
    unittest.main()
