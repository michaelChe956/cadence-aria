#!/usr/bin/env bash
set -euo pipefail

# T13 Author 检测型岗位纪律 wire fixture（fileChange 审批请求、无前置 item 通知）：
# 危险基线下 fileChange 审批请求通常不出现（apply_patch 恒 AutoApprove），但若
# 出现即是 fileChange 事实——adapter 必须先合法应答 {"decision":"decline"}
# （不静默无应答；Decline 不是防写依据），随后走检测纪律中断 turn → Failed。
# 后续 agentMessage/turn/completed 不得消费（本轮不可采信）。

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
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-2}\",\"result\":{\"thread\":{\"id\":\"codex-thread-fc-req\"}}}"
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-3}\",\"result\":{\"turn\":{\"id\":\"turn-fc-req\"}}}"
    # fileChange 审批请求（无前置 item/started 通知）。
    echo '{"jsonrpc":"2.0","id":60,"method":"item/fileChange/requestApproval","params":{"itemId":"fc_req","reason":"natural language reason is not a classifier"}}'
  elif [[ "$line" == *'"id":60'* && "$line" == *'"decision":"decline"'* ]]; then
    # 已合法应答；adapter 随后必须中断 turn——以下「洗白」输出不得被消费。
    echo '{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"type":"agentMessage","id":"msg-late2","text":"whitewash attempt 2"},"threadId":"codex-thread-fc-req","turnId":"turn-fc-req"}}'
    echo '{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"codex-thread-fc-req","turn":{"id":"turn-fc-req","status":"completed"}}}'
    while IFS= read -r _; do :; done
    exit 0
  else
    echo "unexpected line: $line" >&2
    exit 2
  fi
done
