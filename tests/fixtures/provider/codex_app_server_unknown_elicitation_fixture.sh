#!/usr/bin/env bash
set -euo pipefail

# GC7/GC6 wire fixture：codex 出站 request id 必须使用 typed aria 命名空间且
# 0 起（首出站 initialize=aria-0，字面断言）；同一会话内 server 数字 id 0 与
# client aria-0 共存且互不冲突。携数字 id 的 generic elicitation（无
# _meta.codex_approval_kind）必须得到 -32601+data 应答且应答 id 原样回带；
# 同会话第 3 次连续未知形态后 provider 终止。

while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    # 契约字面要求：codex 首出站 request id = aria-0（0 起 seq）。
    if [[ "$line" != *'"id":"aria-0"'* ]]; then
      echo "codex first outbound request id must be exactly \"aria-0\": $line" >&2
      exit 1
    fi
    echo '{"jsonrpc":"2.0","id":"aria-0","result":{"capabilities":{}}}'
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/start"'* ]]; then
    if [[ "$line" != *'"id":"aria-1"'* ]]; then
      echo "codex second outbound request id must be exactly \"aria-1\": $line" >&2
      exit 1
    fi
    echo '{"jsonrpc":"2.0","id":"aria-1","result":{"thread":{"id":"codex-thread-unknown"}}}'
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    if [[ "$line" != *'"id":"aria-2"'* ]]; then
      echo "codex third outbound request id must be exactly \"aria-2\": $line" >&2
      exit 1
    fi
    echo '{"jsonrpc":"2.0","id":"aria-2","result":{"turn":{"id":"turn-1"}}}'
    # 第一次未知 elicitation：server 数字 id 0，无 _meta.codex_approval_kind。
    # 与 client aria-0 在同一会话共存（GC7），值域隔离互不误配。
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
