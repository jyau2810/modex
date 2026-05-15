import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from codex_multi_account.cli import watcher_main_args


class CliArgTests(unittest.TestCase):
    def test_watcher_wrapper_keeps_global_config_before_subcommand(self):
        self.assertEqual(
            watcher_main_args(["--config", "/tmp/cx.toml", "--once"]),
            ["--config", "/tmp/cx.toml", "watch", "--once"],
        )

    def test_watcher_wrapper_allows_plain_watch_args(self):
        self.assertEqual(
            watcher_main_args(["--once", "--dry-run-notify"]),
            ["watch", "--once", "--dry-run-notify"],
        )


if __name__ == "__main__":
    unittest.main()
