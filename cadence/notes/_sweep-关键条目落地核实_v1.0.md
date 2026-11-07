# 关键待排期条目落地核实（v1.0）

> 核对对象：当前 worktree `bugfix-b-11-02`。以实际打开的源码、提交记录及目录盘点为证；缺失文件不作实现推断。

## 结论摘要

1. **审查 finding 范围约束与 owner 排除闭环：部分落地**。WorkItemPlan prompt 有范围维度和有限收敛提示，但 finding 没有归属字段；owner 排除表达/回喂及 validation_reject 门卡诊断未证实。
2. **写范围可见性 + 本地产物屏蔽真机复验：未落地（无复验记录）**。当前 notes 无 10-30 之后复验报告，目标交付说明不在本 worktree。
3. **活跃 change 归档清账：部分落地**。实测 1 个非 archive change；无法读取其 delta 做逐条 requirement 对账，不能宣称已同步。
4. **行数上限治理：部分落地**。guard 上限实测 1200；不存在 GRANDFATHERED 清单，故无法统计其条目或其中 actual==cap 文件数；不能据此称 89 项已治理。
5. **自动化换代竞态 P0-1/P0-2：未核实，不剔除**。指定三个源码路径均不在此 worktree。
6. **审计预算 P1-16/P1-18：未核实，不剔除**。指定 logical_codebase receipt 源码路径不在此 worktree。

## 逐条证据

### 1. 审查 finding 范围约束与 owner 排除闭环 — 部分落地

`src/product/workspace_engine/prompts/review.rs`：
- 第 247–254 行：`审核边界说明：本候选是 WorkItemPlan 整组拆分计划`，列出拆分粒度、依赖完整性、写入范围互斥、跨端拆分、验证计划五维，并称“只审核上述五个维度”。范围约束有部分实现。
- 第 262 行：schema 包含 `review_scope":"outline"`，finding 仅有 severity/message/evidence/impact/required_action，没有 owner/归属字段。
- 第 268–276 行：提示强弱 finding 分级、revise 必须有 finding、后续轮只复核上一轮强返修项，构成有限收敛提示；没有 owner exclusion/处置回馈表达。
- 第 65–74 行：通用 review schema 同样没有归属字段。

`src/product/coding_models/review.rs` 第 105–137 行打开的 `ReviewFinding` 类型含 `file_path`、`line`、`source_stage`、`related_requirements`、`related_work_item_tasks` 等字段，但不含明确 owner/归属字段。

修法拆分：修1范围契约化 **部分**（提示有范围但未核实完整契约）；修2归属字段 **缺**；修3 owner 排除表达与处置回馈 **缺/未证实**；修4收敛规则 **部分**（提示后续轮只复核强返修项）；修5 validation_reject 门卡诊断 **未证实**（相关指定源码不在此 worktree）。

### 2. 写范围可见性 + 本地产物屏蔽真机复验 — 未落地（无记录）

当前 `cadence/notes/` 可见目录没有 2026-10-30 之后相关真机复验报告；`2026-10-30_交付说明_写入范围可见性与本地产物屏蔽_v1.0.md` 不在当前 worktree。没有证据证明 3 项复验完成；此结论不推断功能代码不存在。

### 3. 活跃 change 归档清账 — 部分落地

`find openspec/changes -mindepth 1 -maxdepth 1 -type d ! -name archive -print` 实测唯一结果为 `openspec/changes/mechanicalize-plan-compile-repair`，即 **1 个非 archive change**。`openspec/specs/` 目录清单可见 22 个 capability。当前视图无法打开该 change 的 proposal/spec 文件，不能逐条比较 delta `### Requirement:` 标题与主 spec，故不判已同步。

### 4. 行数上限治理 — 部分落地

已打开 `tests/it_core/large_file_guard.rs`：
- 第 4 行：`const MAX_PRODUCT_CODE_LINES: usize = 1200;`
- 第 5–6 行：扫描 `src/tests/web/src`，扩展名 `rs/ts/tsx`。
- 第 47–50 行：`content.lines().count()`，只将 `line_count > MAX_PRODUCT_CODE_LINES` 作为超限。
- 文件无 `GRANDFATHERED` 清单：条目数 **未定义**，其中 actual==cap 文件数 **无法按该清单统计**（不是测得为 0）。

指定的 2026-10-30 行数待办说明在当前 worktree 不存在，89 个历史顶格文件无法逐项比对。可确认现状阈值 1200，但不能据此认定原 89 项均已解决。

### 5. 自动化换代竞态 P0-1/P0-2 — 未核实，不剔除

指定路径 `src/product/issue_automation_store_parts/cas_internals.inc.rs`、`src/web/handlers/automation_enrollment_reenable.rs`、`web/src/components/lifecycle/useIssueLifecycleGeneration.ts` 在当前 worktree 不存在；对 `src;web/src` 搜索 `bind_plan_ids_locked`、`supersede_terminated_generation_and_cas`、`policy_revision +=` 无命中。无法判断 revision 是否仍递增、CAS 前是否删旧世代、前端是否重新 GET revision；不得据缺文件宣称已修。需在含源码 worktree 继续核实。

### 6. 审计预算 P1-16/P1-18 — 未核实，不剔除

`src/product/logical_codebase/root_recipe_receipt_types.inc.rs`、`root_recipe_receipt_auditor.inc.rs` 在当前工作树不可定位。默认预算、是否硬取 `::DEFAULT`、审计是否计入 `.git/`、生产代码 `with_snapshot_budget()` 调用数均无从实测；仍需核查。

## 仍需排期的真实剩余项

- finding 明确归属字段、owner 排除输入/处置回馈、完整范围契约、validation_reject 门卡诊断。
- 写范围可见性/本地产物屏蔽的 3 项真机复验并形成可核验记录。
- 唯一 active change 的 delta requirement 与主 spec 逐条对账，随后确认归档状态。
- 行数治理与 89 个历史顶格文件逐项对账；恢复可核验 exact-cap/豁免清单或明确替代统计口径。
- 在目标源码存在的 worktree 核实 P0-1/P0-2、P1-16/P1-18 的实现与调用点。

## 可剔除项

- “当前仍有 17 个 active change”这一旧数量口径可剔除：当前实测仅 1 个非 archive 目录；但该 change 的 delta 对账仍待办。
- “当前仍使用旧行数阈值/GRANDFATHERED 清单”可剔除：实际 guard 阈值为 1200，且没有该清单；不能因此剔除 89 项逐项治理。
- 其余事项证据不足，不建议从排期中剔除。

## 实际检索范围与命令

- `git log --oneline -40`
- `find openspec/changes -mindepth 1 -maxdepth 1 -type d ! -name archive -print`
- `find cadence/notes -maxdepth 1 -type f`
- `git log --all --oneline -- <两份指定交付说明路径>`
- 在 `src;web/src` 搜索 `bind_plan_ids_locked|supersede_terminated_generation_and_cas|with_snapshot_budget|GRANDFATHERED|policy_revision +=|DEFAULT.*budget|validation_reject`
- 在 `cadence/notes` 搜索 `89 个|P0-1|P0-2|P1-16|P1-18|真机复验`
- 实际打开：`src/product/workspace_engine/prompts/review.rs`、`src/product/coding_models/review.rs`、`tests/it_core/large_file_guard.rs`；盘点 `openspec/changes/`、`openspec/specs/`、`cadence/notes/`、`src/product/`、`src/web/handlers/`、`web/src/components/lifecycle/`。
