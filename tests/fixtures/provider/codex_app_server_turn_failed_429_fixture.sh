#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "codex 0.133.0"
  exit 0
fi

# E1 定因回放 fixture（wire 形态取自 spike T2-resume-writes.jsonl 第 22 行）：
# 上游 429 限流时 app-server 在 turn/start 应答后立刻下发
# turn/completed{turn.status:"failed", turn.error.message 含 429 文案}，零 agent 输出。
# 最多应答 2 轮 turn/start：
# - 修复前（现状红）：失败轮被误判为完成轮 → in-session 空输出重试发起第 2 轮
#   turn/start → 仍 429 失败 → provider_empty_output（真实 429 错误被吞）；
# - 修复后：第 1 个失败轮即以原始 429 文案终止会话，绝不应出现第 2 轮 turn/start。
turn_count=0
while IFS= read -r line; do
  if [[ "$line" == *'"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-1}\",\"result\":{\"userAgent\":\"cadence-aria-test\"}}"
  elif [[ "$line" == *'"thread/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-2}\",\"result\":{\"thread\":{\"id\":\"codex_turn_failed_thread\"},\"approvalPolicy\":\"never\"}}"
  elif [[ "$line" == *'"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    turn_count=$((turn_count + 1))
    if [[ "$turn_count" -le 2 ]]; then
      echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-3}\",\"result\":{\"turn\":{\"id\":\"codex_turn_failed_turn_${turn_count}\",\"status\":\"inProgress\"}}}"
      echo "{\"jsonrpc\":\"2.0\",\"method\":\"turn/completed\",\"params\":{\"threadId\":\"codex_turn_failed_thread\",\"turn\":{\"id\":\"codex_turn_failed_turn_${turn_count}\",\"items\":[],\"itemsView\":\"notLoaded\",\"status\":\"failed\",\"error\":{\"message\":\"exceeded retry limit, last status: 429 Too Many Requests\",\"codexErrorInfo\":{\"responseTooManyFailedAttempts\":{\"httpStatusCode\":429}},\"additionalDetails\":null,\"misalignment\":null},\"startedAt\":1788597100,\"completedAt\":1788597105,\"durationMs\":4698}}}"
    else
      echo "unexpected turn/start #$turn_count: failed turn must not trigger in-session retry" >&2
      exit 1
    fi
  fi
done
