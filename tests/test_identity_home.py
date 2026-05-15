import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from codex_multi_account.app_config import AppIdentity
from codex_multi_account.identity_home import delete_managed_identity_home, default_new_identity


class IdentityHomeTests(unittest.TestCase):
    def test_default_new_identity_uses_random_modex_config_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)

            identity = default_new_identity([], set(), home_directory=home, random_digits=lambda: "123456")

            self.assertEqual(identity.name, "登录中")
            self.assertEqual(identity.codex_home, home / ".modex" / "123456")

    def test_delete_managed_identity_home_removes_app_managed_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            codex_home = home / ".modex" / "123456"
            codex_home.mkdir(parents=True)
            (codex_home / "auth.json").write_text("{}")

            removed = delete_managed_identity_home(
                AppIdentity("账号 1", codex_home),
                home_directory=home,
            )

            self.assertTrue(removed)
            self.assertFalse(codex_home.exists())

    def test_delete_managed_identity_home_removes_legacy_app_managed_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            codex_home = home / ".codex-modex-account-1"
            codex_home.mkdir()

            removed = delete_managed_identity_home(
                AppIdentity("账号 1", codex_home),
                home_directory=home,
            )

            self.assertTrue(removed)
            self.assertFalse(codex_home.exists())

    def test_delete_managed_identity_home_keeps_custom_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            codex_home = home / "custom-codex-home"
            codex_home.mkdir()

            removed = delete_managed_identity_home(
                AppIdentity("自定义", codex_home),
                home_directory=home,
            )

            self.assertFalse(removed)
            self.assertTrue(codex_home.exists())


if __name__ == "__main__":
    unittest.main()
