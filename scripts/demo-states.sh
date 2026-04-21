#!/usr/bin/env bash
# demo-states.sh: cycle through all Clippy states so you can see the mascot
# animate without triggering real Claude Code hooks.
#
# Usage:
#   ./scripts/demo-states.sh          # cycle forever: idle -> working -> thinking -> alert -> ...
#   ./scripts/demo-states.sh once     # one pass, then idle

set -euo pipefail

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
STATE_BIN="$SCRIPT_DIR/../hooks/clippy-state"
DELAY="${CLIPPY_DEMO_DELAY:-5}"

cycle() {
  echo "→ idle"; "$STATE_BIN" on-stop; sleep "$DELAY"
  echo "→ working"; "$STATE_BIN" on-pre-tool; sleep "$DELAY"
  echo "→ thinking"; "$STATE_BIN" on-user-prompt; sleep "$DELAY"
  echo "→ needs attention"; "$STATE_BIN" on-notification; sleep "$DELAY"
}

if [[ "${1:-}" == "once" ]]; then
  cycle
  "$STATE_BIN" on-stop
  echo "done, back to idle"
  exit 0
fi

trap '"$STATE_BIN" on-stop; echo; echo "back to idle"; exit 0' INT TERM
echo "Looping every ${DELAY}s. Ctrl+C to stop."
while true; do
  cycle
done
