#!/usr/bin/env bash
set -euo pipefail

# Coder 会话 wire fixture（GC6）：MCP elicitation（带 _meta.codex_approval_kind=
# "mcp_tool_call"）必须即时 accept（不经 ApprovalBridge）并走既有 execution
# event 审计通道。

while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-1}\",\"result\":{\"capabilities\":{}}}"
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-2}\",\"result\":{\"thread\":{\"id\":\"codex-thread-coder-mcp\"}}}"
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-3}\",\"result\":{\"turn\":{\"id\":\"turn-coder-mcp\"}}}"
    echo '{"jsonrpc":"2.0","id":60,"method":"mcpServer/elicitation/request","params":{"serverName":"proj_spike","_meta":{"codex_approval_kind":"mcp_tool_call","tool_params":{"text":"hi"}}}}'
  elif [[ "$line" == *'"id":60'* && "$line" == *'"decision":"accept"'* ]]; then
    echo '{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"type":"agentMessage","id":"msg-1","text":"coder mcp approved"},"threadId":"codex-thread-coder-mcp","turnId":"turn-coder-mcp"}}'
    echo '{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"codex-thread-coder-mcp","turn":{"id":"turn-coder-mcp","status":"completed"}}}'
    exit 0
  else
    echo "unexpected line: $line" >&2
    exit 2
  fi
done
