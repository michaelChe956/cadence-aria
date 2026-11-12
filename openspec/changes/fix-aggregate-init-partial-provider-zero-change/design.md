# Design：聚合初始化的零变更判据与失败诊断

## 背景事实（2026-11-12 podman 容器 E2E，只装 Pi）

| 观察 | 证据 |
|---|---|
| provider 选择门正常 | 选中 Pi；Claude Code / Codex `skipped_reason=cli_unavailable` |
| `pre_check` turn 真跑了 | Pi 展开 skill、正确把 `用大陆镜像` 翻译成 `--mirror cn`、执行 `pre-check.sh run --mirror cn --no-interrupt --upgrade` |
| 脚本失败 ⇒ 零落盘 | 脚本 `"overall": "failed"`（现场：base-tools phase 的 `uvx` 装不上——脚本用 `pip install uv` 装 uv，容器无 pip）⇒ `--no-interrupt` 立即中止 ⇒ 聚合根零字节变化 |
| 平台判定 | REQ-BOOT-17 判零变更失败（判定符合契约） |
| **失败面丢事实** | 诊断只有 `root recipe turn produced no workspace change (provider=pi, step=pre_check, zero_change=true)`；脚本的 JSON 报告整段丢失 |
| 健康环境下整条 recipe 可跑通 | 补齐 pip/uvx 后重跑：五个 step 全 completed（run2 亦 completed）⇒ 「只装一家 provider」本身不是阻断点 |
| 脚本是幂等的 | 脚本自带 `check` 模式报告 `all-skipped` / 0 created / 0 updated；重跑可零变更 |

用户现场（2026-11-12，实例不在这台机器上）：失败签名是 `zero_change=true`——turn 正常结束、
零落盘。结合上面的机制可知：**用户的 pre-check 脚本某个 phase 失败了**，而平台把
「哪个 phase 失败、失败原文是什么」整段丢掉了。

## 决策

### 决策 1：失败诊断必须带 provider 的真实输出

零变更本身不说明原因。三种情形（命令没被执行 / 命令执行了但失败 / 命令按契约幂等无变更）
的 provider 输出完全不同。诊断 = 失败事实 + provider 输出尾部（经既有脱敏入口
`sanitize_turn_output`）。

- 输出预算超限时**保留头尾**（各一半）：只留头会把收尾处的脚本报告整段截掉。
- 输出取**尾部**（最后 40 行 / 3000 字节）：脚本报告与失败原文都在收尾处。

### 决策 2：零变更的判据要用「命令是否真的执行且成功」这个事实补齐

零变更判据复用既有 receipt auditor 的结论（不新增第二套「有无变更」判定），但在**判失败之前**
先看一个事实：**该 turn 的工具结果里有没有脚本的成功报告**（`overall=success`）。

- 有成功报告 ⇒ 命令确实执行且按契约没有产物变化 ⇒ 判**成功**（幂等重跑）。
- 有失败报告（`overall=failed`）或没有报告 ⇒ 判**失败**（原语义）。

报告结论**只从工具结果原文读**，不采信模型自述——模型可能复述或编造报告内容。
多份报告时**失败优先**（保守）。

**为什么不改判据、只加事实**：REQ-BOOT-17 的立意是抓「命令未被识别」（表现为 turn 正常结束
但零动作）。本决策**不放宽**这一条——命令没被执行时没有脚本报告，仍判失败。

### 决策 3：不在平台侧重复实现 Cadence-skills 脚本的工具安装

base-tools phase 属于 Cadence-skills 的职责；平台侧只保证失败可行动。若部署镜像缺
`pip`/`uvx` 等前置，平台现在会把脚本的原文报告透出来（用户/运维据此补环境），但**不在
本 change 里**给平台加「装工具」的职责（跨仓职责边界，且会与脚本的安装逻辑漂移）。

### 决策 4：provider 发问导致的失败不在本 change 内

容器现场还观察到第三种失败形态：Pi 在 turn 里发问 ⇒ 平台 fail-closed
（`provider interaction required`，retryable=false）⇒ 后续 turn 不跑。这与「零变更」是不同
故障（失败码、诊断形态都不同），本 change 不动它；已在计划文档里记为独立观察。

## 影响面

| 位置 | 变更 |
|---|---|
| `src/product/logical_codebase/aggregate_initialization_coordinator.rs` | 新增 `RecipeCommandReport` / `AggregateProviderTurnOutcome`；trait 返回类型由 `String` 改为 outcome |
| `src/product/logical_codebase/coordinator_provider_turn.inc.rs` | `BoundedOutput` 保留头尾；`consume_turn` 收集工具结果里的脚本报告；turn 返回 outcome |
| `src/product/logical_codebase/coordinator_frozen_provider.inc.rs` | 新增 `provider_turn_failure_output`（失败事实 + 输出尾部） |
| `src/web/handlers/aggregate_initialization/root_recipe_audit_windows.inc.rs` | 零变更判定：脚本报告成功 ⇒ 成功；否则失败（诊断带 provider 输出） |
| `src/web/handlers/aggregate_initialization/production_dependencies.inc.rs` | 传 outcome（summary + report） |
| 测试替身（src + tests 共 8 处） | 返回类型随 trait 变更机械更新 |
