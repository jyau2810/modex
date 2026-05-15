from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path
from typing import Optional

from .app_server import AppServerError, build_codex_env, request_account, request_rate_limits
from .codex_binary import resolve_codex_binary
from .config import DEFAULT_CONFIG_PATH, Config, IdentityConfig, load_config
from .launch_agent import DEFAULT_LABEL, render_launch_agent
from .notify import notify
from .quota import QuotaSnapshot, WatchState, evaluate_quota, snapshot_from_rate_limits
from .sync import sync_identity_config


def main(argv: Optional[list[str]] = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if args.command == "init":
        return cmd_init(args)
    config = load_config(args.config)
    if args.command == "sync-configs":
        return cmd_sync_configs(args, config)
    if args.command == "login":
        return cmd_login(args, config)
    if args.command == "quota":
        return cmd_quota(args, config)
    if args.command == "watch":
        return cmd_watch(args, config)
    if args.command == "install-launch-agent":
        return cmd_install_launch_agent(args, config)
    parser.print_help()
    return 2


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="cx", description="Modex multi-account helper")
    parser.add_argument(
        "--config",
        default=os.environ.get("CX_CONFIG", str(DEFAULT_CONFIG_PATH)),
        help="Path to config.toml (default: %(default)s)",
    )
    sub = parser.add_subparsers(dest="command")

    init = sub.add_parser("init", help="write an example config")
    init.add_argument("--path", default=str(DEFAULT_CONFIG_PATH))
    init.add_argument("--force", action="store_true")

    sub.add_parser("sync-configs", help="create identity homes and sync non-secret config")

    login = sub.add_parser("login", help="run Codex login for one identity")
    login.add_argument("identity")

    quota = sub.add_parser("quota", help="read monitored identity quotas")
    quota.add_argument("--json", action="store_true")
    quota.add_argument("identities", nargs="*")

    watch = sub.add_parser("watch", help="watch monitored quotas and notify when recovered")
    watch.add_argument("--once", action="store_true")
    watch.add_argument("--dry-run-notify", action="store_true")

    agent = sub.add_parser("install-launch-agent", help="install macOS LaunchAgent for watcher")
    agent.add_argument("--label", default=DEFAULT_LABEL)
    agent.add_argument("--script-path", default=None)
    agent.add_argument("--force", action="store_true")
    return parser


def watcher_main_args(raw_args: list[str]) -> list[str]:
    global_args: list[str] = []
    watch_args: list[str] = []
    index = 0
    while index < len(raw_args):
        arg = raw_args[index]
        if arg == "--config":
            if index + 1 >= len(raw_args):
                raise SystemExit("--config requires a path")
            global_args.extend([arg, raw_args[index + 1]])
            index += 2
            continue
        watch_args.append(arg)
        index += 1
    return [*global_args, "watch", *watch_args]


def cmd_init(args: argparse.Namespace) -> int:
    target = Path(args.path).expanduser()
    if target.exists() and not args.force:
        print(f"{target} already exists; pass --force to overwrite", file=sys.stderr)
        return 1
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(example_config())
    print(f"Wrote {target}")
    return 0


def cmd_sync_configs(args: argparse.Namespace, config: Config) -> int:
    for identity in config.identities.values():
        target = sync_identity_config(
            source_home=config.codex.source_home,
            identity_home=identity.codex_home,
            workspace_id=identity.workspace_id,
        )
        print(f"{identity.name}: synced {target}")
    return 0


def cmd_login(args: argparse.Namespace, config: Config) -> int:
    identity = _identity(config, args.identity)
    identity.codex_home.mkdir(parents=True, exist_ok=True)
    return subprocess.call(
        [resolve_codex_binary(config.codex.binary), "login"],
        env=build_codex_env(identity.codex_home),
    )


def cmd_quota(args: argparse.Namespace, config: Config) -> int:
    identities = _selected_identities(config, args.identities)
    snapshots = [_read_snapshot(config, identity) for identity in identities]
    if args.json:
        print(json.dumps([_snapshot_dict(snapshot) for snapshot in snapshots], ensure_ascii=False))
    else:
        for snapshot in snapshots:
            print(_format_snapshot(snapshot))
    return 0


def cmd_watch(args: argparse.Namespace, config: Config) -> int:
    state = WatchState()
    while True:
        for identity in config.monitored_identities():
            try:
                snapshot = _read_snapshot(config, identity)
            except AppServerError as error:
                print(f"{identity.name}: {error}", file=sys.stderr)
                continue
            event = evaluate_quota(state, snapshot)
            if event is not None:
                notify(
                    "Codex quota recovered",
                    f"{event.identity} has usable Codex quota again.",
                    dry_run=args.dry_run_notify,
                )
        if args.once:
            return 0
        time.sleep(config.codex.poll_seconds)


def cmd_install_launch_agent(args: argparse.Namespace, config: Config) -> int:
    script_path = Path(args.script_path).expanduser() if args.script_path else _default_watch_script()
    target = Path.home() / "Library" / "LaunchAgents" / f"{args.label}.plist"
    if target.exists() and not args.force:
        print(f"{target} already exists; pass --force to overwrite", file=sys.stderr)
        return 1
    target.parent.mkdir(parents=True, exist_ok=True)
    log_dir = Path.home() / "Library" / "Logs" / "modex"
    log_dir.mkdir(parents=True, exist_ok=True)
    target.write_text(
        render_launch_agent(label=args.label, script_path=script_path, config_path=config.path)
    )
    subprocess.run(["launchctl", "unload", str(target)], check=False)
    subprocess.run(["launchctl", "load", str(target)], check=False)
    print(f"Installed {target}")
    return 0


def example_config() -> str:
    return """# Modex 配置。
# 身份名称可自行命名；工具不会硬编码这些名称。

[codex]
binary = "codex"
app_name = "Codex"
source_home = "~/.codex"
poll_seconds = 60

[identities.enterprise]
codex_home = "~/.codex-enterprise"
monitor = false

[identities.business-a]
codex_home = "~/.codex-business-a"
monitor = true

[identities.business-b]
codex_home = "~/.codex-business-b"
monitor = true
"""


def _read_snapshot(config: Config, identity: IdentityConfig) -> QuotaSnapshot:
    payload = request_rate_limits(resolve_codex_binary(config.codex.binary), identity.codex_home)
    return snapshot_from_rate_limits(identity.name, payload)


def _identity(config: Config, name: str) -> IdentityConfig:
    identity = config.identities.get(name)
    if identity is None:
        raise SystemExit(f"unknown identity {name!r}")
    return identity


def _selected_identities(config: Config, names: list[str]) -> list[IdentityConfig]:
    if names:
        return [_identity(config, name) for name in names]
    monitored = config.monitored_identities()
    return monitored if monitored else list(config.identities.values())


def _format_snapshot(snapshot: QuotaSnapshot) -> str:
    primary = _format_window(snapshot.primary)
    secondary = _format_window(snapshot.secondary)
    status = "limited" if snapshot.is_limited else "available"
    return (
        f"{snapshot.identity}: {status}"
        f" plan={snapshot.plan_type or 'unknown'}"
        f" primary={primary}"
        f" secondary={secondary}"
        f" reached={snapshot.reached_type or '-'}"
    )


def _format_window(window: object) -> str:
    if window is None:
        return "-"
    resets = getattr(window, "resets_at")
    return f"{getattr(window, 'used_percent')}%" + (f" reset={resets}" if resets else "")


def _snapshot_dict(snapshot: QuotaSnapshot) -> dict[str, object]:
    def window(value: object) -> Optional[dict[str, object]]:
        if value is None:
            return None
        return {
            "used_percent": getattr(value, "used_percent"),
            "resets_at": getattr(value, "resets_at"),
            "window_duration_mins": getattr(value, "window_duration_mins"),
        }

    return {
        "identity": snapshot.identity,
        "status": "limited" if snapshot.is_limited else "available",
        "plan_type": snapshot.plan_type,
        "primary": window(snapshot.primary),
        "secondary": window(snapshot.secondary),
        "rate_limit_reached_type": snapshot.reached_type,
    }


def _default_watch_script() -> Path:
    return Path(__file__).resolve().parents[2] / "bin" / "codex-quota-watch"


if __name__ == "__main__":
    raise SystemExit(main())
