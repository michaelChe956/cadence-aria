#!/usr/bin/env bash
set -euo pipefail

while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"id\":\"${id:-1}\",\"result\":{\"capabilities\":{}}}"
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    if [[ "$line" != *'"sandbox":"danger-full-access"'* ]]; then
      echo "{\"id\":\"${id:-2}\",\"error\":{\"code\":-32010,\"message\":\"thread/start must request danger-full-access sandbox\"}}" >&2
      exit 1
    fi
    echo "{\"id\":\"${id:-2}\",\"result\":{\"thread\":{\"id\":\"codex-thread-sandbox\"}}}"
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"id\":\"${id:-3}\",\"result\":{\"turn\":{\"id\":\"turn-1\"}}}"
    echo '{"method":"item/completed","params":{"item":{"id":"msg-1","type":"agentMessage","text":"sandbox disabled done"}}}'
    echo '{"method":"turn/completed","params":{"turnId":"turn-1"}}'
    exit 0
  fi
done
