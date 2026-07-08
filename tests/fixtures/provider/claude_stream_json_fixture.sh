#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "claude 2.1.119"
  exit 0
fi

STORY_SPEC='# Story Spec\n\n## 范围\n来源 source id: Issue issue_0001；Claude fixture 生成可审核的候选产物。\n\n## 用户故事\n作为审核者，我希望 Claude fixture 输出结构完整且可追踪。\n\n## 功能需求\n- [REQ-001] Claude fixture generates a valid candidate artifact.\n\n## 成功标准\n- [AC-001] The candidate artifact can proceed to review.\n\n## 待确认项\n无\n\n## 非功能需求\n无\n'

while IFS= read -r line; do
  if [[ "$line" == *'"initialize"'* ]]; then
    continue
  fi
  if [[ "$line" == *'"set_permission_mode"'* ]]; then
    continue
  fi
  if [[ "$line" == *'"user"'* ]]; then
    echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"'"${STORY_SPEC}"'"}]},"session_id":"claude_fixture_session"}'
    echo '{"type":"control_request","request_id":"perm_req_001","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"cargo test","description":"Run cargo tests"},"tool_use_id":"toolu_fixture"}}'
  fi
  if [[ "$line" == *'"control_response"'* ]]; then
    echo '{"type":"result","subtype":"success","is_error":false,"result":"'"${STORY_SPEC}"'","session_id":"claude_fixture_session"}'
    exit 0
  fi
done
