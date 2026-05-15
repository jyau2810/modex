from __future__ import annotations

import json
import os
import select
import subprocess
import time
from pathlib import Path
from typing import Any, Mapping, Optional

from .codex_binary import resolve_codex_binary


class AppServerError(RuntimeError):
    pass


def build_codex_env(codex_home: Path, base_env: Optional[Mapping[str, str]] = None) -> dict[str, str]:
    env = dict(base_env or os.environ)
    env["CODEX_HOME"] = str(codex_home)
    return env


def request_account(codex_binary: str, codex_home: Path, timeout_seconds: float = 20) -> dict[str, Any]:
    return _request(codex_binary, codex_home, "account/read", {}, timeout_seconds)


def request_rate_limits(
    codex_binary: str,
    codex_home: Path,
    timeout_seconds: float = 30,
) -> dict[str, Any]:
    return _request(codex_binary, codex_home, "account/rateLimits/read", None, timeout_seconds)


def _request(
    codex_binary: str,
    codex_home: Path,
    method: str,
    params: object,
    timeout_seconds: float,
) -> dict[str, Any]:
    proc = subprocess.Popen(
        [resolve_codex_binary(codex_binary), "app-server", "--listen", "stdio://"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=build_codex_env(codex_home),
    )
    try:
        _send(proc, 1, "initialize", _initialize_params())
        _send(proc, 2, method, params)
        deadline = time.monotonic() + timeout_seconds
        errors: list[str] = []
        while time.monotonic() < deadline:
            ready, _, _ = select.select([proc.stdout, proc.stderr], [], [], 0.25)
            for stream in ready:
                line = stream.readline()
                if not line:
                    continue
                if stream is proc.stderr:
                    errors.append(line.strip())
                    continue
                message = json.loads(line)
                if message.get("id") != 2:
                    continue
                if "error" in message:
                    raise AppServerError(str(message["error"]))
                result = message.get("result")
                if not isinstance(result, dict):
                    raise AppServerError(f"{method} returned non-object result")
                return result
        detail = "; ".join(errors[-3:])
        raise AppServerError(f"timed out waiting for {method}" + (f": {detail}" if detail else ""))
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=2)
        except subprocess.TimeoutExpired:
            proc.kill()


def _send(proc: subprocess.Popen[str], request_id: int, method: str, params: object) -> None:
    if proc.stdin is None:
        raise AppServerError("app-server stdin is unavailable")
    proc.stdin.write(json.dumps({"id": request_id, "method": method, "params": params}) + "\n")
    proc.stdin.flush()


def _initialize_params() -> dict[str, Any]:
    return {
        "clientInfo": {
            "name": "modex",
            "title": "Modex",
            "version": "0.1.0",
        },
        "capabilities": {
            "experimentalApi": True,
            "optOutNotificationMethods": [],
        },
    }
