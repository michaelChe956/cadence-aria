#!/usr/bin/env bash
set -euo pipefail

# r19 续修回归 fixture(fix 轮 2):消费 initialize 写入后立即 kill -9 自身;
# 同进程组 `sleep 300` 继承 stdout/stderr 管道写端,EOF 被无限推迟——
# 会话泵对 initialize 应答的等待只能等 60s RPC 超时(r19 前形态)。
# exit-watch 竞速必须使会话在秒级以 Failed 终结。

if [[ "${1:-}" == "--version" ]]; then
  echo "kimi 0.34.0"
  exit 0
fi

while IFS= read -r line; do
  if [[ "$line" == *'"initialize"'* ]]; then
    sleep 300 &
    kill -9 $$
  fi
done
