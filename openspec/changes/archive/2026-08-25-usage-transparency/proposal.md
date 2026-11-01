# Proposal: usage-transparency

## Why
「输入 token 减少」是既定优化目标，但四 provider 中三家（pi/codex/kimi）的协议层不携带 per-turn usage（真机实测：pi get_state.cost 恒 null、codex turn/completed 无 usage、kimi ACP 未实现 issue#2394/#1855），数据只存在于各家 CLI 的本地会话文件（三家均实测有完整 token 记录）。同时用户要求 token 用量在聊天消息气泡中可见。

## What Changes
- 后端：pi/codex/kimi 三个 provider 的 turn 终止时读取各自本地会话文件的 usage 记录（防御性解析，读不到不上报），经既有 UsageReport→execution_event 管道落 timeline/WS
- 前端：聊天消息气泡渲染 author/reviewer 的 token 用量（紧凑单行：input/output/cache_read）
- codex parse 嵌套 fallback 已修（10bbe803）

## Capabilities
### New Capabilities
- `usage-transparency`: 本地会话文件 usage 采集 + 气泡展示

## Non-goals
- kimi/pi/codex 协议层修复（等官方）；金额成本换算；历史会话回填。
