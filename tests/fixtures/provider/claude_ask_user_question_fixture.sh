#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "claude 2.1.160"
  exit 0
fi

STORY_SPEC='# Story Spec\n\n## 范围\n来源 source id: Issue issue_0001；AskUserQuestion fixture 在用户回答后生成候选产物。\n\n## 用户故事\n作为审核者，我希望 Claude Code 原生提问后仍继续同一个 provider 会话。\n\n## 功能需求\n- [REQ-001] Claude AskUserQuestion fixture continues with a valid candidate artifact.\n\n## 成功标准\n- [AC-001] The candidate artifact can proceed to review after the user answers.\n\n## 待确认项\n无\n\n## 非功能需求\n无\n'

sent_request=0
while IFS= read -r line; do
  if [[ "$line" == *'"initialize"'* ]]; then
    continue
  fi
  if [[ "$line" == *'"set_permission_mode"'* ]]; then
    continue
  fi
  if [[ "$sent_request" == "0" && "$line" == *'"user"'* ]]; then
    sent_request=1
    echo '{"type":"control_request","request_id":"ask_req_001","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{"question":"Drink?","options":[{"label":"Tea"},{"label":"Coffee"}]}]},"tool_use_id":"toolu_question"}}'
    continue
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
    echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_question","name":"AskUserQuestion","input":{"questions":[{"question":"Drink?","options":[{"label":"Tea"},{"label":"Coffee"}]}]}}]}}'
    continue
  fi
  if [[ "$line" == *'"tool_result"'* ]]; then
    if [[ "$line" != *'"tool_use_id":"toolu_question"'* || "$line" != *'Your questions have been answered'* || "$line" != *'Drink?'* || "$line" != *'Tea'* ]]; then
      echo "missing AskUserQuestion tool_result: $line" >&2
      exit 44
    fi
    echo '{"type":"result","subtype":"success","is_error":false,"result":"'"${STORY_SPEC}"'","session_id":"claude_fixture_session"}'
    exit 0
  fi
done
