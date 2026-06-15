#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "claude 2.1.160"
  exit 0
fi

sent_question=0
while IFS= read -r line; do
  if [[ "$line" == *'"initialize"'* ]]; then
    continue
  fi
  if [[ "$line" == *'"set_permission_mode"'* ]]; then
    continue
  fi
  if [[ "$sent_question" == "0" && "$line" == *'"user"'* ]]; then
    sent_question=1
    echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_question","name":"AskUserQuestion","input":{"questions":[{"question":"Continue?","options":[{"label":"Yes"},{"label":"No"}]}]}}]}}'
    continue
  fi
  if [[ "$line" == *'"tool_result"'* ]]; then
    echo '{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_question","is_error":true,"content":"User refused to answer"}]}}'
    # 正常不会走到这里：provider 在识别到 is_error 后会立即返回错误。
    echo '{"type":"result","subtype":"success","is_error":false,"result":"should not reach here","session_id":"claude_error_session"}'
    exit 0
  fi
done
