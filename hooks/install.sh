#!/usr/bin/env bash
# Install Clippy hooks into ~/.claude/settings.json.
# Idempotent: safe to re-run. Adds or replaces the "clippy" marker block.
#
# Usage:
#   ./install.sh [--uninstall]

set -euo pipefail

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
STATE_BIN="$SCRIPT_DIR/clippy-state"

if [[ ! -x "$STATE_BIN" ]]; then
  chmod +x "$STATE_BIN"
fi

SETTINGS_DIR="$HOME/.claude"
SETTINGS_FILE="$SETTINGS_DIR/settings.json"
mkdir -p "$SETTINGS_DIR"
[[ -f "$SETTINGS_FILE" ]] || echo "{}" > "$SETTINGS_FILE"

UNINSTALL=false
if [[ "${1:-}" == "--uninstall" ]]; then
  UNINSTALL=true
fi

python3 - "$SETTINGS_FILE" "$STATE_BIN" "$UNINSTALL" <<'PY'
import json, sys, pathlib

settings_path = pathlib.Path(sys.argv[1])
state_bin = sys.argv[2]
uninstall = sys.argv[3] == "True"

data = {}
if settings_path.stat().st_size > 0:
    data = json.loads(settings_path.read_text())
data.setdefault("hooks", {})

MARKER = "clippy"

def strip(hooks_list):
    return [h for h in hooks_list if h.get("_source") != MARKER]

def add(event_key, subcommand, matcher=None):
    entry = {
        "_source": MARKER,
        "hooks": [{"type": "command", "command": f"{state_bin} {subcommand}"}],
    }
    if matcher is not None:
        entry["matcher"] = matcher
    data["hooks"].setdefault(event_key, [])
    data["hooks"][event_key] = strip(data["hooks"][event_key])
    if not uninstall:
        data["hooks"][event_key].append(entry)

# Map CC hook events → state transitions
add("UserPromptSubmit", "on-user-prompt")
add("PreToolUse", "on-pre-tool", matcher="*")
add("PostToolUse", "on-post-tool", matcher="*")
add("Notification", "on-notification")
add("Stop", "on-stop")
add("SessionStart", "on-session-start")
add("SessionEnd", "on-session-end")
add("PreCompact", "on-compact-start")

# Clean empty lists
for k in list(data["hooks"].keys()):
    if not data["hooks"][k]:
        del data["hooks"][k]

settings_path.write_text(json.dumps(data, indent=2) + "\n")
print(f"{'Uninstalled' if uninstall else 'Installed'} Clippy hooks in {settings_path}")
PY
