#!/usr/bin/env bash
set -euo pipefail

# I1 wire fixture（GC5 三联动在 resume 路径）：策略 input 的 thread/resume 请求
# 必须原样携带 sandbox="read-only" + approvalPolicy="on-request"（与 start 同源），
# 且带原 threadId；不得回退 danger-full-access。

while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"id\":\"${id:-aria-0}\",\"result\":{\"capabilities\":{}}}"
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/resume"'* ]]; then
    if [[ "$line" != *'"sandbox":"read-only"'* ]]; then
      echo "policy thread/resume must carry sandbox=read-only: $line" >&2
      exit 1
    fi
    if [[ "$line" != *'"approvalPolicy":"on-request"'* ]]; then
      echo "policy thread/resume must carry approvalPolicy=on-request: $line" >&2
      exit 1
    fi
    if [[ "$line" != *'"threadId":"codex-thread-resume-policy"'* ]]; then
      echo "thread/resume must resume the original thread id: $line" >&2
      exit 1
    fi
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"id\":\"${id:-aria-1}\",\"result\":{\"thread\":{\"id\":\"codex-thread-resume-policy\"}}}"
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"id\":\"${id:-aria-2}\",\"result\":{\"turn\":{\"id\":\"turn-resume-policy\"}}}"
    echo '{"method":"item/completed","params":{"item":{"id":"msg-1","type":"agentMessage","text":"policy resume done"}}}'
    echo '{"method":"turn/completed","params":{"turnId":"turn-resume-policy"}}'
    exit 0
  else
    echo "unexpected line: $line" >&2
    exit 2
  fi
done
