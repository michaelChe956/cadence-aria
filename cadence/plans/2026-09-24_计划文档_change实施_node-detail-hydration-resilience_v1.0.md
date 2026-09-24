# change 实施 node-detail-hydration-resilience 实施计划 v1.0

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 修复 F-47——快照不清空已水合 node detail（merge 语义）、水合覆盖全终态节点、token 位 pending 占位、pi usage 兜底接入反向扫描、usage 失败可观测。

**Architecture:** 前端三件（store merge/hook 集合/占位）走 vitest；后端两件（local_usage pi 复用 F-45 扫描器/emit-落盘可观测）走 cargo。两文件面零交集，可并行实施。

**Tech Stack:** TS（vitest+tsc）+ Rust（cargo lib）。

**Spec:** `openspec/changes/node-detail-hydration-resilience/`（strict valid）；诊断依据 `.superpowers/sdd/2026-09-23_计划文档_change实施_plan-compile-gate-visibility_v1.0/f47-token-flaky-diagnosis.md`（含探针场景与全部行号）。

## Global Constraints

- 不往 WS session_state 快照加 author/reviewer detail 内联（白名单 session_state.rs:77-88 不动）。
- kimi 扫描器（F-45 产物 `latest_matching_line`）只复用不改动。
- 诊断写失败不影响 provider 会话与 gate 行为（REQ-NDR-05）。
- 前端命令：`cd web && npx vitest --run <路径>` + `pnpm exec tsc --noEmit`；后端照仓规（--lib 定向、禁 -j）。
- 行号漂移以符号定位（诊断行号为 2026-09-24 时点）。

## Review Focus

1. merge 版本倒挂（快照内联旧数据覆盖 REST 新数据）→ Task 1 内联优先用例
2. 水合扩面引发全量预取风暴（应维持按需）→ Task 2 集合断言
3. pending 占位闪烁（水合极快时占位一闪）→ Task 3 终态转换用例（占位仅 detail 未到时）
4. pi 扫描器复用改动 kimi 行为 → Task 4 kimi 用例零改动全绿
5. 可观测 warn 只写 stderr 仍不可检索（服务器无日志文件——诊断 concern）→ Task 5 落盘失败必须 durable 化（诊断事件）

---

### Task 1: 快照 merge（REQ-NDR-01，前端）

**Files:** `web/src/state/workspace-ws-store.ts:268-271`（快照应用重建处）+ 测试。

**Steps:**

- [ ] **1.1** 失败测试（F47Diag 探针三步转正式用例）：①快照→空 detail；②setNodeDetail（含 usage）→metadata.usage=1；③再一帧快照→**仍=1**（现状红：回 0）。另：快照内联某节点 detail 时该节点以内联呈现（内联优先）。
- [ ] **1.2** 跑红 → merge 实现（保留未内联已水合节点；内联覆盖对应节点）→ 跑绿。
- [ ] **1.3** vitest 相关面 + tsc → commit `fix(ws-store): 快照 merge 保留已水合 node detail（REQ-NDR-01，F-47）`。

### Task 2: 水合集合全终态（REQ-NDR-02，前端）

**Files:** `useCockpitNodeDetailHydration.ts:33-41`、`ChatWorkspacePageLegacy.tsx:320-330` + 测试。

**Steps:**

- [ ] **2.1** 失败测试：failed/aborted/interrupted 节点进入水合集合（completed+active/selected 既有集合不变）；按需语义不变（无全量预取断言：不可见节点不拉）。
- [ ] **2.2** 跑红 → 集合判定扩展 → 跑绿；failed 节点 token 可见用例（合成 failed 节点+usage 事件）。
- [ ] **2.3** vitest+tsc → commit `fix(hydration): 水合集合纳入 failed/aborted/interrupted（REQ-NDR-02，F-47）`。

### Task 3: token 位 pending 占位（REQ-NDR-03，前端）

**Files:** `TimelineNodeList.tsx:283-311` + 测试。

**Steps:**

- [ ] **3.1** 失败测试：summary 在而 detail 未到→token 位渲染 pending 占位；detail 到→数值或确认缺失终态；detail 在但无 usage 事件→缺失态（非 pending）。
- [ ] **3.2** 跑红 → 占位实现（纯呈现层）→ 跑绿；vitest+tsc → commit `feat(ui): token 位未水合 pending 占位（REQ-NDR-03，F-47）`。

### Task 4: pi usage 兜底接入反向扫描（REQ-NDR-04，后端）

**Files:** `src/cross_cutting/local_usage.rs`（read_pi_usage :24-29 固定 tail_lines 窗口）+ 内联测试。

**Steps:**

- [ ] **4.1** 失败测试：pi 会话文件 usage 记录在 128KiB 尾窗之外（大行构造）→ 现实现 None（红）→ 接入 `latest_matching_line` + 200ms 短重试（kimi 同构）→ 读到。pi 既有用例零改动全绿；kimi 用例零改动全绿（只复用不改）。
- [ ] **4.2** 跑红→绿；fmt/clippy → commit `fix(usage): pi 兜底读取接入反向块扫描与短重试（REQ-NDR-04，F-47）`。

### Task 5: usage 失败可观测（REQ-NDR-05，后端）

**Files:** `src/cross_cutting/pi_provider/session.rs:669-706`（emit_pi_usage 三静默分支）、usage 落盘点（`timeline.rs:337-345` let _ = 处）+ 测试。

**Steps:**

- [ ] **5.1** 失败测试：①emit 三环节（rpc/parse/file）失败各落结构化 warn（环节标签）②落盘失败写可检索诊断事件（durable 化，非仅 stderr——Review Focus 5）③诊断写失败不影响 gate/provider 行为。
- [ ] **5.2** 跑红→实现（warn 结构化+落盘失败诊断事件，通道按 execution_events 惯例）→跑绿；fmt/clippy → commit `feat(usage): emit/落盘失败可观测（REQ-NDR-05，F-47）`。

### Task 6: 门禁收口

- [ ] **6.1** 前端 vitest 全量 + tsc 0；后端 lib 全量 + it_web（handler 面未触则记录依据）；strict 复跑；guard 绿；刷新场景手动复验脚本（快照帧注入后 token 保持）交 controller 浏览器复验清单。

---

## Self-Review

覆盖：REQ-NDR-01→Task 1、02→Task 2、03→Task 3、04→Task 4、05→Task 5；Non-Goals 钉死（Constraints）。Review Focus 五条全挂测试。前端 Task 1-3 串行（同域 store/hook/组件链）；后端 Task 4-5 与前端零交集可并行。
