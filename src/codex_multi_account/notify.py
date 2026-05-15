from __future__ import annotations

import subprocess


def notify(title: str, message: str, dry_run: bool = False) -> None:
    if dry_run:
        print(f"{title}: {message}")
        return
    script = f'display notification "{_escape(message)}" with title "{_escape(title)}"'
    subprocess.run(["osascript", "-e", script], check=False)


def _escape(value: str) -> str:
    return value.replace("\\", "\\\\").replace('"', '\\"')
