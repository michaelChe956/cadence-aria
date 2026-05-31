#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "claude 2.1.119"
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
    echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Claude fixture chunk"}]},"session_id":"claude_fixture_session"}'
    echo '{"type":"control_request","request_id":"perm_req_001","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"cargo test","description":"Run cargo tests"},"tool_use_id":"toolu_fixture"}}'
  fi
  if [[ "$line" == *'"control_response"'* ]]; then
    echo '{"type":"result","subtype":"success","is_error":false,"result":"Claude fixture chunk done","session_id":"claude_fixture_session"}'
    exit 0
  fi
done
