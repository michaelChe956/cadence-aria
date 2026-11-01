#!/usr/bin/env bash
set -euo pipefail

# I2 wire fixture（GC6 未知 item 形态）：未知 `item/*/requestApproval` 必须收到
# {"decision":"decline"} 应答（id 原样回带）；未知 item 与未知 elicitation 的
# 连续未知计入同一会话计数，第 3 次触发 unknown_approval_storm 终止。

while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"id\":\"${id:-aria-0}\",\"result\":{\"capabilities\":{}}}"
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"id\":\"${id:-aria-1}\",\"result\":{\"thread\":{\"id\":\"codex-thread-unknown-item\"}}}"
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"id\":\"${id:-aria-2}\",\"result\":{\"turn\":{\"id\":\"turn-1\"}}}"
    # 第 1 次未知：未识别的 item/*/requestApproval 形态（server 数字 id 70）。
    echo '{"jsonrpc":"2.0","id":70,"method":"item/unrecognizedForm/requestApproval","params":{"itemId":"u_1"}}'
  elif [[ "$line" == *'"id":70'* ]]; then
    if [[ "$line" != *'"decision":"decline"'* ]]; then
      echo "unknown item approval must receive decision=decline: $line" >&2
      exit 1
    fi
    # 第 2 次未知：generic elicitation（无 _meta.codex_approval_kind，id 71）。
    echo '{"jsonrpc":"2.0","id":71,"method":"mcpServer/elicitation/request","params":{"serverName":"proj_spike"}}'
  elif [[ "$line" == *'"id":71'* && "$line" == *'-32601'* ]]; then
    # 第 3 次未知：另一种未识别 item 形态（id 72）→ 终止。
    echo '{"jsonrpc":"2.0","id":72,"method":"item/anotherNewKind/requestApproval","params":{"itemId":"u_3"}}'
  elif [[ "$line" == *'"id":72'* ]]; then
    if [[ "$line" != *'"decision":"decline"'* ]]; then
      echo "third unknown item must still receive decision=decline before termination: $line" >&2
      exit 1
    fi
    exit 0
  else
    echo "unexpected line: $line" >&2
    exit 2
  fi
done
