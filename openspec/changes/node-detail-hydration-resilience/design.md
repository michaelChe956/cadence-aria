# Design: node-detail-hydration-resilience

> 完整诊断依据：`.superpowers/sdd/2026-09-23_计划文档_change实施_plan-compile-gate-visibility_v1.0/f47-token-flaky-diagnosis.md`（F47Diag，探针实测+pi 会话文件数值对齐）。

## Context

F-47 B 面：`workspace-ws-store.ts:268-271` 快照到达无条件重建 nodeDetails 空壳不 merge；水合去重 ref（`useCockpitNodeDetailHydration.ts:27/43-46`、`ChatWorkspacePageLegacy.tsx:162/331-334`）只在 sessionId 变化时清空→不再二次拉取→token 行消失不自愈。WS 快照内联白名单（`session_state.rs:77-88`）不含 author_run/reviewer_run。B3：水合集合只含 completed+active/selected。A 面：pi get_state 不返回 cost（pi 0.86.1 rpc-mode.js:347-362）→ usage 全靠 `local_usage.rs` 兜底（128KiB/200 行旧窗口、无重试）；emit/落盘 `let _ =` 静默（session.rs:669-706、timeline.rs:337-345）；node_002（1/32）实证从未落盘。

## Decisions

### D1 B2 用 merge 语义而非「快照后清 ref」

两个等效实现中选 merge：清 ref 方案会引发水合风暴（每帧快照全量重拉），merge 只在 store 层保留已水合数据、快照内联覆盖对应节点，与「按需 REST 拉取」架构一致。若实现发现 merge 引入内联/REST 数据版本倒挂，回退清 ref 方案（spec REQ-NDR-01 已写等效条款）。

### D2 B3 水合集合=呈现节点全集

`useCockpitNodeDetailHydration` 的集合判定从 completed+active/selected 扩到含 failed/aborted/interrupted；维持按需拉取（不全量预取）。

### D3 B1 pending 占位纯呈现层

TimelineNodeList token 行在 detail 未到时渲染 pending 态；确认无 usage 事件后转缺失态。不往 WS summary 加 token 字段（proposal Non-Goal）。

### D4 A1 pi 接入 F-45 扫描器

`read_pi_usage` 从 `tail_lines` 固定窗口改为 `latest_matching_line` 反向块扫描+200ms 重试，与 `read_kimi_usage` 同构；kimi 路径零改动。

### D5 A2 可观测性最小面

emit_pi_usage 三条静默 warn 分支升级为结构化 warn（带环节标签 rpc/parse/file）；落盘 `let _ =` 改为捕获失败并写诊断事件（复用 kind=artifact 之外新 kind=usage_diag 或等价既有通道——实施时按 execution_events 惯例定，超 bounded 原则）；不影响 provider 会话与 gate。

## Risks / Trade-offs

- merge 实现的版本语义（快照内联 vs REST 已水合的优先级）——REQ-NDR-01 场景二钉「内联优先」
- B3 扩面增加 REST 拉取量（失败节点通常少量，可接受）
- A2 结构化 warn 的日志落盘通道（服务器 stderr 无文件——diagnosis concern）——实施时评估 warn 是否需要 durable 化，避免只写 stderr 仍不可检索

## Open Questions（实施首步验证）

- F47Diag 判别实验（复现 pi author 盯页面：实时出现过→落盘失败；从未出现→RPC/parse）可在 A1/A2 落地后用可观测数据自然回答，不阻塞实施
