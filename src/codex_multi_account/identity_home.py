from __future__ import annotations

import shutil
import secrets
from pathlib import Path
from typing import Callable, Collection

from .app_config import AppIdentity


MANAGED_HOME_PREFIX = ".codex-modex-account-"
MANAGED_HOME_DIR = ".modex"
RANDOM_HOME_DIGITS = 12


def default_new_identity(
    identities: Collection[AppIdentity],
    pending_names: Collection[str],
    *,
    home_directory: Path | None = None,
    random_digits: Callable[[], str] | None = None,
) -> AppIdentity:
    home = home_directory or Path.home()
    next_digits = random_digits or _random_digits
    codex_home = _new_managed_identity_home(home, next_digits)
    return AppIdentity(
        name="登录中",
        codex_home=codex_home,
        monitor=False,
    )


def managed_identity_home(index: int, *, home_directory: Path | None = None) -> Path:
    home = home_directory or Path.home()
    return home / MANAGED_HOME_DIR / str(index)


def delete_managed_identity_home(identity: AppIdentity, *, home_directory: Path | None = None) -> bool:
    codex_home = identity.codex_home.expanduser()
    if not is_managed_identity_home(codex_home, home_directory=home_directory):
        return False
    if not codex_home.exists():
        return False
    shutil.rmtree(codex_home)
    return True


def is_managed_identity_home(path: Path, *, home_directory: Path | None = None) -> bool:
    home = (home_directory or Path.home()).expanduser()
    expanded = path.expanduser()
    if expanded.parent.name == MANAGED_HOME_DIR:
        return expanded.name.isdigit() and bool(expanded.name)
    name = expanded.name
    if expanded.parent != home:
        return False
    if not name.startswith(MANAGED_HOME_PREFIX):
        return False
    suffix = name.removeprefix(MANAGED_HOME_PREFIX)
    return suffix.isdigit() and bool(suffix)


def _new_managed_identity_home(home: Path, random_digits: Callable[[], str]) -> Path:
    for _attempt in range(100):
        candidate = home / MANAGED_HOME_DIR / random_digits()
        if not candidate.exists():
            return candidate
    raise RuntimeError("无法生成唯一账号配置目录")


def _random_digits() -> str:
    return f"{secrets.randbelow(10 ** RANDOM_HOME_DIGITS):0{RANDOM_HOME_DIGITS}d}"
