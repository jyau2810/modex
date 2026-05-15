import tempfile
import textwrap
import unittest
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from codex_multi_account.sync import clean_identity_home, sync_identity_auth, sync_identity_config


class SyncTests(unittest.TestCase):
    def test_syncs_non_secret_config_without_copying_auth(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source_home = root / "source"
            identity_home = root / "identity"
            source_home.mkdir()
            (source_home / "auth.json").write_text('{"secret": true}')
            (source_home / "config.toml").write_text(
                textwrap.dedent(
                    """
                    model = "gpt-5.5"

                    [projects."/tmp/project"]
                    trust_level = "trusted"
                    """
                ).strip()
            )

            sync_identity_config(
                source_home=source_home,
                identity_home=identity_home,
                workspace_id="workspace-123",
            )

            rendered = (identity_home / "config.toml").read_text()
            self.assertIn('model = "gpt-5.5"', rendered)
            self.assertIn('forced_login_method = "chatgpt"', rendered)
            self.assertIn('forced_chatgpt_workspace_id = "workspace-123"', rendered)
            self.assertFalse((identity_home / "auth.json").exists())

    def test_sync_drops_mcp_env_secrets(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source_home = root / "source"
            identity_home = root / "identity"
            source_home.mkdir()
            (source_home / "config.toml").write_text(
                textwrap.dedent(
                    """
                    [mcp_servers.yuque]
                    command = "npx"

                    [mcp_servers.yuque.env]
                    YUQUE_GROUP_TOKEN = "secret"
                    NORMAL_SETTING = "also-not-copied"

                    [projects."/tmp/project"]
                    trust_level = "trusted"
                    """
                ).strip()
            )

            sync_identity_config(source_home=source_home, identity_home=identity_home)

            rendered = (identity_home / "config.toml").read_text()
            self.assertIn("[mcp_servers.yuque]", rendered)
            self.assertNotIn("YUQUE_GROUP_TOKEN", rendered)
            self.assertNotIn("secret", rendered)
            self.assertNotIn("[mcp_servers.yuque.env]", rendered)
            self.assertIn("[projects.", rendered)

    def test_sync_preserves_identity_local_projects_and_sessions(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source_home = root / "source"
            identity_home = root / "identity"
            sessions = identity_home / "sessions"
            source_home.mkdir()
            sessions.mkdir(parents=True)
            (sessions / "conversation.jsonl").write_text("{}\n")
            (source_home / "config.toml").write_text(
                textwrap.dedent(
                    """
                    model = "gpt-5.5"
                    """
                ).strip()
            )
            (identity_home / "config.toml").write_text(
                textwrap.dedent(
                    """
                    [projects."/tmp/local-project"]
                    trust_level = "trusted"
                    """
                ).strip()
            )

            sync_identity_config(source_home=source_home, identity_home=identity_home)

            rendered = (identity_home / "config.toml").read_text()
            self.assertIn('model = "gpt-5.5"', rendered)
            self.assertIn('[projects."/tmp/local-project"]', rendered)
            self.assertIn('trust_level = "trusted"', rendered)
            self.assertEqual((sessions / "conversation.jsonl").read_text(), "{}\n")

    def test_sync_identity_auth_only_replaces_source_auth(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source_home = root / "source"
            identity_home = root / "identity"
            sessions = source_home / "sessions"
            source_home.mkdir()
            identity_home.mkdir()
            sessions.mkdir()
            (source_home / "auth.json").write_text('{"account_id": "old"}')
            (source_home / "config.toml").write_text(
                textwrap.dedent(
                    """
                    model = "gpt-5.5"

                    [projects."/tmp/keep"]
                    trust_level = "trusted"
                    """
                ).strip()
            )
            (sessions / "conversation.jsonl").write_text("{}\n")
            (identity_home / "auth.json").write_text('{"account_id": "new"}')
            (identity_home / "config.toml").write_text('model = "other"\n')

            sync_identity_auth(source_home=source_home, identity_home=identity_home)

            self.assertEqual((source_home / "auth.json").read_text(), '{"account_id": "new"}')
            self.assertIn('[projects."/tmp/keep"]', (source_home / "config.toml").read_text())
            self.assertEqual((sessions / "conversation.jsonl").read_text(), "{}\n")

    def test_clean_identity_home_keeps_only_auth_json(self):
        with tempfile.TemporaryDirectory() as tmp:
            identity_home = Path(tmp) / "identity"
            identity_home.mkdir()
            (identity_home / "auth.json").write_text("{}")
            (identity_home / "config.toml").write_text("model = 'old'\n")
            (identity_home / "logs_2.sqlite").write_text("runtime")
            nested = identity_home / "log"
            nested.mkdir()
            (nested / "codex-login.log").write_text("noise")

            clean_identity_home(identity_home)

            self.assertEqual([path.name for path in identity_home.iterdir()], ["auth.json"])

    def test_clean_identity_home_unlinks_directory_symlinks_without_following(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            identity_home = root / "identity"
            identity_home.mkdir()
            outside = root / "outside"
            outside.mkdir()
            (outside / "keep.txt").write_text("keep")
            (identity_home / "auth.json").write_text("{}")
            (identity_home / "linked").symlink_to(outside, target_is_directory=True)

            clean_identity_home(identity_home)

            self.assertEqual([path.name for path in identity_home.iterdir()], ["auth.json"])
            self.assertEqual((outside / "keep.txt").read_text(), "keep")


if __name__ == "__main__":
    unittest.main()
