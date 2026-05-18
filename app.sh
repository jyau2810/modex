#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OS_NAME="$(uname -s)"
OUTPUT_DIR="${MODEX_OUTPUT_DIR:-}"

case "$OS_NAME" in
  Darwin)
    APP_NAME="Modex.app"
    APP_PATH="$ROOT_DIR/src-tauri/target/release/bundle/macos/$APP_NAME"
    DEFAULT_OPEN_COMMAND="open"
    ;;
  Linux)
    APP_NAME="modex"
    APP_PATH="$ROOT_DIR/src-tauri/target/release/$APP_NAME"
    DEFAULT_OPEN_COMMAND=""
    ;;
  MINGW* | MSYS* | CYGWIN*)
    APP_NAME="modex.exe"
    APP_PATH="$ROOT_DIR/src-tauri/target/release/$APP_NAME"
    DEFAULT_OPEN_COMMAND=""
    ;;
  *)
    printf 'Unsupported system: %s\n' "$OS_NAME" >&2
    exit 1
    ;;
esac

if [[ -n "$OUTPUT_DIR" ]]; then
  APP_PATH="$OUTPUT_DIR/$APP_NAME"
fi

if [[ ! -e "$APP_PATH" ]]; then
  printf 'Modex app not found for %s at %s\n' "$OS_NAME" "$APP_PATH" >&2
  printf 'Run ./build.sh first.\n' >&2
  exit 1
fi

printf 'Opening %s\n' "$APP_PATH"

if [[ -n "${MODEX_OPEN_COMMAND:-}" ]]; then
  "$MODEX_OPEN_COMMAND" "$APP_PATH"
elif [[ -n "$DEFAULT_OPEN_COMMAND" ]]; then
  "$DEFAULT_OPEN_COMMAND" "$APP_PATH"
else
  exec "$APP_PATH"
fi
