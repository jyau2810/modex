from __future__ import annotations

import os
import re
from pathlib import Path
from typing import Any


_SECTION_RE = re.compile(r"^\[([A-Za-z0-9_.-]+)\]$")


def load_toml_subset(path: Path) -> dict[str, Any]:
    """Parse the small TOML subset used by this tool's config.

    The macOS system Python on many machines is still 3.9, so relying on
    tomllib would make a personal automation tool less portable. This parser is
    intentionally narrow: dotted sections, string/bool/int values, and comments.
    """

    data: dict[str, Any] = {}
    current = data
    for line_no, raw in enumerate(path.read_text().splitlines(), start=1):
        line = _strip_comment(raw).strip()
        if not line:
            continue
        section_match = _SECTION_RE.match(line)
        if section_match:
            current = data
            for part in section_match.group(1).split("."):
                current = current.setdefault(part, {})
            continue
        if "=" not in line:
            raise ValueError(f"{path}:{line_no}: expected key = value")
        key, value = line.split("=", 1)
        current[key.strip()] = _parse_value(value.strip(), path, line_no)
    return data


def expand_path(value: str) -> Path:
    return Path(os.path.expandvars(os.path.expanduser(value))).resolve()


def _strip_comment(line: str) -> str:
    in_quote = False
    escaped = False
    output: list[str] = []
    for char in line:
        if escaped:
            output.append(char)
            escaped = False
            continue
        if char == "\\" and in_quote:
            output.append(char)
            escaped = True
            continue
        if char == '"':
            in_quote = not in_quote
            output.append(char)
            continue
        if char == "#" and not in_quote:
            break
        output.append(char)
    return "".join(output)


def _parse_value(value: str, path: Path, line_no: int) -> Any:
    if value in {"true", "false"}:
        return value == "true"
    if value.startswith('"') and value.endswith('"'):
        return bytes(value[1:-1], "utf-8").decode("unicode_escape")
    try:
        return int(value)
    except ValueError:
        pass
    raise ValueError(f"{path}:{line_no}: unsupported value {value!r}")
