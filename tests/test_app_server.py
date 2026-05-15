import os
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from codex_multi_account.app_server import build_codex_env


class AppServerTests(unittest.TestCase):
    def test_build_codex_env_sets_identity_home_without_mutating_input(self):
        original = {"PATH": "/bin", "CODEX_HOME": "/old"}
        env = build_codex_env(Path("/tmp/custom-codex-home"), original)

        self.assertEqual(env["CODEX_HOME"], "/tmp/custom-codex-home")
        self.assertEqual(original["CODEX_HOME"], "/old")
        self.assertEqual(env["PATH"], "/bin")


if __name__ == "__main__":
    unittest.main()
