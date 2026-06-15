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
    continue
  fi
  if [[ "$line" == *'"tool_result"'* ]]; then
    # 正常不会走到这里，因为测试会在 choice 阶段取消。
    sleep 30
  fi
done
