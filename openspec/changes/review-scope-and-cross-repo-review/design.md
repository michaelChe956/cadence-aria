# 设计：多仓评审范围收敛与 coding 跨仓最终评审契约化

## Context

本 change 把两份已获批技术方案落成 OpenSpec 契约，不含实现：

| 来源 | 关键内容 | 契约落点 |
|---|---|---|
| `cadence/designs/2026-10-22_技术方案_story-design-workitem评审修正_v1.2.md`（commit `24fedc80`） | 形态门禁、删 primary、lc 不开窗、材料全量+稳定注入、逐仓实况与基线、must-fix 四类、finding 完整性与旧轮证据、splitter 分层、会话 UUID 身份 | REQ-PLN-11..17、REQ-ENV-13/14 |
| `cadence/designs/2026-10-22_技术方案_coding跨仓最终评审_v1.1.md`（commit `ded59f08`） | 条件化删每仓 internal review / GroupFinalReview、issue 级跨仓评审恰好一次、advisory 红线、留痕去确认人/通道、`source_stage` 新成员、存量 attempt 兼容 | REQ-GFR-CR-01..07 |

两份方案共享同一条硬约束：**单仓库代码库（`logical_codebase_id == None`）行为必须与今天逐项一致**，新行为只挂多仓分支。本 change 的每条 requirement 都带单仓不变场景，这是契约级红线，不是实现提示。

### 已读并定点复核（bash 实测，2026-10-23；本仓 `read`/CodeGraph 快照过期，冲突以 bash 为准）

| 锚点 | 已观察事实 | 契约影响 |
|---|---|---|
| `src/product/models/project.rs:143` | `pub logical_codebase_id: Option<String>`（`Some`=lc id，`None`=单仓） | 唯一形态判定键，两个 capability 侧都用它 |
| `src/product/reviewer_window_store.rs:60-61` | `role_run_window_id` 返回 `{attempt}-{role_run}-w01` | 单仓身份必须一字不改；多仓身份改走会话唯一 id |
| `src/product/workspace_engine/review/drive.rs:24,78,363,407` | 入口（开窗）与内层（`_with_window`）已分离 | 多仓只在入口加门，无需改名或重构内层 |
| `src/product/workspace_engine/review/drive.rs:1689,1752,1816,1842` | `plan_review_window_worktree` / `begin_plan_review_window` / `assert_plan_review_window` / `reviewer_verify_policy` | 单仓路径全部保留；`assert_plan_review_window` 内部按形态分派，6 处调用点不动 |
| `src/product/logical_codebase/planning_context_set.rs:160-255` | `InventoryInjectionBudget` soft/hard 裁剪 + `omitted_member_ids`/`truncated` | 多仓改全量渲染 + 绝对上限，缺员标记整体取消 |
| `src/product/workspace_engine/artifact_constraints.rs:507` | `reviewer_boundary_rules_for(workspace_type)` | 增形态参数，多仓追加四类必查，单仓字符串不变 |
| `src/product/workspace_engine/review/structured_output.rs:226` | `parse_review_completion_for_active_node` | 多仓在此加 finding 完整性门，不改 `parsers.rs` 公共签名 |
| `src/product/workspace_engine/prompts/history_compaction.rs:250,347,352` | `render_round_summary` / `FindingSummary` / `extract_findings` 只认 severity+required_action | 多仓增 message/evidence，单仓走原输出 |
| `src/product/work_item_split_engine/context.rs:147-164` | `summarize_repository_structure` 吞 `read_dir` 错误、`truncate(30)`、`(empty repository)` | 多仓分层+显式失败，单仓原样 |
| `src/product/coding_models/execution.rs:13-34` | 7 个阶段，`order()` 穷尽匹配 | 新增成员只为 `source_stage`，必须补 `order()` arm 但不改变既有相对序 |
| `src/product/coding_workspace_engine/review_parser.rs:138-155` | `deserialize_review_finding_source_stage` 严格字符串表，未登记值硬失败 | 新字符串 `issue_cross_repo_review` 必须登记 |
| `src/product/coding_workspace_engine/legacy_group_final_review.inc.rs` | 386 行，仅测试调用 | 多仓停用；执行入口属死代码，删除属实现范围 |
| `src/product/coding_workspace_engine/handoffs.rs:582,604` | `maybe_complete_issue_delivery` / `prepare_group_final_confirm_from_readiness` | 跨仓评审的两个触发钩子 |
| `src/web/coding_attention_info.rs:13-17` | kind 集合不含 `cross_repo_review` | 新增提醒 kind 的落点 |

## Goals / Non-Goals

**目标**

- 19 条对外承诺逐条落到可判定的 Requirement + Scenario，每条至少一个 WHEN/THEN。
- 单仓不变可验收：窗口/事件/基线/门控/digest/prompt 字节，各有对应场景。
- 下游 worker 不需要重新推导设计即可实现：形态判定键、分派点、既有可复用函数、每条的验收口径都在此写死。

**非目标**

- 不做多成员对账域、集合屏障、报告成员分节、根 guard、物理 no-exec、ZeroWrite 分档。
- 不新建 store/账本/窗口类型/前端组件；身份只用一个会话记录字段承载。
- 不修 verdict 结构错误的静默降级（已知缺陷待办），不改 severity 分级与 revision 轮规则（两形态都不改）。
- 不统一 coding 侧两处重复的 GroupFinalReview prompt 文本（合并必然改动单仓 prompt）。
- 不给跨仓评审做重试、人工「再评一次」入口或 plan-defect 路由。

## Decisions

### D1 形态门禁：单一判定键 + 条件化分支

判定键只用 `IssueRecord.logical_codebase_id`。引擎内（有会话句柄处）收敛成一个私有判定（紧邻 `is_aggregate_planning_session` 放置，但**不复用**它——后者是「聚合视野」启发式，不是形态判据）；无引擎句柄处（splitter 静态方法、`IssueRecord` 作为入参的解析函数）直接用 `issue.logical_codebase_id.is_some()`。

所有删除项一律**条件化**：不物理删除单仓仍在用的共享路径，新逻辑只挂多仓分支。coding 侧读取失败或 issue 不存在一律按 `None` 处理（安全默认：读不到绝不切换行为）。

替代方案（按 `RepositoryRouting` Legacy/Logical/FailClosed 判定、按「issue 涉及几个仓」计数）均不采用：前者按「manifest+selection 是否存在」判，不等价于形态；后者是 owner 已否决的口径。

### D2 评审范围：集合权威，不设 primary

多仓评审范围 = 该 issue 的 `IssueCodebaseSelection` 解析出的成员集合（1~N）。`primary` 只保留为 UI 归属/兼容投影的语义，**不参与评审范围**：删除 primary 锚定事件与「任意单一 primary 仓库」措辞，但保留 `resolve_selected_logical_repository` 函数本体（4 处活调用），仅改入参命名与注释。

单仓侧不改：`plan_review_window_worktree` 的 `.git` 早返回是单仓唯一出口，primary 分支在其后不可达，因此移除该分支对单仓零影响；但保留早返回本身。

### D3 材料：全量 + 绝对上限 + 稳定注入面

- 清单全量渲染，取消 soft/hard 裁剪与 `omitted_member_count`/`omitted_member_ids`/`truncated`。
- 绝对上限 **500 KiB**（owner 裁定），超限 `InvalidRecord{kind:"planning_material_over_budget"}` 阻断本轮，不启动 provider。数值进 spec（可验收），渲染细节不进。
- 材料块进 `[canonical_inputs]` 稳定区（插在「Repository 路径」之后、「关联上下文」之前），指令块留消息尾部；保留既有聚合 scope marker 常量，`must_replay_raw` 加宽到「有 canonical inputs 且（reviewer 模式 或 含该 marker）」。
- 单仓**不注入**成员材料：一旦注入会命中聚合 marker，使 `is_aggregate_planning_session` / `is_aggregate_story_or_design` 与 structured-output 协议误判为聚合会话。这是单仓不变性的硬理由，写进 spec 的单仓场景。

边界：`logical_codebase/aggregate_index/freshness.rs` 另一套裁剪渲染（`AggregateIndexRenderer`）本批核查未找到生产调用链，属死预算逻辑，本 change 不改，留作待办。

### D4 逐仓实况与基线：HEAD 分支，失败只影响本仓

逐仓实况固定五行（职责/关键目录/入口/构建·测试命令/基线），缺项写「未识别」。新增成员实况渲染模块，装配点在 LC 专属 resolver 之后（单仓永不进入）。

基线 = `git rev-parse --abbrev-ref HEAD`（复用既有 `run_git`）；输出 `HEAD`（detached）、命令失败、空仓（无 commit）→ 该仓 `基线: 未识别(<诊断>)`，**其余仓继续、评审继续**。单仓的 `resolve_effective_base_branch` → `default_local_branch`（main→master 猜测）与 `resolve_issue_baseline_tree` 的 lc 短路代码一字不改，因此单仓仍 fail-closed。

### D5 身份：会话唯一 id，不建映射 store

多仓 reviewer 与 author 的身份 = 会话记录上的一个新字段（`#[serde(default)]`），创建会话时生成 `Uuid::new_v4().simple()`；存量记录字段为空时首次读取生成并**持久化一次**（已有值不覆盖），保证同会话跨轮/跨 repair 恒定。删除随会话记录磁盘删除自动消失，不新建映射 store、不写清理钩子、不用 uuid v5 派生。

`reviewer_verify_policy` 改为「有活动窗口走窗口编号，否则走会话身份」——单仓恒有窗口，因此 8 处消费点零改动、digest 逐字节不变；author 的 `resource_scope_id` 在多仓分支改由会话身份派生，单仓保留原文。

风险：中（改的是合同 digest 域），但只走多仓分支，单仓由 `Some` 分支保证不变。

### D6 判据与容错：只多仓，且都挂在既有函数内部分派

- must-fix 四类：`reviewer_boundary_rules_for` 增形态参数，仅多仓追加；单仓同 `Vec`、同循环、同字符串。
- finding 完整性：不改 `parsers.rs`（调用面含历史 JSON 与测试），在 `parse_review_completion_for_active_node` 解析成功后加多仓分支；`malformed_findings` 与既有 malformed 同路 → 不在 fatal 集合、不可 repair → `needs_human` + 明确诊断码。
- 旧轮证据：`FindingSummary` 增 message/evidence，`HistoryCompactionInput` 增 `detail_findings: bool`，4 个调用点传形态判定；`false` 时渲染短路为今天的字符串。
- splitter：签名增 `logical: bool`，5 个调用点传形态判定；`false` 走原 4 行实现。

单仓证据始终是「`false` 分支 = 原实现逐字」，不接受任何「重写后语义等价」。

### D7 coding 侧：一次 issue 级评审，替代每仓收口评审

- 多仓：不启动 per-attempt InternalReviewer、不执行 GroupFinalReview；存量 `stage=internal_pr_review` 的 attempt 走 readiness → FinalConfirm（与今天 fresh group 的兼容路径同构），零 provider 启动。`CodingExecutionStage::InternalPrReview` 保留不删（durable 存量依赖它）。
- 保留：code reviewer（per work item）及其 `RunCoderFix` / 转人工分诊门；组形态（同仓多 work item）；组就绪检查与人工最终确认。
- 新增：issue 级跨仓评审记录（issue 维度，不是 attempt 阶段），材料为逐仓 diff 分段（按 `logical_repository_id` 稳定排序，与 code reviewer 同源 diff 口径），每段与全包都有字节预算，超限按仓顺序截断并在材料中显式标注被截断的仓。
- provider：`internal_reviewer` → 缺失回退同快照 `code_reviewer` → 两者皆缺则 `Skipped`。回退必要的实证理由：两者都是 `#[serde(default)] Option<ProviderName>`，存量旧 JSON 只会出现「缺 internal_reviewer 而 code_reviewer 在场」这一种空缺形态。
- 幂等：评审记录按 issue 唯一，已存在不再触发；重复进入收口钩子不重复评审。
- 触发钩子复用既有两个收口点（group 走 `prepare_group_final_confirm_from_readiness`，WorkItem scope 走 `maybe_complete_issue_delivery`），per-attempt 收口逻辑本身不改。

### D8 advisory 红线、留痕与来源枚举

- advisory：结论不进门、不改 attempt status/stage、不建 pending gate、不开评审窗、不進 plan-defect 路由；失败/超时/不可解析记 `Failed`，不阻断、不自动重试。
- 留痕：单点写入（两条确认入口都经过 `handle_final_confirm`），幂等键 `(cross_repo_review_id, attempt_id)`；内容只有时间 + 评审记录 id + 结论 + findings 摘要，**不含确认人与通道**（结构键 id/project/issue/plan/attempt 保留，它们是落盘路径与幂等键）。
- 来源枚举：`IssueCrossRepoReview` 只作 `findings.source_stage` 取值，序列化 `issue_cross_repo_review`；既有 7 个成员的序列化字符串与 `order()` 相对序不变；`order()`/目录名/DTO 文案/WS 白名单等穷尽 `match` 必须补 arm（编译面），但 attempt.stage 永不为新值。确实需要登记到 `review_parser.rs` 的严格字符串表，否则 provider 输出的 cross-repo finding 解析直接硬失败。
- 呈现面只读：issue 级 durable 记录、attention 新 kind、plan 最终确认卡片的两个可选字段；ready/blocked 判定不改。

### D9 单仓不变 = 契约级验收，不靠「语义等价」

每条 requirement 的单仓场景都写成可观察断言：窗口记录与 window id、早返回路径、baseline fail-closed、prompt/摘要字节、canonical digest、provider 启动次数。实现时凡是「重写单仓可达路径」的方案一律退回条件化分支。

## 契约 ↔ 设计条目映射（权威）

派工契约的 1~19 编号（按 A~E 分组）与两份设计稿的章节条目号**不是同一套编号**，引用时以本表为准。

| 设计稿条目 | 内容 | Requirement |
|---|---|---|
| v1.2 §17 | 每会话 UUID 身份（lc 用 UUID，单仓保持窗口编号） | REQ-ENV-13 |
| v1.2 §1 | 删除「主仓 / 主成员」概念，评审范围取集合 | REQ-PLN-11 |
| v1.2 §2 | 单值字段降级为兼容（评审目标随范围取 LC 根，不要求成员 worktree） | REQ-PLN-11 |
| v1.2 §3 | lc 会话不再开评审窗（单仓保留窗口 + 零锁恢复） | REQ-ENV-14 |
| v1.2 §4 | 仓库清单全量注入，超预算显式阻断（500 KiB） | REQ-PLN-12 |
| v1.2 §6 | 材料改稳定注入（单仓不注入成员清单） | REQ-PLN-13 |
| v1.2 §5 | 补逐仓实况（职责/关键目录/入口/构建测试命令/基线） | REQ-PLN-14 |
| v1.2 §16 | 逐仓基线 = 各仓当前 HEAD 分支 | REQ-PLN-14 |
| v1.2 §8 | 补实质 must-fix 四类（单仓文案逐字不变） | REQ-PLN-15 |
| v1.2 §11 | finding 缺 `evidence`/`required_action` 不收 | REQ-PLN-16 |
| v1.2 §13 | 旧轮摘要保留 `message`/`evidence` | REQ-PLN-16 |
| v1.2 §14 | WorkItem splitter 目录摘要分层 + 失败显式 | REQ-PLN-17 |
| v1.2 §9 / §10 / §12 | 不改：severity 分级、revision 轮只看旧问题、verdict 结构错误静默降级 | 非目标（无 requirement，见 Goals / Non-Goals 与 Risks） |
| v1.1 §三 A1~A4 + §十二 | 条件化停用每仓 internal review / GroupFinalReview 与存量 attempt 兼容 | REQ-GFR-CR-01、REQ-GFR-CR-02 |
| v1.1 §七 E + §一 | 触发恰好一次、按 issue 幂等、不新增 attempt 阶段 | REQ-GFR-CR-03 |
| v1.1 §四 B3 / B4 / B5 | 全新 prompt、逐仓 diff 分段材料、provider 回退链 | REQ-GFR-CR-04 |
| v1.1 §五 C | advisory 语义与红线清单 | REQ-GFR-CR-05 |
| v1.1 §六 D | 已知风险确认留痕（去确认人/通道） | REQ-GFR-CR-06 |
| v1.1 §四 B1 / B2 / B6 | issue 级记录模型、输出 schema、`source_stage` 新枚举与严格字符串表 | REQ-GFR-CR-07 |
| v1.2 §九、v1.1 §八 F | 单仓回归与多仓夹具验收 | 各 requirement 的 Scenario |

## 与在途 change 的重叠面与归档顺序

本 change **不覆盖任何在途条款**：三个 capability 的 delta 全部是 `## ADDED Requirements`，与在途 change 的 MODIFIED/ADDED header 无重叠。逐一核对：

| 在途 change | 该 capability 的在途条款 | 关系 |
|---|---|---|
| `provider-desandbox-full-open` | `logical-codebase-aggregate-planning` 改 REQ-PLN-01/06；`session-policy-envelope` 改 REQ-ENV-01/02/03/05/09 并 ADDED REQ-ENV-HOST-01/REVIEW-B/CHECK-01 | 无 header 重叠；**语义上需对齐一处**：REQ-ENV-REVIEW-B 原文为「所有 Reviewer 运行」采用 B 档窗口，本 change 的 REQ-ENV-14 对多仓文档环节材料评审作出范围限定 |
| `lc-root-initialization` | `session-policy-envelope` 改 REQ-ENV-01/03/04/06/07/08 并 ADDED REQ-ENV-10/11；`logical-codebase-aggregate-planning` 改 REQ-PLN-01/03/06/07 | 无 header 重叠 |
| `fix-planning-session-anchor-recovery` | `session-policy-envelope` 改 REQ-ENV-04/08；`logical-codebase-aggregate-planning` 改 REQ-PLN-03/05/07 并 ADDED REQ-PLN-08 | 无 header 重叠 |
| `fix-error-experience-structural-governance` | `logical-codebase-aggregate-planning` ADDED REQ-PLN-09/10 | 无 header 重叠 |
| `aggregate-policy-root-publication` | `session-policy-envelope` ADDED REQ-ENV-12 | 无 header 重叠 |
| `batch1-coding-autopilot-mainline` / `fix-local-delivery-and-chain-truth` | `coding-run-operability` ADDED REQ-CRO-07..12 | 本 change 不写 `coding-run-operability`，无重叠 |

**REQ-ENV-REVIEW-B 的冲突处理（必须显式）**：REQ-ENV-REVIEW-B 目前只存在于 `provider-desandbox-full-open` 的 delta（未归档，主规范中不存在），因此本 change 无法以 MODIFIED 修订它。REQ-ENV-14 用范围限定措辞落地：多仓文档环节评审是只读材料评审，不进入 B 档窗口档；需要验证执行面的评审（coding 侧审查）仍受 REQ-ENV-REVIEW-B 约束。归档硬门见 `tasks.md` 第 5 组：若 `provider-desandbox-full-open` 先归档，本 change 归档前必须把该限定补进主规范 REQ-ENV-REVIEW-B（MODIFIED），使主规范不出现「所有 Reviewer 运行都用窗口」与 REQ-ENV-14 并存的自相矛盾。

`group-final-review-triage` 当前无任何在途 change 触碰，本 change 是该 capability 的唯一在途修订方。

## 风险与已知取舍

- **身份改 digest 域**：多仓 reviewer-verify canonical digest 值会变；单仓不变。若多仓存量会话在升级后重放旧 digest，会判为 resume 不一致 → 按既有超时/新建会话路径处理，属预期（本批不做迁移映射）。
- **finding 完整性抬高多仓返修率**：缺证据即 `needs_human`，是 owner 明确的取舍（不允许「空证据也通过」）。
- **材料全量注入带来 prompt 体积上升**：用 500 KiB 硬上限阻断而非裁剪，宁可显式失败，符合「不静默缺员」。
- **已知缺陷不修**：verdict 结构错误静默降级、severity 与 revision 轮规则维持现状；本 change 的 Non-Goals 明确列出，避免实现阶段顺手改掉单仓 prompt 字节。
