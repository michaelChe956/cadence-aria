#!/usr/bin/env bash
set -euo pipefail

# I1 wire fixture（T13，Direct 冻结片段改版）：Author 岗位纪律 input 的
# thread/resume 请求必须原样携带 sandbox="danger-full-access" +
# approvalPolicy="on-request"（与 start 同源；旧 read-only 三联动不复现），
# 且带原 threadId。

while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"id\":\"${id:-aria-0}\",\"result\":{\"capabilities\":{}}}"
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/resume"'* ]]; then
    if [[ "$line" != *'"sandbox":"danger-full-access"'* ]]; then
      echo "author discipline thread/resume must carry sandbox=danger-full-access: $line" >&2
      exit 1
    fi
    if [[ "$line" != *'"approvalPolicy":"on-request"'* ]]; then
      echo "author discipline thread/resume must carry approvalPolicy=on-request: $line" >&2
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
