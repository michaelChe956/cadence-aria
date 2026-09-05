#!/usr/bin/env bash
set -euo pipefail

# GC7/GC6 wire fixture：出站请求必须使用 aria-<seq> 字符串 id；携数字 id 的
# generic elicitation（无 _meta.codex_approval_kind）必须得到 -32601+data 应答
# 且应答 id 原样回带；同会话第 3 次连续未知形态后 provider 终止。

while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    if [[ "$line" != *'"id":"aria-'* ]]; then
      echo "codex peer outbound request ids must use the aria namespace: $line" >&2
      exit 1
    fi
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-1}\",\"result\":{\"capabilities\":{}}}"
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-2}\",\"result\":{\"thread\":{\"id\":\"codex-thread-unknown\"}}}"
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-3}\",\"result\":{\"turn\":{\"id\":\"turn-1\"}}}"
    # 第一次未知 elicitation：server 数字 id 0，无 _meta.codex_approval_kind。
    echo '{"jsonrpc":"2.0","id":0,"method":"mcpServer/elicitation/request","params":{"serverName":"proj_spike"}}'
  elif [[ "$line" == *'"id":0'* && "$line" == *'-32601'* ]]; then
    if [[ "$line" != *'"unsupported_approval_kind"'* ]]; then
      echo "elicitation error reply must carry fixed reason data: $line" >&2
      exit 1
    fi
    echo '{"jsonrpc":"2.0","id":1,"method":"mcpServer/elicitation/request","params":{"serverName":"proj_spike"}}'
  elif [[ "$line" == *'"id":1'* && "$line" == *'-32601'* ]]; then
    echo '{"jsonrpc":"2.0","id":2,"method":"mcpServer/elicitation/request","params":{"serverName":"proj_spike"}}'
  elif [[ "$line" == *'"id":2'* && "$line" == *'-32601'* ]]; then
    exit 0
  else
    echo "unexpected line: $line" >&2
    exit 2
  fi
done
