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
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"result\":{\"sessionId\":\"kimi_text_fixture\"}}"
  elif [[ "$line" == *'"session/prompt"'* ]]; then
    echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"kimi_text_fixture","update":{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"private thought"}}}}'
    echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"kimi_text_fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Kimi fixture output"}}}}'
    sleep 0.02
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-3},\"result\":{\"stopReason\":\"end_turn\"}}"
    exit 0
  fi
done
