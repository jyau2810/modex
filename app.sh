#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUTPUT_DIR="${MODEX_OUTPUT_DIR:-"$ROOT_DIR/dist"}"
OPEN_COMMAND="${MODEX_OPEN_COMMAND:-open}"
APP_PATH="$OUTPUT_DIR/Modex.app"
BUILD_VENV_DIR="${MODEX_BUILD_VENV_DIR:-"$ROOT_DIR/.venv-build"}"
REQUIREMENTS_FILE="$ROOT_DIR/requirements-dev.txt"
PINNED_PYTHON="${MODEX_BUILD_PYTHON_VERSION:-3.12}"
FORCE_BUILD="${MODEX_FORCE_BUILD:-0}"

if [[ "$FORCE_BUILD" != "1" && -d "$APP_PATH" ]]; then
  printf 'Opening %s\n' "$APP_PATH"
  "$OPEN_COMMAND" "$APP_PATH"
  exit 0
fi

select_python() {
  if [[ -n "${MODEX_PYTHON:-}" ]]; then
    printf '%s\n' "$MODEX_PYTHON"
    return 0
  fi

  local candidates=(
    python3.13
    python3.12
    python3.11
    /opt/homebrew/bin/python3.13
    /opt/homebrew/bin/python3.12
    /opt/homebrew/bin/python3.11
    /Library/Frameworks/Python.framework/Versions/3.13/bin/python3
    /Library/Frameworks/Python.framework/Versions/3.12/bin/python3
    /Library/Frameworks/Python.framework/Versions/3.11/bin/python3
    python3
  )

  local candidate resolved
  for candidate in "${candidates[@]}"; do
    if ! resolved="$(command -v "$candidate" 2>/dev/null)"; then
      continue
    fi
    if "$resolved" -c 'import sys; print(sys.version_info[:2])' >/dev/null 2>&1; then
      printf '%s\n' "$resolved"
      return 0
    fi
  done
  return 1
}

python_can_build_tk_app() {
  local python_executable="$1"
  "$python_executable" -c 'import tkinter; root = tkinter.Tk(); root.withdraw(); root.destroy()' >/dev/null 2>&1
}

install_build_requirements() {
  local python_executable="$1"
  if command -v uv >/dev/null 2>&1; then
    uv pip install --python "$python_executable" -r "$REQUIREMENTS_FILE" >/dev/null
  else
    if ! "$python_executable" -m pip --version >/dev/null 2>&1; then
      return 1
    fi
    "$python_executable" -m pip --disable-pip-version-check install -r "$REQUIREMENTS_FILE" >/dev/null
  fi
}

bootstrap_build_venv() {
  if [[ -x "$BUILD_VENV_DIR/bin/python" ]] && ! python_can_build_tk_app "$BUILD_VENV_DIR/bin/python"; then
    rm -rf "$BUILD_VENV_DIR"
  fi

  if [[ ! -x "$BUILD_VENV_DIR/bin/python" ]]; then
    if command -v uv >/dev/null 2>&1; then
      uv venv --python "$PINNED_PYTHON" "$BUILD_VENV_DIR"
    else
      return 1
    fi
  fi

  if ! python_can_build_tk_app "$BUILD_VENV_DIR/bin/python"; then
    return 1
  fi

  install_build_requirements "$BUILD_VENV_DIR/bin/python"
}

resolve_build_python() {
  if [[ -n "${MODEX_PYTHON:-}" ]]; then
    printf '%s\n' "$MODEX_PYTHON"
    return 0
  fi

  if bootstrap_build_venv; then
    printf '%s\n' "$BUILD_VENV_DIR/bin/python"
    return 0
  fi
  return 1
}

BUILD_PYTHON="$(resolve_build_python || true)"
if [[ -z "$BUILD_PYTHON" ]]; then
  cat >&2 <<'EOF'
Unable to prepare Modex build environment.
Install uv so Modex can create its pinned local build Python, then run:
  ./app.sh
or specify a known-good Python directly:
  MODEX_PYTHON=/path/to/python3 ./app.sh
EOF
  exit 1
fi
export MODEX_PYTHON="${MODEX_PYTHON:-$BUILD_PYTHON}"

printf 'Building Modex.app into %s\n' "$OUTPUT_DIR"
"$BUILD_PYTHON" "$ROOT_DIR/scripts/build_app.py" --output-dir "$OUTPUT_DIR"

printf 'Opening %s\n' "$APP_PATH"
"$OPEN_COMMAND" "$APP_PATH"
