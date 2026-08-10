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
  elif [[ "$line" == *'"session/new"'* || "$line" == *'"session/load"'* ]]; then
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"result\":{\"sessionId\":\"kimi_tool_fixture\"}}"
  elif [[ "$line" == *'"session/prompt"'* ]]; then
    echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"kimi_tool_fixture","update":{"sessionUpdate":"tool_call","toolCallId":"tool_1","title":"Bash","kind":"execute","status":"pending","rawInput":"{\"command\":\"pwd\"}"}}}'
    echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"kimi_tool_fixture","update":{"sessionUpdate":"tool_call_update","toolCallId":"tool_1","status":"in_progress","content":[{"type":"content","content":{"type":"text","text":"/"}}]}}}'
    echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"kimi_tool_fixture","update":{"sessionUpdate":"tool_call_update","toolCallId":"tool_1","status":"completed","content":[{"type":"content","content":{"type":"text","text":"tmp\n"}}]}}}'
    echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"kimi_tool_fixture","update":{"sessionUpdate":"tool_call","toolCallId":"tool_2","title":"Bash","kind":"execute","status":"pending","rawInput":"{\"command\":\"false\"}"}}}'
    echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"kimi_tool_fixture","update":{"sessionUpdate":"tool_call_update","toolCallId":"tool_2","status":"failed","content":[{"type":"content","content":{"type":"text","text":"failed"}}]}}}'
    sleep 0.02
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-3},\"result\":{\"stopReason\":\"end_turn\"}}"
    exit 0
  fi
done
