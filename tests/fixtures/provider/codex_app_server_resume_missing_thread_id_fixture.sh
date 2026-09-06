#!/usr/bin/env bash
set -euo pipefail

# F1（最终审）wire fixture：thread/resume 应答不携带 thread id（真实损坏形态：
# result 对象缺 /thread/id 与 /id）。旧实现会回退请求中的旧 session id 冒充
# 握手确认并继续 turn/start；修复后握手必须失败（本 fixture 不应答 turn/start，
# 若被调用即说明旧 id 冒充路径复活）。

while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"id\":\"${id:-1}\",\"result\":{\"capabilities\":{}}}"
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/resume"'* ]]; then
    if [[ "$line" != *'"threadId":"codex-thread-123"'* ]]; then
      echo "{\"id\":\"${id:-2}\",\"error\":{\"code\":-32001,\"message\":\"unexpected resume threadId\"}}" >&2
      exit 1
    fi
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    # 应答缺 thread id：握手必须失败，不得回退请求 id。
    echo "{\"id\":\"${id:-2}\",\"result\":{}}"
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"id\":\"${id:-3}\",\"error\":{\"code\":-32004,\"message\":\"turn/start reached without a confirmed resume thread id\"}}" >&2
    exit 1
  fi
done
