#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "--version" ]]; then echo "kimi 0.34.0"; exit 0; fi
while IFS= read -r line; do
  id="$(printf '%s' "$line" | sed -n 's/.*"id"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
  if [[ "$line" == *'"initialize"'* ]]; then
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-1},\"result\":{\"protocolVersion\":1,\"agentCapabilities\":{\"loadSession\":true,\"sessionCapabilities\":{\"resume\":{}}}}}"
  elif [[ "$line" == *'"session/new"'* ]]; then
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"result\":{\"sessionId\":\"fast_fixture\"}}"
  elif [[ "$line" == *'"session/prompt"'* ]]; then
    echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fast_fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"first"}}}}'
    echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fast_fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":" second"}}}}'
    echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fast_fixture","update":{"sessionUpdate":"tool_call","toolCallId":"fast_tool","title":"Bash","kind":"execute","status":"pending","rawInput":"{\"command\":\"true\"}"}}}'
    echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fast_fixture","update":{"sessionUpdate":"tool_call_update","toolCallId":"fast_tool","status":"completed","content":[{"type":"content","content":{"type":"text","text":"ok"}}]}}}'
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-3},\"result\":{\"stopReason\":\"end_turn\"}}"
    exit 0
  fi
done
