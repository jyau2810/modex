import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[1]
BUILD_SCRIPT = PROJECT_ROOT / "scripts" / "build_app.py"
APP_SCRIPT = PROJECT_ROOT / "app.sh"


def load_build_app_module():
    spec = importlib.util.spec_from_file_location("build_app", BUILD_SCRIPT)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


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
            env["MODEX_PYTHON"] = str(tmp_path / "missing-python")

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
            marker = tmp_path / "opened.txt"
            open_stub = tmp_path / "open_stub.sh"
            open_stub.write_text(
                "#!/usr/bin/env bash\n"
                "printf '%s\\n' \"$1\" > \"$MODEX_OPEN_MARKER\"\n"
            )
            open_stub.chmod(0o755)

            build_stub = tmp_path / "build_stub.py"
            build_stub.write_text(
                "#!/usr/bin/env python3\n"
                "import argparse\n"
                "from pathlib import Path\n"
                "p = argparse.ArgumentParser()\n"
                "p.add_argument('--output-dir', required=True)\n"
                "args = p.parse_args()\n"
                "app = Path(args.output_dir) / 'Modex.app'\n"
                "app.mkdir(parents=True, exist_ok=True)\n"
                "print(app)\n"
            )
            build_stub.chmod(0o755)

            env = os.environ.copy()
            env["MODEX_OUTPUT_DIR"] = str(tmp_path)
            env["MODEX_OPEN_COMMAND"] = str(open_stub)
            env["MODEX_OPEN_MARKER"] = str(marker)
            env["MODEX_PYTHON"] = str(sys.executable)

            scripts_dir = PROJECT_ROOT / "scripts"
            original = scripts_dir / "build_app.py"
            backup = scripts_dir / "build_app.py.bak"
            original.rename(backup)
            try:
                Path(original).write_text(build_stub.read_text())
                subprocess.run(
                    [str(APP_SCRIPT)],
                    check=True,
                    cwd="/tmp",
                    env=env,
                )
            finally:
                original.unlink(missing_ok=True)
                backup.rename(original)

            app = tmp_path / "Modex.app"
            self.assertTrue(app.exists())
            self.assertEqual(marker.read_text(), f"{app}\n")

    def test_app_shell_script_uses_uv_pip_when_build_venv_has_no_pip(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            bin_dir = tmp_path / "bin"
            bin_dir.mkdir()
            venv_dir = tmp_path / ".venv-build"
            output_dir = tmp_path / "dist"
            marker = tmp_path / "opened.txt"
            uv_log = tmp_path / "uv.log"

            fake_python_template = tmp_path / "fake_python.sh"
            fake_python_template.write_text(
                "#!/usr/bin/env bash\n"
                "if [[ \"$1\" == \"-c\" ]]; then exit 0; fi\n"
                "if [[ \"$1\" == \"-m\" && \"$2\" == \"pip\" ]]; then\n"
                "  echo 'No module named pip' >&2\n"
                "  exit 1\n"
                "fi\n"
                f"exec {sys.executable!r} \"$@\"\n"
            )
            fake_python_template.chmod(0o755)

            uv_stub = bin_dir / "uv"
            uv_stub.write_text(
                "#!/usr/bin/env bash\n"
                f"printf '%s\\n' \"$*\" >> {str(uv_log)!r}\n"
                "if [[ \"$1\" == \"venv\" ]]; then\n"
                "  mkdir -p \"$4/bin\"\n"
                f"  cp {str(fake_python_template)!r} \"$4/bin/python\"\n"
                "  chmod +x \"$4/bin/python\"\n"
                "  exit 0\n"
                "fi\n"
                "if [[ \"$1\" == \"pip\" && \"$2\" == \"install\" ]]; then exit 0; fi\n"
                "exit 1\n"
            )
            uv_stub.chmod(0o755)

            open_stub = tmp_path / "open_stub.sh"
            open_stub.write_text(
                "#!/usr/bin/env bash\n"
                "printf '%s\\n' \"$1\" > \"$MODEX_OPEN_MARKER\"\n"
            )
            open_stub.chmod(0o755)

            build_stub = tmp_path / "build_stub.py"
            build_stub.write_text(
                "#!/usr/bin/env python3\n"
                "import argparse\n"
                "from pathlib import Path\n"
                "p = argparse.ArgumentParser()\n"
                "p.add_argument('--output-dir', required=True)\n"
                "args = p.parse_args()\n"
                "app = Path(args.output_dir) / 'Modex.app'\n"
                "app.mkdir(parents=True, exist_ok=True)\n"
                "print(app)\n"
            )

            env = os.environ.copy()
            env.pop("MODEX_PYTHON", None)
            env["PATH"] = f"{bin_dir}{os.pathsep}{env['PATH']}"
            env["MODEX_OUTPUT_DIR"] = str(output_dir)
            env["MODEX_OPEN_COMMAND"] = str(open_stub)
            env["MODEX_OPEN_MARKER"] = str(marker)
            env["MODEX_BUILD_VENV_DIR"] = str(venv_dir)

            scripts_dir = PROJECT_ROOT / "scripts"
            original = scripts_dir / "build_app.py"
            backup = scripts_dir / "build_app.py.bak"
            original.rename(backup)
            try:
                Path(original).write_text(build_stub.read_text())
                subprocess.run(
                    [str(APP_SCRIPT)],
                    check=True,
                    cwd="/tmp",
                    env=env,
                )
            finally:
                original.unlink(missing_ok=True)
                backup.rename(original)

            self.assertEqual(marker.read_text(), f"{output_dir / 'Modex.app'}\n")
            self.assertIn("pip install", uv_log.read_text())

    def test_write_spec_contains_expected_fields(self):
        build_app = load_build_app_module()
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            spec_path = tmp_path / "Modex.spec"
            repo_root = PROJECT_ROOT
            output_dir = tmp_path / "dist"
            output_dir.mkdir()

            build_app._write_spec(spec_path, repo_root, output_dir)
            content = spec_path.read_text()

            self.assertIn("bundle_identifier='local.modex'", content)
            self.assertIn("CodexAccountManager.py", content)
            self.assertIn("name='Modex.app'", content)

    def test_ensure_pyinstaller_installed_raises_when_missing(self):
        build_app = load_build_app_module()

        original_import = __import__

        def fake_import(name, *args, **kwargs):
            if name == "PyInstaller":
                raise ImportError("missing")
            return original_import(name, *args, **kwargs)

        import builtins

        builtins_import = builtins.__import__
        builtins.__import__ = fake_import
        try:
            with self.assertRaises(RuntimeError) as cm:
                build_app._ensure_pyinstaller_installed()
        finally:
            builtins.__import__ = builtins_import

        self.assertIn("PyInstaller is required", str(cm.exception))


if __name__ == "__main__":
    unittest.main()
