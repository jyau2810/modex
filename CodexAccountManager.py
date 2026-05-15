#!/usr/bin/env python3
from __future__ import annotations

import sys
import queue
import subprocess
import threading
import tkinter as tk
from dataclasses import dataclass
from pathlib import Path
from tkinter import messagebox, ttk
from typing import Callable, Optional


def _bootstrap_import_path() -> None:
    here = Path(__file__).resolve()
    source_package = here.parent / "src"
    if source_package.exists():
        sys.path.insert(0, str(source_package))
    sys.path.insert(0, str(here.parent))


_bootstrap_import_path()

from codex_multi_account.app_config import (  # noqa: E402
    AppIdentity,
    load_app_settings,
    save_app_settings,
)
from codex_multi_account.gui_model import GuiViewModel  # noqa: E402
from codex_multi_account.identity_home import (  # noqa: E402
    default_new_identity,
    delete_managed_identity_home,
    is_managed_identity_home,
)


APP_TITLE = "Modex"


@dataclass
class PendingLogin:
    identity: AppIdentity
    process: object = None


@dataclass
class IdentityRow:
    row: int
    current_var: tk.StringVar
    name_var: tk.StringVar
    current_label: ttk.Label
    name_label: ttk.Label
    quota_widget: tk.Widget
    login_widget: tk.Widget


class ModexApp(tk.Tk):
    def __init__(self) -> None:
        super().__init__()
        self.title(APP_TITLE)
        self.geometry("1120x660")
        self.minsize(980, 600)
        self.settings = load_app_settings()
        self.model = GuiViewModel(self.settings)
        self.monitor_running = True
        self.monitor_after_id: Optional[str] = None
        self.loading_dialog: Optional[LoadingDialog] = None
        self.pending_logins: dict[str, PendingLogin] = {}
        self.background_results: queue.Queue[tuple[Callable[[object], None], object]] = queue.Queue()
        self.identity_status_vars: dict[str, tk.StringVar] = {}
        self.identity_rows: dict[str, IdentityRow] = {}
        self._identity_canvas: Optional[tk.Canvas] = None
        self._identity_rows_frame: Optional[ttk.Frame] = None
        self._identity_mousewheel_bound = False
        self.last_check_var = tk.StringVar(value=self.model.last_check_label)
        self.notification_var = tk.StringVar(value="点击“新增账号”开始登录。")
        self._build_menu()
        self._build_root()
        self.after(100, self._drain_background_results)
        self.after(300, self._initial_refresh_all_async)

    def _build_menu(self) -> None:
        menu_bar = tk.Menu(self)
        file_menu = tk.Menu(menu_bar, tearoff=False)
        file_menu.add_command(label="新增账号", command=self._add_account)
        file_menu.add_command(label="设置", command=self._show_settings_dialog)
        file_menu.add_separator()
        file_menu.add_command(label="退出 Modex", command=self.destroy)
        menu_bar.add_cascade(label="文件", menu=file_menu)

        edit_menu = tk.Menu(menu_bar, tearoff=False)
        edit_menu.add_command(label="剪切", command=lambda: self._send_virtual_event("<<Cut>>"))
        edit_menu.add_command(label="复制", command=lambda: self._send_virtual_event("<<Copy>>"))
        edit_menu.add_command(label="粘贴", command=lambda: self._send_virtual_event("<<Paste>>"))
        menu_bar.add_cascade(label="编辑", menu=edit_menu)

        help_menu = tk.Menu(menu_bar, tearoff=False)
        help_menu.add_command(
            label="关于 Modex",
            command=lambda: messagebox.showinfo("关于 Modex", "Modex 用于管理多个 Codex 身份。"),
        )
        menu_bar.add_cascade(label="帮助", menu=help_menu)
        self.config(menu=menu_bar)

    def _send_virtual_event(self, event_name: str) -> None:
        focused = self.focus_get()
        if focused is not None:
            focused.event_generate(event_name)

    def _build_root(self) -> None:
        self.columnconfigure(0, weight=1)
        self.rowconfigure(0, weight=1)
        root = ttk.Frame(self, padding=18)
        root.grid(row=0, column=0, sticky="nsew")
        root.columnconfigure(0, weight=1)
        root.rowconfigure(1, weight=1)

        header = ttk.Frame(root)
        header.grid(row=0, column=0, sticky="ew", pady=(0, 14))
        header.columnconfigure(0, weight=1)
        ttk.Label(header, text=APP_TITLE, font=("Helvetica", 24, "bold")).grid(
            row=0,
            column=0,
            sticky="w",
        )
        ttk.Button(header, text="设置", command=self._show_settings_dialog).grid(row=0, column=1)

        self.identities_frame = ttk.LabelFrame(root, text="账号", padding=12)
        self.identities_frame.grid(row=1, column=0, sticky="nsew")
        self.identities_frame.columnconfigure(0, weight=1)
        self.identities_frame.rowconfigure(1, weight=1)
        self._render_identities()

    def _render_identities(self) -> None:
        self.model.refresh_current_identity()
        for child in self.identities_frame.winfo_children():
            child.destroy()
        self._identity_canvas = None
        self._identity_rows_frame = None
        self.identity_status_vars.clear()
        self.identity_rows.clear()

        actions = ttk.Frame(self.identities_frame)
        actions.grid(row=0, column=0, sticky="ew", pady=(0, 12))
        actions.columnconfigure(5, weight=1)
        ttk.Button(actions, text="新增账号", command=self._add_account).grid(row=0, column=0)
        ttk.Button(actions, text="全部刷新", command=self._refresh_all_async).grid(
            row=0,
            column=1,
            padx=(8, 0),
        )
        ttk.Label(actions, text="上次检查").grid(row=0, column=2, padx=(18, 0))
        ttk.Label(actions, textvariable=self.last_check_var).grid(row=0, column=3, padx=(8, 0))
        ttk.Label(actions, textvariable=self.notification_var, foreground="#475467").grid(
            row=0,
            column=5,
            sticky="e",
        )

        rows_parent = self._build_identity_rows_container()

        if not self.settings.identities:
            ttk.Label(
                rows_parent,
                text="暂无账号。点击“新增账号”完成登录后，账号会出现在这里。",
                foreground="#666666",
            ).grid(row=0, column=0, columnspan=7, sticky="w", pady=(12, 0))
            return

        ttk.Label(rows_parent, text="当前").grid(row=0, column=0, sticky="w")
        ttk.Label(rows_parent, text="账号").grid(row=0, column=1, sticky="w")
        ttk.Label(rows_parent, text="配额").grid(row=0, column=2, sticky="w")
        ttk.Label(rows_parent, text="状态").grid(row=0, column=3, sticky="w")
        ttk.Label(rows_parent, text="操作").grid(row=0, column=4, columnspan=3, sticky="w")
        for index, identity in enumerate(self.settings.identities):
            row = 1 + index * 2
            if index > 0:
                ttk.Separator(rows_parent, orient="horizontal").grid(
                    row=row - 1,
                    column=0,
                    columnspan=7,
                    sticky="ew",
                    pady=(8, 8),
                )
            self.identity_status_vars[identity.name] = tk.StringVar(
                value=self.model.quota_label(identity.name)
            )
            is_current = self.model.is_current_identity(identity.name)
            current_var = tk.StringVar(value="当前" if is_current else "")
            current_label = ttk.Label(
                rows_parent,
                textvariable=current_var,
                foreground="#0b63ce" if is_current else "#666666",
            )
            current_label.grid(row=row, column=0, sticky="w", pady=6)
            name_var = tk.StringVar(value=identity.name)
            name_label = ttk.Label(
                rows_parent,
                textvariable=name_var,
                font=("Helvetica", 13, "bold"),
                foreground="#0b63ce" if is_current else "#1f1f1f",
            )
            name_label.grid(
                row=row,
                column=1,
                sticky="w",
                padx=(0, 10),
            )
            quota_widget = self._render_quota_cell(row, 2, identity.name)
            login_widget = self._render_login_state_cell(row, 3, identity)
            ttk.Button(
                rows_parent,
                text="切换账号",
                command=lambda name=identity.name: self._switch_identity(name),
            ).grid(row=row, column=4)
            ttk.Button(
                rows_parent,
                text="配置目录",
                command=lambda name=identity.name: self._open_identity_directory(name),
            ).grid(row=row, column=5, padx=(4, 0))
            ttk.Button(
                rows_parent,
                text="删除",
                command=lambda name=identity.name: self._delete_identity(name),
            ).grid(row=row, column=6, padx=(4, 0))
            self.identity_rows[identity.name] = IdentityRow(
                row=row,
                current_var=current_var,
                name_var=name_var,
                current_label=current_label,
                name_label=name_label,
                quota_widget=quota_widget,
                login_widget=login_widget,
            )

    def _build_identity_rows_container(self) -> ttk.Frame:
        canvas = tk.Canvas(self.identities_frame, highlightthickness=0)
        canvas.grid(row=1, column=0, sticky="nsew")

        rows_frame = ttk.Frame(canvas)
        rows_frame.columnconfigure(2, weight=1)
        rows_frame.columnconfigure(3, weight=1)
        window_id = canvas.create_window((0, 0), window=rows_frame, anchor="nw")

        def update_scroll_region(_event: tk.Event) -> None:
            canvas.configure(scrollregion=canvas.bbox("all"))

        def stretch_rows(event: tk.Event) -> None:
            canvas.itemconfigure(window_id, width=event.width)

        rows_frame.bind("<Configure>", update_scroll_region)
        canvas.bind("<Configure>", stretch_rows)
        if not self._identity_mousewheel_bound:
            self.bind_all("<MouseWheel>", self._scroll_identity_rows, add="+")
            self._identity_mousewheel_bound = True

        self._identity_canvas = canvas
        self._identity_rows_frame = rows_frame
        return rows_frame

    def _identity_rows_parent(self) -> tk.Widget:
        return self._identity_rows_frame or self.identities_frame

    def _is_pointer_over_identity_rows(self) -> bool:
        if self._identity_canvas is None:
            return False
        pointer_x = self.winfo_pointerx()
        pointer_y = self.winfo_pointery()
        left = self._identity_canvas.winfo_rootx()
        top = self._identity_canvas.winfo_rooty()
        right = left + self._identity_canvas.winfo_width()
        bottom = top + self._identity_canvas.winfo_height()
        return left <= pointer_x <= right and top <= pointer_y <= bottom

    def _scroll_identity_rows(self, event: tk.Event) -> Optional[str]:
        if self._identity_canvas is None or event.delta == 0:
            return None
        if not self._is_pointer_over_identity_rows():
            return None
        self._identity_canvas.yview_scroll(-1 if event.delta > 0 else 1, "units")
        return "break"

    def _refresh_identity_rows(self) -> None:
        self.model.refresh_current_identity()
        names = [identity.name for identity in self.settings.identities]
        if names != list(self.identity_rows.keys()):
            self._render_identities()
            return
        for identity in self.settings.identities:
            row = self.identity_rows[identity.name]
            is_current = self.model.is_current_identity(identity.name)
            row.current_var.set("当前" if is_current else "")
            row.current_label.configure(foreground="#0b63ce" if is_current else "#666666")
            row.name_var.set(identity.name)
            row.name_label.configure(foreground="#0b63ce" if is_current else "#1f1f1f")
            row.quota_widget.destroy()
            row.quota_widget = self._render_quota_cell(row.row, 2, identity.name)
            row.login_widget.destroy()
            row.login_widget = self._render_login_state_cell(row.row, 3, identity)

    def _render_login_state_cell(self, row: int, column: int, identity: AppIdentity) -> tk.Widget:
        parent = self._identity_rows_parent()
        if self.model.is_login_expired_identity(identity.name):
            frame = ttk.Frame(parent)
            frame.grid(row=row, column=column, padx=(10, 4), sticky="w")
            ttk.Label(frame, text="登录过期", foreground="#b42318").grid(
                row=0,
                column=0,
                sticky="w",
            )
            ttk.Button(
                frame,
                text="重新登录",
                command=lambda name=identity.name: self._login(name),
            ).grid(row=0, column=1, padx=(8, 0))
            return frame
        if self.model.is_logged_in_identity(identity.name):
            label = ttk.Label(
                parent,
                text="已登录",
                foreground="#067647",
            )
            label.grid(row=row, column=column, padx=(10, 4), sticky="w")
            return label
        button = ttk.Button(
            parent,
            text="登录",
            command=lambda name=identity.name: self._login(name),
        )
        button.grid(row=row, column=column, padx=(10, 4))
        return button

    def _render_quota_cell(self, row: int, column: int, identity_name: str) -> tk.Widget:
        display = self.model.quota_display(identity_name)
        frame = ttk.Frame(self._identity_rows_parent())
        frame.grid(row=row, column=column, sticky="ew", padx=(0, 10), pady=4)
        frame.columnconfigure(1, weight=1)

        status_color = "#b42318" if display.is_limited or display.error else "#067647"
        if display.is_unknown:
            status_color = "#667085"
        ttk.Label(frame, text=display.status, foreground=status_color).grid(row=0, column=0, sticky="w")
        ttk.Label(frame, text=display.plan, foreground="#475467").grid(
            row=0,
            column=1,
            sticky="w",
            padx=(8, 0),
        )
        if display.error:
            ttk.Label(frame, text=display.error, foreground="#b42318", wraplength=220).grid(
                row=1,
                column=0,
                columnspan=2,
                sticky="ew",
                pady=(3, 0),
            )
            return frame

        credits_row = 1
        if display.primary_label:
            self._quota_progress(frame, 1, display.primary_label, display.primary_percent)
            credits_row = 2
        if display.secondary_label:
            self._quota_progress(frame, credits_row, display.secondary_label, display.secondary_percent)
            credits_row += 1
        ttk.Label(frame, text=display.credits, foreground="#667085").grid(
            row=credits_row,
            column=0,
            columnspan=2,
            sticky="w",
            pady=(2, 0),
        )
        return frame

    def _quota_progress(self, parent: ttk.Frame, row: int, label: str, percent: int) -> None:
        ttk.Label(parent, text=label, width=26, foreground="#475467").grid(
            row=row,
            column=0,
            sticky="w",
            pady=(2, 0),
        )
        self._used_quota_bar(parent, row, 1, percent)

    def _used_quota_bar(self, parent: ttk.Frame, row: int, column: int, percent: int) -> None:
        width = 135
        height = 12
        value = max(0, min(100, percent))
        fill_width = int(width * value / 100)
        canvas = tk.Canvas(
            parent,
            width=width,
            height=height,
            highlightthickness=0,
            background="#eaecf0",
        )
        canvas.grid(row=row, column=column, sticky="w", padx=(8, 0), pady=(2, 0))
        canvas.create_rectangle(0, 2, width, height - 2, fill="#eaecf0", outline="")
        if fill_width <= 0:
            return
        color = "#2e90fa"
        if value >= 90:
            color = "#d92d20"
        elif value >= 70:
            color = "#f79009"
        canvas.create_rectangle(0, 2, fill_width, height - 2, fill=color, outline="")

    def _show_settings_dialog(self) -> None:
        SettingsDialog(self, on_save=self._settings_saved)

    def _settings_saved(self, codex_binary: str, poll_seconds: int) -> None:
        self.settings.codex_binary = codex_binary
        self.settings.poll_seconds = poll_seconds
        save_app_settings(self.settings)
        self.notification_var.set("设置已保存。")

    def _add_account(self) -> None:
        self._start_add_account_login(self._default_new_identity())

    def _delete_identity(self, name: str) -> None:
        try:
            identity = self.model.identity_named(name)
        except ValueError as error:
            messagebox.showerror("账号不存在", str(error))
            return
        prompt = f"确定要从 Modex 中删除“{identity.name}”吗？"
        if is_managed_identity_home(identity.codex_home):
            prompt += "\n\n这会同时清理该账号的本地登录配置。"
        if not messagebox.askyesno("删除账号", prompt):
            return
        try:
            removed_home = delete_managed_identity_home(identity)
        except OSError as error:
            messagebox.showerror("删除账号失败", f"清理本地登录配置失败：{error}")
            return
        self.model.delete_identity(identity.name)
        save_app_settings(self.settings)
        self._render_identities()
        suffix = "，并已清理本地登录配置" if removed_home else ""
        self.notification_var.set(f"已删除账号：{identity.name}{suffix}")

    def _open_identity_directory(self, name: str) -> None:
        try:
            identity = self.model.identity_named(name)
        except ValueError as error:
            messagebox.showerror("账号不存在", str(error))
            return
        try:
            identity.codex_home.mkdir(parents=True, exist_ok=True)
        except OSError as error:
            messagebox.showerror("无法打开配置目录", f"创建目录失败：{error}")
            return
        opener = "open" if sys.platform == "darwin" else "xdg-open"
        try:
            subprocess.Popen([opener, str(identity.codex_home)])
        except OSError as error:
            messagebox.showerror("无法打开配置目录", str(error))

    def _identity_name_for_home(self, codex_home: Path) -> Optional[str]:
        expanded = codex_home.expanduser()
        for identity in self.settings.identities:
            if identity.codex_home.expanduser() == expanded:
                return identity.name
        return None

    def _default_new_identity(self) -> AppIdentity:
        return default_new_identity(self.settings.identities, self.pending_logins.keys())

    def _start_add_account_login(self, identity: AppIdentity) -> None:
        if identity.name in self.pending_logins:
            messagebox.showerror("无法新增账号", f"账号正在登录中：{identity.name}")
            return
        try:
            self.model._ensure_unique_identity_name(identity.name)
        except ValueError as error:
            messagebox.showerror("无法新增账号", str(error))
            return

        try:
            if self.model.login_completed(identity):
                self.model.cleanup_identity_home(identity)
                self.model.add_identity(identity)
                added_name = self._identity_name_for_home(identity.codex_home) or identity.name
                save_app_settings(self.settings)
                self._render_identities()
                self.notification_var.set(f"检测到已登录账号，已添加：{added_name}")
                self._refresh_identity_with_loading(added_name)
                return
        except Exception:
            pass

        result = self.model.login_pending_identity(identity)
        self.pending_logins[identity.name] = PendingLogin(identity=identity, process=result.process)
        self.notification_var.set(f"{result.message}。完成登录后会自动加入列表。")
        self._poll_pending_login(identity.name)

    def _poll_pending_login(self, name: str) -> None:
        pending = self.pending_logins.get(name)
        if pending is None:
            return

        def on_done(result: object) -> None:
            if name not in self.pending_logins:
                return
            if isinstance(result, Exception):
                self.notification_var.set(f"检查登录状态失败：{result}")
                self.after(3000, lambda: self._poll_pending_login(name))
                return
            if result:
                completed = self.pending_logins.pop(name, pending)
                self._terminate_login_process(completed.process)
                self.model.cleanup_identity_home(completed.identity)
                try:
                    self.model.add_identity(completed.identity)
                except ValueError as error:
                    messagebox.showerror("无法添加账号", str(error))
                    return
                added_name = self._identity_name_for_home(completed.identity.codex_home) or completed.identity.name
                save_app_settings(self.settings)
                self._render_identities()
                self.notification_var.set(f"登录成功，已添加账号：{added_name}")
                self._refresh_identity_with_loading(added_name)
                return
            self.notification_var.set(f"等待“{pending.identity.name}”完成登录...")
            self.after(3000, lambda: self._poll_pending_login(name))

        self._run_background(lambda: self.model.login_completed(pending.identity), on_done)

    def _terminate_login_process(self, process: object) -> None:
        if process is None:
            return
        poll = getattr(process, "poll", None)
        if callable(poll) and poll() is not None:
            return
        terminate = getattr(process, "terminate", None)
        if callable(terminate):
            terminate()

    def _selected_identity_or_alert(self) -> Optional[AppIdentity]:
        if self.model.selected_identity_name is None:
            messagebox.showinfo("请选择账号", "请先选择一个账号。")
            return None
        try:
            return self.model.selected_identity()
        except ValueError as error:
            messagebox.showerror("账号不存在", str(error))
            return None

    def _setup_saved(self) -> None:
        self.model = GuiViewModel(self.settings)
        self._render_identities()
        self.last_check_var.set(self.model.last_check_label)

    def _login(self, name: str) -> None:
        if name in self.pending_logins:
            self.notification_var.set(f"等待“{name}”完成登录...")
            return
        try:
            identity = self.model.identity_named(name)
            result = self.model.login_identity(name)
        except Exception as error:  # noqa: BLE001 - UI boundary.
            messagebox.showerror("登录失败", str(error))
            return
        self.pending_logins[name] = PendingLogin(identity=identity, process=result.process)
        self.notification_var.set(f"{result.message}。完成登录后会自动刷新。")
        self._poll_existing_login(name)

    def _poll_existing_login(self, name: str) -> None:
        pending = self.pending_logins.get(name)
        if pending is None:
            return

        def on_done(result: object) -> None:
            if name not in self.pending_logins:
                return
            if isinstance(result, Exception):
                self.notification_var.set(f"检查登录状态失败：{result}")
                self.after(3000, lambda: self._poll_existing_login(name))
                return
            if result:
                completed = self.pending_logins.pop(name, pending)
                self._terminate_login_process(completed.process)
                self.model.cleanup_identity_home(completed.identity)
                refreshed_name = self._identity_name_for_home(completed.identity.codex_home) or completed.identity.name
                self._render_identities()
                self.notification_var.set(f"登录成功：{refreshed_name}")
                self._refresh_identity_with_loading(refreshed_name)
                return
            self.notification_var.set(f"等待“{pending.identity.name}”完成登录...")
            self.after(3000, lambda: self._poll_existing_login(name))

        self._run_background(lambda: self.model.login_completed(pending.identity), on_done)

    def _switch_identity(self, name: str) -> None:
        try:
            result = self.model.switch_identity(name)
        except Exception as error:  # noqa: BLE001 - UI boundary.
            messagebox.showerror("切换账号失败", str(error))
            return
        if self.model.is_dirty:
            save_app_settings(self.settings)
            self.model.is_dirty = False
        self._refresh_identity_rows()
        self.notification_var.set(result.message)

    def _refresh_identity_async(self, name: str) -> None:
        self._run_background(lambda: self.model.refresh_identity(name), self._after_refresh)

    def _refresh_identity_with_loading(self, name: str) -> None:
        self._show_loading(f"正在刷新“{name}”的配额...")
        self._run_background(lambda: self.model.refresh_identity(name), self._after_manual_refresh)

    def _initial_refresh_all_async(self) -> None:
        if not self.settings.identities:
            return
        self._show_loading("正在刷新配额...")
        self._run_background(self._refresh_all_work, self._after_initial_refresh)

    def _refresh_all_async(self) -> None:
        if not self.settings.identities:
            self.notification_var.set("暂无账号可刷新。")
            return
        self._show_loading("正在刷新配额...")
        self._run_background(self._refresh_all_work, self._after_manual_refresh)

    def _refresh_all_work(self) -> list[object]:
        login_status = self.model.refresh_login_statuses()
        return [login_status, *self.model.refresh_all()]

    def _after_manual_refresh(self, result: object) -> None:
        self._hide_loading()
        self._after_refresh(result)
        self._schedule_monitor_tick()

    def _after_initial_refresh(self, result: object) -> None:
        self._hide_loading()
        self._after_refresh(result)
        self._schedule_monitor_tick()

    def _after_refresh(self, result: object) -> None:
        if self.model.is_dirty:
            save_app_settings(self.settings)
            self.model.is_dirty = False
        self._refresh_identity_rows()
        self.last_check_var.set(self.model.last_check_label)
        if isinstance(result, Exception):
            self.notification_var.set(f"刷新失败：{result}")
        elif isinstance(result, list):
            failures = [item for item in result if hasattr(item, "ok") and not item.ok]
            if failures:
                self.notification_var.set(f"刷新完成，{len(failures)} 个账号失败。")
            else:
                self.notification_var.set("配额已刷新。")
        elif hasattr(result, "message"):
            self.notification_var.set(result.message)

    def _run_monitor_tick(self) -> None:
        if not self.monitor_running:
            return
        self.monitor_after_id = None
        self._run_background(self.model.monitor_tick, self._after_monitor_tick)

    def _after_monitor_tick(self, result: object) -> None:
        self._after_refresh(result)
        self._schedule_monitor_tick()

    def _schedule_monitor_tick(self) -> None:
        if not self.monitor_running or self.monitor_after_id is not None:
            return
        delay = max(10, self.settings.poll_seconds) * 1000
        self.monitor_after_id = self.after(delay, self._run_monitor_tick)

    def _show_loading(self, message: str) -> None:
        if self.loading_dialog is not None:
            self.loading_dialog.update_message(message)
            return
        self.loading_dialog = LoadingDialog(self, message)

    def _hide_loading(self) -> None:
        if self.loading_dialog is None:
            return
        self.loading_dialog.close()
        self.loading_dialog = None

    def _run_background(self, work, on_done) -> None:
        def runner() -> None:
            try:
                result = work()
            except Exception as error:  # noqa: BLE001 - UI boundary.
                result = error
            self.background_results.put((on_done, result))

        threading.Thread(target=runner, daemon=True).start()

    def _drain_background_results(self) -> None:
        while True:
            try:
                on_done, result = self.background_results.get_nowait()
            except queue.Empty:
                break
            on_done(result)
        self.after(100, self._drain_background_results)


class LoadingDialog(tk.Toplevel):
    def __init__(self, parent: ModexApp, message: str) -> None:
        super().__init__(parent)
        self.title("请稍候")
        self.geometry("320x130")
        self.resizable(False, False)
        self.transient(parent)
        self.grab_set()
        self.protocol("WM_DELETE_WINDOW", lambda: None)
        self.message_var = tk.StringVar(value=message)
        self._build()
        self.update_idletasks()
        x = parent.winfo_x() + max(0, (parent.winfo_width() - self.winfo_width()) // 2)
        y = parent.winfo_y() + max(0, (parent.winfo_height() - self.winfo_height()) // 2)
        self.geometry(f"+{x}+{y}")

    def _build(self) -> None:
        frame = ttk.Frame(self, padding=18)
        frame.grid(row=0, column=0, sticky="nsew")
        self.columnconfigure(0, weight=1)
        self.rowconfigure(0, weight=1)
        ttk.Label(frame, textvariable=self.message_var).grid(row=0, column=0, sticky="w")
        self.progress = ttk.Progressbar(frame, mode="indeterminate", length=260)
        self.progress.grid(row=1, column=0, sticky="ew", pady=(16, 0))
        self.progress.start(12)

    def update_message(self, message: str) -> None:
        self.message_var.set(message)

    def close(self) -> None:
        self.progress.stop()
        self.grab_release()
        self.destroy()


class SettingsDialog(tk.Toplevel):
    def __init__(self, parent: ModexApp, *, on_save: Callable[[str, int], None]) -> None:
        super().__init__(parent)
        self.title("设置 Modex")
        self.geometry("620x220")
        self.transient(parent)
        self.grab_set()
        self.on_save = on_save
        self.binary_var = tk.StringVar(value=parent.settings.codex_binary)
        self.poll_var = tk.StringVar(value=str(parent.settings.poll_seconds))
        self._build()

    def _build(self) -> None:
        frame = ttk.Frame(self, padding=18)
        frame.grid(row=0, column=0, sticky="nsew")
        self.columnconfigure(0, weight=1)
        self.rowconfigure(0, weight=1)
        frame.columnconfigure(1, weight=1)

        ttk.Label(frame, text="Codex 可执行文件").grid(row=0, column=0, sticky="w", pady=(0, 8))
        ttk.Entry(frame, textvariable=self.binary_var).grid(row=0, column=1, sticky="ew", pady=(0, 8))
        ttk.Label(frame, text="轮询间隔（秒）").grid(row=1, column=0, sticky="w")
        ttk.Entry(frame, textvariable=self.poll_var, width=8).grid(row=1, column=1, sticky="w")

        buttons = ttk.Frame(frame)
        buttons.grid(row=2, column=0, columnspan=2, sticky="e", pady=(20, 0))
        ttk.Button(buttons, text="取消", command=self.destroy).grid(row=0, column=0, padx=(0, 8))
        ttk.Button(buttons, text="保存", command=self._save).grid(row=0, column=1)

    def _save(self) -> None:
        try:
            poll_seconds = max(10, int(self.poll_var.get()))
        except ValueError:
            messagebox.showerror("设置无效", "轮询间隔必须是数字。")
            return
        self.on_save(self.binary_var.get().strip() or "codex", poll_seconds)
        self.destroy()


def main() -> int:
    app = ModexApp()
    app.mainloop()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
