import os
import subprocess
import struct
import tempfile
import unittest
import json
import shutil
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[1]
APP_SCRIPT = PROJECT_ROOT / "app.sh"
BUILD_SCRIPT = PROJECT_ROOT / "build.sh"


def current_os_name():
    return subprocess.check_output(["uname", "-s"], text=True).strip()


def app_name_for_current_system():
    os_name = current_os_name()
    if os_name == "Darwin":
        return "Modex.app"
    if os_name.startswith(("MINGW", "MSYS", "CYGWIN")):
        return "modex.exe"
    if os_name == "Linux":
        return "modex"
    raise AssertionError(f"unsupported test OS: {os_name}")


def release_app_path_for_current_system(root=PROJECT_ROOT):
    os_name = current_os_name()
    if os_name == "Darwin":
        return root / "src-tauri" / "target" / "release" / "bundle" / "macos" / "Modex.app"
    if os_name.startswith(("MINGW", "MSYS", "CYGWIN")):
        return root / "src-tauri" / "target" / "release" / "modex.exe"
    if os_name == "Linux":
        return root / "src-tauri" / "target" / "release" / "modex"
    raise AssertionError(f"unsupported test OS: {os_name}")


def npm_build_args_for_current_system():
    if current_os_name() == "Darwin":
        return "run tauri build -- --bundles app"
    return "run tauri build"


def create_fake_app(path):
    if path.suffix == ".app":
        path.mkdir(parents=True)
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("#!/usr/bin/env bash\n")
    path.chmod(0o755)


class AppPackagingTests(unittest.TestCase):
    def test_app_shell_script_opens_existing_app_without_building(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            app = tmp_path / app_name_for_current_system()
            create_fake_app(app)
            marker = tmp_path / "opened.txt"
            npm_log = tmp_path / "npm.log"
            npm_stub = tmp_path / "npm"
            npm_stub.write_text(
                "#!/usr/bin/env bash\n"
                f"printf '%s\\n' \"$*\" >> {str(npm_log)!r}\n"
            )
            npm_stub.chmod(0o755)
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
            env["PATH"] = f"{tmp_path}{os.pathsep}{env['PATH']}"

            subprocess.run(
                [str(APP_SCRIPT)],
                check=True,
                cwd="/tmp",
                env=env,
            )

            self.assertEqual(marker.read_text(), f"{app}\n")
            self.assertFalse(npm_log.exists())

    def test_app_shell_script_reports_missing_app_without_building(self):
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
            env["MODEX_OUTPUT_DIR"] = str(tmp_path)

            result = subprocess.run(
                [str(APP_SCRIPT)],
                check=False,
                cwd="/tmp",
                env=env,
                text=True,
                capture_output=True,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Run ./build.sh first", result.stderr)
            self.assertFalse(npm_log.exists())

    def test_build_shell_script_builds_current_system_app(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            project = tmp_path / "project"
            project.mkdir()
            script = project / "build.sh"
            shutil.copy2(BUILD_SCRIPT, script)
            script.chmod(0o755)
            create_fake_app(project / "node_modules" / ".bin" / "tauri")

            bin_dir = tmp_path / "bin"
            bin_dir.mkdir()
            npm_log = tmp_path / "npm.log"
            npm_stub = bin_dir / "npm"
            cargo_stub = bin_dir / "cargo"
            cargo_stub.write_text("#!/usr/bin/env bash\n")
            cargo_stub.chmod(0o755)
            release_app = release_app_path_for_current_system(project)
            npm_stub.write_text(
                "#!/usr/bin/env bash\n"
                f"printf '%s\\n' \"$*\" >> {str(npm_log)!r}\n"
                f"if [[ \"$*\" == *\"run tauri build\"* ]]; then\n"
                f"  if [[ {str(release_app.suffix == '.app').lower()} == true ]]; then\n"
                f"    mkdir -p {str(release_app)!r}\n"
                f"  else\n"
                f"    mkdir -p {str(release_app.parent)!r}\n"
                f"    printf '#!/usr/bin/env bash\\n' > {str(release_app)!r}\n"
                f"    chmod +x {str(release_app)!r}\n"
                f"  fi\n"
                f"fi\n"
            )
            npm_stub.chmod(0o755)

            env = os.environ.copy()
            env["PATH"] = f"{bin_dir}{os.pathsep}{env['PATH']}"
            env["MODEX_OUTPUT_DIR"] = str(tmp_path / "out")

            subprocess.run(
                [str(script)],
                check=True,
                cwd="/tmp",
                env=env,
            )

            app = tmp_path / "out" / app_name_for_current_system()
            self.assertTrue(app.exists())
            self.assertEqual(npm_log.read_text().splitlines(), [npm_build_args_for_current_system()])

    def test_build_shell_script_repairs_missing_tauri_cli_launcher(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            project = tmp_path / "project"
            project.mkdir()
            (project / "node_modules").mkdir()
            script = project / "build.sh"
            shutil.copy2(BUILD_SCRIPT, script)
            script.chmod(0o755)

            bin_dir = tmp_path / "bin"
            bin_dir.mkdir()
            npm_log = tmp_path / "npm.log"
            release_app = release_app_path_for_current_system(project)
            npm_stub = bin_dir / "npm"
            cargo_stub = bin_dir / "cargo"
            cargo_stub.write_text("#!/usr/bin/env bash\n")
            cargo_stub.chmod(0o755)
            npm_stub.write_text(
                "#!/usr/bin/env bash\n"
                f"printf '%s\\n' \"$*\" >> {str(npm_log)!r}\n"
                f"if [[ \"$*\" == \"install\" ]]; then\n"
                f"  mkdir -p {str(project / 'node_modules' / '.bin')!r}\n"
                f"  printf '#!/usr/bin/env bash\\n' > {str(project / 'node_modules' / '.bin' / 'tauri')!r}\n"
                f"  chmod +x {str(project / 'node_modules' / '.bin' / 'tauri')!r}\n"
                f"fi\n"
                f"if [[ \"$*\" == *\"run tauri build\"* ]]; then\n"
                f"  if [[ {str(release_app.suffix == '.app').lower()} == true ]]; then\n"
                f"    mkdir -p {str(release_app)!r}\n"
                f"  else\n"
                f"    mkdir -p {str(release_app.parent)!r}\n"
                f"    printf '#!/usr/bin/env bash\\n' > {str(release_app)!r}\n"
                f"    chmod +x {str(release_app)!r}\n"
                f"  fi\n"
                f"fi\n"
            )
            npm_stub.chmod(0o755)

            env = os.environ.copy()
            env["PATH"] = f"{bin_dir}{os.pathsep}{env['PATH']}"
            env["MODEX_OUTPUT_DIR"] = str(tmp_path / "out")

            subprocess.run(
                [str(script)],
                check=True,
                cwd="/tmp",
                env=env,
            )

            self.assertEqual(npm_log.read_text().splitlines()[:2], ["install", npm_build_args_for_current_system()])

    def test_build_shell_script_reports_missing_cargo(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            project = tmp_path / "project"
            project.mkdir()
            script = project / "build.sh"
            shutil.copy2(BUILD_SCRIPT, script)
            script.chmod(0o755)
            create_fake_app(project / "node_modules" / ".bin" / "tauri")

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
            env["MODEX_CARGO_COMMAND"] = str(tmp_path / "missing-cargo")

            result = subprocess.run(
                [str(script)],
                check=False,
                cwd="/tmp",
                env=env,
                text=True,
                capture_output=True,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Cargo is required", result.stderr)
            self.assertFalse(npm_log.exists())

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
