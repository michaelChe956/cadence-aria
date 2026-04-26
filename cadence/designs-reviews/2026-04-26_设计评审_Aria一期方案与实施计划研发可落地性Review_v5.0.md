# Aria 一期方案与实施计划研发可落地性 Review v5.0

> 修复后复查版。v4.0 中记录的阻塞问题已复查闭合；本文件作为当前最新 Review 结论。

## 1. Review 目标

面向研发实施复查以下内容是否仍存在“看不懂如何做”“计划与设计不一致”“实现边界不清”的问题：

- MVP 设计：`cadence/designs/2026-04-23_技术方案_Aria一期MVP精简设计_v1.2.md`
- 实现总契约：`cadence/designs/2026-04-23_技术方案_Aria一期实现总契约_v1.0.md`
- IO / Provider 契约：`cadence/designs/2026-04-26_技术方案_Aria_IO协作协议与Provider契约_v1.1.md`
- 研发导读：`cadence/designs/2026-04-26_技术方案_Aria一期研发导读与实施拆解_v1.1.md`
- 评审后规格补齐：`cadence/designs/2026-04-26_技术方案_Aria一期评审后实施规格补齐_v1.3.md`
- P1-P4 实施计划与总览：`cadence/plans/2026-04-26_计划文档_实施计划_Aria一期实现计划_*.md`

## 2. 总结论

本轮追加复查发现 4 个会影响研发落地口径一致性的文档残留，已修复。修复后未发现新的 P0 / P1 阻塞问题。

研发可以按当前 MVP 设计、实现总契约、规格补齐与 P1-P4 计划继续落地。一期关键实现口径已经对齐：

| 主题 | 当前结论 |
|------|----------|
| OpenSpec scope | 一期固定 `openspecScope = "main"`，P2 skeleton 使用 `specs/main/spec.md`；多 scope 进入 gate / manual intervention |
| P3 Provider 基线 | `provider_adapter_baseline` 独立验证，不再依赖后续 `provider_context_builder.rs` |
| `_aria.traceability_refs` | provider / handler 不手工定稿；daemon 在归一化时生成或校正 |
| M20 / N20-N22 | `M20 integration_prepare_impl` 覆盖 `N20/N21/N22`；`N20` 生成 candidate commit；`N22` 只消费该 commit |
| N23 Git 操作 | 一期执行 `git cherry-pick --no-commit <candidateCommitSha>`；冲突 abort 后回流 `N19` |
| N26 followup | provider 只产出候选 `dispatch_package` 或 patch task delta；OpenSpec `tasks.md` 由 daemon 通过 Document Operation 更新并触发 bundle stale / recompile |

未发现 MVP 设计或实现总契约中需要用户裁决的新增设计错误。本轮只修正了与既有裁定不一致的措辞残留，没有改变 MVP / 总契约的核心语义。

## 3. v4.0 阻塞项闭合情况

| v4 问题 | 当前状态 | 研发如何落地 |
|---------|----------|--------------|
| OpenSpec scope / skeleton 路径不一致 | 已闭合 | P2 固定创建 `openspec/changes/<changeId>/specs/main/spec.md`；compiler 只读 `main`；发现多个 scope 返回 `openspec_multiple_scopes_unsupported` |
| P3 Task 1 测试依赖后续 context builder | 已闭合 | 先写 `tests/provider_adapter_baseline.rs`，只覆盖 DTO、fake provider、ProviderRunRecord baseline；context builder 放到后续任务 |
| P4 将 N20 直接放入执行链导致边界不清 | 已闭合 | Task 2 只做 `N16-N19`；Task 3 实现 `M20/N20-N22` 与 `N23-N24` |
| candidate commit 生成点与传递字段不清 | 已闭合 | `N20` ready 前生成 `candidateCommitSha`，写入 runtime state / snapshot / `N22` 输入；缺失则不得进入 ready 或 `N23` |
| N22/N23 Git 操作边界不清 | 已闭合 | `N22` 做 prepare / preflight；`N23` 执行 cherry-pick、conflict abort、rollback record |
| N26 是否可直接改 OpenSpec 不清 | 已闭合 | provider 不直接修改 OpenSpec Markdown；daemon 通过 Document Operation 更新并重编译 bundle |

## 4. 本轮额外发现并已修复的残留

| 编号 | 问题 | 修复 |
|------|------|------|
| R1 | 总契约节点矩阵仍写 `N16-N19` “必须回填 `_aria.traceability_refs`”，可能让研发误以为 provider / handler 手工填正式字段 | 改为 `_aria.traceability_refs` 由 daemon 归一化生成或校正 |
| R2 | IO 契约中 Superpowers 执行记录“不允许方式”写成“直接驱动 N20”，容易误解为外部执行记录可推进 `N20` | 改为只作为 `N20 ready_for_integration` 的正式输入之一，ready / block / rework 决策仍由 daemon 生成 |
| R3 | IO 契约仍保留泛化 “merge/rebase/cherry-pick” 表述，和 P4 一期 cherry-pick 规格不够一致 | 改为一期执行 `git cherry-pick --no-commit <candidateCommitSha>`；candidate commit 由 `N20` 生成，N23 控制 cherry-pick / rollback |
| R4 | P2 计划文件清单仍残留 `tests/fixtures/openspec/changes/sample-change/specs/sample/spec.md`，与固定 `openspecScope = "main"` 冲突 | 改为 `tests/fixtures/openspec/changes/sample-change/specs/main/spec.md` |
| R5 | 规格补齐文档 fixture 树仍残留 `specs/sample/spec.md` | 改为 `specs/main/spec.md` |
| R6 | 总契约和 IO 契约仍有泛化 `specs/**/spec.md` 表述，容易让研发误以为一期可以消费任意 OpenSpec scope | 改为 `specs/main/spec.md`，并保留多 scope 进入 gate / manual intervention 的裁定 |
| R7 | IO 契约主路径表中 `N25 final_review` 仍写 `followup 挂 gate 或 N26`，容易被理解为可绕过 gate 直达 `N26` | 改为 followup 先进入 approval gate，用户确认后才允许 `N26`；`N26` 行补充 taskConstraints 和 provider/daemon 写权限边界 |

## 5. 当前研发实施路径是否清楚

清楚。研发按阶段执行时的落地点如下：

| 阶段 | 实施抓手 |
|------|----------|
| P1 | 固化 `changeId`、wire schema、runtime state、事件与 checkpoint 基线 |
| P2 | 建立 Document Operation、Projection compiler、OpenSpec bundle compiler；锁定 `specs/main/spec.md` |
| P3 | 先落 provider adapter baseline，再落 context builder、prompt registry、规划节点 |
| P4 | 先扩展 `N16-N20/N24/N25-N27` contract / workflow / prompt registry；再按 Task 1-5 串执行准备、执行报告、集成链路、最终收口与 smoke |

P4 的关键边界已经足够可执行：

| 边界 | 实现要求 |
|------|----------|
| `N16-N19` | provider 产出候选 report；daemon 归一化、生成 `_aria.traceability_refs`、校验 projection / OpenSpec coverage |
| `M20/N20-N22` | 一个实现单元保留三个协议 snapshot；`covered_protocol_nodes()` 返回 `["N20", "N21", "N22"]` |
| `N20` | 生成 candidate commit，并记录 `candidateCommitSha`；失败则 gate / manual intervention，不进入 ready |
| `N22` | 只消费 `N20` 的 `candidateCommitSha`，做 integration branch / preflight / `preMergeSha` 记录 |
| `N23` | 执行 cherry-pick；冲突必须 abort 并路由 `N19` |
| `N26` | gate approve 后才进入；provider 只输出候选，daemon 负责 OpenSpec 更新、bundle stale / recompile、新 dispatch 约束校验 |

## 6. 非阻塞观察

历史 Review 文档 v1.0-v4.0 保留了当时的评审问题，不能作为当前实施状态判断依据。研发查看评审结论时应以本 v5.0 为最新状态。

原始上游协议 / artifact 文档中仍保留 `merge` / `rebase` / `cherry_pick` 等更通用枚举，这是上游协议能力范围；一期实现以 MVP、实现总契约、规格补齐与 P4 计划中收窄后的 cherry-pick 规则为准。

## 7. 验证记录

已执行以下复查：

| 检查 | 结果 |
|------|------|
| 搜索 `task-scope` / `specs/<scope>` / `specs/sample/spec.md` / `specs/**/spec.md` / 旧 scope 表述 | 未在目标设计与计划中发现会误导一期实现的残留 |
| 搜索 `必须回填 _aria.traceability_refs` / `直接驱动 N20` / `merge/rebase/cherry-pick` | 已清理目标文档中的误导性残留 |
| 搜索 `provider_adapter_baseline` | P3 Task 1 与完成判定已对齐 |
| 搜索 `candidateCommitSha` / `N22` / `N23` / `N26 Document Operation` | 关键实现边界已在设计与 P4 计划中对齐 |
