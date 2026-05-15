import json
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from codex_multi_account.app_config import (
    AppIdentity,
    AppSettings,
    default_app_config_path,
    default_app_settings,
    load_app_settings,
    save_app_settings,
)


class AppConfigTests(unittest.TestCase):
    def test_default_settings_are_gui_first_and_configurable(self):
        home = Path("/Users/alex")
        settings = default_app_settings(
            home_directory=home,
            current_workspace=home / "Documents" / "Projects" / "Personal" / "my-life",
        )

        self.assertFalse(settings.has_completed_setup)
        self.assertEqual(settings.codex_binary, "codex")
        self.assertEqual(settings.poll_seconds, 60)
        self.assertEqual(settings.identities, [])

    def test_json_roundtrip_uses_application_support_config(self):
        with tempfile.TemporaryDirectory() as tmp:
            config_path = Path(tmp) / "Library" / "Application Support" / "Modex" / "config.json"
            settings = AppSettings(
                codex_binary="/opt/codex",
                app_name="Codex",
                poll_seconds=75,
                source_home=Path("/Users/alex/.codex"),
                has_completed_setup=True,
                current_identity_name="Business",
                identities=[
                    AppIdentity(
                        name="Business",
                        codex_home=Path("/Users/alex/.codex-business"),
                        monitor=True,
                        workspace_id="workspace-123",
                    )
                ],
            )

            save_app_settings(settings, config_path)
            loaded = load_app_settings(config_path=config_path, migrate_from_toml=False)

            self.assertEqual(loaded, settings)
            self.assertEqual(
                default_app_config_path(home_directory=Path("/Users/alex")),
                Path("/Users/alex/Library/Application Support/Modex/config.json"),
            )

    def test_loads_old_app_config_path_and_migrates_to_modex_path(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            old_config = root / "Library" / "Application Support" / "Codex Account Manager" / "config.json"
            new_config = root / "Library" / "Application Support" / "Modex" / "config.json"
            old_config.parent.mkdir(parents=True)
            old_config.write_text(
                json.dumps(
                    {
                        "version": 1,
                        "codexBinary": "codex",
                        "appName": "Codex",
                        "pollSeconds": 60,
                        "sourceHome": "/Users/alex/.codex",
                        "hasCompletedSetup": True,
                        "identities": [
                            {
                                "name": "Business",
                                "codexHome": "/Users/alex/.codex-business",
                                "monitor": True,
                                "workspaceId": None,
                            }
                        ],
                        "workspaces": [{"name": "old", "path": "/Users/alex/old"}],
                    }
                )
            )

            loaded = load_app_settings(config_path=new_config, old_config_path=old_config)

            self.assertTrue(loaded.has_completed_setup)
            self.assertEqual(loaded.identities[0].name, "Business")
            self.assertTrue(new_config.exists())

    def test_normalizes_previous_default_identity_names(self):
        raw = {
            "version": 1,
            "codexBinary": "codex",
            "appName": "Codex",
            "pollSeconds": 60,
            "sourceHome": "/Users/alex/.codex",
            "hasCompletedSetup": True,
            "identities": [
                {"name": "企业", "codexHome": "/Users/alex/.codex-enterprise", "monitor": False},
                {"name": "业务 A", "codexHome": "/Users/alex/.codex-business-a", "monitor": True},
                {"name": "Business B", "codexHome": "/Users/alex/.codex-business-b", "monitor": True},
            ],
        }
        with tempfile.TemporaryDirectory() as tmp:
            config_path = Path(tmp) / "config.json"
            config_path.write_text(json.dumps(raw))

            loaded = load_app_settings(config_path=config_path, migrate_from_toml=False)

        self.assertEqual([identity.name for identity in loaded.identities], [
            "企业版",
            "商业版 A",
            "商业版 B",
        ])

    def test_migrates_existing_toml_when_json_is_missing(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            legacy = root / "config.toml"
            legacy.write_text(
                textwrap.dedent(
                    """
                    [codex]
                    binary = "/usr/local/bin/codex"
                    app_name = "Codex"
                    source_home = "~/original-codex"
                    poll_seconds = 45

                    [identities.biz]
                    codex_home = "~/custom-codex"
                    monitor = true
                    workspace_id = "workspace-123"

                    [workspaces.life]
                    path = "~/Life"
                    """
                ).strip()
            )
            config_path = root / "config.json"

            loaded = load_app_settings(config_path=config_path, legacy_toml_path=legacy)

            self.assertEqual(loaded.codex_binary, "/usr/local/bin/codex")
            self.assertEqual(loaded.poll_seconds, 45)
            self.assertEqual(loaded.identities[0].name, "biz")
            self.assertEqual(loaded.identities[0].codex_home, Path.home() / "custom-codex")
            self.assertTrue(loaded.identities[0].monitor)
            self.assertEqual(loaded.identities[0].workspace_id, "workspace-123")
            self.assertTrue(config_path.exists())
            self.assertEqual(json.loads(config_path.read_text())["version"], 1)


if __name__ == "__main__":
    unittest.main()
