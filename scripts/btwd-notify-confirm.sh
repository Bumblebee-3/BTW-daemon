#!/usr/bin/env bash
set -euo pipefail

# Usage:
#   btwd-notify-confirm.sh <request_id> <title> <body>
#
# Shows a desktop notification with Yes/No actions.
# When clicked, writes either "yes" or "no" to:
#   ${XDG_RUNTIME_DIR}/btwd-confirm-<request_id>

request_id="${1:-}"
title="${2:-btwd}"
body="${3:-Confirm?}"

if [[ -z "$request_id" ]]; then
  echo "missing request_id" >&2
  exit 2
fi

runtime_dir="${XDG_RUNTIME_DIR:-/tmp}"
out_path="${runtime_dir}/btwd-confirm-${request_id}"

# For swaync, we can still create action buttons via notify-send.
# IMPORTANT: do NOT attempt to programmatically invoke actions here.
# Instead, we show actions and wait for the user click via DBus.
if command -v swaync-client >/dev/null 2>&1 && command -v notify-send >/dev/null 2>&1; then
  # notify-send prints the notification id to stdout when actions are used
  # (implementation varies, so we defensively parse the last integer).
  nid_raw="$(notify-send -a btwd -u critical -t 0 \
    --action="yes=Yes" \
    --action="no=No" \
    "$title" "$body" 2>/dev/null || true)"

  nid="$(printf '%s' "$nid_raw" | grep -Eo '[0-9]+' | tail -n 1 || true)"
  if [[ -z "$nid" ]]; then
    exit 0
  fi

  # Wait for ActionInvoked(notification_id, action_key) and write it to the spool file.
  # Use a timeout so the helper never hangs forever.
  if command -v gdbus >/dev/null 2>&1; then
    action="$(timeout 30s gdbus monitor --session --dest org.freedesktop.Notifications 2>/dev/null \
      | awk -v nid="$nid" '
          /ActionInvoked/ {
            line=$0;
            # Match: uint32 <nid> then string "yes"/"no".
            if (match(line, /uint32[[:space:]]+([0-9]+)/, m) && m[1]==nid) {
              if (match(line, /string[[:space:]]+\"(yes|no)\"/, a)) { print a[1]; exit }
            }
          }
        ' || true)"
    if [[ "$action" == "yes" || "$action" == "no" ]]; then
      printf '%s' "$action" >"$out_path"
    fi
  fi

  exit 0
fi

# Prefer dunstify if present (supports actions + prints chosen action).
if command -v dunstify >/dev/null 2>&1; then
  # dunstify prints the selected action key to stdout.
  action="$(dunstify -a btwd -u critical -t 0 \
    -A yes,Yes -A no,No \
    "$title" "$body" || true)"
  if [[ "$action" == "yes" || "$action" == "no" ]]; then
    printf '%s' "$action" >"$out_path"
    exit 0
  fi
  exit 0
fi

# notify-send generally does not support actions. Fall back to a plain notification.
if command -v notify-send >/dev/null 2>&1; then
  notify-send "$title" "$body" -u critical -t 10000 || true
  exit 0
fi

exit 0
