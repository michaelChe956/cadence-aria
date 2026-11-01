# usage-transparency Specification

## Purpose

让每个 provider turn 的 token 用量（input/output/cache_read）在协议层缺失时从 CLI 本地会话文件采集，并在前端消息气泡可见。

## Requirements

### Requirement: 本地会话文件 usage 采集（SHALL）
系统 SHALL 在 pi/codex/kimi 的 provider turn 终止时，按各 CLI 的本地会话文件布局读取最近的 per-turn usage 记录并经 UsageReport 上报；文件缺失、格式变化或字段缺省时 SHALL 静默不上报（不影响主流程）。kimi SHALL 读取 agents/*/wire.jsonl 的 usage.record（usageScope=="turn"）并覆盖 subagent 用量。

#### Scenario: 正常采集
- **WHEN** kimi turn 完成且 wire.jsonl 存在 usage.record
- **THEN** UsageReport 发出且 input≈inputOther+inputCacheCreation、cache_read=inputCacheRead

#### Scenario: 容错降级
- **WHEN** 会话文件不存在或无 usage 记录
- **THEN** 不上报、不报错、主流程不受影响

### Requirement: 消息气泡 token 展示（SHALL）
前端 SHALL 在 assistant/reviewer 消息气泡渲染对应 turn 的 token 用量（input/output/cache_read 紧凑展示）；usage 缺失时 SHALL 不显示该行。

#### Scenario: 气泡显示
- **WHEN** WS 收到 kind=usage 的 execution_event 且对应消息气泡存在
- **THEN** 气泡显示 token 行（如 "输入 89,035 · 输出 8,896 · 缓存 230,976"）

#### Scenario: 无数据不显示
- **WHEN** 无 usage 事件
- **THEN** 气泡无 token 行，无占位符
