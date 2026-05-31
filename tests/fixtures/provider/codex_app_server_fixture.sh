#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "codex 0.124.0"
  exit 0
fi

while IFS= read -r line; do
  if [[ "$line" == *'"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-1},\"result\":{\"capabilities\":{}}}"
  elif [[ "$line" == *'"thread/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"result\":{\"thread\":{\"id\":\"codex_thread_fixture\"},\"model\":\"gpt-5.3-codex\"}}"
  elif [[ "$line" == *'"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-3},\"result\":{}}"
    echo '{"jsonrpc":"2.0","method":"codex/event","params":{"msg":{"type":"item_completed","item":{"type":"message","role":"assistant","content":[{"type":"text","text":"Codex fixture chunk"}]}}}}'
    echo '{"jsonrpc":"2.0","id":44,"method":"codex/server_request","params":{"type":"command_execution_request_approval","request_id":44,"params":{"item_id":"codex_perm_001","command":"cargo test"}}}'
  elif [[ "$line" == *'"command_execution_request_approval"'* || "$line" == *'"Accept"'* || "$line" == *'"accept"'* ]]; then
    echo '{"jsonrpc":"2.0","method":"codex/event","params":{"msg":{"type":"turn_completed","turn_status":"completed"}}}'
    exit 0
  fi
done
