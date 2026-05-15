import importlib.util
import plistlib
import json
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

            app = tmp_path / "Modex.app"
            self.assertTrue(app.exists())
            self.assertEqual(marker.read_text(), f"{app}\n")

    def test_build_script_creates_unsigned_app_without_xcode(self):
        with tempfile.TemporaryDirectory() as tmp:
            subprocess.run(
                [sys.executable, str(BUILD_SCRIPT), "--output-dir", tmp],
                check=True,
                cwd=PROJECT_ROOT,
            )

            app = Path(tmp) / "Modex.app"
            executable = app / "Contents" / "MacOS" / "Modex"
            info_plist = app / "Contents" / "Info.plist"

            self.assertTrue(executable.exists())
            self.assertTrue(executable.stat().st_mode & 0o111)
            self.assertIn(
                executable.read_bytes()[:4],
                {b"\xfe\xed\xfa\xcf", b"\xcf\xfa\xed\xfe", b"\xca\xfe\xba\xbe", b"\xca\xfe\xba\xbf"},
            )
            self.assertEqual(
                plistlib.loads(info_plist.read_bytes())["CFBundleExecutable"],
                "Modex",
            )
            self.assertTrue((app / "Contents" / "Resources" / "app" / "CodexAccountManager.py").exists())
            launch_config = app / "Contents" / "Resources" / "app" / "launch_config.json"
            self.assertTrue(launch_config.exists())
            python_executable = Path(json.loads(launch_config.read_text())["python_executable"])
            self.assertTrue(python_executable.exists())
            subprocess.run(
                [
                    str(python_executable),
                    "-c",
                    "import tkinter; root = tkinter.Tk(); root.withdraw(); root.destroy()",
                ],
                check=True,
            )

    def test_python_probe_does_not_create_tk_root_for_unsupported_tk(self):
        build_app = load_build_app_module()
        calls = []

        def fake_run(args, **kwargs):
            calls.append(args)
            script = args[-1]
            if "print(tkinter.TkVersion)" in script:
                return subprocess.CompletedProcess(args, 0, stdout="8.5\n", stderr="")
            if "root = tkinter.Tk()" in script:
                raise AssertionError("unsafe Tk root probe should not run for Tk 8.5")
            raise AssertionError(f"unexpected probe: {script}")

        original_run = build_app.subprocess.run
        build_app.subprocess.run = fake_run
        try:
            ok, detail = build_app._python_can_create_tk_root(Path("/usr/bin/python3"))
        finally:
            build_app.subprocess.run = original_run

        self.assertFalse(ok)
        self.assertIn("Tk 8.5", detail)
        self.assertEqual(len(calls), 1)


if __name__ == "__main__":
    unittest.main()
