#!/usr/bin/env bash
set -euo pipefail

while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    echo '{"id":1,"result":{"capabilities":{}}}'
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/start"'* ]]; then
    echo '{"id":999,"error":{"code":-32000,"message":"thread/start must not be called during resume"}}' >&2
    exit 1
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    if [[ "$line" != *'"threadId":"codex-thread-123"'* ]]; then
      echo '{"id":2,"error":{"code":-32001,"message":"unexpected threadId"}}' >&2
      exit 1
    fi
    echo '{"id":2,"result":{"id":"turn-1"}}'
    echo '{"method":"item/completed","params":{"item":{"id":"msg-1","type":"agentMessage","text":"resumed done"}}}'
    echo '{"method":"turn/completed","params":{"turnId":"turn-1"}}'
    exit 0
  fi
done
