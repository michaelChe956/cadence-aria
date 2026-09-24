# Proposal

## Why

F-47 实证缺陷（两面并存）：
- **B 展示面（间歇性主因）**：REST 水合完成后任一帧 WS session_state 快照到达 → 前端 `workspace-ws-store` **无条件重建 nodeDetails 为空壳（不 merge 已水合数据）** → author/reviewer 节点 token 行静默消失**且不自愈**（水合去重 ref 不清空、不再二次拉取）；WS 快照内联白名单不含 author_run/reviewer_run，所有 workspace 类型同暴露。次因：水合集合只含 completed+active/selected（failed/aborted 节点永不水合）；水合未完成期间 token 位无 pending 占位。
- **A 数据面（低频 1/32）**：pi provider 的 usage 依赖本地 jsonl 兜底读取（pi 0.86.1 get_state 不返回 cost），兜底仍是 128KiB/200 行旧窗口无重试——F-45 只加固了 kimi；且 emit/落盘失败全程静默（`let _ =`），node_002 实证从未落盘而无任何可观测痕迹。

## What Changes

1. **快照 merge 语义（B2）**：WS 快照到达时对已水合 nodeDetails 做 merge 而非空壳重建（或等效：快照后清空水合去重 ref 允许二次拉取）；store 级回归测试锁死。
2. **水合集合完整性（B3）**：failed/aborted/interrupted 节点纳入水合集合。
3. **token 位 pending 占位（B1）**：detail 未到时 token 位渲染 pending 态而非直接不渲染（纯呈现层）。
4. **pi usage 兜底可靠性（A1）**：`read_pi_usage` 复用 F-45 通用反向块扫描器 `latest_matching_line` + 短重试，与 kimi 同构。
5. **usage 失败可观测（A2）**：emit_pi_usage 失败与落盘失败不再静默——结构化诊断事件/可见 warn，可区分失败环节。

## Capabilities

### New Capabilities

- `node-detail-hydration-resilience`: timeline 节点 detail 的 WS 快照/REST 水合一致性契约（merge 语义、水合集合完整性、pending 占位）与 provider usage 落盘可靠性/可观测性契约

### Modified Capabilities

（无；与 `usage-transparency` 的关系在 sync 时核对——A1/A2 若与其既有 requirement 重叠，归档前合并措辞）

## Non-Goals

- 不改 WS session_state 快照的内联 detail 白名单（不往快照里塞 author_run/reviewer_run detail——按需 REST 拉取的架构不变）
- 不动 F-45 已交付的 kimi 扫描器行为（A1 是把 pi 接到同一扫描器上）
- 不做 pi RPC 面判别实验的实施（诊断已给实验设计，A2 落地后可观测性自然覆盖）
- 不改 TimelineNodeList 之外的呈现组件
