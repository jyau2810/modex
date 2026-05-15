from __future__ import annotations

import shutil
from pathlib import Path
from typing import Callable, Iterable, Optional


DEFAULT_CODEX_APP_CLI = Path("/Applications/Codex.app/Contents/Resources/codex")


def resolve_codex_binary(
    configured: str,
    *,
    which: Callable[[str], Optional[str]] = shutil.which,
    app_cli_paths: Optional[Iterable[Path]] = None,
) -> str:
    value = configured.strip() or "codex"
    if "/" in value:
        return str(Path(value).expanduser())

    path_match = which(value)
    if path_match:
        return path_match

    if value == "codex":
        for candidate in app_cli_paths or (DEFAULT_CODEX_APP_CLI,):
            if candidate.exists():
                return str(candidate)

    return value
