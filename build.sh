#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OS_NAME="$(uname -s)"
OUTPUT_DIR="${MODEX_OUTPUT_DIR:-}"
CARGO_COMMAND="${MODEX_CARGO_COMMAND:-cargo}"
RUSTUP_BIN_DIR="/opt/homebrew/opt/rustup/bin"
CARGO_HOME_BIN_DIR="$HOME/.cargo/bin"

if [[ -d "$RUSTUP_BIN_DIR" ]]; then
  export PATH="$RUSTUP_BIN_DIR:$PATH"
elif [[ -d "$CARGO_HOME_BIN_DIR" ]]; then
  export PATH="$CARGO_HOME_BIN_DIR:$PATH"
fi

has_local_tauri_cli() {
  [[ -f "$ROOT_DIR/node_modules/.bin/tauri" ]] ||
    [[ -f "$ROOT_DIR/node_modules/.bin/tauri.cmd" ]] ||
    [[ -f "$ROOT_DIR/node_modules/.bin/tauri.ps1" ]]
}

case "$OS_NAME" in
  Darwin)
    APP_NAME="Modex.app"
    APP_PATH="$ROOT_DIR/src-tauri/target/release/bundle/macos/$APP_NAME"
    BUILD_COMMAND=(npm run tauri build -- --bundles app)
    ;;
  Linux)
    APP_NAME="modex"
    APP_PATH="$ROOT_DIR/src-tauri/target/release/$APP_NAME"
    BUILD_COMMAND=(npm run tauri build)
    ;;
  MINGW* | MSYS* | CYGWIN*)
    APP_NAME="modex.exe"
    APP_PATH="$ROOT_DIR/src-tauri/target/release/$APP_NAME"
    BUILD_COMMAND=(npm run tauri build)
    ;;
  *)
    printf 'Unsupported system: %s\n' "$OS_NAME" >&2
    exit 1
    ;;
esac

cd "$ROOT_DIR"

if ! command -v npm >/dev/null 2>&1; then
  printf 'npm is required to build Modex, but it was not found in PATH.\n' >&2
  exit 1
fi

if [[ ! -d "$ROOT_DIR/node_modules" ]] || ! has_local_tauri_cli; then
  npm install
fi

if ! has_local_tauri_cli; then
  printf 'Tauri CLI was not found in node_modules/.bin after npm install.\n' >&2
  exit 1
fi

if ! command -v "$CARGO_COMMAND" >/dev/null 2>&1; then
  printf 'Cargo is required to build the Tauri app, but it was not found in PATH.\n' >&2
  printf 'Install Rust from https://rustup.rs/ or make sure cargo is available in PATH.\n' >&2
  exit 1
fi

"${BUILD_COMMAND[@]}"

if [[ ! -e "$APP_PATH" ]]; then
  printf 'Build finished, but Modex app was not found at %s\n' "$APP_PATH" >&2
  exit 1
fi

if [[ -n "$OUTPUT_DIR" ]]; then
  OUTPUT_APP_PATH="$OUTPUT_DIR/$APP_NAME"
  mkdir -p "$OUTPUT_DIR"
  rm -rf "$OUTPUT_APP_PATH"

  if [[ -d "$APP_PATH" ]]; then
    cp -R "$APP_PATH" "$OUTPUT_APP_PATH"
  else
    cp "$APP_PATH" "$OUTPUT_APP_PATH"
    chmod +x "$OUTPUT_APP_PATH"
  fi

  APP_PATH="$OUTPUT_APP_PATH"
fi

printf 'Built %s\n' "$APP_PATH"
