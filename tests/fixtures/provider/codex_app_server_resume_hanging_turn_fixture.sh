#!/usr/bin/env bash
set -euo pipefail

while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    echo '{"id":1,"result":{"capabilities":{}}}'
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/resume"'* ]]; then
    echo '{"id":2,"result":{"thread":{"id":"codex-thread-stale"}}}'
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    echo '{"id":3,"result":{"turn":{"id":"turn-stale"}}}'
    sleep 10
    exit 0
  fi
done
