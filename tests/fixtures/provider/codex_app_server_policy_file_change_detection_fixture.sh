#!/usr/bin/env bash
set -euo pipefail

# T13 Author 检测型岗位纪律 wire fixture（Direct Direct frozen fragment）：thread/start
# 必须 sandbox=danger-full-access + approvalPolicy=on-request；turn 内出现
# fileChange item 通知（item/started，无任何审批请求）即触发检测纪律——
# adapter 必须：ToolPolicyWarning(author_file_change_detected) → durable
# file_change_detected（含处置事实）→ 中断 turn（kill）。随后的 agentMessage
# 与 turn/completed（「迟到 Completed」）不得被消费——本轮 Failed、不可采信。

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
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-2}\",\"result\":{\"thread\":{\"id\":\"codex-thread-detect\"}}}"
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-3}\",\"result\":{\"turn\":{\"id\":\"turn-detect\"}}}"
    # fileChange item 通知先行（无审批请求）——检测型纪律的唯一真实信号
    # （danger 下 apply_patch 恒 AutoApprove，审批请求不出现）。
    echo '{"jsonrpc":"2.0","method":"item/started","params":{"item":{"type":"fileChange","id":"fc_detect","path":"src/main.rs","changeType":"modify"}}}'
    # 迟到 Completed 洗白探针：adapter 中断 turn 后不得消费以下两行。
    echo '{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"type":"agentMessage","id":"msg-late","text":"whitewash attempt"},"threadId":"codex-thread-detect","turnId":"turn-detect"}}'
    echo '{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"codex-thread-detect","turn":{"id":"turn-detect","status":"completed"}}}'
    # 保持存活等待 kill 链终止（不应再收到任何请求）。
    while IFS= read -r _; do :; done
    exit 0
  else
    echo "unexpected line: $line" >&2
    exit 2
  fi
done
