from __future__ import annotations

import os
from pathlib import Path
from typing import Optional


FORCED_KEYS = {
    "forced_login_method",
    "forced_chatgpt_workspace_id",
}
SECRET_KEY_PARTS = ("token", "secret", "password", "api_key", "apikey")


def sync_identity_config(
    *,
    source_home: Path,
    identity_home: Path,
    workspace_id: Optional[str] = None,
) -> Path:
    """Copy non-secret Codex config into an identity home.

    This intentionally never copies auth.json or token stores. The caller must
    run `cx login <identity>` to create credentials inside that identity home.
    """

    source_config = source_home / "config.toml"
    target = identity_home / "config.toml"
    identity_home.mkdir(parents=True, exist_ok=True)
    try:
        os.chmod(identity_home, 0o700)
    except PermissionError:
        pass
    base = source_config.read_text() if source_config.exists() else ""
    existing = target.read_text() if target.exists() else ""
    merged = _merge_config_text(_strip_secret_config(base), existing)
    rendered = _apply_forced_settings(merged, workspace_id)
    target.write_text(rendered)
    try:
        os.chmod(target, 0o600)
    except PermissionError:
        pass
    return target


def sync_identity_auth(
    *,
    source_home: Path,
    identity_home: Path,
) -> Path:
    """Make an identity active by replacing only the source auth file."""

    source_home.mkdir(parents=True, exist_ok=True)
    try:
        os.chmod(source_home, 0o700)
    except PermissionError:
        pass
    source_auth = source_home / "auth.json"
    identity_auth = identity_home / "auth.json"
    if not identity_auth.exists():
        raise FileNotFoundError(f"账号缺少登录凭据：{identity_auth}")
    temporary = source_auth.with_name(f"{source_auth.name}.modex-tmp")
    temporary.write_bytes(identity_auth.read_bytes())
    try:
        os.chmod(temporary, 0o600)
    except PermissionError:
        pass
    os.replace(temporary, source_auth)
    return source_auth


def clean_identity_home(identity_home: Path) -> None:
    """Keep an identity directory as a credential store only."""

    identity_home = identity_home.expanduser()
    if not identity_home.exists():
        return
    for child in identity_home.iterdir():
        if child.name == "auth.json" and child.is_file():
            continue
        if child.is_symlink():
            child.unlink(missing_ok=True)
        elif child.is_dir():
            _remove_tree(child)
        else:
            child.unlink(missing_ok=True)


def _remove_tree(path: Path) -> None:
    for child in path.iterdir():
        if child.is_symlink():
            child.unlink(missing_ok=True)
        elif child.is_dir():
            _remove_tree(child)
        else:
            child.unlink(missing_ok=True)
    path.rmdir()


def _apply_forced_settings(base: str, workspace_id: Optional[str] = None) -> str:
    lines = []
    for line in base.splitlines():
        key = line.split("=", 1)[0].strip()
        if key in FORCED_KEYS:
            continue
        lines.append(line)
    while lines and not lines[-1].strip():
        lines.pop()
    lines.append('forced_login_method = "chatgpt"')
    if workspace_id:
        lines.append(f'forced_chatgpt_workspace_id = "{_escape(workspace_id)}"')
    return "\n".join(lines) + "\n"


def _merge_config_text(source: str, existing: str) -> str:
    if not existing.strip():
        return source
    source_preamble, source_sections = _split_config(source)
    existing_preamble, existing_sections = _split_config(existing)
    existing_section_names = {header for header, _lines in existing_sections}
    lines: list[str] = []
    _append_lines(lines, _merge_preamble(source_preamble, existing_preamble))
    for header, section_lines in source_sections:
        if header not in existing_section_names:
            _append_lines(lines, section_lines)
    for _header, section_lines in existing_sections:
        _append_lines(lines, section_lines)
    return "\n".join(lines)


def _split_config(text: str) -> tuple[list[str], list[tuple[str, list[str]]]]:
    preamble: list[str] = []
    sections: list[tuple[str, list[str]]] = []
    current = preamble
    for line in text.splitlines():
        header = _section_header(line)
        if header is not None:
            current = [line]
            sections.append((header, current))
        else:
            current.append(line)
    return preamble, sections


def _merge_preamble(source: list[str], existing: list[str]) -> list[str]:
    existing_keys = {key for line in existing if (key := _assignment_key(line)) is not None}
    merged = [line for line in source if _assignment_key(line) not in existing_keys]
    if merged and existing:
        while merged and not merged[-1].strip():
            merged.pop()
        merged.append("")
    merged.extend(existing)
    return merged


def _append_lines(target: list[str], lines: list[str]) -> None:
    clean = list(lines)
    while clean and not clean[0].strip():
        clean.pop(0)
    while clean and not clean[-1].strip():
        clean.pop()
    if not clean:
        return
    if target:
        while target and not target[-1].strip():
            target.pop()
        target.append("")
    target.extend(clean)


def _section_header(line: str) -> Optional[str]:
    stripped = line.strip()
    if stripped.startswith("[") and stripped.endswith("]"):
        return stripped
    return None


def _assignment_key(line: str) -> Optional[str]:
    stripped = line.strip()
    if not stripped or stripped.startswith("#") or stripped.startswith("[") or "=" not in stripped:
        return None
    return stripped.split("=", 1)[0].strip()


def _strip_secret_config(base: str) -> str:
    lines: list[str] = []
    skipping_secret_section = False
    for line in base.splitlines():
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            section = stripped.strip("[]").replace('"', "")
            skipping_secret_section = section.endswith(".env")
            if skipping_secret_section:
                continue
        if skipping_secret_section:
            continue
        key = stripped.split("=", 1)[0].strip().lower()
        if key and any(part in key for part in SECRET_KEY_PARTS):
            continue
        lines.append(line)
    return "\n".join(lines)


def _escape(value: str) -> str:
    return value.replace("\\", "\\\\").replace('"', '\\"')
