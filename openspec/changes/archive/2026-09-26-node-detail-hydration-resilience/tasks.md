# Tasks: node-detail-hydration-resilience

**工作包性质**：只登记高层工作包与验收口径；精确步骤由 `superpowers:writing-plans` 展开到 `cadence/plans/`。

**全局边界**：不往 WS 快照加 author/reviewer detail 内联；不动 kimi 扫描器（F-45 产物）；诊断写失败不影响 provider 会话与 gate 行为。

## 1. 快照 merge 与水合完整性（前端）

- [x] 1.1 workspace-ws-store 快照应用改为 merge 保留已水合 nodeDetails（内联优先），store 级回归测试锁死「水合后快照到达不清空」（F47Diag 探针三步场景转正式用例）（REQ-NDR-01）。
- [x] 1.2 水合集合纳入 failed/aborted/interrupted 呈现节点；failed 节点 token 可见用例（REQ-NDR-02）。
- [x] 1.3 TimelineNodeList token 位 pending 占位（水合未到→pending；到→数值/缺失终态）（REQ-NDR-03）。

## 2. pi usage 兜底与可观测性（后端）

- [x] 2.1 read_pi_usage 接入 latest_matching_line 反向扫描+短重试（kimi 同构，F-45 扫描器复用）；大会话文件可读红绿用例（REQ-NDR-04）。
- [x] 2.2 emit_pi_usage 三条静默分支结构化 warn（环节标签）；usage 落盘失败写可检索诊断事件（durable 化评估见 design D5），不影响 gate（REQ-NDR-05）。

## 3. 回归与门禁

- [x] 3.1 前端 vitest 全量+tsc；后端 lib/it_web 按触及面；change strict 校验；刷新场景手动复验脚本（快照帧注入后 token 保持）。
