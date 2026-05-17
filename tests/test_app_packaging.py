import os
import subprocess
import struct
import tempfile
import unittest
import json
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[1]
APP_SCRIPT = PROJECT_ROOT / "app.sh"


class AppPackagingTests(unittest.TestCase):
    def test_app_shell_script_opens_existing_app_without_building(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            app = tmp_path / "Modex.app"
            app.mkdir()
            marker = tmp_path / "opened.txt"
            open_stub = tmp_path / "open_stub.sh"
            open_stub.write_text(
                "#!/usr/bin/env bash\n"
                "printf '%s\\n' \"$1\" > \"$MODEX_OPEN_MARKER\"\n"
            )
            open_stub.chmod(0o755)

            env = os.environ.copy()
            env["MODEX_OUTPUT_DIR"] = str(tmp_path)
            env["MODEX_OPEN_COMMAND"] = str(open_stub)
            env["MODEX_OPEN_MARKER"] = str(marker)

            subprocess.run(
                [str(APP_SCRIPT)],
                check=True,
                cwd="/tmp",
                env=env,
            )

            self.assertEqual(marker.read_text(), f"{app}\n")

    def test_app_shell_script_builds_and_runs_app(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            bin_dir = tmp_path / "bin"
            bin_dir.mkdir()
            marker = tmp_path / "opened.txt"
            npm_log = tmp_path / "npm.log"
            open_stub = tmp_path / "open_stub.sh"
            open_stub.write_text(
                "#!/usr/bin/env bash\n"
                "printf '%s\\n' \"$1\" > \"$MODEX_OPEN_MARKER\"\n"
            )
            open_stub.chmod(0o755)

            npm_stub = bin_dir / "npm"
            app = PROJECT_ROOT / "src-tauri" / "target" / "release" / "bundle" / "macos" / "Modex.app"
            npm_stub.write_text(
                "#!/usr/bin/env bash\n"
                f"printf '%s\\n' \"$*\" >> {str(npm_log)!r}\n"
                f"mkdir -p {str(app)!r}\n"
            )
            npm_stub.chmod(0o755)

            env = os.environ.copy()
            env["PATH"] = f"{bin_dir}{os.pathsep}{env['PATH']}"
            env["MODEX_OUTPUT_DIR"] = str(tmp_path)
            env["MODEX_OPEN_COMMAND"] = str(open_stub)
            env["MODEX_OPEN_MARKER"] = str(marker)
            env["MODEX_FORCE_BUILD"] = "1"

            subprocess.run(
                [str(APP_SCRIPT)],
                check=True,
                cwd="/tmp",
                env=env,
            )

            app = tmp_path / "Modex.app"
            self.assertTrue(app.exists())
            self.assertEqual(marker.read_text(), f"{app}\n")
            self.assertIn("run tauri build -- --bundles app", npm_log.read_text())

    def test_app_shell_script_runs_tauri_dev_mode(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            bin_dir = tmp_path / "bin"
            bin_dir.mkdir()
            npm_log = tmp_path / "npm.log"
            npm_stub = bin_dir / "npm"
            npm_stub.write_text(
                "#!/usr/bin/env bash\n"
                f"printf '%s\\n' \"$*\" >> {str(npm_log)!r}\n"
            )
            npm_stub.chmod(0o755)

            env = os.environ.copy()
            env["PATH"] = f"{bin_dir}{os.pathsep}{env['PATH']}"
            env["MODEX_DEV"] = "1"

            subprocess.run(
                [str(APP_SCRIPT)],
                check=True,
                cwd="/tmp",
                env=env,
            )

            self.assertIn("run tauri dev", npm_log.read_text())

    def test_tauri_bundle_uses_high_resolution_dock_icon(self):
        config = json.loads((PROJECT_ROOT / "src-tauri" / "tauri.conf.json").read_text())
        icons = config["bundle"]["icon"]
        icon_png = PROJECT_ROOT / "src-tauri" / "icons" / "icon.png"
        icon_width, icon_height = struct.unpack(">II", icon_png.read_bytes()[16:24])

        self.assertIn("icons/icon.icns", icons)
        self.assertIn("icons/icon.png", icons)
        self.assertEqual((icon_width, icon_height), (1024, 1024))
        self.assertGreaterEqual((PROJECT_ROOT / "src-tauri" / "icons" / "icon.icns").stat().st_size, 50_000)


if __name__ == "__main__":
    unittest.main()
