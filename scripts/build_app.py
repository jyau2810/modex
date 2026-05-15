#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import plistlib
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


APP_NAME = "Modex"
BUNDLE_ID = "local.modex"


def main() -> int:
    parser = argparse.ArgumentParser(description="Build an unsigned macOS .app without Xcode.")
    parser.add_argument("--output-dir", default="dist")
    args = parser.parse_args()

    script_path = Path(__file__).resolve()
    repo_root = script_path.parents[1]
    output_dir = Path(args.output_dir).expanduser().resolve()
    bundle = output_dir / f"{APP_NAME}.app"
    old_bundle = output_dir / "Codex Account Manager.app"
    contents = bundle / "Contents"
    macos = contents / "MacOS"
    resources_app = contents / "Resources" / "app"

    if bundle.exists():
        shutil.rmtree(bundle)
    if old_bundle.exists():
        shutil.rmtree(old_bundle)
    macos.mkdir(parents=True)
    resources_app.mkdir(parents=True)

    python_executable = _select_python_executable()
    shutil.copy2(repo_root / "CodexAccountManager.py", resources_app / "CodexAccountManager.py")
    shutil.copytree(
        repo_root / "src" / "codex_multi_account",
        resources_app / "codex_multi_account",
    )
    (resources_app / "launch_config.json").write_text(
        json.dumps({"python_executable": str(python_executable)}, indent=2) + "\n"
    )
    _write_launcher(macos / APP_NAME)
    _write_plist(contents / "Info.plist")
    print(bundle)
    return 0


def _select_python_executable() -> Path:
    errors: list[str] = []
    for candidate in _candidate_python_executables():
        ok, detail = _python_can_create_tk_root(candidate)
        if ok:
            return candidate
        errors.append(f"{candidate}: {detail}")
    raise RuntimeError(
        "No Python executable with a working tkinter runtime was found. "
        "Set MODEX_PYTHON to a Python executable that can create a Tk window.\n"
        + "\n".join(errors)
    )


def _candidate_python_executables() -> list[Path]:
    candidates: list[Path] = []
    env_python = os.environ.get("MODEX_PYTHON")
    if env_python:
        candidates.append(Path(env_python).expanduser())
    candidates.append(Path(sys.executable))
    for name in ("python3.13", "python3.12", "python3.11", "python3.10", "python3"):
        resolved = shutil.which(name)
        if resolved:
            candidates.append(Path(resolved))
    for path in (
        "/opt/homebrew/bin/python3.13",
        "/opt/homebrew/bin/python3.12",
        "/opt/homebrew/bin/python3.11",
        "/usr/local/bin/python3.13",
        "/usr/local/bin/python3.12",
        "/usr/local/bin/python3.11",
        "/Library/Frameworks/Python.framework/Versions/3.13/bin/python3",
        "/Library/Frameworks/Python.framework/Versions/3.12/bin/python3",
        "/Library/Frameworks/Python.framework/Versions/3.11/bin/python3",
        "/usr/bin/python3",
    ):
        candidates.append(Path(path))

    unique: list[Path] = []
    seen: set[str] = set()
    for candidate in candidates:
        try:
            resolved = candidate.resolve()
        except OSError:
            resolved = candidate
        key = str(resolved)
        if key in seen or not candidate.exists():
            continue
        seen.add(key)
        unique.append(resolved)
    return unique


def _python_can_create_tk_root(python_executable: Path) -> tuple[bool, str]:
    version_probe = "import tkinter\nprint(tkinter.TkVersion)\n"
    try:
        version_result = subprocess.run(
            [str(python_executable), "-c", version_probe],
            check=False,
            capture_output=True,
            text=True,
            timeout=10,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        return False, str(exc)
    if version_result.returncode != 0:
        detail = (
            version_result.stderr
            or version_result.stdout
            or f"exit {version_result.returncode}"
        ).strip()
        return False, detail

    version_text = version_result.stdout.strip().splitlines()[-1]
    try:
        tk_version = tuple(int(part) for part in version_text.split(".")[:2])
    except ValueError:
        return False, f"Unsupported Tk version: {version_text}"
    if tk_version < (8, 6):
        return False, f"Tk {version_text} is too old for this macOS runtime"

    probe = (
        "import tkinter\n"
        "root = tkinter.Tk()\n"
        "root.withdraw()\n"
        "root.destroy()\n"
    )
    try:
        result = subprocess.run(
            [str(python_executable), "-c", probe],
            check=False,
            capture_output=True,
            text=True,
            timeout=10,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        return False, str(exc)
    if result.returncode == 0:
        return True, "ok"
    detail = (result.stderr or result.stdout or f"exit {result.returncode}").strip()
    return False, detail


def _write_launcher(target: Path) -> None:
    cc = shutil.which("cc")
    if not cc:
        raise RuntimeError("C compiler 'cc' is required to build the macOS app launcher.")
    with tempfile.TemporaryDirectory() as tmp:
        source = Path(tmp) / "launcher.c"
        source.write_text(_launcher_source())
        subprocess.run([cc, str(source), "-o", str(target)], check=True)
    target.chmod(target.stat().st_mode | 0o755)


def _launcher_source() -> str:
    return r'''
#include <mach-o/dyld.h>
#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static void strip_line(char *text) {
    size_t length = strlen(text);
    while (length > 0 && (text[length - 1] == '\n' || text[length - 1] == '\r')) {
        text[length - 1] = '\0';
        length--;
    }
}

int main(int argc, char **argv) {
    char executable[PATH_MAX];
    uint32_t size = sizeof(executable);
    if (_NSGetExecutablePath(executable, &size) != 0) {
        fprintf(stderr, "Executable path is too long.\n");
        return 1;
    }

    char resolved[PATH_MAX];
    if (realpath(executable, resolved) == NULL) {
        perror("realpath");
        return 1;
    }

    char *last_slash = strrchr(resolved, '/');
    if (last_slash == NULL) {
        fprintf(stderr, "Unable to locate app bundle directory.\n");
        return 1;
    }
    *last_slash = '\0';

    char app_dir[PATH_MAX];
    if (snprintf(app_dir, sizeof(app_dir), "%s/../Resources/app", resolved) >= (int)sizeof(app_dir)) {
        fprintf(stderr, "App resource path is too long.\n");
        return 1;
    }

    char config_path[PATH_MAX];
    if (snprintf(config_path, sizeof(config_path), "%s/launch_config.json", app_dir) >= (int)sizeof(config_path)) {
        fprintf(stderr, "Launch config path is too long.\n");
        return 1;
    }

    FILE *config = fopen(config_path, "r");
    if (config == NULL) {
        perror("fopen launch_config.json");
        return 1;
    }

    char config_text[PATH_MAX * 2];
    size_t read_count = fread(config_text, 1, sizeof(config_text) - 1, config);
    fclose(config);
    config_text[read_count] = '\0';

    const char *key = "\"python_executable\"";
    char *key_pos = strstr(config_text, key);
    if (key_pos == NULL) {
        fprintf(stderr, "launch_config.json is missing python_executable.\n");
        return 1;
    }
    char *colon = strchr(key_pos, ':');
    char *start = colon ? strchr(colon, '"') : NULL;
    if (start == NULL) {
        fprintf(stderr, "Unable to parse python_executable.\n");
        return 1;
    }
    start++;
    char *end = strchr(start, '"');
    if (end == NULL) {
        fprintf(stderr, "Unable to parse python_executable.\n");
        return 1;
    }
    *end = '\0';
    strip_line(start);

    char script_path[PATH_MAX];
    if (snprintf(script_path, sizeof(script_path), "%s/CodexAccountManager.py", app_dir) >= (int)sizeof(script_path)) {
        fprintf(stderr, "Script path is too long.\n");
        return 1;
    }

    setenv("PYTHONPATH", app_dir, 1);
    execl(start, start, script_path, (char *)NULL);
    perror("exec python");
    return 127;
}
'''


def _write_plist(target: Path) -> None:
    target.write_bytes(
        plistlib.dumps(
            {
                "CFBundleDevelopmentRegion": "zh_CN",
                "CFBundleExecutable": APP_NAME,
                "CFBundleIdentifier": BUNDLE_ID,
                "CFBundleInfoDictionaryVersion": "6.0",
                "CFBundleName": APP_NAME,
                "CFBundlePackageType": "APPL",
                "CFBundleShortVersionString": "0.1.0",
                "CFBundleVersion": "1",
                "LSMinimumSystemVersion": "12.0",
                "NSHighResolutionCapable": True,
            }
        )
    )


if __name__ == "__main__":
    raise SystemExit(main())
