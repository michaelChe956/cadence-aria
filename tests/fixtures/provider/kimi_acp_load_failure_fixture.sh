#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "--version" ]]; then echo "kimi 0.34.0"; exit 0; fi
while IFS= read -r line; do
  id="$(printf '%s' "$line" | sed -n 's/.*"id"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
  if [[ "$line" == *'"initialize"'* ]]; then
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-1},\"result\":{\"protocolVersion\":1,\"agentCapabilities\":{\"loadSession\":true,\"sessionCapabilities\":{\"resume\":{}}}}}"
  elif [[ "$line" == *'"session/load"'* ]]; then
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"error\":{\"code\":-32001,\"message\":\"session unavailable\"}}"
    sleep 0.05
    exit 1
  elif [[ "$line" == *'"session/new"'* || "$line" == *'"session/prompt"'* ]]; then
    echo 'unexpected fallback request' >&2
    exit 98
  fi
done
