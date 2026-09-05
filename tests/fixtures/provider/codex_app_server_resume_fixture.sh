#!/usr/bin/env bash
set -euo pipefail

saw_resume=0

while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"id\":\"${id:-1}\",\"result\":{\"capabilities\":{}}}"
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"id\":\"${id:-2}\",\"error\":{\"code\":-32000,\"message\":\"thread/start must not be called during resume\"}}" >&2
    exit 1
  elif [[ "$line" == *'"method":"thread/resume"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    if [[ "$line" != *'"threadId":"codex-thread-123"'* ]]; then
      echo "{\"id\":\"${id:-2}\",\"error\":{\"code\":-32001,\"message\":\"unexpected resume threadId\"}}" >&2
      exit 1
    fi
    saw_resume=1
    echo "{\"id\":\"${id:-2}\",\"result\":{\"thread\":{\"id\":\"codex-thread-123\"}}}"
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    if [[ "$saw_resume" != "1" ]]; then
      echo "{\"id\":\"${id:-3}\",\"error\":{\"code\":-32002,\"message\":\"turn/start before thread/resume\"}}" >&2
      exit 1
    fi
    if [[ "$line" != *'"threadId":"codex-thread-123"'* ]]; then
      echo "{\"id\":\"${id:-3}\",\"error\":{\"code\":-32003,\"message\":\"unexpected turn threadId\"}}" >&2
      exit 1
    fi
    echo "{\"id\":\"${id:-3}\",\"result\":{\"turn\":{\"id\":\"turn-1\"}}}"
    echo '{"method":"item/completed","params":{"item":{"id":"msg-1","type":"agentMessage","text":"resumed done"}}}'
    echo '{"method":"turn/completed","params":{"turnId":"turn-1"}}'
    exit 0
  fi
done
