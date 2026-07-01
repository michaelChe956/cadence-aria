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
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"result\":{\"thread\":{\"id\":\"codex_multi_user_input_thread\"},\"approvalPolicy\":\"never\"}}"
  elif [[ "$line" == *'"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-3},\"result\":{\"turn\":{\"id\":\"codex_multi_user_input_turn\",\"status\":\"inProgress\"}}}"
    echo '{"jsonrpc":"2.0","id":91,"method":"item/tool/requestUserInput","params":{"threadId":"codex_multi_user_input_thread","turnId":"codex_multi_user_input_turn","itemId":"ask_multi","questions":[{"id":"startup","header":"启动","question":"启动自检策略？","options":[{"label":"每次启动都自检"},{"label":"仅失败后自检"}]},{"id":"scope","header":"范围","question":"影响范围？","options":[{"label":"仅 Story Spec"},{"label":"Story/Design/Work Item 共享链路"}]},{"id":"mcp_events","header":"事件","question":"MCP 事件输出？","options":[{"label":"输出 MCP 事件"},{"label":"仅记录日志"}]}]}}'
  elif [[ "$line" == *'"answers"'* ]]; then
    if [[ "$line" != *'"startup"'* || "$line" != *'"每次启动都自检"'* ]]; then
      echo '{"jsonrpc":"2.0","method":"turn/failed","params":{"msg":{"type":"turn_failed","message":"missing startup answer"}}}'
      exit 0
    fi
    if [[ "$line" != *'"scope"'* || "$line" != *'"Story/Design/Work Item 共享链路"'* ]]; then
      echo '{"jsonrpc":"2.0","method":"turn/failed","params":{"msg":{"type":"turn_failed","message":"missing scope answer"}}}'
      exit 0
    fi
    if [[ "$line" != *'"mcp_events"'* || "$line" != *'"输出 MCP 事件"'* ]]; then
      echo '{"jsonrpc":"2.0","method":"turn/failed","params":{"msg":{"type":"turn_failed","message":"missing mcp_events answer"}}}'
      exit 0
    fi
    echo '{"jsonrpc":"2.0","method":"item/agentMessage/delta","params":{"threadId":"codex_multi_user_input_thread","turnId":"codex_multi_user_input_turn","itemId":"message_001","delta":"Codex received all answers"}}'
    echo '{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"codex_multi_user_input_thread","turn":{"id":"codex_multi_user_input_turn","status":"completed"}}}'
    exit 0
  fi
done
