#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OPEN_COMMAND="${MODEX_OPEN_COMMAND:-open}"
TAURI_APP_PATH="$ROOT_DIR/src-tauri/target/release/bundle/macos/Modex.app"
OUTPUT_DIR="${MODEX_OUTPUT_DIR:-}"
APP_PATH="${TAURI_APP_PATH}"
FORCE_BUILD="${MODEX_FORCE_BUILD:-0}"

if [[ -n "$OUTPUT_DIR" ]]; then
  APP_PATH="$OUTPUT_DIR/Modex.app"
fi

cd "$ROOT_DIR"

if [[ ! -d "$ROOT_DIR/node_modules" ]]; then
  npm install
fi

if [[ "${MODEX_DEV:-0}" == "1" ]]; then
  exec npm run tauri dev
fi

if [[ "$FORCE_BUILD" != "1" && -d "$APP_PATH" ]]; then
  printf 'Opening %s\n' "$APP_PATH"
  "$OPEN_COMMAND" "$APP_PATH"
  exit 0
fi

npm run tauri build -- --bundles app

if [[ -n "$OUTPUT_DIR" && "$APP_PATH" != "$TAURI_APP_PATH" ]]; then
  mkdir -p "$OUTPUT_DIR"
  rm -rf "$APP_PATH"
  cp -R "$TAURI_APP_PATH" "$APP_PATH"
fi

if [[ "$(uname -s)" == "Darwin" && -d "$APP_PATH" ]]; then
  printf 'Opening %s\n' "$APP_PATH"
  "$OPEN_COMMAND" "$APP_PATH"
fi
