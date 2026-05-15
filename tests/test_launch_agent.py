import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from codex_multi_account.launch_agent import render_launch_agent


class LaunchAgentTests(unittest.TestCase):
    def test_launch_agent_uses_config_path_and_script_path(self):
        rendered = render_launch_agent(
            label="com.example.codex-watch",
            script_path=Path("/opt/cx/bin/codex-quota-watch"),
            config_path=Path("/Users/me/.config/cx/config.toml"),
        )

        self.assertIn("<string>com.example.codex-watch</string>", rendered)
        self.assertIn("<string>/opt/cx/bin/codex-quota-watch</string>", rendered)
        self.assertIn("<key>CX_CONFIG</key>", rendered)
        self.assertIn("<string>/Users/me/.config/cx/config.toml</string>", rendered)


if __name__ == "__main__":
    unittest.main()
