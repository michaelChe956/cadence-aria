# Design: WorkItem 对话式人工门与 advance 接口（阶段 3）

## Context

阶段 2 单候选 MVP 已收官归档：auto 模式 codex/pi 实证 Confirmed；人工门现状为一次性 `HumanConfirm` 决策消息（confirm/request-change），SC 的 manual revision 走 legacy 中文标题约束链被拒（r27 实证断点）；plan Confirmed 后无受控推进接口；legacy coding 为 WorkItemGroup attempt（共享 worktree、order_index 串行 unit、进度 UI、amendment 链俱全），而 SC 在 plan confirm 时创建的一 WI 一个 session 与 coding attempt/unit 无运行绑定。阶段 1 已沉淀 typed outcome、预算、防环指纹、门快照原子写入与 auto stopped 接管契约，全部复用。

约束：阶段 2 退役门未全过（pi 938.04s > 12min 子项，用户裁决 1c）→ 旧协议与 legacy 入口全部保留；SC author prompt 预算 19,000 余量仅 66B（红线：教学先调常量）→ revision prompt 独立预算；弱模型教学泛化三连教训 → 教学写死作用域与反面清单。

## Goals / Non-Goals

**Goals:**

- 对话式人工门：typed 反馈回合、durable turn、预算/单飞/幂等、SC 专属修订路径
- `advance`：Confirmed → group coding ready 的幂等编排（ready-only）
- group coding 依赖就绪门（依赖图+handoff readiness 权威，order_index 仅 tie-breaker）
- 失败留在同一 attempt 内；plan 缺陷接 SC amendment 链

**Non-Goals:**

- 不重构 coding engine 内部；不改 SC author prompt 预算常量
- 不做多仓 coding（阶段 4）；不删除任何旧协议
- 不让 SC per-WI session 成为第二 coding 执行面
- 不做「先攒多条反馈再批量应用」的反馈编辑器形态（oracle 已排除）
- kimi 修复验证、95% 测量、平台级规则获取：独立专项，不入本 change

## Decisions

### D1：回合=typed `human_gate_feedback`，非任意消息（用户批准，oracle 窄化）

一个回合=服务端接受的 typed 命令；approve/abandon/重连/ack 不耗预算不触发 provider；`command_id`（客户端幂等键）+ 服务端 `turn_id` 双键：同 command_id 返回同 turn_id，同 turn_id 不重复扣预算/启动 provider。备选「攒消息+显式应用」（需反馈队列/apply 命令/队列持久化/取消语义）被 oracle 排除：扩状态面却不解决 SC revision 断点。

### D2：单飞 + 预算预留即扣、失败不退（用户批准）

同一门至多一个非终态 turn，冲突回 `gate_busy` 不排队；预算在 durable 预留时扣减（防客户端重试绕过）；validation_reject/provider_err 不退还（防无限试错，与 manual_repairs 现有语义一致）；provider 瞬断同 turn_id 内 attempt_no++ 恢复，不新建 turn。预算耗尽的语义对齐阶段 1 既有契约（见 D10）。

### D3：SC revision 专属路径（用户批准）

feedback+当前候选全文+grammar 边界 → 独立预算常量 `SC_MANUAL_REVISION_PROMPT_QUALITY_BUDGET_BYTES`（初值实施时实测上调整百级）→ provider 输出完整修订版 markdown → 确定性前言修剪（复用 d192755e）→ SC compiler → canonical validator。注入 language.md 全文（222B）+优先规则句（715B），不注入 code-usage/code-reading 摘要，不重注项目规则全文。教学写死「只改反馈点名内容，其余逐字保留」+反面清单。绝不经过 legacy 中文标题约束链（`src/product/workspace_engine/artifact_constraints.rs:189-215` 一带）。

### D4：approve→compile→Confirmed，advance 独立幂等（用户批准）

approve 关门后启动既有确定性 compile，成功才进 durable Confirmed（不绕过阶段 2 fail-closed）；门关闭不自动触发 advance。oracle 发现的文档/源码分歧以此为准（现状 `controls.rs:35-48` 先 compile）。

### D5：advance=ready-only 幂等编排（用户批准，oracle 版）

advance 执行：前置校验（Confirmed+子 session 存在+无 active compile/revision）→ AdvanceRecord durable → group 初始化 journal → 唯一 group attempt+binding（不可变 plan_revision_id）+worktree lock+依赖拓扑 units → Ready 返回入口。**不启动 coding provider**——现有 group attempt 的 provider 由 coding WS `StartCoding` 触发，阶段 3 不另造自动启动协议；将来需要时单独定义 `auto_start_coding`。备选「advance 批量拉起全部 coding」被排除（少一次点击不值得新恢复协议）。幂等双键+plan 唯一性：同 command_id 或同 plan_id 重发都返回同一 attempt。现有 `/api/tasks/{task_id}/advance` 属旧通用 task runtime（`src/web/runtime/tasks.rs:70`），不复用不混淆。

### D6：基底 X——复用 WorkItemGroup attempt，SC session 只做只读投影（用户批准，oracle 推荐）

SC plan 对应至多一个当前 group attempt（scope=WorkItemGroup，work_item_group_id=plan_id）；每 WI 一个 unit（logical_work_item_id+work_item_revision_id+order_index+dependency IDs）；SC 的 per-WI session 为 plan 侧记录，coding 状态如需呈现只能从 group attempt/unit 派生只读投影，不成第二状态机。备选 Y（per-WI session 自驱动 coding）被排除：需复制共享 worktree/unit/review/handoff/amendment 全套，形成双权威状态源，等于 coding engine 重构。

### D7：依赖就绪门，仅限 SC-linked attempt（用户批准 1a，oracle 范围收窄）

依赖图为权威顺序来源；unit 启动前置=直接依赖全部 Completed+handoff 已发布+handoff revision 与当前 plan binding 匹配；未就绪 Pending 不跳过；全不就绪进等待态；环/未知/自依赖/handoff 不匹配 fail-closed；共享 worktree 单 active unit 不变式保留。**范围收窄**：本门仅适用于经 advance 创建的 SC-linked WorkItemGroup attempt；legacy 入口创建的 group attempt 保持既有 order_index 行为（`advance_to_next_group_unit`，group.rs:32-60）并有隔离回归测试锁死——阶段 3 不改 legacy 行为、不重构 coding engine。

### D8：入口隔离与旧协议保留（阶段 2 裁决继承）

SC plan 仅经 advance 准入 group coding；legacy 入口原样；共享初始化逻辑必须共享唯一性/binding/worktree lock/replay 四约束；flow_kind 不可变。旧 WS 协议在 REQ-WSC-07 退役门全过前不删。

### D9：决策 1a 归属指针（用户批准 1a）

依赖门收窄为仅 SC-linked attempt 的裁决内容已并入 D7（本文件）；本条仅作编号占位避免断链，无独立内容。


## Risks / Trade-offs

| 风险 | 等级 | 缓解 |
|---|---|---|
| SC 子 session 与 group attempt 绑定是新焊接面 | 高 | D6 绑定关系写进契约；SC session 只读投影 |
| 依赖门是 SC-linked attempt 的新语义面 | 高 | fail-closed 优先；legacy 路径行为隔离回归锁死（D7） |
| SC revision prompt 是新 prompt 家族（Case A/B 教训） | 中 | 独立预算+教学作用域钉死+反面清单；campaign 实跑验收 |
| advance 与旧 runtime advance 命名相邻 | 中 | 新命令走 WS typed 决策族，文档明确不复用旧 REST 面 |
| 门内多轮增加 session 状态面 | 中 | HumanGateTurn 独立记录，不动 HumanGateSnapshot 既有字段语义 |
| interactive 实跑依赖 provider 环境稳定性 | 中 | driver 多轮脚本模拟+瞬断同 turn 恢复；环境 flaky 复跑纪律沿用 |

## 审查后增补裁决（2026-08-31，reviewer Important 修复，用户批准 2a/3a）

### D10：预算耗尽语义对齐阶段 1（reviewer Important-1，用户 2a）

初稿写的「预算耗尽→强制关门进 needs_human 终态」与阶段 1 主 spec（REQ-TOP-02/06：预算耗尽时返修反馈 SHALL 被拒绝，门只留批准/终止）构成 normative 矛盾且未提交 MODIFIED delta。裁决：**删除强制关门**，对齐阶段 1——预算耗尽→拒绝新反馈（带原因），门保持开启仅接受 approve/abandon。不改写阶段 1 语义，不提 MODIFIED delta。

### D11：amendment 回合的宿主与预算（reviewer Important-2，用户 3a）

coding 阶段 plan 缺陷的 amendment 反馈回合挂在**原 SC plan session** 的对话门上（amendment 上下文重开）；预算接续该 session 快照的 `manual_repairs_remaining`（单一预算源）；HumanGateTurn 归属该 plan session；group attempt 侧不产生新预算账。

### D12：前端 UI 后置 + 接管入口兑现（reviewer Minor-6/7，用户同意）

web/ 前端 UI（对话门界面、advance 按钮）写入 Non-Goals——本 change 后端+campaign driver 端到端验收先行（沿用阶段 2 模式）。阶段 1 REQ-TOP-06 延迟项「auto stopped 接管操作入口」由本 change 兑现为 REQ-CG-06。

## 审查后增补裁决（第二轮，2026-08-31，oracle P1/P2 缺口，用户批准「1 2 3 同意 + P1 按推荐修」）

### D13：turn 预留原子事务（oracle P1-2）

session 预算扣减 + HumanGateTurn Reserved + provider 幂等键作为同一 durable reservation 在 session record 上 CAS 原子提交；恢复按 reservation 状态释放/等待/继续。这是「预算只扣一次、同 command_id 不重复启动」可证明的前提；没有它，turn 独立记录与预算扣减之间存在可观察的崩溃窗口。

### D14：in-flight turn 期间终止决定也回 gate_busy（oracle P2-1，用户拍板）

approve/abandon 与并发反馈一样受单飞约束：turn 非终态时终止决定返回 `gate_busy` 不关门，turn 进终态后才处理。避免「provider 还在改、门已关、修订结果无人接」的竞态。

### D15：Work Item 进展/成果纳入后端契约（oracle P1-5 + P2-2，用户拍板）

REQ-GCE-04 新增只读投影 requirement：每 WI 的 unit 状态/当前与最终 commit、code review 结果、handoff 结果、失败/阻塞原因、plan revision binding、group 聚合进度；从 group attempt/unit 确定性派生，非第二状态机；验收只要求既有数据面可读，UI 后置。「前端后置」不等于后端行为「如需呈现」。用户第一问（看每个 WI 进展/成果）由此落地。

### D16：admission_kind + PlanAmendmentContext + 接管端点扩展（oracle P1-1/P1-3/P1-4）

- attempt 创建时写入持久化 `admission_kind`（sc_advance/legacy_group）；依赖门与隔离测试仅以此为准，不从 flow_kind/命名推断（P1-1）。
- amendment 重开门通过 `PlanAmendmentContext` durable 关联（plan_session_id/group_attempt_id/触发 unit/finding/previous_plan_revision_id/resume_target）；原 plan session 为唯一门宿主，断线恢复由它负责；amendment approve 走「修订校验通过→新 revision→binding 更新」链，不复用首次 compile 入口（P1-3）。
- 接管不新增端点：扩展现有 `takeover_stopped_needs_human`（`src/web/handlers/workspace_session.rs`）的接管后恢复语义；child 继承 parent 快照（不可变引用或校验快照）/预算/候选引用；重复接管返回同一 child；前提重申（仅无 fatal/persistence diagnostic）（P1-4）。

### D17：advance 状态矩阵（oracle P2-2，用户拍板）

已存在 attempt 为 Initializing/Ready/Running/AwaitingPlanAmendment/Completed → 返回该 attempt 的 durable 状态；Failed/Aborted → 返回原 attempt 及失败/中止原因，不隐式建第二个；显式 restart 语义另立。

### D18：反馈与重试边界（oracle P2-5）

反馈文本与 revision prompt 受确定性 bounded-field 限制，超限在创建 turn 前零副作用拒绝；`attempt_no` 固定上限；每次真实 provider start 写 ledger，逻辑预算每回合只耗一次，不以 WS 事件推断启动次数。
