#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NETWORK="${ATHO_UI_NETWORK:-regnet}"
APP_NAME="${ATHO_UI_APP_NAME:-Atho}"
LOG_FILE="${ATHO_UI_LOG_FILE:-/tmp/atho-qt-ui-smoke.log}"

if ! command -v osascript >/dev/null 2>&1; then
  echo "skip: osascript not available" >&2
  exit 2
fi

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "skip: macOS UI automation only" >&2
  exit 2
fi

if ! /usr/bin/python3 - <<'PY'
import subprocess
import sys

try:
    subprocess.run(
        ["osascript", "-e", 'tell application "System Events" to UI elements enabled'],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        timeout=5,
    )
except Exception:
    sys.exit(1)
PY
then
  echo "skip: Accessibility automation is not enabled for this shell" >&2
  exit 2
fi

TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/atho-qt-ui-smoke.XXXXXX")"
export HOME="$TMP_ROOT/home"
export ATHO_DATA_DIR="$TMP_ROOT/data"
mkdir -p "$HOME" "$ATHO_DATA_DIR"

cleanup() {
  if [[ -n "${APP_PID:-}" ]]; then
    kill "$APP_PID" >/dev/null 2>&1 || true
    wait "$APP_PID" >/dev/null 2>&1 || true
  fi
  rm -rf "$TMP_ROOT"
}
trap cleanup EXIT

(
  cd "$ROOT_DIR"
  cargo run -p atho-qt --bin atho-qt -- --network "$NETWORK" --local-node >"$LOG_FILE" 2>&1
) &
APP_PID=$!

sleep 8

osascript <<APPLESCRIPT
tell application "System Events"
  tell process "atho-qt"
    set frontmost to true
    repeat 60 times
      if exists window 1 then exit repeat
      delay 1
    end repeat
    if not (exists window 1) then error "Atho window did not appear"
    try
      click button "Send" of window 1
      delay 1
      click button "Receive" of window 1
      delay 1
      click button "Transactions" of window 1
      delay 1
      click button "Refresh" of window 1
      delay 1
      click button "Overview" of window 1
      delay 1
    on error errMsg
      error "Qt smoke automation failed: " & errMsg
    end try
  end tell
end tell
APPLESCRIPT

echo "qt ui smoke ok"
