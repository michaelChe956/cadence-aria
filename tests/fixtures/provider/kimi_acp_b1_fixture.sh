#!/usr/bin/env bash
# B1-1(波 2)Kimi adapter 资源登记接线测试夹具。
#
#   hold(默认)     —— 正常 ACP 握手/会话;prompt 终态应答后子进程保持存活
#                     (最多 5 秒或 stdin 关闭),用于证明「协议 Completed
#                     不提前注销」与真实 PID+starttime 登记。
#   handshake_fail —— initialize 应答 JSON-RPC error 后短暂存活再退出:覆盖
#                     会话错误路径收口(组信号/回收)后注销。
#   no_hold        —— 与 hold 同形,但 prompt 终态应答后立即退出:供
#                     「无载体零登记」等无需存活窗口的用例快速收口。
#   hold_turn      —— prompt 只发一条 agent_message_chunk 后挂起 60 秒
#                     (不回 prompt result):轮次在途,供「取消收口后注销」。
#
# KIMI_B1_SPAWN_MARKER(可选) —— 任意一次调用(含 --version)先追加一行标记,
# 供「预留失败零 spawn」探针使用。
set -euo pipefail

if [[ -n "${KIMI_B1_SPAWN_MARKER:-}" ]]; then
  printf '%s\n' "${1:-session}" >> "$KIMI_B1_SPAWN_MARKER"
fi

mode="${KIMI_B1_MODE:-hold}"

if [[ "${1:-}" == "--version" ]]; then
  echo "kimi 0.34.0"
  exit 0
fi

while IFS= read -r line; do
  id="$(printf '%s' "$line" | sed -n 's/.*"id"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
  if [[ "$line" == *'"initialize"'* ]]; then
    if [[ "$mode" == "handshake_fail" ]]; then
      echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-1},\"error\":{\"code\":-32000,\"message\":\"b1 fixture handshake failure\"}}"
      sleep 0.3
      exit 0
    fi
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-1},\"result\":{\"protocolVersion\":1,\"agentCapabilities\":{\"loadSession\":true,\"sessionCapabilities\":{\"resume\":{}}}}}"
  elif [[ "$line" == *'"session/new"'* ]]; then
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"result\":{\"sessionId\":\"b1_fixture\"}}"
  elif [[ "$line" == *'"session/prompt"'* ]]; then
    echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"b1_fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"b1 output"}}}}'
    if [[ "$mode" == "hold_turn" ]]; then
      # 轮次在途:不回 prompt result,保持存活等待取消收口。
      sleep 60
      exit 0
    fi
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-3},\"result\":{\"stopReason\":\"end_turn\"}}"
    if [[ "$mode" == "hold" ]]; then
      # Completed 已发,子进程仍存活(不依赖 stdin):登记必须仍在。
      sleep 5
    fi
    exit 0
  fi
done
