# P0 三条落地状态核实

> 本报告依据当前 worktree 源码判断；引用行号对应核实时该工作树的源码。未将历史说明文档当作实现证据。

```text
命令：cd /home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02 && pwd
输出：/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02
```

## 结论摘要

1. 2026-10-30 五条修法：修1 **已落地**；修2 **已落地**（schema 已有处置归属和目标 WI 字段）；修3 **已落地**；修4 **已落地**；修5 **已落地**。结论严格限于 Work Item Plan 单候选 reviewer/gate 链。
2. v2.0 §7：改动1 **已落地**；改动2 **已落地**（基础 IR 投影先留空，随后必经统一归属解析补齐）；改动3 **已落地**。路线 A **已完成**。路线 B **未完成**，PlanMemberTargets 中没有 checkout 目录前缀表，四个 target key 的平台派生及前缀表接线尚非现有实现。
3. P0-1 竞态：已确认该竞态路径的触发机制是绑定新 plan/session 时 `bind_plan_ids_locked` 增加 policy_revision，导致已读取旧 revision 的 re-enable 序列步骤① CAS 冲突。**这不是全系统所有 revision 递增的唯一来源**：Enable/Disable、terminate、rebind、terminated enrollment 取代也会递增。去掉绑定递增可消除“绑定落在①与②之间”的特定竞态，但**不能据此断言任何场景下前端都不需 revision 一致性**；步骤② Disable 会递增 revision，步骤③必须使用步骤②响应的当前 revision。现有前端已如此实现。

## 问题1｜2026-10-30 五条修法

| 修法 | 判定 | 当前源码证据 |
|---|---|---|
| 修1：finding 范围契约化 | **已落地**（单候选审查采用所有 reviewer builder 共用硬约束；不是原样散落在每个 builder 的独立文案） | `src/product/workspace_engine/prompts/owner_guidance_rules.rs:16-34`：`FINDING_SCOPE_RULES` 标明“所有 reviewer 输出契约共享的唯一 finding 范围与收敛口径”；`:25-28` 原文：“finding MUST 落在本 Work Item 的写入范围 / 验收标准 / 能力契约之内。‘仓库内其它文档的当前状态’‘产品侧规范或流程待办’‘与本 Work Item 无关的外部事实’不是本 Work Item 的缺口——若仍要提示，MUST 标 disposition: \"advisory\"，且 required_action 不得要求作者执行”。`src/product/workspace_engine/prompts/review_parts/single_candidate_input.inc.rs:241-242` 还限定：“只审核 Plan Review Context 中的权威 canonical contract、依赖拓扑、跨 WorkItem 契约供需与机械校验摘要”。`review.rs` 中其他 reviewer schema 没有逐条重复这句，但由 `reviewer_output_contract` 共用条款注入；`owner_guidance_rules.rs:52-98` 有源码级 7 处接线断言。 |
| 修2：findings 归属字段 | **已落地** | 单候选 schema：`src/product/workspace_engine/prompts/review_parts/single_candidate_input.inc.rs:274-276`，逐字包含 `"disposition":"actionable|advisory"` 与 `"target_work_item_id":"WI-001"`。类型定义：`src/web/workspace_ws_types/review.rs:130-141`，`disposition: Option<ReviewFindingDisposition>`（枚举 Actionable/Advisory，字段缺失按 actionable）；`target_work_item_id: Option<String>`；平台计算的 `finding_key: Option<String>`。解析：`src/product/workspace_engine/parsers.rs:710-712`。字段是可选归属/路由元数据，不是必填文件路径；没有 `file_path` 字段。 |
| 修3：owner 排除 + durable 处置回喂 | **已落地** | 单候选门收到 exclusion 入站后调用 durable 保存：`src/product/workspace_engine/conversational_gate.rs:1020-1024` 记录该操作是“排除此项”并说明 durable；`:1055-1067` 构建 `HumanDispositionRecord`（finding key、Excluded、时间）并 `record_human_dispositions`。落盘：`src/product/lifecycle_store/human_dispositions.rs:65-109`，路径 `human-dispositions.json`，按 finding_key 幂等写入。下一轮 prompt：`src/product/workspace_engine/prompts/review_parts/single_candidate_input.inc.rs:264-267` 注入 human dispositions；`src/product/workspace_engine/prompts/review_context.rs:642-679` 将清单渲染为 `[human_dispositions]`，并写明 `MUST NOT` 再报 actionable。另有平台端强制 disposition 归一，见 `src/product/workspace_engine/review/disposition_override.rs:4-17,51-66`。 |
| 修4：收敛性 | **已落地** | `src/product/workspace_engine/prompts/owner_guidance_rules.rs:33-34`：“重复报告同一事实时 MUST 在 message 里说明‘上一轮的处置为何没有解决它’；已处置且事实未变 → MUST NOT 报为 actionable。”单候选重复轮细则：`src/product/workspace_engine/prompts/review_context.rs:64-70`；上一轮 fingerprint 过滤/回喂及重复说明要求在 `:512-518,548-586`，`:586` 原文要求 evidence 显式标注 repeated 并说明为何自动修订未能关闭，不得改写措辞隐式重复。处置事实未变时另外由 disposition_override 强制改 advisory（前项源码）。 |
| 修5：validation_reject 诊断、门卡可行动信息与编译诊断落盘 | **已落地** | `src/product/workspace_engine/conversational_gate.rs:1071-1108` 仅在 Failed + ValidationReject 时生成并 durable 保存拒绝信息；`:1111-1132` 回传拒收帧。持久化 `src/product/lifecycle_store/human_dispositions.rs:151-172`，逐 turn 存在 `human-gate-rejections/{turn_id}.json`；编译诊断独立落盘为 `candidate-compile-status.json`，`:219-243` 且明确 detail 不截断。门卡字段 `web/src/components/chat-workspace/entries/GatePromptEntry.tsx:920-960`；显示块 `:1120-1161`，依次显示“上一条反馈被拒收”、被拒反馈、原因、下一步动作、逐字诊断。该问题要求的“编译诊断落盘”则由 `:219-243` 与编译失败 durable 状态实现；门卡读到 rejection diagnostics 原文。 |

### 关于“七处 schema”范围

当前目标是 **Work Item Plan 单候选链**，对应专用 schema 在 `single_candidate_input.inc.rs:274-281`，有 `disposition` 与 `target_work_item_id`，共享范围规则由 `reviewer_output_contract` 注入。通用 `prompts/review.rs` 中不带这些字段的其他 schema 分属 outline、batch、projection 等不同链，不能用它们否定单候选链已落地，也不能把它们当作本报告所查对象。共享规则测试以 `owner_guidance_rules.rs:52-98` 明确覆盖七次 reviewer contract 调用及 schema 字段。

## 问题2｜技术方案 v2.0 §7 三处改动、路线 A/B

| 改动 | 判定 | 当前源码证据 |
|---|---|---|
| 1. 唯一物理仓→逻辑成员反查，门与 finalizer 共用 | **已落地** | 唯一定义 `src/product/workspace_engine/member_attribution.rs:31-51`：`pub(crate) fn reverse_member_logical(physical: &str, physical_by_logical: &BTreeMap<LogicalRepositoryId, String>) -> MemberReverse`，三态 Unique/Absent/Ambiguous，禁止多命中猜测。候选门路径：`member_attribution.rs:128-163` 的 `resolve_member_targets` 调用 `:149`（single_repository）和 `:155`（未声明 intent）；入口 `src/product/workspace_engine/plan_projection.rs:981-1004`，`:994` 调统一 resolver。Finalizer：`src/product/workspace_engine/compile/finalizer.rs:899-930`，`:930` 调同一函数。IR adapter 也调用同一实现：`src/product/workspace_engine/compile/ir_adapter.rs:224-240`。 |
| 2. 候选门 single_repository 反查而非恒空 | **已落地** | `member_attribution.rs:82-86` 说明基础投影 `SingleRepository` 只知物理仓，先设 `(Some(repository_id), None)`；并非最终仍空。`resolve_member_targets` 在 `:147-150` 对该 target 用 `reverse_member_logical`，结果唯一命中填入 `logical_target`（`:158-161`）。结构门的实际调用 `plan_projection.rs:989-1004`，`:994` 在投有效图前完成补齐。无授权成员表/legacy 单仓则 `:126-127,995-1002` 保持旧语义清空，不误作漏实现。 |
| 3. compile failure owner 提示由 witness 事实生成，且有带后果选项 | **已落地** | `src/product/workspace_engine/compile_failure_surface.rs:466-471` 明确 owner 提示“只由 witness 事实构成，不断言根因在 design”；witness 缺失时不虚构原因/位次/选项（`:469-470,503-507`）。选项函数 `:80-109` 给两种可选修法；第一项后果在 `:85-92`，第二项及后果在 `:99-105`。调用渲染 `:482-500`，逐项输出“——后果：…”。 |

**路线 A：已完成。** 证据即上表唯一反查函数及候选结构门、Final Compile（并 IR adapter）共用调用。

**路线 B：尚未完成，剩余项：**

- 将 Plan Intent 的四个 target key 改为平台派生；当前 `PlanMemberTargets` 只有 `physical_by_logical` 与 fallback，定义见 `src/product/work_item_plan_compiler/lower.rs:44-53`，不存在 checkout 目录前缀表字段。
- 将平台派生的 checkout 目录前缀表接入 validator 的 member checkout prefix 核验。`src/product/work_item_plan_compiler/validate.rs` 的上下文存在 `member_checkout_prefixes` 消费点，但该表不在 `PlanMemberTargets`（本报告检查结论）；另有 `src/product/workspace_engine/draft_batch/member_checkout_prefixes.rs` 独立解析 helper，不等同于 Plan Intent 四个 target key 的平台派生接线。

## 问题3｜P0-1 policy_revision 竞态机制边界

### bind_plan 自动化调用与时点

自动化生成路径不是 HTTP 手动 bind_plan：`src/web/handlers/lifecycle/plan_preparation.rs:278-285` 是 `ensure_enrolled_plan`；`:332-350` 通过 `IssueAutomationStore::ensure_plan_binding` 的 create 闭包调用 `prepare_plan_records`，传入准备好的 plan/session ID。创建回调后 `src/product/issue_automation_store.rs:423-447` 重读 enrollment 并调用 `bind_plan_ids_locked`，`:447` 是共享绑定点。首次自动 prepare 因创建 plan/session 成功后立刻在锁内绑定并持久化；因此确实是自动生成链完成 prepare 后的绑定，但绑定由 `ensure_plan_binding` 触发，不是外部 HTTP 的 `bind_plan`。直接 `bind_plan` 公共入口在 `issue_automation_store.rs:287-314`，不是本自动生成闭包的调用方。

绑定递增源码：`src/product/issue_automation_store_parts/cas_internals.inc.rs:30-58`，只有 `(None,None)` 新绑定分支 `:55` 加一；注释 `:49-50` 说补全 durable plan/session 身份、binding 不是换代。该增量与 C1 注释意图（绑定不应换代）不一致，是竞态敏感点。

### 所有 policy_revision 递增路径

- **绑定补全（竞态触发点）**：`src/product/issue_automation_store_parts/cas_internals.inc.rs:55`。`bind_plan_ids_locked` 新的 plan/session binding。
- **首次 Enable**：`cas_internals.inc.rs:420-426` 初始化 `policy_revision: 1`（不是 `+=`）；新建 enrollment。
- **disabled 后 Enable 重开**：`cas_internals.inc.rs:443-500`，`:500`；授权 payload/启用状态重开。
- **Disable**：`cas_internals.inc.rs:504-508`，`:506`；启用置 false。
- **终止 plan**：`src/product/issue_automation_store_parts/command_writes_impl.inc.rs:9-67`，`:62-66`；设置 `enabled=false`、terminated_at、终止账本并递增。
- **显式 rebind**：`command_writes_impl.inc.rs:84-190`，`:172-178`；更新 current binding、source/provider，递增 policy revision；binding_version 同时换代。
- **terminated enrollment 人工取代重开**：`src/product/issue_automation_store_parts/cas_internals.inc.rs:606-617,683-695`，`:694`；清 terminated、解绑 plan/session、轮换 prepare intent 与 binding 并递增。
- 未递增的状态写：attention fact upsert / resolve 明确注明不递增（`command_writes_impl.inc.rs:280-285,339-341`）；追加命令账本 `:228-275` 只改 ledger 与 updated_at，不是策略改版。检索 `policy_revision +=` / `policy_revision =` 后，业务递增路径如上；测试夹具中的字面赋值不构成生产写路径。

### 前端 runReenableSequence 的 revision 链

- 入口先 `getAutomationEnrollment` 一次：`web/src/components/lifecycle/useIssueLifecycleGeneration.ts:348-355`。
- `runReenableSequence` 内没有其他 `getAutomationEnrollment`。若旧 enrollment 没 target，代码可能调用 `getAutomationTarget`（`:503-515`），不是 enrollment GET；在正常有 target 的路径不需要它。
- 步骤① Enable 使用 `current.policy_revision`，响应赋回 `current`：`:528-547`。
- 步骤② Disable 使用此时 `current.policy_revision`，响应再次赋回 `current`：`:564-569`。
- 步骤③ Enable 使用步骤②响应 `current.policy_revision`：`:572-586`。

### 结论：竞态必要条件 / 只改服务端是否足够

对已描述的特定 stale-revision 竞态，必要交错是：前端取得 terminated enrollment 的 revision R；其间自动链新建并绑定 plan/session、服务端 bind 更新 `policy_revision` 为 R+1；步骤①再用 R 发 Enable，服务端 CAS 按当前 revision 拒绝。源码依据：绑定 `:55` 递增；前端取旧记录 `useIssueLifecycleGeneration.ts:354`，步骤① `:541-547` 使用它；服务端 CAS 检查 revision 的 `issue_automation_store.rs:269-272`（非特殊幂等分支）。故**特定竞态的额外 revision 变化由 bind_plan/ensure_plan_binding 的绑定递增造成**，但不是全局唯一的递增机制（上一节列出其余路径）。

最小修法若定义为“只让 plan/session 绑定不改变 policy_revision”，那么可以消除上述①之前由自动绑定制造的冲突，前端三步代码不必为此改动；然而不能笼统说前端无需 revision 一致性：Disable 本身在服务端递增 revision（`cas_internals.inc.rs:504-508`），③必须用②响应的新 revision；当前前端的动态 `current` 恰好满足这一点。①也必须防止和真正并发的授权写入（Enable/Disable/terminate/rebind）竞争，因此 CAS revision 校验不应被移除。只改 bind 递增是此竞态的最小服务端修正方向，不等于取消 CAS 或前端 revision 用法。

## 实际检索范围与命令

工作目录确认：`cd /home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02 && pwd`，输出 `/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`。

源码读取/检索的实际文件范围：

- `src/product/workspace_engine/prompts/owner_guidance_rules.rs`
- `src/product/workspace_engine/prompts/review.rs`
- `src/product/workspace_engine/prompts/review_parts/single_candidate_input.inc.rs`
- `src/product/workspace_engine/prompts/review_parts/draft_input.inc.rs`
- `src/product/workspace_engine/prompts/review_context.rs`
- `src/product/workspace_engine/review/disposition_override.rs`
- `src/product/workspace_engine/conversational_gate.rs`
- `src/product/workspace_engine/conversational_gate_parts/dispositions.inc.rs`
- `src/product/workspace_engine/parsers.rs`、`src/product/workspace_engine/parsers/lenient_findings.rs`
- `src/web/workspace_ws_types/review.rs`
- `src/product/lifecycle_store/human_dispositions.rs`
- `web/src/components/chat-workspace/entries/GatePromptEntry.tsx`
- `web/src/state/workspace-chat-rebuild.ts`、`web/src/hooks/workspace-ws-message-handler.ts`、`web/src/state/workspace-ws-store-human-gate.ts`
- `src/product/workspace_engine/member_attribution.rs`
- `src/product/workspace_engine/plan_projection.rs`
- `src/product/workspace_engine/compile/finalizer.rs`、`src/product/workspace_engine/compile/ir_adapter.rs`
- `src/product/work_item_plan_compiler/lower.rs`、`src/product/work_item_plan_compiler/validate.rs`
- `src/product/workspace_engine/draft_batch/member_checkout_prefixes.rs`
- `src/product/issue_automation_store.rs`
- `src/product/issue_automation_store_parts/cas_internals.inc.rs`
- `src/product/issue_automation_store_parts/command_writes_impl.inc.rs`
- `src/web/handlers/lifecycle/plan_preparation.rs`
- `web/src/components/lifecycle/useIssueLifecycleGeneration.ts`

实际检索命令（均在上述工作目录执行；多次 `rg -n` 因定位需要更换关键词/文件）：

```bash
pwd
rg -n "reverse_member_logical|planning_structure_items_from_ir|compile_failure_surface|PlanMemberTargets|bind_plan_ids_locked|policy_revision|runReenableSequence|getAutomationEnrollment|validation_reject" src web
rg -n "REQ-WSC-1[2-7]|finding.*范围|已处置|排除此项|validation_reject|target_work_item_id|belongs_to_work_item|file_path" src/product/workspace_engine
rg -n "policy_revision \+=|policy_revision =" src/product/issue_automation_store_parts src/product/issue_automation_store.rs
rg -n "bind_plan\(|ensure_plan_binding\(" src/product src/web
rg -n "HumanGateTurnRejected|rejected_feedback|rejection_reason|rejection_next_actions|rejection_diagnostics" web/src src/web/workspace_ws_types
rg -n "target_work_item_id|pub disposition|pub finding_key|pub owner_question|pub owner_options|struct ReviewFinding" src/web/workspace_ws_types src/product/workspace_engine
```
