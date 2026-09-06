#!/usr/bin/env bash
set -euo pipefail

# F3 Task 4.1 修复轮（I2）：workspace 侧真实事件链 wire fixture。供
# WorkspaceEngine review drive 的真实 CodexProvider 策略会话使用：首次 review
# run 走 thread/start、结构化输出修复 run 走 thread/resume，两者都必须携带
# sandbox=read-only + approvalPolicy=on-request（GC5 三联动在两条路径同时成立）；
# 三类审批（fileChange/commandExecution/MCP elicitation）由 adapter 即时决策，
# 随后以 agentMessage 完成 turn。

while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-1}\",\"result\":{\"capabilities\":{}}}"
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/start"'* || "$line" == *'"method":"thread/resume"'* ]]; then
    if [[ "$line" != *'"sandbox":"read-only"'* || "$line" != *'"approvalPolicy":"on-request"'* ]]; then
      echo "policy thread start/resume must use read-only sandbox + on-request approval policy: $line" >&2
      exit 1
    fi
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-2}\",\"result\":{\"thread\":{\"id\":\"codex-thread-ws-audit\"}}}"
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-3}\",\"result\":{\"turn\":{\"id\":\"turn-ws-audit\"}}}"
    # fileChange item 先行（diff 经 item id 关联缓存），随后三类审批。
    echo '{"jsonrpc":"2.0","method":"item/started","params":{"item":{"type":"fileChange","id":"fc_1","path":"src/main.rs","changeType":"modify"}}}'
    echo '{"jsonrpc":"2.0","id":50,"method":"item/fileChange/requestApproval","params":{"itemId":"fc_1","reason":"natural language reason is not a classifier"}}'
  elif [[ "$line" == *'"id":50'* && "$line" == *'"decision":"decline"'* ]]; then
    echo '{"jsonrpc":"2.0","id":51,"method":"item/commandExecution/requestApproval","params":{"itemId":"cmd_1","command":"cargo build"}}'
  elif [[ "$line" == *'"id":51'* && "$line" == *'"decision":"decline"'* ]]; then
    echo '{"jsonrpc":"2.0","id":52,"method":"mcpServer/elicitation/request","params":{"serverName":"proj_spike","_meta":{"codex_approval_kind":"mcp_tool_call","tool_params":{"text":"hi"}}}}'
  elif [[ "$line" == *'"id":52'* && "$line" == *'"decision":"accept"'* ]]; then
    echo '{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"type":"agentMessage","id":"msg-1","text":"policy approvals done"},"threadId":"codex-thread-ws-audit","turnId":"turn-ws-audit"}}'
    echo '{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"codex-thread-ws-audit","turn":{"id":"turn-ws-audit","status":"completed"}}}'
    exit 0
  else
    echo "unexpected line: $line" >&2
    exit 2
  fi
done
