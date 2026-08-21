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
    echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_question","name":"AskUserQuestion","input":{"questions":[{"question":"Scope?","options":[{"label":"Global"},{"label":"Project"}]}]}}]}}'
    # 旧版 CLI 只发 tool_use、不发 control_request；aria 必须报告协议不兼容。
    exit 0
  fi
  if [[ "$line" == *'"tool_result"'* ]]; then
    echo "aria must not inject AskUserQuestion tool_result: $line" >&2
    exit 44
  fi
done
