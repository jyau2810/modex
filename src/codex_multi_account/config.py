from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Optional, Union

from .minitoml import expand_path, load_toml_subset


DEFAULT_CONFIG_PATH = Path.home() / ".config" / "codex-account-manager" / "config.toml"


@dataclass(frozen=True)
class CodexSettings:
    binary: str = "codex"
    app_name: str = "Codex"
    poll_seconds: int = 60
    source_home: Path = Path.home() / ".codex"


@dataclass(frozen=True)
class IdentityConfig:
    name: str
    codex_home: Path
    monitor: bool = False
    workspace_id: Optional[str] = None


@dataclass(frozen=True)
class Config:
    path: Path
    codex: CodexSettings
    identities: dict[str, IdentityConfig]

    def monitored_identities(self) -> list[IdentityConfig]:
        return [identity for identity in self.identities.values() if identity.monitor]


def load_config(path: Optional[Union[str, Path]] = None) -> Config:
    config_path = Path(path).expanduser() if path else DEFAULT_CONFIG_PATH
    raw = load_toml_subset(config_path)
    codex_raw = _table(raw, "codex")
    codex = CodexSettings(
        binary=str(codex_raw.get("binary", "codex")),
        app_name=str(codex_raw.get("app_name", "Codex")),
        poll_seconds=int(codex_raw.get("poll_seconds", 60)),
        source_home=expand_path(str(codex_raw.get("source_home", "~/.codex"))),
    )
    identities = {
        name: _identity_from_raw(name, value)
        for name, value in _table(raw, "identities").items()
    }
    if not identities:
        raise ValueError(f"{config_path}: at least one [identities.<name>] section is required")
    return Config(config_path, codex, identities)


def _identity_from_raw(name: str, raw: object) -> IdentityConfig:
    table = _ensure_table(raw, f"identities.{name}")
    if "codex_home" not in table:
        raise ValueError(f"[identities.{name}] codex_home is required")
    workspace_id = table.get("workspace_id")
    return IdentityConfig(
        name=name,
        codex_home=expand_path(str(table["codex_home"])),
        monitor=bool(table.get("monitor", False)),
        workspace_id=str(workspace_id) if workspace_id else None,
    )


def _table(raw: dict[str, object], key: str) -> dict[str, object]:
    value = raw.get(key, {})
    return _ensure_table(value, key)


def _ensure_table(value: object, label: str) -> dict[str, object]:
    if not isinstance(value, dict):
        raise ValueError(f"[{label}] must be a TOML table")
    return value
