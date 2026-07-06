# 技术方案：Coding Workspace 流程精简（coder + reviewer 双角色）

- 版本：v1.0
- 日期：2026-07-06
- 分支：feat-b-0630
- 关联：`cadence/notes/2026-07-04_流程冗长问题记录.md`

## 1. 背景与目标

当前 Coding Workspace 执行层按角色串行推进，链路为：

```
PrepareContext → WorktreePrepare → Coding → Testing → CodeReview → Rework → ReviewRequest → InternalPrReview → FinalConfirm
                                      ↑        ↑         ↑（每个执行阶段后都插一次 analyst rework）
                                   coder   tester    code_reviewer
                                        tester_plan
                                        tester_execute
```

实际使用中链路过长、失败返工放大耗时、交互过碎（详见问题记录）。

**目标**：Coding Workspace 只保留 **coder** 与 **reviewer** 两类角色职责。

- **去掉 tester 阶段**：测试职责并入 coder。普通 work item 由 coder 自己写测试并跑通；集成/E2E 测试类 work item（拆分层产出的 `integration`/`e2e` kind）由 coder 做完整流程测试。
- **去掉 analyst 阶段（Rework）**：不再由 analyst 把审查结论翻译成返修指令。
- **调整 reviewer 语义**：
  - `code_reviewer`：对**单个 work item** 做代码级审查（push 前，看 diff）。
  - `internal_reviewer`：所有 work item **全部完成后**做一次全局 PR 级审查（而非每个 work item 各跑一次）。
- **返修衔接**：reviewer 输出 `request_changes` 时，直接把 findings 作为返修指令，**复用之前的 coder provider（resume session、增量喂入 findings）** 重跑 Coding。自动循环，受 `max_auto_rework` 上限约束，超上限才停下等人。

精简后执行链：

```
PrepareContext → WorktreePrepare → Coding → CodeReview →（request_changes 时增量返修回 Coding）
    ... 所有 work item 完成后 → ReviewRequest → InternalPrReview（全局一次）→ FinalConfirm
```

## 2. 现状关键发现（代码级）

| # | 发现 | 位置 |
|---|------|------|
| 1 | 主编排在一个 `'pipeline` 循环里，逐阶段串行 | `src/web/coding_ws_handler/runner.rs:93` `execute_start_coding_flow` |
| 2 | Testing 固定跑 `tester_plan` + `tester_execute` 两次独立 provider 调用 | `runner.rs:246-268` |
| 3 | Testing 后**无论通过与否都进 analyst**（放大器） | `testing_parser.rs:229` `testing_report_should_enter_analyst` 全返回 `true` |
| 4 | 每个阶段前有 5 秒倒计时 stage gate，到期自动推进 | `gates.rs:18,20` `await_stage_gate` |
| 5 | 阶段→角色映射 | `src/product/coding_workspace_runner.rs:32` `coding_provider_role_for_stage` |
| 6 | Coder 自检契约偏弱（未强制「0 tests 不算通过」「新文件进编译树」） | `prompts.rs:113` `build_coding_prompt` |
| 7 | **coder session resume 机制已存在**（`should_resume_provider_conversation` 仅对 Coder 返回 true） | `lifecycle.rs:31`，`should_resume_provider_conversation` |
| 8 | Rework 现同时承担：调 analyst provider + 驱动 coder 重跑 + `max_auto_rework` 计数 | `rework.rs:15` `execute_rework_with_commands`，`rework.rs:279` |
| 9 | **Group scope 下 code_review 已是每 work item 一次，全部完成后才进 ReviewRequest → InternalPrReview**；单 work item scope 下每个 work item 各跑一次 internal review | `group.rs:32` `advance_to_next_group_unit`，`runner.rs:560` |
| 10 | 阶段枚举含 `Testing`/`Rework`，`order()` 依赖其序号 | `coding_models/execution.rs:10-36` |

**重要**：发现 #9 说明「internal_reviewer 全局化」在 Group scope 下**已基本成立**，主要改造点在单 work item 编排与语义统一。

## 3. 决策记录（已与用户确认）

1. 去掉哪层 tester：**执行层 Testing 阶段**，测试并入 coder。
2. analyst：**彻底移除**（含 RerunTesting verdict）。
3. reviewer 保留：`code_reviewer`（单 work item）+ `internal_reviewer`（全部完成后全局一次）。
4. 返修触发：reviewer `request_changes` → **自动返修**，复用 coder provider 增量喂入 findings 重跑 Coding，超 `max_auto_rework` 上限才停下等人。

## 4. 改造方案

### 4.1 摘除 Testing 阶段（执行层）

- `runner.rs`：删除 Testing 分支（约 `226-364`），含双 provider 调用与 testing→analyst 联动。Coding 完成后直接进入 CodeReview 段。
- `coding_provider_role_for_stage`：`Testing` 不再映射角色（或在新编排中永不进入）。
- 阶段枚举 `CodingExecutionStage::Testing` **保留**（历史 attempt 快照与 `order()` 兼容），新流程不再进入。
- Tester 相关 provider 配置（`tester_plan`/`tester_execute`）：保留字段以兼容旧快照，前端角色配置区隐藏 tester 选择项。

### 4.2 移除 analyst（Rework）阶段

- `runner.rs`：删除各阶段后的 `execute_rework_with_commands`（analyst）调用点，以及 `testing_result_acceptance_pending_analyst` 分支（`runner.rs:145`）。
- `analyst_parser.rs`：移除 `RerunTesting` verdict 与 `Testing → CodeReview` 映射。
- `CodingExecutionStage::Rework` 枚举**保留**（兼容），语义改为「reviewer 驱动的 coder 返修轮次」而非「analyst 决策」。
- `rework.rs`：拆分职责——保留「驱动 coder 重跑 + `max_auto_rework` 计数 + timeline 节点」的部分，去掉「调 analyst provider、解析 analyst verdict」的部分。role run 的 role 从 `Analyst` 改记为 `Coder`（返修轮次归属 coder）。

### 4.3 reviewer 驱动的自动返修（替代 analyst 衔接）

新逻辑（在 CodeReview 段内）：

1. `execute_code_review_with_commands` 得到 `review_report`。
2. `verdict == approve`：进入下一 work item（group）或 ReviewRequest。
3. `verdict == request_changes`：
   - 若 `rework_count < max_auto_rework`：把 `review_report.findings` 组装成返修 evidence，用**之前的 coder provider**（`provider_resume_session_id_for_attempt` + `CodingProviderRole::Coder`）**增量喂入**（resume session，只追加 reviewer findings，不重建完整上下文），重跑 Coding，`rework_count += 1`，回到 CodeReview。
   - 若达到上限：停下等人，弹门禁（返修/人工接管/终止）。
4. `verdict == blocked`：停下等人。

关键实现点：

- 复用现有 `should_resume_provider_conversation(Coder) == true` 与 `record_attempt_provider_session`，coder 的 provider_session_id 已在首轮 Coding 后记录。
- 增量 prompt：新增 `build_coding_rework_from_review_prompt`（或复用 `build_coding_delta_prompt`），只带上 reviewer findings + 「本轮必须优先修复」指令，依赖 resume session 承接历史上下文。
- 返修使用的 provider = coder provider（`snapshot.coder`），**不是** analyst/reviewer provider。

### 4.4 internal_reviewer 全局化

- **Group scope**：现状已满足（全部 unit 完成 → ReviewRequest → InternalPrReview 一次），仅需在移除 analyst 后清理编排。
- **单 work item scope**：一个 issue 若只有一个 work item，行为等价于「完成后做一次全局 review」，天然满足；若单 attempt 内不含 group，则 InternalPrReview 保持「该 attempt 完成后一次」。
- 语义统一表述：InternalPrReview = 「issue 下所有 work item 完成后、push 之后的一次全局 PR 审查」。

### 4.5 强化 coder 自检契约

`build_coding_prompt` / `build_coding_delta_prompt` 增加硬约束：

- 必须实际执行验证命令并贴出结果（`cargo check`/`clippy`/`test` 或前端对应命令）。
- **`0 tests` 视为未通过**，不得据此宣称测试通过。
- 新增源文件必须接入编译树（Rust 的 `mod`/crate 挂载；前端的 import/export）。
- 完成前检查 `git diff`/`status`，确认落点文件真实变更。

## 5. 阶段流转对比

**改造前**（单 work item）：
```
Coding → [gate] Testing(plan+execute) → [analyst rework] → [gate] CodeReview → [analyst rework]
→ ReviewRequest → [gate] InternalPrReview → [analyst rework] → FinalConfirm
```

**改造后**（单 work item）：
```
Coding → [gate] CodeReview →(approve)→ ReviewRequest → InternalPrReview → FinalConfirm
                    │
                    └(request_changes 且未超上限)→ 用 coder provider 增量返修重跑 Coding → CodeReview
                    └(request_changes 超上限 / blocked)→ 停下等人（门禁）
```

**改造后**（group）：
```
[每个 work item] Coding → CodeReview →(approve)→ 下一个 work item
... 全部完成 → ReviewRequest → InternalPrReview（全局一次）→ FinalConfirm
```

## 6. 兼容性与风险

| 项 | 处理 |
|----|------|
| 历史 attempt 快照含 Testing/Rework/analyst 数据 | 枚举与字段保留，仅新流程不再产生；反序列化兼容 |
| 前端 store/类型含 tester/analyst 字段 | 保留类型，UI 隐藏 tester/analyst 相关展示与配置；`CodingRoleProviderConfigSnapshot` 字段不删 |
| `max_auto_rework` 语义 | 从「analyst 判定返修上限」改为「reviewer 驱动返修上限」，取值不变 |
| 测试兜底弱化风险 | 靠 §4.5 coder 自检契约 + 独立 integration/e2e work item（拆分层）双重兜底 |
| provider 增量喂入失败（session 丢失） | 回退为全量 `build_coding_prompt`（`revision_resume_fallback` 已有类似机制可参照） |

## 7. 影响文件清单（预估）

后端：
- `src/web/coding_ws_handler/runner.rs`（主编排，改动最大）
- `src/web/coding_ws_handler/runner_support.rs`（移除 `testing_result_acceptance_pending_analyst` 等）
- `src/web/coding_ws_handler/gates.rs`（阶段 gate 集合调整）
- `src/product/coding_workspace_engine/rework.rs`（拆分 analyst / coder 返修职责）
- `src/product/coding_workspace_engine/testing*.rs`（Testing 执行不再被编排调用，评估保留/裁剪）
- `src/product/coding_workspace_engine/analyst_parser.rs`（移除 RerunTesting）
- `src/product/coding_workspace_engine/prompts.rs`（coder 自检契约 + 新返修 prompt）
- `src/product/coding_workspace_runner.rs`（`coding_provider_role_for_stage`）

前端：
- `web/src/state/coding-workspace-store.ts`、`web/src/api/types/coding.ts`（隐藏 tester/analyst 展示，保留类型）
- 角色 provider 配置面板（隐藏 tester/analyst 选择）

测试：
- 编排相关集成测试（`tests/it_web/...`、`coding_workspace_engine/tests/...`）需按新链路重写。

## 8. 待确认 / 后续

- 运行契约声明（问题记录 07-05 补充）：开始运行前声明本次会跑哪些阶段、哪些自动继续、哪些必停——建议作为本方案的配套项，在编排稳定后补一层「运行策略摘要」事件。
- 是否需要保留一个「快速 smoke check」轻量验证（不引入独立 tester 角色，仅 coder 内部执行）——本方案默认不引入独立角色，smoke 由 coder 自检契约覆盖。
