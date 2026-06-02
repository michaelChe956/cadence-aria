#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "codex 0.130.0"
  exit 0
fi

while IFS= read -r line; do
  if [[ "$line" == *'"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    if [[ "$line" != *'"clientInfo"'* ]]; then
      echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-1},\"error\":{\"code\":-32600,\"message\":\"Invalid request: missing field clientInfo\"}}"
      exit 0
    fi
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-1},\"result\":{\"userAgent\":\"cadence-aria-test\"}}"
  elif [[ "$line" == *'"thread/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    if [[ "$line" != *'"approvalPolicy":"never"'* && "$line" != *'"approvalPolicy":"on-request"'* ]]; then
      echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"error\":{\"code\":-32600,\"message\":\"Invalid request: unknown approvalPolicy\"}}"
      exit 0
    fi
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"result\":{\"thread\":{\"id\":\"codex_current_thread_fixture\"},\"approvalPolicy\":\"never\"}}"
  elif [[ "$line" == *'"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    cwd="$(pwd -P)"
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-3},\"result\":{\"turn\":{\"id\":\"codex_current_turn_fixture\",\"status\":\"inProgress\"}}}"
    echo "{\"jsonrpc\":\"2.0\",\"method\":\"item/started\",\"params\":{\"threadId\":\"codex_current_thread_fixture\",\"turnId\":\"codex_current_turn_fixture\",\"item\":{\"type\":\"commandExecution\",\"id\":\"cmd_001\",\"command\":\"pwd\",\"cwd\":\"$cwd\"}}}"
    echo "{\"jsonrpc\":\"2.0\",\"method\":\"item/completed\",\"params\":{\"threadId\":\"codex_current_thread_fixture\",\"turnId\":\"codex_current_turn_fixture\",\"item\":{\"type\":\"commandExecution\",\"id\":\"cmd_001\",\"command\":\"pwd\",\"cwd\":\"$cwd\",\"aggregatedOutput\":\"$cwd\\n\",\"exitCode\":0}}}"
    echo '{"jsonrpc":"2.0","method":"item/agentMessage/delta","params":{"threadId":"codex_current_thread_fixture","turnId":"codex_current_turn_fixture","itemId":"message_001","delta":"# Story Spec\n\n## 功能需求\n- [REQ-001] Codex current fixture generates a valid candidate artifact.\n\n## 成功标准\n- [AC-001] The candidate artifact can proceed to review."}}'
    echo '{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"type":"agentMessage","id":"message_001","text":"# Story Spec\n\n## 功能需求\n- [REQ-001] Codex current fixture generates a valid candidate artifact.\n\n## 成功标准\n- [AC-001] The candidate artifact can proceed to review.","phase":"final_answer"},"threadId":"codex_current_thread_fixture","turnId":"codex_current_turn_fixture"}}'
    echo '{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"codex_current_thread_fixture","turn":{"id":"codex_current_turn_fixture","status":"completed"}}}'
    exit 0
  fi
done
