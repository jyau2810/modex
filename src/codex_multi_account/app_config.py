from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Optional

from .config import DEFAULT_CONFIG_PATH, Config, load_config


APP_NAME = "Modex"
OLD_APP_NAME = "Codex Account Manager"
CONFIG_VERSION = 1
DEFAULT_IDENTITY_NAME_MAP = {
    "Enterprise": "企业版",
    "Business A": "商业版 A",
    "Business B": "商业版 B",
    "企业": "企业版",
    "业务 A": "商业版 A",
    "业务 B": "商业版 B",
}


@dataclass
class AppIdentity:
    name: str
    codex_home: Path
    monitor: bool = False
    workspace_id: Optional[str] = None


@dataclass
class AppSettings:
    codex_binary: str = "codex"
    app_name: str = "Codex"
    poll_seconds: int = 60
    source_home: Path = Path.home() / ".codex"
    has_completed_setup: bool = False
    current_identity_name: Optional[str] = None
    identities: list[AppIdentity] = field(default_factory=list)


def default_app_config_path(home_directory: Optional[Path] = None) -> Path:
    home = home_directory or Path.home()
    return home / "Library" / "Application Support" / APP_NAME / "config.json"


def old_app_config_path(home_directory: Optional[Path] = None) -> Path:
    home = home_directory or Path.home()
    return home / "Library" / "Application Support" / OLD_APP_NAME / "config.json"


def default_app_settings(
    *,
    home_directory: Optional[Path] = None,
    current_workspace: Optional[Path] = None,
) -> AppSettings:
    home = home_directory or Path.home()
    return AppSettings(
        source_home=home / ".codex",
        identities=[],
    )


def load_app_settings(
    *,
    config_path: Optional[Path] = None,
    old_config_path: Optional[Path] = None,
    legacy_toml_path: Optional[Path] = None,
    migrate_from_toml: bool = True,
) -> AppSettings:
    target = config_path or default_app_config_path()
    if target.exists():
        return _settings_from_dict(json.loads(target.read_text()))
    old_target = old_config_path or old_app_config_path()
    if old_target.exists():
        settings = _settings_from_dict(json.loads(old_target.read_text()))
        save_app_settings(settings, target)
        return settings
    legacy = legacy_toml_path or DEFAULT_CONFIG_PATH
    if migrate_from_toml and legacy.exists():
        settings = _settings_from_legacy(load_config(legacy))
        save_app_settings(settings, target)
        return settings
    return default_app_settings()


def save_app_settings(settings: AppSettings, config_path: Optional[Path] = None) -> Path:
    target = config_path or default_app_config_path()
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(_settings_to_dict(settings), indent=2, ensure_ascii=False) + "\n")
    try:
        target.chmod(0o600)
    except PermissionError:
        pass
    return target


def _settings_from_legacy(config: Config) -> AppSettings:
    return AppSettings(
        codex_binary=config.codex.binary,
        app_name=config.codex.app_name,
        poll_seconds=config.codex.poll_seconds,
        source_home=config.codex.source_home,
        has_completed_setup=False,
        current_identity_name=None,
        identities=[
            AppIdentity(
                name=identity.name,
                codex_home=identity.codex_home,
                monitor=identity.monitor,
                workspace_id=identity.workspace_id,
            )
            for identity in config.identities.values()
        ],
    )


def _settings_to_dict(settings: AppSettings) -> dict[str, Any]:
    return {
        "version": CONFIG_VERSION,
        "codexBinary": settings.codex_binary,
        "appName": settings.app_name,
        "pollSeconds": settings.poll_seconds,
        "sourceHome": str(settings.source_home),
        "hasCompletedSetup": settings.has_completed_setup,
        "currentIdentityName": settings.current_identity_name,
        "identities": [
            {
                "name": identity.name,
                "codexHome": str(identity.codex_home),
                "monitor": identity.monitor,
                "workspaceId": identity.workspace_id,
            }
            for identity in settings.identities
        ],
    }


def _settings_from_dict(raw: dict[str, Any]) -> AppSettings:
    return AppSettings(
        codex_binary=str(raw.get("codexBinary", "codex")),
        app_name=str(raw.get("appName", "Codex")),
        poll_seconds=int(raw.get("pollSeconds", 60)),
        source_home=Path(str(raw.get("sourceHome", Path.home() / ".codex"))).expanduser(),
        has_completed_setup=bool(raw.get("hasCompletedSetup", False)),
        current_identity_name=_optional_str(raw.get("currentIdentityName")),
        identities=[
            AppIdentity(
                name=_normalize_identity_name(str(identity["name"])),
                codex_home=Path(str(identity["codexHome"])).expanduser(),
                monitor=bool(identity.get("monitor", False)),
                workspace_id=_optional_str(identity.get("workspaceId")),
            )
            for identity in raw.get("identities", [])
        ],
    )


def _optional_str(value: object) -> Optional[str]:
    return str(value) if value else None


def _normalize_identity_name(name: str) -> str:
    return DEFAULT_IDENTITY_NAME_MAP.get(name, name)
