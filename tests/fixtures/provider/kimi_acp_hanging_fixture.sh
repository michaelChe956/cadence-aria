#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "kimi 0.34.0"
  exit 0
fi

while IFS= read -r line; do
  id="$(printf '%s' "$line" | sed -n 's/.*"id"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
  if [[ "$line" == *'"initialize"'* ]]; then
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-1},\"result\":{\"protocolVersion\":1,\"agentCapabilities\":{\"loadSession\":true,\"sessionCapabilities\":{\"resume\":{}}}}}"
  elif [[ "$line" == *'"session/new"'* ]]; then
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"result\":{\"sessionId\":\"hanging_kimi_fixture\"}}"
  elif [[ "$line" == *'"session/prompt"'* ]]; then
    # Keep stdin readable so the test can prove ACP session/cancel arrives before
    # ProcessManager's mandatory termination fallback.
    if IFS= read -r cancel_line; then
      if [[ "$cancel_line" == *'"session/cancel"'* ]]; then
        [[ -z "${KIMI_CANCEL_MARKER:-}" ]] || : > "$KIMI_CANCEL_MARKER"
        echo 'Kimi cancellation received' >&2
        sleep 30
      fi
    fi
  fi
done
