#!/usr/bin/env python3
from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from pathlib import Path

APP_NAME = "Modex"
BUNDLE_NAME = f"{APP_NAME}.app"


def main() -> int:
    parser = argparse.ArgumentParser(description="Build an unsigned macOS .app with PyInstaller.")
    parser.add_argument("--output-dir", default="dist")
    args = parser.parse_args()

    script_path = Path(__file__).resolve()
    repo_root = script_path.parents[1]
    output_dir = Path(args.output_dir).expanduser().resolve()
    output_dir.mkdir(parents=True, exist_ok=True)

    if sys.platform != "darwin":
        raise RuntimeError("PyInstaller app build is only supported on macOS")

    _validate_build_python_runtime(Path(sys.executable))
    _ensure_pyinstaller_installed()

    spec_path = repo_root / "Modex.spec"
    _write_spec(spec_path, repo_root, output_dir)

    app_path = output_dir / BUNDLE_NAME
    if app_path.exists():
        shutil.rmtree(app_path)

    subprocess.run(
        [
            sys.executable,
            "-m",
            "PyInstaller",
            "--noconfirm",
            "--clean",
            str(spec_path),
        ],
        check=True,
        cwd=repo_root,
    )

    print(app_path)
    return 0


def _ensure_pyinstaller_installed() -> None:
    try:
        __import__("PyInstaller")
    except ImportError as exc:
        raise RuntimeError(
            "PyInstaller is required. Install it with: python3 -m pip install pyinstaller"
        ) from exc


def _validate_build_python_runtime(python_executable: Path) -> None:
    probe = (
        "import tkinter\n"
        "print(tkinter.TkVersion)\n"
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
        raise RuntimeError(f"Unable to run build Python '{python_executable}': {exc}") from exc

    if result.returncode != 0:
        detail = (result.stderr or result.stdout or f"exit {result.returncode}").strip()
        raise RuntimeError(
            "Build Python is not usable for Tk macOS app packaging. "
            "Please use MODEX_PYTHON with a Python 3.11+ that can create a Tk window.\n"
            f"Python: {python_executable}\n"
            f"Details: {detail}"
        )


def _write_spec(spec_path: Path, repo_root: Path, output_dir: Path) -> None:
    app_script = (repo_root / "CodexAccountManager.py").as_posix()
    src_dir = (repo_root / "src").as_posix()
    dist_path = output_dir.as_posix()
    build_path = (repo_root / "build").as_posix()

    spec_path.write_text(
        (
            "# -*- mode: python ; coding: utf-8 -*-\n"
            "from PyInstaller.utils.hooks import collect_submodules\n\n"
            "hiddenimports = collect_submodules('tkinter')\n\n"
            "a = Analysis(\n"
            f"    ['{app_script}'],\n"
            f"    pathex=['{src_dir}'],\n"
            "    binaries=[],\n"
            "    datas=[],\n"
            "    hiddenimports=hiddenimports,\n"
            "    hookspath=[],\n"
            "    hooksconfig={},\n"
            "    runtime_hooks=[],\n"
            "    excludes=[],\n"
            "    noarchive=False,\n"
            "    optimize=0,\n"
            ")\n"
            "pyz = PYZ(a.pure)\n"
            "exe = EXE(\n"
            "    pyz,\n"
            "    a.scripts,\n"
            "    [],\n"
            "    exclude_binaries=True,\n"
            f"    name='{APP_NAME}',\n"
            "    debug=False,\n"
            "    bootloader_ignore_signals=False,\n"
            "    strip=False,\n"
            "    upx=True,\n"
            "    console=False,\n"
            "    disable_windowed_traceback=False,\n"
            "    argv_emulation=False,\n"
            "    target_arch=None,\n"
            "    codesign_identity=None,\n"
            "    entitlements_file=None,\n"
            ")\n"
            "coll = COLLECT(\n"
            "    exe,\n"
            "    a.binaries,\n"
            "    a.datas,\n"
            "    strip=False,\n"
            "    upx=True,\n"
            "    upx_exclude=[],\n"
            f"    name='{APP_NAME}',\n"
            f"    distpath='{dist_path}',\n"
            f"    workpath='{build_path}',\n"
            ")\n"
            "app = BUNDLE(\n"
            "    coll,\n"
            f"    name='{BUNDLE_NAME}',\n"
            "    icon=None,\n"
            "    bundle_identifier='local.modex',\n"
            ")\n"
        ),
        encoding="utf-8",
    )


if __name__ == "__main__":
    raise SystemExit(main())
