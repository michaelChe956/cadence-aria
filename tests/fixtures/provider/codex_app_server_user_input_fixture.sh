#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "codex 0.133.0"
  exit 0
fi

while IFS= read -r line; do
  if [[ "$line" == *'"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-1},\"result\":{\"userAgent\":\"cadence-aria-test\"}}"
  elif [[ "$line" == *'"thread/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"result\":{\"thread\":{\"id\":\"codex_user_input_thread\"},\"approvalPolicy\":\"never\"}}"
  elif [[ "$line" == *'"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-3},\"result\":{\"turn\":{\"id\":\"codex_user_input_turn\",\"status\":\"inProgress\"}}}"
    echo '{"jsonrpc":"2.0","id":77,"method":"item/tool/requestUserInput","params":{"threadId":"codex_user_input_thread","turnId":"codex_user_input_turn","itemId":"ask_1","questions":[{"id":"complexity","header":"需求确认","question":"请选择复杂度","options":[{"label":"O(n)","description":"线性复杂度"},{"label":"O(1)","description":"常数复杂度"}]}]}}'
  elif [[ "$line" == *'"answers"'* ]]; then
    if [[ "$line" != *'"complexity"'* || "$line" != *'"O(n)"'* ]]; then
      echo '{"jsonrpc":"2.0","method":"turn/failed","params":{"msg":{"type":"turn_failed","message":"unexpected answer payload"}}}'
      exit 0
    fi
    echo '{"jsonrpc":"2.0","method":"item/agentMessage/delta","params":{"threadId":"codex_user_input_thread","turnId":"codex_user_input_turn","itemId":"message_001","delta":"Codex received O(n)"}}'
    echo '{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"codex_user_input_thread","turn":{"id":"codex_user_input_turn","status":"completed"}}}'
    exit 0
  fi
done
