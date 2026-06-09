#!/usr/bin/env bash
set -euo pipefail

while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    echo '{"id":1,"result":{"capabilities":{}}}'
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/start"'* ]]; then
    if [[ "$line" == *'"ephemeral":true'* ]]; then
      echo '{"id":2,"error":{"code":-32004,"message":"thread/start must create a persistent thread for resume"}}' >&2
      exit 1
    fi
    echo '{"id":2,"result":{"thread":{"id":"codex-thread-persistent"}}}'
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    if [[ "$line" != *'"threadId":"codex-thread-persistent"'* ]]; then
      echo '{"id":3,"error":{"code":-32003,"message":"unexpected turn threadId"}}' >&2
      exit 1
    fi
    echo '{"id":3,"result":{"turn":{"id":"turn-1"}}}'
    echo '{"method":"item/completed","params":{"item":{"id":"msg-1","type":"agentMessage","text":"persistent thread done"}}}'
    echo '{"method":"turn/completed","params":{"turnId":"turn-1"}}'
    exit 0
  fi
done
