#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "claude 2.1.160"
  exit 0
fi

while IFS= read -r line; do
  if [[ "$line" == *'"initialize"'* ]]; then
    continue
  fi
  if [[ "$line" == *'"set_permission_mode"'* ]]; then
    continue
  fi
  if [[ "$line" == *'"user"'* ]]; then
    echo '{"type":"control_request","request_id":"ask_req_001","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{"question":"Drink?","options":[{"label":"Tea"},{"label":"Coffee"}]}]},"tool_use_id":"toolu_question"}}'
  fi
  if [[ "$line" == *'"control_response"'* ]]; then
    if [[ "$line" != *'"subtype":"success"'* ]]; then
      echo "missing SDK success subtype: $line" >&2
      exit 42
    fi
    if [[ "$line" != *'"updatedInput"'* || "$line" != *'"answers"'* || "$line" != *'"Drink?"'* || "$line" != *'"Tea"'* ]]; then
      echo "missing selected answer: $line" >&2
      exit 43
    fi
    echo '{"type":"result","subtype":"success","is_error":false,"result":"choice continued","session_id":"claude_fixture_session"}'
    exit 0
  fi
done
