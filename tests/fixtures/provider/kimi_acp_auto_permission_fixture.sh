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
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"result\":{\"sessionId\":\"kimi_auto_permission_fixture\"}}"
  elif [[ "$line" == *'"session/prompt"'* ]]; then
    echo '{"jsonrpc":"2.0","id":"auto-permission","method":"session/request_permission","params":{"options":[{"optionId":"allow-once","name":"Allow once","kind":"allow_once"},{"optionId":"reject-once","name":"Reject once","kind":"reject_once"}],"toolCall":{"toolCallId":"auto-tool","title":"Bash","content":{"type":"text","text":"pwd"}}}}'
    response=""
    if IFS= read -r response; then
      if [[ "$response" != *'"optionId":"allow-once"'* || "$response" != *'"outcome":"selected"'* ]]; then
        echo "unexpected auto permission response: $response" >&2
        exit 1
      fi
    else
      exit 1
    fi
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-3},\"result\":{\"stopReason\":\"end_turn\"}}"
    exit 0
  fi
done
