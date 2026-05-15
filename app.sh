#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUTPUT_DIR="${MODEX_OUTPUT_DIR:-"$ROOT_DIR/dist"}"
OPEN_COMMAND="${MODEX_OPEN_COMMAND:-open}"
APP_PATH="$OUTPUT_DIR/Modex.app"

printf 'Building Modex.app into %s\n' "$OUTPUT_DIR"
python3 "$ROOT_DIR/scripts/build_app.py" --output-dir "$OUTPUT_DIR"

printf 'Opening %s\n' "$APP_PATH"
"$OPEN_COMMAND" "$APP_PATH"
