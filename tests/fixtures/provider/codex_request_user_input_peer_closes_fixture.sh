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
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"result\":{\"thread\":{\"id\":\"codex_input_thread\"},\"approvalPolicy\":\"never\"}}"
  elif [[ "$line" == *'"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-3},\"result\":{\"turn\":{\"id\":\"codex_input_turn\",\"status\":\"inProgress\"}}}"
    echo '{"jsonrpc":"2.0","id":88,"method":"item/tool/requestUserInput","params":{"threadId":"codex_input_thread","turnId":"codex_input_turn","itemId":"ask_1","questions":[{"id":"confirm","header":"确认","question":"继续？","options":[{"label":"是"},{"label":"否"}]}]}}'
    # 关闭标准输入读端，使 provider 写入 JSON-RPC 响应时立即收到 Broken pipe，
    # 但保持 stdout 打开，避免 provider 因读到 EOF 而误判为其他错误。
    exec 0<&-
    sleep 5
  fi
done
