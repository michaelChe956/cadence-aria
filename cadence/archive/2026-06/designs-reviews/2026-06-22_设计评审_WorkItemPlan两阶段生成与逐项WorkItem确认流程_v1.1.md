# WorkItemPlan 两阶段生成与逐项 Work Item 确认流程设计评审

## 文档信息

- 文档类型：设计评审
- 版本：v1.1
- 日期：2026-06-22
- 分支：feat-b-0616
- 被评审方案：`cadence/designs/2026-06-22_技术方案_WorkItemPlan两阶段生成与逐项WorkItem确认流程_v1.3.md`
- 评审方式：只读调研 + 与当前 `WorkspaceEngine` / WS contract / 前端 store / `LifecycleStore` 对照

## 评审结论

v1.3 已基本吸收 v1.0 评审中的主方向修订：自动模式收敛为串行批量、`repository_profile` 从 WorkItemPlan 主流程退出、reviewer 开关回到统一语义、Draft 阶段改为独立记录。这些方向是正确的。

但当前方案仍有 2 个 P0 与 3 个 P1 缺口会直接影响实现计划拆解。建议先修订 v1.3，再拆任务；否则实现阶段会在 Draft 历史、最终编译幂等、review verdict 路由和人工入口上出现返工。

## 项目进度摸底

- worktree：`.worktrees/feat-b-0616`
- 分支状态：`feat-b-0616...origin/feat-b-0616`，工作区干净，`git pull --ff-only` 显示已是最新。
- 分支内容：相对 `origin/main` 该分支已包含 WorkItem 拆分、WorkItemPlan 对话式 Workspace、配置弹窗、分组删除、WorkItemPlan 流式进度与恢复等实现和文档，变更面覆盖后端 engine/store/WS、前端 store/page/component、集成测试。
- 当前方案状态：v1.3 标注为“待实现计划拆解”，尚未进入两阶段生成与逐项确认的实现计划。

## 主要问题

### P0-1 Draft store 以 `outline_id` 为主键，无法同时满足历史保留与新轮次活跃记录

方案把 `WorkItemDraftRecord` 文件路径定义为 `.aria/.../work_item_plan_drafts/<plan_id>/<outline_id>.json`，并说明 Draft 阶段以 `outline_id` 作为文件名与主键。同时又要求旧 draft 标记 `superseded`、timeline 历史保留、新轮次可复制旧 draft 为新的 `accepted` record。

这三个要求在同一个 `<outline_id>.json` 文件上不能同时成立：

- 同一 outline 多次重写时，新 draft 会覆盖旧 draft，旧 draft 的完整内容无法作为 `superseded` 历史保留。
- `plan_reopen_required` 后，如果返修后的 Outline 复用相同 `outline_id`，新一轮 active draft 与旧轮次 superseded draft 会争用同一主键。
- 刷新恢复时，timeline node 只能指向一个当前 `<outline_id>.json`，无法稳定回放历史 run 对应的 draft 内容。

建议修订：

- 引入不可变 `draft_id` 或 `generation_round + outline_id + attempt` 作为 Draft record 主键。
- `outline_id` 只作为业务关联字段，不作为唯一文件名。
- active draft 通过索引或 `current_draft_id` 指向；历史 draft 永不覆盖。
- 阶段 4 编译只读取当前轮次 `accepted` 且未 `superseded` 的 draft。

相关位置：方案 v1.3 第 402-433 行、784-789 行。

### P0-2 最终 strict validator 的 item 级失败会打破“已确认中间项不可重写”的串行约束

方案在串行模式中明确“不存在重写已确认的中间项的回退路径”，前序已确认项保持不变。但阶段 4 才运行 full `WorkItemSplitValidator`，并规定 item 级失败时“定位到具体 work item，返回该 item 的重写入口”。

这会形成冲突：

- 如果最终 strict validator 发现第 1 个已确认 item 的 verification plan 或 scope 有错误，而第 2、3 个 item 已基于它的 handoff 生成并确认，单独重写第 1 项会使后续项上下文失效。
- 如果不允许重写第 1 项，最终编译无法修复。
- 如果只重写第 1 项但不声明后续项失效规则，会编译出基于旧上下文生成的后续 work items。

建议修订：

- 在每个 item 进入 `accepted` 前运行可定位到 draft 的局部 strict validation，尽量把 item 级错误提前到当前 item。
- 阶段 4 若仍发现早期 item 级失败，必须定义依赖失效规则：目标 item 及所有 downstream drafts 标记 `superseded` 并重新生成。
- 自动模式保持整组重写即可，但串行模式不能只写“返回该 item 重写入口”。

相关位置：方案 v1.3 第 186 行、236-240 行、879-886 行、888-915 行。

### P1-1 `final_compile` 缺少原子性、恢复与 abort 语义

方案的最终编译要顺序创建真实 `LifecycleWorkItemRecord`、`VerificationPlan`、`IssueWorkItemPlan.dependency_graph` 和 child workspace sessions，并在矩阵中允许 `final_compile` 阶段接收 `abort`。当前 `LifecycleStore` 是多个 JSON 文件顺序写入，现有 candidate 替换流程也是先删旧 work item / verification plan / repository profile，再逐个创建新文件，最后更新 plan。

如果 final compile 过程中断、abort、进程崩溃或写到一半失败，可能留下：

- 真实 work item 已创建，但 plan 未完成更新。
- verification plan 已写入，但 child session 未创建。
- child session 创建了一部分，strict validator 失败后无法回滚。
- 刷新恢复时不知道是继续编译、回滚还是人工处理。

建议修订：

- 明确 `final_compile` 是不可取消的短事务，或将 `abort` 改成“请求停止下一步但不打断已开始的持久化提交”。
- 使用 compile transaction record：`compile_id`、状态、步骤、created ids、commit marker。
- 所有真实实体创建必须幂等，重复执行同一 `compile_id` 不产生重复 child session。
- strict validator 通过前先写临时区域或 staged records，通过后一次性提交当前 plan 指针。

相关位置：方案 v1.3 第 225-233 行、287-301 行、917-955 行；现有 `LifecycleStore::replace_issue_work_item_plan_candidate` 为多文件顺序写入，见 `src/product/lifecycle_store.rs` 第 708-820 行。

### P1-2 新 reviewer verdict 没有完整落到 WS / ReviewGate 协议

方案为单 item reviewer 增加 `plan_reopen_required`，为 batch reviewer 增加 `revise_batch` 和 `plan_reopen_required`。但当前协议只有：

- `ReviewVerdictType = pass | revise | needs_human`
- `ReviewGate = requires_revision | user_confirm_allowed | user_triage_required`
- `parse_review_json` 只接受 `pass/revise/needs_human`，未知 verdict 会退化为人工确认。

v1.3 只说明 prompt schema 和新增输入消息，没有明确扩展 `ReviewComplete`、timeline node metadata、前端 review 状态和 ReviewGate 路由。实现时如果 provider 真的输出 `plan_reopen_required` 或 `revise_batch`，当前 contract 会把它当作无法解析的 review，丢失“回 Outline 返修”或“整组重写”的精确语义。

建议修订：

- 明确新增 `ReviewVerdictType` 或新增 WorkItemPlan 专属 review verdict payload。
- 明确 `plan_reopen_required` 映射到哪种 stage/node：进入 Outline revision、进入人工决策，还是先清理/supersede drafts。
- 明确 `revise_batch` 与普通 `revise` 的区别：前端操作区、后端 handler 和 provider run kind 都要能区分。
- 补充 `ReviewComplete` / `SessionState` / timeline node metadata 的兼容策略。

相关位置：方案 v1.3 第 693-719 行、749-773 行、975-985 行；当前协议见 `src/web/workspace_ws_types.rs` 第 564-609 行，当前解析见 `src/product/workspace_engine.rs` 第 4897-4935 行。

### P1-3 `context_blockers` 与 Outline 轻量校验失败缺少可操作的人工入口

方案要求旧 Design spec 信息不足时，Outline author 可返回 `context_blockers[]`，后端停在 `outline_running` 的人工确认/返修入口，不进入 `outline_confirm`。错误处理章节又说轻量校验失败时停在 `outline_running` 直接返修，连续超过上限后转 `human_confirm`。

但 Stage / WS 矩阵中 `outline_running` 只允许 `abort`，没有用户补充反馈、选择“继续探索”、进入人工决策或带反馈重写的输入消息。当前前端也只把 `running/cross_review/revision` 当作 provider active stage，不显示人工操作区。

建议修订：

- `context_blockers` 这类需要用户判断的问题应进入明确的 `human_confirm` 或 `author_confirm` 派生节点，而不是停留在 `outline_running`。
- 若继续使用 `outline_running`，必须新增可接收用户反馈的 WS 输入，并定义 UI 操作区。
- 区分“可自动返修的 outline validator errors”和“必须人工补充上下文的 context blockers”。

相关位置：方案 v1.3 第 91-93 行、287-300 行、837-841 行。

## 次要问题

### P2-1 `repository_profile` 移除范围还不完整

方案明确移除 `IssueWorkItemPlan.repository_profile_ref`，并删除 validator 中 `repository_profile_missing` / `repository_profile_low_confidence`。但当前模型中 `VerificationPlan.repository_profile_ref`、`CreateVerificationPlanInput.repository_profile_ref`、WS/HTTP DTO 的 repository profile 展示也仍存在。方案没有说明这些字段在 WorkItemPlan 新流程中是删除、保留但置空，还是只在非 WorkItemPlan 场景继续使用。

建议补充字段级兼容策略，避免实现时只删 plan 字段，却留下 dangling reference 或前端仍展示 Repository Profile 区块。

相关位置：方案 v1.3 第 448 行、811-812 行、915 行；当前模型见 `src/product/models.rs` 第 612-618 行、632-642 行。

### P2-2 自动模式上下文术语不准确

自动模式没有逐项用户确认，但方案仍多处说 per-item prompt 携带“前序已确认 work item”。自动模式中这些只能是“本轮已生成并通过调度器接收的 draft”，不是用户确认的 accepted item。

建议把自动模式上下文统一改成“previous generated draft records in current batch”，并明确这些 draft 在整组确认前的状态不是 `accepted`，否则 Draft store 状态机和 prompt 上下文会混用串行语义。

相关位置：方案 v1.3 第 190-202 行、721-732 行。

## 需确认的流程问题

1. `plan_reopen_required` 触发后，是否默认让所有已确认 drafts 失效，还是允许用户逐项选择复用？
2. 最终 strict validator 若定位到早期已确认 item，是否应强制该 item 的所有 downstream drafts 失效？
3. `context_blockers[]` 是进入人工确认节点，还是在 outline author run 内通过 choice request 继续交互？

## 建议下一步

先修订 v1.3，至少补齐 Draft record 不可变主键、final compile 事务/恢复、review verdict 协议扩展、context blocker 人工入口、strict validator 失败后的 downstream invalidation 规则。修订后再拆实现计划，计划中应把 Story/Design/普通 Work Item Workspace 的共享链路回归测试作为显式 DoD，确保新增 WorkItemPlan 专属 node type 和 payload 不影响既有三类 Workspace。
