#!/usr/bin/env bash
set -euo pipefail

# LC danger 会话 wire fixture（T12→T13 改版）：Author 承载角色的 LC 会话
# thread/start 必须 sandbox=danger-full-access + approvalPolicy=on-request
# （全相位 danger 基线）；commandExecution 审批按定稿 Author 策略得到即时
# {"decision":"accept"}（与 Claude/Pi 对齐）；MCP elicitation（带
# _meta.codex_approval_kind="mcp_tool_call"）得到 {"decision":"accept"}。
# fileChange 检测纪律（item 事实→中断 turn→本轮 Failed）由 direct 检测
# fixture 在 LC 路径复测，本 fixture happy path 不含 fileChange。

while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-1}\",\"result\":{\"capabilities\":{}}}"
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/start"'* ]]; then
    if [[ "$line" != *'"sandbox":"danger-full-access"'* || "$line" != *'"approvalPolicy":"on-request"'* ]]; then
      echo "lc danger thread/start must use danger-full-access sandbox + on-request approval policy: $line" >&2
      exit 1
    fi
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-2}\",\"result\":{\"thread\":{\"id\":\"codex-thread-policy\"}}}"
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-3}\",\"result\":{\"turn\":{\"id\":\"turn-policy\"}}}"
    echo '{"jsonrpc":"2.0","id":51,"method":"item/commandExecution/requestApproval","params":{"itemId":"cmd_1","command":"cargo build"}}'
  elif [[ "$line" == *'"id":51'* && "$line" == *'"decision":"accept"'* ]]; then
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
