from __future__ import annotations

import html
from pathlib import Path
from typing import Optional


DEFAULT_LABEL = "local.modex.quota-watch"


def render_launch_agent(
    *,
    label: str,
    script_path: Path,
    config_path: Path,
    log_dir: Optional[Path] = None,
) -> str:
    logs = log_dir or (Path.home() / "Library" / "Logs" / "modex")
    return f"""<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{_xml(label)}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{_xml(str(script_path))}</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>CX_CONFIG</key>
    <string>{_xml(str(config_path))}</string>
  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>{_xml(str(logs / "quota-watch.out.log"))}</string>
  <key>StandardErrorPath</key>
  <string>{_xml(str(logs / "quota-watch.err.log"))}</string>
</dict>
</plist>
"""


def _xml(value: str) -> str:
    return html.escape(value, quote=True)
