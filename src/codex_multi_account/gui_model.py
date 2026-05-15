from __future__ import annotations

import base64
import json
import os
import platform
import re
import shutil
import subprocess
import tempfile
import time
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Iterator, Optional

from .app_config import AppIdentity, AppSettings
from .app_server import build_codex_env, request_rate_limits
from .codex_binary import resolve_codex_binary
from .identity_home import is_managed_identity_home
from .notify import notify
from .quota import QuotaSnapshot, WatchState, evaluate_quota, snapshot_from_rate_limits
from .sync import clean_identity_home, sync_identity_auth


QuotaReader = Callable[[AppSettings, AppIdentity], QuotaSnapshot]
Opener = Callable[[AppSettings, AppIdentity], "GuiActionResult"]
Activator = Callable[[AppSettings], "GuiActionResult"]
LoginRunner = Callable[[AppSettings, AppIdentity], "GuiActionResult"]
LoginStatusReader = Callable[[AppSettings, AppIdentity], bool]
CurrentHomeReader = Callable[[AppSettings], Optional[Path]]
Notifier = Callable[[str, str], None]


@dataclass(frozen=True)
class GuiActionResult:
    ok: bool
    message: str
    process: Optional[subprocess.Popen] = None


@dataclass(frozen=True)
class QuotaDisplay:
    status: str
    plan: str
    primary_label: str
    primary_percent: int
    secondary_label: str
    secondary_percent: int
    credits: str
    is_limited: bool = False
    is_unknown: bool = False
    error: Optional[str] = None


class GuiViewModel:
    def __init__(
        self,
        settings: AppSettings,
        *,
        quota_reader: Optional[QuotaReader] = None,
        opener: Optional[Opener] = None,
        activator: Optional[Activator] = None,
        login_runner: Optional[LoginRunner] = None,
        login_status_reader: Optional[LoginStatusReader] = None,
        current_home_reader: Optional[CurrentHomeReader] = None,
        notifier: Optional[Notifier] = None,
    ) -> None:
        self.settings = settings
        self.quota_reader = quota_reader or read_quota_snapshot
        self.opener = opener or open_codex_app
        self.activator = activator or activate_codex_app
        self.login_runner = login_runner or run_login
        self.login_status_reader = login_status_reader or login_status
        self.current_home_reader = current_home_reader or read_running_codex_home
        self.notifier = notifier or notify
        original_current_identity_name = settings.current_identity_name
        names_changed = self._refresh_identity_names_from_auth()
        self.selected_identity_name = settings.identities[0].name if settings.identities else None
        self.current_identity_name = self.detect_current_identity_name()
        self.logged_in_identity_names = {
            identity.name for identity in settings.identities if _has_local_auth(identity)
        }
        self.expired_identity_names: set[str] = set()
        self.snapshots: dict[str, QuotaSnapshot] = {}
        self.errors: dict[str, str] = {}
        self.watch_state = WatchState()
        self.last_check_label = "从未检查"
        self.is_dirty = names_changed or settings.current_identity_name != original_current_identity_name

    def selected_identity(self) -> AppIdentity:
        if self.selected_identity_name is None:
            raise ValueError("未配置身份")
        return self.identity_named(self.selected_identity_name)

    def identity_named(self, name: str) -> AppIdentity:
        for identity in self.settings.identities:
            if identity.name == name:
                return identity
        raise ValueError(f"未知身份：{name!r}")

    def set_monitor(self, name: str, enabled: bool) -> None:
        self.set_business_plan(name, enabled)

    def set_business_plan(self, name: str, enabled: bool) -> None:
        self.identity_named(name).monitor = enabled
        self.is_dirty = True

    def add_identity(self, identity: AppIdentity) -> None:
        existing_name = self._identity_name_for_home(identity.codex_home)
        if existing_name is not None:
            self.selected_identity_name = existing_name
            existing_identity = self.identity_named(existing_name)
            if _has_local_auth(existing_identity) or _has_local_auth(identity):
                self._mark_logged_in(existing_name)
            return
        identity = self._identity_with_auth_name(identity)
        self._ensure_unique_identity_name(identity.name)
        self.settings.identities.append(identity)
        self.settings.has_completed_setup = True
        self.selected_identity_name = identity.name
        if _has_local_auth(identity):
            self._mark_logged_in(identity.name)
        self.is_dirty = True

    def _refresh_identity_names_from_auth(self) -> bool:
        changed = False
        reserved: set[str] = set()
        original_current = self.settings.current_identity_name
        for identity in self.settings.identities:
            base_name = _auth_identity_display_name(identity.codex_home) or identity.name
            name = _unique_identity_name(base_name, reserved)
            reserved.add(name)
            if name == identity.name:
                continue
            old_name = identity.name
            identity.name = name
            if self.settings.current_identity_name == old_name:
                self.settings.current_identity_name = name
            if getattr(self, "selected_identity_name", None) == old_name:
                self.selected_identity_name = name
            if getattr(self, "current_identity_name", None) == old_name:
                self.current_identity_name = name
            if hasattr(self, "logged_in_identity_names") and old_name in self.logged_in_identity_names:
                self.logged_in_identity_names.discard(old_name)
                self.logged_in_identity_names.add(name)
            if hasattr(self, "expired_identity_names") and old_name in self.expired_identity_names:
                self.expired_identity_names.discard(old_name)
                self.expired_identity_names.add(name)
            if hasattr(self, "snapshots") and old_name in self.snapshots:
                self.snapshots[name] = self.snapshots.pop(old_name)
            if hasattr(self, "errors") and old_name in self.errors:
                self.errors[name] = self.errors.pop(old_name)
            changed = True
        return changed or self.settings.current_identity_name != original_current

    def _identity_with_auth_name(self, identity: AppIdentity) -> AppIdentity:
        reserved = {existing.name for existing in self.settings.identities}
        base_name = _auth_identity_display_name(identity.codex_home) or identity.name
        name = _unique_identity_name(base_name, reserved)
        return AppIdentity(name, identity.codex_home, identity.monitor, identity.workspace_id)

    def update_identity(self, original_name: str, updated: AppIdentity) -> None:
        index = self._identity_index(original_name)
        if updated.name != original_name:
            self._ensure_unique_identity_name(updated.name)
        self.settings.identities[index] = updated
        if self.selected_identity_name == original_name:
            self.selected_identity_name = updated.name
        if self.current_identity_name == original_name:
            self.current_identity_name = updated.name
            self.settings.current_identity_name = updated.name
        was_logged_in = original_name in self.logged_in_identity_names
        was_expired = original_name in self.expired_identity_names
        self.logged_in_identity_names.discard(original_name)
        self.expired_identity_names.discard(original_name)
        if was_expired:
            self.expired_identity_names.add(updated.name)
        elif was_logged_in or _has_local_auth(updated):
            self.logged_in_identity_names.add(updated.name)
        self.snapshots.pop(original_name, None)
        self.errors.pop(original_name, None)
        self.is_dirty = True

    def update_identity_name(self, original_name: str, new_name: str) -> None:
        name = new_name.strip()
        if not name:
            raise ValueError("账号名称不能为空。")
        identity = self.identity_named(original_name)
        self.update_identity(
            original_name,
            AppIdentity(name, identity.codex_home, identity.monitor, identity.workspace_id),
        )

    def update_identity_home(self, name: str, new_home: str) -> None:
        home = new_home.strip()
        if not home:
            raise ValueError("账号目录不能为空。")
        identity = self.identity_named(name)
        updated_home = Path(home).expanduser()
        self._rename_identity_home(identity.codex_home.expanduser(), updated_home)
        self.update_identity(
            name,
            AppIdentity(identity.name, updated_home, identity.monitor, identity.workspace_id),
        )

    def delete_identity(self, name: str) -> None:
        index = self._identity_index(name)
        del self.settings.identities[index]
        self.snapshots.pop(name, None)
        self.errors.pop(name, None)
        self.logged_in_identity_names.discard(name)
        self.expired_identity_names.discard(name)
        if not self.settings.identities:
            self.selected_identity_name = None
            self.current_identity_name = None
            self.settings.current_identity_name = None
            self.settings.has_completed_setup = False
        elif self.selected_identity_name == name:
            self.selected_identity_name = self.settings.identities[min(index, len(self.settings.identities) - 1)].name
        if self.current_identity_name == name:
            self.current_identity_name = None
            self.settings.current_identity_name = None
        self.is_dirty = True

    def detect_current_identity_name(self) -> Optional[str]:
        try:
            current_home = self.current_home_reader(self.settings)
        except Exception:
            current_home = None
        if current_home is not None:
            matched = self._identity_name_for_home(current_home)
            if matched is not None:
                self.settings.current_identity_name = matched
                return matched
            if self._source_home_matches_persisted_identity(current_home):
                return self.settings.current_identity_name
            self.settings.current_identity_name = None
            return None
        if self.settings.current_identity_name and self._identity_exists(self.settings.current_identity_name):
            return self.settings.current_identity_name
        return None

    def refresh_current_identity(self) -> Optional[str]:
        self.current_identity_name = self.detect_current_identity_name()
        return self.current_identity_name

    def is_current_identity(self, name: str) -> bool:
        return self.current_identity_name == name

    def switch_identity(self, name: str) -> GuiActionResult:
        identity = self.identity_named(name)
        if self.is_current_identity(name):
            return self.activator(self.settings)
        result = self.opener(self.settings, identity)
        if result.ok:
            self.current_identity_name = name
            self.settings.current_identity_name = name
            self.selected_identity_name = name
            self.is_dirty = True
        return result

    def login_completed(self, identity: AppIdentity) -> bool:
        completed = self.login_status_reader(self.settings, identity)
        if completed:
            exists = self._identity_exists(identity.name)
            if exists:
                self._mark_logged_in(identity.name)
            if exists and self._refresh_identity_names_from_auth():
                self.is_dirty = True
        elif _has_local_auth(identity):
            self._mark_login_expired(identity.name)
        return completed

    def is_logged_in_identity(self, name: str) -> bool:
        try:
            identity = self.identity_named(name)
        except ValueError:
            return False
        if name in self.expired_identity_names:
            return False
        if _has_local_auth(identity):
            self.logged_in_identity_names.add(name)
        return name in self.logged_in_identity_names

    def is_login_expired_identity(self, name: str) -> bool:
        return name in self.expired_identity_names

    def refresh_login_statuses(self) -> GuiActionResult:
        failures = 0
        for identity in self.settings.identities:
            try:
                completed = self.login_status_reader(self.settings, identity)
            except Exception:
                failures += 1
                continue
            if completed:
                self._mark_logged_in(identity.name)
            elif _has_local_auth(identity):
                self._mark_login_expired(identity.name)
            else:
                self._mark_logged_out(identity.name)
                self.cleanup_identity_home(identity)
        if failures:
            return GuiActionResult(False, f"{failures} 个账号登录状态检查失败")
        return GuiActionResult(True, "登录状态已刷新")

    def refresh_identity(self, name: str) -> GuiActionResult:
        identity = self.identity_named(name)
        try:
            snapshot = self.quota_reader(self.settings, identity)
        except Exception as error:  # noqa: BLE001 - GUI should surface process errors instead of crashing.
            message = str(error)
            self.errors[name] = message
            if _is_login_expiration_error(message) and _has_local_auth(identity):
                self._mark_login_expired(name)
            return GuiActionResult(False, message)
        self.snapshots[name] = snapshot
        self.errors.pop(name, None)
        self._mark_logged_in(name)
        should_monitor = _is_business_plan(snapshot.plan_type)
        if identity.monitor != should_monitor:
            identity.monitor = should_monitor
            self.is_dirty = True
        self.last_check_label = time.strftime("%Y-%m-%d %H:%M:%S")
        if should_monitor:
            event = evaluate_quota(self.watch_state, snapshot)
            if event is not None:
                self._notify_quota_recovery(name)
        else:
            self.watch_state.limited_identities.discard(name)
        return GuiActionResult(True, "配额已刷新")

    def refresh_all(self) -> list[GuiActionResult]:
        return [self.refresh_identity(identity.name) for identity in self.settings.identities]

    def monitor_tick(self) -> list[GuiActionResult]:
        results: list[GuiActionResult] = []
        for identity in self.settings.identities:
            if not identity.monitor:
                continue
            result = self.refresh_identity(identity.name)
            results.append(result)
        return results

    def login_identity(self, name: str) -> GuiActionResult:
        return self.login_runner(self.settings, self.identity_named(name))

    def login_pending_identity(self, identity: AppIdentity) -> GuiActionResult:
        return self.login_runner(self.settings, identity)

    def open_selected_identity(self) -> GuiActionResult:
        return self.switch_identity(self.selected_identity().name)

    def quota_label(self, identity_name: str) -> str:
        display = self.quota_display(identity_name)
        if display.error is not None:
            return f"错误：{display.error}"
        if display.is_unknown:
            return display.status
        parts = [display.status, display.plan]
        if display.primary_label:
            parts.append(display.primary_label)
        if display.secondary_label:
            parts.append(display.secondary_label)
        return " · ".join(parts)

    def quota_display(self, identity_name: str) -> QuotaDisplay:
        if identity_name in self.errors:
            is_free_plan = self._plan_label_for_identity(identity_name) == "免费版"
            return QuotaDisplay(
                status="错误",
                plan=self._plan_label_for_identity(identity_name),
                primary_label="每周已用 -" if is_free_plan else "5小时已用 -",
                primary_percent=0,
                secondary_label="" if is_free_plan else "每周已用 -",
                secondary_percent=0,
                credits="额度未知",
                is_unknown=True,
                error=self.errors[identity_name],
            )
        snapshot = self.snapshots.get(identity_name)
        if snapshot is None:
            is_free_plan = self._plan_label_for_identity(identity_name) == "免费版"
            return QuotaDisplay(
                status="未检查",
                plan=self._plan_label_for_identity(identity_name),
                primary_label="每周已用 -" if is_free_plan else "5小时已用 -",
                primary_percent=0,
                secondary_label="" if is_free_plan else "每周已用 -",
                secondary_percent=0,
                credits="额度未知",
                is_unknown=True,
            )
        if _is_free_plan(snapshot.plan_type):
            primary_label, primary_percent = _window_display("每周", snapshot.secondary or snapshot.primary)
            secondary_label, secondary_percent = "", 0
        elif _is_enterprise_plan(snapshot.plan_type) and snapshot.primary is None and snapshot.secondary is None:
            primary_label, primary_percent = "", 0
            secondary_label, secondary_percent = "", 0
        else:
            primary_label, primary_percent = _window_display("5小时", snapshot.primary)
            secondary_label, secondary_percent = _window_display("每周", snapshot.secondary)
        status = "受限" if snapshot.is_limited else "可用"
        return QuotaDisplay(
            status=status,
            plan=_plan_label(snapshot.plan_type),
            primary_label=primary_label,
            primary_percent=primary_percent,
            secondary_label=secondary_label,
            secondary_percent=secondary_percent,
            credits=_credits_label(snapshot),
            is_limited=snapshot.is_limited,
        )

    def _notify_quota_recovery(self, identity_name: str) -> None:
        display = self.quota_display(identity_name)
        quota_parts = []
        if display.primary_label:
            quota_parts.append(display.primary_label)
        if display.secondary_label:
            quota_parts.append(display.secondary_label)
        parts = [display.plan, display.status, *quota_parts, display.credits]
        message = f"{identity_name}: {' · '.join(parts)}"
        self.notifier("Codex 配额已恢复", message)

    def _identity_index(self, name: str) -> int:
        for index, identity in enumerate(self.settings.identities):
            if identity.name == name:
                return index
        raise ValueError(f"未知身份：{name!r}")

    def _ensure_unique_identity_name(self, name: str) -> None:
        if any(identity.name == name for identity in self.settings.identities):
            raise ValueError(f"身份已存在：{name}")

    def _identity_exists(self, name: str) -> bool:
        return any(identity.name == name for identity in self.settings.identities)

    def _identity_name_for_home(self, codex_home: Path) -> Optional[str]:
        current = _normalized_path(codex_home)
        for identity in self.settings.identities:
            if _normalized_path(identity.codex_home) == current:
                return identity.name
        matched_by_auth = self._identity_name_for_auth_account(codex_home)
        if matched_by_auth is not None:
            return matched_by_auth
        return None

    def _plan_label_for_identity(self, identity_name: str) -> str:
        snapshot = self.snapshots.get(identity_name)
        if snapshot is not None and snapshot.plan_type:
            return _plan_label(snapshot.plan_type)
        try:
            identity = self.identity_named(identity_name)
        except ValueError:
            return "计划未知"
        return _plan_label(_auth_plan_type(identity.codex_home))

    def _identity_name_for_auth_account(self, codex_home: Path) -> Optional[str]:
        auth_key = _auth_identity_match_key(codex_home)
        if auth_key is None:
            return None
        matches = [
            identity.name
            for identity in self.settings.identities
            if _auth_identity_match_key(identity.codex_home) == auth_key
        ]
        if len(matches) == 1:
            return matches[0]
        return None

    def _source_home_matches_persisted_identity(self, codex_home: Path) -> bool:
        if _normalized_path(codex_home) != _normalized_path(self.settings.source_home):
            return False
        current_name = self.settings.current_identity_name
        if not current_name or not self._identity_exists(current_name):
            return False
        return _auth_matches(codex_home, self.identity_named(current_name).codex_home)

    def _mark_logged_in(self, name: str) -> None:
        self.logged_in_identity_names.add(name)
        self.expired_identity_names.discard(name)

    def _mark_logged_out(self, name: str) -> None:
        self.logged_in_identity_names.discard(name)
        self.expired_identity_names.discard(name)

    def _mark_login_expired(self, name: str) -> None:
        self.logged_in_identity_names.discard(name)
        self.expired_identity_names.add(name)

    def _rename_identity_home(self, current_home: Path, updated_home: Path) -> None:
        if current_home == updated_home or not current_home.exists():
            return
        if not current_home.is_dir():
            raise ValueError(f"当前账号目录不是文件夹：{current_home}")
        if updated_home.exists():
            raise ValueError(f"目标账号目录已存在：{updated_home}")
        try:
            updated_home.parent.mkdir(parents=True, exist_ok=True)
            current_home.rename(updated_home)
        except OSError as error:
            raise ValueError(f"无法重命名账号目录：{error}") from error

    def cleanup_identity_home(self, identity: AppIdentity) -> None:
        if is_managed_identity_home(identity.codex_home):
            clean_identity_home(identity.codex_home)


def read_quota_snapshot(settings: AppSettings, identity: AppIdentity) -> QuotaSnapshot:
    with temporary_auth_home(identity.codex_home) as codex_home:
        payload = request_rate_limits(settings.codex_binary, codex_home)
    return snapshot_from_rate_limits(identity.name, payload)


def run_login(settings: AppSettings, identity: AppIdentity) -> GuiActionResult:
    codex_binary = resolve_codex_binary(settings.codex_binary)
    identity.codex_home.mkdir(parents=True, exist_ok=True)
    log_dir = Path.home() / "Library" / "Logs" / "modex"
    log_dir.mkdir(parents=True, exist_ok=True)
    log_file = open(log_dir / f"{identity.codex_home.name}-login.log", "a")
    try:
        proc = subprocess.Popen(
            _login_command(codex_binary, identity),
            env=build_codex_env(identity.codex_home),
            stdout=log_file,
            stderr=log_file,
        )
    finally:
        log_file.close()
    return GuiActionResult(True, f"已打开浏览器登录：{identity.name}", process=proc)


def _login_command(codex_binary: str, identity: AppIdentity) -> list[str]:
    command = [codex_binary, "login", "-c", 'forced_login_method="chatgpt"']
    if identity.workspace_id:
        command.extend(["-c", f'forced_chatgpt_workspace_id="{_escape_config_value(identity.workspace_id)}"'])
    return command


def login_status(settings: AppSettings, identity: AppIdentity) -> bool:
    codex_binary = resolve_codex_binary(settings.codex_binary)
    if _has_local_auth(identity):
        with temporary_auth_home(identity.codex_home) as codex_home:
            result = subprocess.run(
                [codex_binary, "login", "status"],
                env=build_codex_env(codex_home),
                capture_output=True,
                text=True,
                check=False,
            )
            return result.returncode == 0
    result = subprocess.run(
        [codex_binary, "login", "status"],
        env=build_codex_env(identity.codex_home),
        capture_output=True,
        text=True,
        check=False,
    )
    return result.returncode == 0


def open_codex_app(
    settings: AppSettings,
    identity: AppIdentity,
) -> GuiActionResult:
    sync_identity_auth(source_home=settings.source_home, identity_home=identity.codex_home)
    launch_cwd = _sanitize_project_state_for_launch(settings)
    codex_binary = resolve_codex_binary(settings.codex_binary)
    if platform.system() == "Darwin":
        subprocess.run(
            ["osascript", "-e", f'tell application "{_escape_applescript(settings.app_name)}" to quit'],
            check=False,
        )
        time.sleep(1)
    subprocess.Popen(
        [codex_binary, "app"],
        env=build_codex_env(settings.source_home),
        cwd=str(launch_cwd) if launch_cwd is not None else None,
    )
    return GuiActionResult(True, f"正在切换到账号：{identity.name}")


def activate_codex_app(settings: AppSettings) -> GuiActionResult:
    if platform.system() == "Darwin":
        subprocess.Popen(["open", "-a", settings.app_name])
        return GuiActionResult(True, "正在打开 Codex")
    codex_binary = resolve_codex_binary(settings.codex_binary)
    subprocess.Popen([codex_binary, "app"])
    return GuiActionResult(True, "正在打开 Codex")


def read_running_codex_home(settings: AppSettings) -> Optional[Path]:
    result = subprocess.run(
        ["ps", "eww", "-axo", "pid,args"],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        return None
    for line in result.stdout.splitlines():
        current_home = _codex_home_from_process_line(settings, line)
        if current_home is not None:
            return current_home
    for pid in _codex_app_server_pids(result.stdout):
        lsof = subprocess.run(
            ["lsof", "-p", pid],
            capture_output=True,
            text=True,
            check=False,
        )
        if lsof.returncode != 0:
            continue
        current_home = _home_from_lsof_output(settings, lsof.stdout)
        if current_home is not None:
            return current_home
    return None


def _escape_applescript(value: str) -> str:
    return value.replace("\\", "\\\\").replace('"', '\\"')


def _escape_config_value(value: str) -> str:
    return value.replace("\\", "\\\\").replace('"', '\\"')


@contextmanager
def temporary_auth_home(identity_home: Path) -> Iterator[Path]:
    auth_file = identity_home.expanduser() / "auth.json"
    if not auth_file.exists():
        raise FileNotFoundError(f"账号缺少登录凭据：{auth_file}")
    with tempfile.TemporaryDirectory(prefix="modex-auth-") as tmp:
        temporary_home = Path(tmp)
        shutil.copy2(auth_file, temporary_home / "auth.json")
        try:
            os.chmod(temporary_home / "auth.json", 0o600)
        except PermissionError:
            pass
        yield temporary_home


def _sanitize_project_state_for_launch(settings: AppSettings) -> Optional[Path]:
    state_file = settings.source_home.expanduser() / ".codex-global-state.json"
    if not state_file.exists():
        return _fallback_launch_cwd()
    try:
        state = json.loads(state_file.read_text())
    except (OSError, json.JSONDecodeError):
        return _fallback_launch_cwd()
    if not isinstance(state, dict):
        return _fallback_launch_cwd()

    changed = False
    launch_candidates: list[str] = []
    for key in ("active-workspace-roots", "project-order", "electron-saved-workspace-roots"):
        value = state.get(key)
        if not isinstance(value, list):
            continue
        cleaned = _valid_project_roots(value, source_home=settings.source_home)
        if cleaned != value:
            state[key] = cleaned
            changed = True
        launch_candidates.extend(cleaned)

    launch_cwd = _first_existing_project_root(launch_candidates)
    active_roots = state.get("active-workspace-roots")
    if isinstance(active_roots, list) and not active_roots and launch_cwd is not None:
        state["active-workspace-roots"] = [str(launch_cwd)]
        changed = True

    if changed:
        temporary = state_file.with_name(f"{state_file.name}.modex-tmp")
        temporary.write_text(json.dumps(state, indent=2, ensure_ascii=False) + "\n")
        os.replace(temporary, state_file)
    return launch_cwd or _fallback_launch_cwd()


def _valid_project_roots(values: list[object], *, source_home: Path) -> list[str]:
    roots: list[str] = []
    for value in values:
        if not isinstance(value, str):
            continue
        root = _clean_project_root(value, source_home=source_home)
        if root is not None and str(root) not in roots:
            roots.append(str(root))
    return roots


def _clean_project_root(value: str, *, source_home: Path) -> Optional[Path]:
    if not value:
        return None
    path = Path(value).expanduser()
    if not path.is_absolute() or path == Path(path.anchor) or path == source_home.expanduser():
        return None
    try:
        if not path.exists() or not path.is_dir():
            return None
    except OSError:
        return None
    return path


def _first_existing_project_root(values: list[str]) -> Optional[Path]:
    for value in values:
        path = Path(value).expanduser()
        try:
            if path.exists() and path.is_dir():
                return path
        except OSError:
            continue
    return None


def _fallback_launch_cwd() -> Optional[Path]:
    home = Path.home()
    if home.exists() and home.is_dir():
        return home
    return None


def _window_label(window: object) -> str:
    if window is None:
        return "-"
    used = getattr(window, "used_percent")
    resets_at = getattr(window, "resets_at")
    return f"{used}%" + (f" 重置={resets_at}" if resets_at else "")


def _window_display(title: str, window: object) -> tuple[str, int]:
    if window is None:
        return f"{title}已用 -", 0
    used = max(0, min(100, int(getattr(window, "used_percent"))))
    label = f"{title}已用 {used}%"
    reset_label = _reset_time_label(getattr(window, "resets_at"))
    if reset_label:
        label = f"{label} · {reset_label}刷新"
    return label, used


def _plan_label(plan_type: Optional[str]) -> str:
    if not plan_type:
        return "计划未知"
    normalized = plan_type.lower()
    if "business" in normalized:
        return "企业版"
    if normalized in {"enterprise", "enterprise_plus"}:
        return "企业版"
    if normalized in {"team", "teams"}:
        return "团队版"
    if normalized in {"pro", "plus"}:
        return "个人版"
    if normalized == "free":
        return "免费版"
    return plan_type


def _is_free_plan(plan_type: Optional[str]) -> bool:
    return bool(plan_type) and plan_type.lower() == "free"


def _is_enterprise_plan(plan_type: Optional[str]) -> bool:
    if not plan_type:
        return False
    normalized = plan_type.lower()
    return "business" in normalized or normalized in {"enterprise", "enterprise_plus"}


def _reset_time_label(resets_at: object) -> Optional[str]:
    if not resets_at:
        return None
    try:
        timestamp = int(resets_at)
    except (TypeError, ValueError):
        return None
    return time.strftime("%m-%d %H:%M", time.localtime(timestamp))


def _is_business_plan(plan_type: Optional[str]) -> bool:
    if not plan_type:
        return False
    normalized = plan_type.lower()
    return normalized in {"team", "teams"}


def _credits_label(snapshot: QuotaSnapshot) -> str:
    if snapshot.credits_unlimited:
        return "额度无限"
    if snapshot.credits_has_credits:
        return "额度可用"
    if snapshot.credits_has_credits is False:
        return "无额外额度"
    return "额度未知"


def _normalized_path(path: Path) -> str:
    return str(path.expanduser())


def _has_local_auth(identity: AppIdentity) -> bool:
    return (identity.codex_home.expanduser() / "auth.json").exists()


def _codex_home_from_process_line(settings: AppSettings, line: str) -> Optional[Path]:
    if "CODEX_HOME=" not in line:
        return None
    lower = line.lower()
    if "app-server" in lower and ("--listen" in lower or "stdio://" in lower):
        return None
    ignored = (
        "node_repl",
        "skycomputeruseclient",
        "visual studio code",
        "code helper",
        "modex.app",
        "python",
        "pytest",
    )
    if any(marker in lower for marker in ignored):
        return None
    app_name = settings.app_name.lower()
    accepted = (
        f"/contents/macos/{app_name}",
        "/contents/macos/codex",
        "/contents/resources/codex app",
        " codex app ",
        " codex app-server",
    )
    if not any(marker in lower for marker in accepted):
        return None
    match = re.search(r"(?:^|\s)CODEX_HOME=([^\s]+)", line)
    if not match:
        return None
    return Path(match.group(1)).expanduser()


def _auth_account_id(codex_home: Path) -> Optional[str]:
    auth_file = codex_home.expanduser() / "auth.json"
    if not auth_file.exists():
        return None
    try:
        raw = json.loads(auth_file.read_text())
    except (OSError, json.JSONDecodeError):
        return None
    tokens = raw.get("tokens")
    if isinstance(tokens, dict) and tokens.get("account_id"):
        return str(tokens["account_id"])
    account_id = raw.get("account_id")
    return str(account_id) if account_id else None


def _auth_identity_match_key(codex_home: Path) -> Optional[tuple[str, ...]]:
    account_id = _auth_account_id(codex_home)
    user_id = _auth_user_identifier(codex_home)
    if account_id and user_id:
        return ("account-user", account_id, user_id)
    if user_id:
        return ("user", user_id)
    if account_id:
        return ("account", account_id)
    return None


def _auth_user_identifier(codex_home: Path) -> Optional[str]:
    claims = _auth_id_token_claims(codex_home)
    sub = _claim_text(claims.get("sub"))
    if sub:
        return f"sub:{sub}"
    email = _claim_text(claims.get("email"))
    if email:
        return f"email:{email.lower()}"
    username = _claim_text(claims.get("preferred_username"))
    if username:
        return f"username:{username.lower()}"
    return None


def _claim_text(value: object) -> Optional[str]:
    text = str(value).strip() if value else ""
    return text or None


def _auth_identity_display_name(codex_home: Path) -> Optional[str]:
    claims = _auth_id_token_claims(codex_home)
    email = claims.get("email")
    display = str(email).strip() if email else ""
    if not display:
        name = claims.get("name") or claims.get("preferred_username")
        display = str(name).strip() if name else ""
    if not display:
        return None
    plan = _plan_label(_auth_plan_type(codex_home))
    if plan != "计划未知":
        return f"{display} · {plan}"
    return display


def _auth_id_token_claims(codex_home: Path) -> dict[str, object]:
    auth_file = codex_home.expanduser() / "auth.json"
    if not auth_file.exists():
        return {}
    try:
        raw = json.loads(auth_file.read_text())
    except (OSError, json.JSONDecodeError):
        return {}
    tokens = raw.get("tokens")
    if not isinstance(tokens, dict):
        return {}
    return _jwt_payload(tokens.get("id_token"))


def _unique_identity_name(base_name: str, reserved: set[str]) -> str:
    name = base_name.strip() or "账号"
    if name not in reserved:
        return name
    index = 2
    while f"{name} {index}" in reserved:
        index += 1
    return f"{name} {index}"


def _auth_matches(left_home: Path, right_home: Path) -> bool:
    left_key = _auth_identity_match_key(left_home)
    right_key = _auth_identity_match_key(right_home)
    if left_key and right_key:
        return left_key == right_key
    left_auth = left_home.expanduser() / "auth.json"
    right_auth = right_home.expanduser() / "auth.json"
    try:
        return left_auth.read_bytes() == right_auth.read_bytes()
    except OSError:
        return False


def _auth_plan_type(codex_home: Path) -> Optional[str]:
    auth_file = codex_home.expanduser() / "auth.json"
    if not auth_file.exists():
        return None
    try:
        raw = json.loads(auth_file.read_text())
    except (OSError, json.JSONDecodeError):
        return None
    tokens = raw.get("tokens")
    if not isinstance(tokens, dict):
        return None
    for token_name in ("access_token", "id_token"):
        claims = _jwt_payload(tokens.get(token_name))
        auth_claims = claims.get("https://api.openai.com/auth")
        if isinstance(auth_claims, dict) and auth_claims.get("chatgpt_plan_type"):
            return str(auth_claims["chatgpt_plan_type"])
    return None


def _jwt_payload(token: object) -> dict[str, object]:
    if not isinstance(token, str) or token.count(".") < 2:
        return {}
    payload = token.split(".", 2)[1]
    payload += "=" * (-len(payload) % 4)
    try:
        decoded = base64.urlsafe_b64decode(payload)
        data = json.loads(decoded)
    except (ValueError, json.JSONDecodeError):
        return {}
    return data if isinstance(data, dict) else {}


def _is_login_expiration_error(message: str) -> bool:
    normalized = message.lower()
    indicators = (
        "not logged in",
        "login required",
        "please login",
        "please log in",
        "unauthorized",
        "forbidden",
        "invalid_grant",
        "invalid token",
        "expired token",
        "token expired",
        "401",
        "403",
    )
    return any(indicator in normalized for indicator in indicators)


def _codex_app_server_pids(ps_output: str) -> list[str]:
    pids: list[str] = []
    for line in ps_output.splitlines():
        lower = line.lower()
        if "codex app-server" not in lower:
            continue
        if "--listen" in lower or "stdio://" in lower:
            continue
        parts = line.strip().split(maxsplit=1)
        if parts:
            pids.append(parts[0])
    return pids


def _home_from_lsof_output(settings: AppSettings, output: str) -> Optional[Path]:
    candidates = [settings.source_home.expanduser()]
    candidates.extend(identity.codex_home.expanduser() for identity in settings.identities)
    for home in candidates:
        if any(_path_appears_in_lsof_line(home, line) for line in output.splitlines()):
            return home
    return None


def _path_appears_in_lsof_line(path: Path, line: str) -> bool:
    prefix = re.escape(str(path))
    return re.search(prefix + r"(?=$|[\s/])", line) is not None
