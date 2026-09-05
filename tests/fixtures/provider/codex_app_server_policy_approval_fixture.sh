#!/usr/bin/env bash
set -euo pipefail

# 策略会话 wire fixture（GC5/GC6）：thread/start 必须三联动 sandbox=read-only +
# approvalPolicy=on-request；fileChange/commandExecution 审批必须得到即时
# {"decision":"decline"}（不经 ApprovalBridge 上抛）；MCP elicitation（带
# _meta.codex_approval_kind="mcp_tool_call"）必须得到 {"decision":"accept"}。

while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-1}\",\"result\":{\"capabilities\":{}}}"
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/start"'* ]]; then
    if [[ "$line" != *'"sandbox":"read-only"'* || "$line" != *'"approvalPolicy":"on-request"'* ]]; then
      echo "policy thread/start must use read-only sandbox + on-request approval policy: $line" >&2
      exit 1
    fi
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-2}\",\"result\":{\"thread\":{\"id\":\"codex-thread-policy\"}}}"
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-3}\",\"result\":{\"turn\":{\"id\":\"turn-policy\"}}}"
    # fileChange item 先行（diff 经 item id 关联缓存），随后三类审批。
    echo '{"jsonrpc":"2.0","method":"item/started","params":{"item":{"type":"fileChange","id":"fc_1","path":"src/main.rs","changeType":"modify"}}}'
    echo '{"jsonrpc":"2.0","id":50,"method":"item/fileChange/requestApproval","params":{"itemId":"fc_1","reason":"natural language reason is not a classifier"}}'
  elif [[ "$line" == *'"id":50'* && "$line" == *'"decision":"decline"'* ]]; then
    echo '{"jsonrpc":"2.0","id":51,"method":"item/commandExecution/requestApproval","params":{"itemId":"cmd_1","command":"cargo build"}}'
  elif [[ "$line" == *'"id":51'* && "$line" == *'"decision":"decline"'* ]]; then
    echo '{"jsonrpc":"2.0","id":52,"method":"mcpServer/elicitation/request","params":{"serverName":"proj_spike","_meta":{"codex_approval_kind":"mcp_tool_call","tool_params":{"text":"hi"}}}}'
  elif [[ "$line" == *'"id":52'* && "$line" == *'"decision":"accept"'* ]]; then
    echo '{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"type":"agentMessage","id":"msg-1","text":"policy approvals done"},"threadId":"codex-thread-policy","turnId":"turn-policy"}}'
    echo '{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"codex-thread-policy","turn":{"id":"turn-policy","status":"completed"}}}'
    exit 0
  else
    echo "unexpected line: $line" >&2
    exit 2
  fi
done
