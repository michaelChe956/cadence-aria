#!/usr/bin/env bash
set -euo pipefail

# Author 岗位纪律 wire fixture（T13，Direct 冻结片段改版）：thread/start 必须
# sandbox=danger-full-access + approvalPolicy=on-request（检测型岗位纪律，
# 不再是 read-only 档）；commandExecution 审批按定稿 Author 策略得到即时
# {"decision":"accept"}（与 Claude/Pi 对齐，不用 never 自禁）；MCP elicitation
# （带 _meta.codex_approval_kind="mcp_tool_call"）得到 {"decision":"accept"}。
# fileChange 检测纪律由独立 fixture 覆盖（本 fixture 的 happy path 不含
# fileChange——事件流检得 fileChange 事实即中断 turn、本轮 Failed）。

while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-1}\",\"result\":{\"capabilities\":{}}}"
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/start"'* ]]; then
    if [[ "$line" != *'"sandbox":"danger-full-access"'* || "$line" != *'"approvalPolicy":"on-request"'* ]]; then
      echo "author discipline thread/start must use danger-full-access sandbox + on-request approval policy: $line" >&2
      exit 1
    fi
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-2}\",\"result\":{\"thread\":{\"id\":\"codex-thread-policy\"}}}"
  elif [[ "$line" == *'"method":"thread/resume"'* ]]; then
    if [[ "$line" != *'"sandbox":"danger-full-access"'* ]]; then
      echo "author discipline thread/resume must carry sandbox=danger-full-access: $line" >&2
      exit 1
    fi
    if [[ "$line" != *'"approvalPolicy":"on-request"'* ]]; then
      echo "author discipline thread/resume must carry approvalPolicy=on-request: $line" >&2
      exit 1
    fi
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-4}\",\"result\":{\"thread\":{\"id\":\"codex-thread-policy\"}}}"
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-3}\",\"result\":{\"turn\":{\"id\":\"turn-policy\"}}}"
    # 已知形态两类：commandExecution（Author 定稿=Accept）先行，随后 MCP。
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
