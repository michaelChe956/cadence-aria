# change 实施 ears-delivery-normalization 实施计划 v1.0

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** SC 交付侧 EARS 关键字空白确定性归一化 + generate 段编译语法失败教学重驱扩面，修复 F-48（`」THE SYSTEM SHALL` 缺空格→终态）。

**Architecture:** 归一化并入既有 `prepare_author_delivery_for_compile` 归一化层（与结构标题归一化同入口）；重驱白名单在编译 loop 扩 parse 语法类；prompt 判例净增。

**Tech Stack:** Rust（cargo test --locked --lib 定向；fmt/clippy 门禁）。

**Spec:** `openspec/changes/ears-delivery-normalization/`（strict valid）；诊断依据 `.superpowers/sdd/2026-09-23_计划文档_change实施_plan-compile-gate-visibility_v1.0/f48-ears-fail-diagnosis.md`。

## Global Constraints

- 不放宽 `is_ears_statement` 字面量契约（grammar.rs:20-24 EARS_*_PREFIX 不动）。
- 不动 contract_autorepair 既有收敛范围；不动 Final Compile recovery 门。
- 「每 candidate 至多一次教学重驱」上限不变（single_candidate.rs 编译 loop 语义）。
- 归一化只允许三类空白操作：`WHEN` 后补半角空格、`THE SYSTEM SHALL` 前补半角空格、关键字邻位 U+3000/NBSP→半角；正文与 CJK 标点零触碰；救回落审计诊断/事件（与结构标题归一化同形态）。
- prompt 预算红线 22,000 字节（prompts.rs:96/255），判例净增不破线。
- 测试命令规范照仓规（禁 -j、定向带 --lib）。

## Review Focus

1. 归一化误改正文（CJK 标点被改/非关键字行被碰）→ Task 1 用例
2. 重驱扩面突破一次上限（两类失败先后命中）→ Task 2 用例
3. fixture 保真（durable 原文提取的尾换行/编码陷阱——F-46 教训：python json 读，勿 jq -r）→ Task 1 Step 1.1
4. 判例与既有 CJK 空格规则重复/冲突 → Task 3 断言
5. 未知标题仍 fail-closed（归一化扩面不吞结构标题归一化的既有拒绝语义）→ Task 1.2

---

### Task 0: 重驱白名单类别集实查（Open Question 收口）

- [ ] **0.1** grep parse.rs 全量 parse 语法错误类别枚举（invalid_ears/invalid_work_item_id 等未收敛者清单），对照 contract_autorepair 已消费类别，得出「可教学 parse 类」精确集合写入报告（决定 Task 2 白名单）。

### Task 1: EARS 关键字空白归一化（REQ-WSC-02）

**Files:** `normalize.rs`（prepare_author_delivery_for_compile 归一化层；以符号定位实际路径）+ 测试。

**Steps:**

- [ ] **1.1** fixture 提取：python json 解析 `.aria/projects/project_0001/issues/issue_0002/workspace-timelines/workspace_session_0009/timeline_node_details/timeline_node_002.json` 的 `streaming_content`（17,005 字符），存 `src/product/workspace_engine/tests/fixtures/f48_plan_ears_raw.md`（逐字节保真，勿 jq）。
- [ ] **1.2** 失败测试：①fixture 全文经归一化后编译通过（F-48 端到端救回，红→绿主用例）②最小样本 `WHEN条件」THE SYSTEM SHALL 响应` 补空格后合法 ③`WHEN条件 THE SYSTEM SHALL响应`（WHEN 后缺空格）④关键字邻位 U+3000/NBSP 归半角 ⑤归一化诊断/事件落一条 ⑥CJK 标点与正文逐字保留（非关键字行 diff 为空）。
- [ ] **1.3** 跑红 → 实现三类空白修复（护栏：仅 EARS 关键字字面量邻位）→ 跑绿。
- [ ] **1.4** 回归：非空白语法错误不救（缺 WHEN 前缀照旧拒）+ 既有结构标题归一化/未知标题 fail-closed 用例零改动全绿。
- [ ] **1.5** fmt/clippy → commit `feat(normalize): EARS 关键字空白确定性归一化（REQ-WSC-02，F-48）`。

### Task 2: 教学重驱白名单扩展（REQ-WSC-09）

**Files:** `src/web/workspace_ws_handler/run/single_candidate.rs:484-556`（first_round_failure 匹配 :494-497）+ 测试。

**Steps:**

- [ ] **2.1** 失败测试：①invalid_ears 触发恰一次教学重驱（回灌 code:line:message；重驱成功继续链路/仍失败终态）②两类 parse 失败先后命中→重驱总共一次 ③非 parse 类失败不触发（直接既有路径）④重驱后终态消息带 `(after one teaching re-drive)` 后缀（既有语义）。
- [ ] **2.2** 跑红 → 白名单扩为 Task 0 集合 → 跑绿（诊断 open item ⑥：此面现状无网，本任务补齐）。
- [ ] **2.3** fmt/clippy → commit `feat(sc): 编译语法失败教学重驱白名单扩展（REQ-WSC-09，F-48）`。

### Task 3: prompt 判例与门禁收口

**Steps:**

- [ ] **3.1** prompts.rs:180 CJK 空格规则后补 `」` 收尾正/反判例各一（净增，红：contract 断言缺判例；绿：含判例且 22,000 红线内）；fmt/clippy → commit `feat(prompt): EARS CJK 收尾空格判例（F-48 随批）`。
- [ ] **3.2** 门禁：`cargo test --locked --lib` 全量 + `--test it_web`（编译 loop 在 web handler 面必跑）+ strict 复跑 + large_file_guard；证据落报告。

---

## Self-Review

覆盖：REQ-WSC-02 新场景→Task 1；REQ-WSC-09→Task 2；prompt→Task 3.1；Non-Goals 零触碰（Constraints 钉死）。类型一致：fixture 名/commit message/白名单集合（Task 0→2 传递）。Review Focus 五条全挂测试。
