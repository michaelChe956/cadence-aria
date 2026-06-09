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
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"result\":{\"thread\":{\"id\":\"codex_current_permission_thread\"},\"approvalPolicy\":\"on-request\"}}"
  elif [[ "$line" == *'"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-3},\"result\":{\"turn\":{\"id\":\"codex_current_permission_turn\",\"status\":\"inProgress\"}}}"
    echo '{"jsonrpc":"2.0","id":44,"method":"item/commandExecution/requestApproval","params":{"threadId":"codex_current_permission_thread","turnId":"codex_current_permission_turn","itemId":"cmd_approval_001","startedAtMs":1790000000000,"command":"/bin/zsh -lc '\''pnpm -C web install --frozen-lockfile'\''","cwd":"/tmp"}}'
  elif [[ "$line" == *'"id":44'* ]]; then
    if [[ "$line" == *'"result":{"decision":"accept"}'* && "$line" != *'"method"'* && "$line" != *'"response"'* ]]; then
      echo '{"jsonrpc":"2.0","method":"item/agentMessage/delta","params":{"threadId":"codex_current_permission_thread","turnId":"codex_current_permission_turn","itemId":"message_001","delta":"permission accepted"}}'
      echo '{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"codex_current_permission_thread","turn":{"id":"codex_current_permission_turn","status":"completed"}}}'
      exit 0
    fi
    echo "unexpected approval response: $line" >&2
    exit 2
  fi
done
