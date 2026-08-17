# Design: spec-design-dialog-revision

## 1. 目标状态机（Story/Design workspace）

```
PrepareContext → Running(AuthorRun)
     └→ AuthorConfirm ─────────────────────────────────────┐
          ├─ Revise{feedback} → Revision → Running → AuthorConfirm（循环）
          ├─ AcceptWithReview → CrossReview → AuthorConfirm（报告进对话流）
          └─ AcceptFinalize → Completed
```

- `WorkspaceStage` 枚举不变；HumanConfirm 与 ReviewDecision 不再出现在 Story/Design 流转中（枚举保留，WorkItem/WorkItemPlan 类型不受影响）。
- review 结果不驱动状态机：`ReviewGate::RequiresRevision` 等路由结果在 Story/Design 类型下统一改为"格式化报告消息 → 回 AuthorConfirm"。

## 2. 协议变更（WsInMessage / AuthorDecision）

- `AuthorDecision` 变体调整为：
  - `Revise { feedback: String }`（新增）
  - `AcceptWithReview`（新增，替代 Accept 的送审语义）
  - `AcceptFinalize`（新增，定稿语义）
  - `Accept` / `Reject` 保留反序列化兼容：`Accept` 路由——新记录按 `reviewer_enabled_at_start`（创建默认值）等价对应新变体（已送审后再次 Accept 仍按创建默认值，即未启用→定稿）；旧记录（None）按有效态（现状行为）；`Reject` 在 Story/Design 由 engine 层 `handle_author_decision` 返回引导性错误（协议守卫仅校验消息类型×阶段，不感知 workspace_type）。
- 阶段守卫（`workspace_ws_handler/protocol.rs`，AuthorConfirm 守卫为 :52-63，HumanConfirm 守卫为 :74-83）：AuthorConfirm 阶段仅放行 `author_decision`（新变体，反馈统一走 `Revise{feedback: String}` 单通道，不引入其他反馈消息）；ReviewDecision 阶段守卫对 Story/Design 不再可达。

## 3. 引擎实现（workspace_engine）

- `handle_author_decision`（decisions.rs）新增分支（追加 match 臂，不重写既有结构）：
  - `Revise{feedback}`：非空校验 → `pending_revision_context = feedback` → `transition_stage(Revision)` → 启动 author 修订 run → 完成后 `enter_author_confirm`（summary 带改动摘要）
  - `AcceptWithReview`：进入 CrossReview（复用现有 ReviewerRun 通道）；若 `reviewer_provider.is_none()`（创建时未启用 review），从保留的创建快照 reviewer 选择恢复（见下）执行一轮评审；快照中无 reviewer 选择时返回引导性错误，阶段不变
  - `AcceptFinalize`：复用现有定稿逻辑（`mark_latest_artifact_confirmed` → Completed）
- **临时送审的 reviewer 来源与持久化闭环（provisional 恢复）**：
  - 创建时（`start_generation`）无论 `reviewer_enabled` 与否，将快照原始 reviewer 选择与默认值语义落盘到两个新字段：`provisional_reviewer_provider: Option<ProviderName>`（快照 reviewer 原始值）与 `reviewer_enabled_at_start: Option<bool>`（创建时 review 默认值；None 表示旧记录，无法追溯）；`reviewer_enabled=false` 时仍清空 `reviewer_provider`/`review_rounds`（默认行为不变）。
  - 落盘四处：`models/workspace.rs` `WorkspaceSessionRecord` 加两字段（`#[serde(default)]`）、`types.rs` `WorkspaceSession` 加两字段并 `from_record` 恢复、`lifecycle.rs` `start_generation` 写入并扩展落盘调用——否则 WebSocket 重连（`socket.rs:67,113,118` 走 `from_record` 重建 session）后 provisional 丢失。
  - **判定依据（不能用 `reviewer_provider.is_none()`）**：重连后 `from_record` 恒 `Some(record.reviewer_provider)`（record 字段非 Option，`types.rs:110`），且 `reviewer_enabled=false` 时落盘 reviewer fallback 为 author（`lifecycle.rs:649-652`）、`review_rounds` 保持创建时值 ≥1——`is_none()` 重连后恒 false，provisional 分支永不触发。改用：`reviewer_enabled_at_start == Some(false)` 判定"未启用"，`provisional_reviewer_provider.is_some()` 判定可恢复；`AcceptWithReview` 命中两者时恢复 provisional 并置 `review_rounds=1` 执行评审，provisional 为 None 则返回引导性错误。旧记录（`reviewer_enabled_at_start=None`）按有效态回退（与现状一致）。
  - 项目级 `ProviderDefaults` 仅为出站展示类型（out.rs:59），不作为来源。
- **⚠ provisional 前置依赖（前端配合修改）**：当前前端 `providerConfigFor`（`web/src/pages/ChatWorkspacePageParts.tsx:346-348`）在 `reviewer_enabled=false` 时将快照 reviewer 置 `null` 发送，provisional 会空转；需同步改为未启用时仍携带用户已选 reviewer（后端仍按现状清空，仅 provisional 保留），且 `review_rounds` 与 `reviewerEnabled` 解耦——未启用时仍传 0。旧前端版本创建的存量会话无 provisional，临时送审走引导性错误分支，行为自洽。
- `review/routing.rs`：Story/Design 类型下 `RequiresRevision` 等所有路由终点改为回 AuthorConfirm，reviewer verdict 经 `format_review_feedback` 作为消息推送。
- 修订 run 复用现有 Revision → provider run → ArtifactPane 版本机制；每轮新版本，历史保留。

## 4. Prompt 设计

- **新增独立函数** `build_author_revision_prompt`（不修改 `build_revision_full_prompt` / `build_revision_delta_prompt`，规避与 add-monorepo 分支的文本冲突）：输入为当前产物 markdown + 用户自由文本反馈，要求输出修订版并附「改动摘要」小节（改了哪几处、为什么）。
- reviewer prompt 不变；review 报告经现有 `format_review_feedback` 渲染进对话流。

## 5. 前端（web）

- `ChatInputBar`（author_confirm 阶段）：输入框启用（placeholder："输入修改意见，或直接确认"）；三动作：
  - 「发送反馈」（输入非空时可用）→ `Revise`
  - 「确认并送审」→ `AcceptWithReview`
  - 「确认定稿」→ `AcceptFinalize`
  - 按 `reviewer_enabled` 配置默认高亮送审或定稿按钮，两者始终可点
- 移除「重新编写」按钮；review 报告与改动摘要以对话消息渲染（复用 ReviewVerdictEntry 模式，改动摘要新增 entry 类型）。

## 6. 存量会话恢复

- 存量迁移落点：`WorkspaceEngine::new_persistent` 的 fallback 链（`lifecycle.rs:282-300`，与既有 `recover_complete_artifact_misclassified_as_text_fallback` 等并列），而非 `interrupted_run_recovery.rs`（其入口仅覆盖 stage==PrepareContext 的断线 run 恢复）；检测到 Story/Design 会话 stage 为 HumanConfirm 或 ReviewDecision 时，迁移到 AuthorConfirm（保留 artifact、消息历史；若有 review verdict 则一并注入对话流），不丢弃任何数据。
- 修订 run（Revision 阶段）断线重连（**扩展现有恢复机制，非复用**）：现有 `interrupted_run_recovery.rs` 仅支持 Review / WorkItemDraftGeneration 两臂（`:4-7,82-121`），需新增 Revision 恢复臂：`RecoverableInterruptedOperation::Revision` 变体 + 失败修订 AuthorRun 节点检测 + `retry_interrupted_run` 新臂 + `decisions/inbound.rs` `provider_run_kind_for_interrupted_recovery` 映射到既有 `ProviderRunKind::Revision`。“完成后重连”路径无需新机制（`provider_drive.rs:745-791` 完成路径已 update_artifact + enter_author_confirm）。

## 7. 与 add-monorepo 分支的冲突规避

- 重叠文件四处：`types.rs`（+12 行）、`lifecycle.rs`（+75 行）、`prompts/revision.rs`（+171/-6，主体为 RoutingReferenceContext 参数 threading + 测试）、**`web/workspace_ws_handler/run/provider_run.rs`**（monorepo 新增 WorkItemPlanOutlineRebuild 臂、ReviewOnly gateway 分支及 `resolve_plan_author_launch` 参数贯穿）——最后一处恰与本 change tasks 2.5 的修订 run 接线同文件。
- 规避策略：新增逻辑全部走新函数/新文件（`build_author_revision_prompt`、新测试文件 `tests/author_revision_loop.rs` 等）；decisions.rs 只追加 match 分支；provider_run.rs 的修订 run 接线走**新增 match 臂与独立函数**，不触碰 monorepo 已改的 `match run_kind` 既有臂与 ReviewOnly 分支；本 change 先合入 main，monorepo 分支后 rebase。

## 8. 测试策略

- 引擎单测（新文件）：修订循环（反馈→修订→回 AuthorConfirm）、空反馈拒绝、Reject 引导性错误、AcceptWithReview/AcceptFinalize 路由、review 后回 AuthorConfirm、reviewer pass 不自动定稿、provisional reviewer 恢复与无快照送审报错、修订 run 断线重连两例。
- WS 协议测试：新变体 roundtrip、阶段守卫、旧变体（Accept/Reject）兼容行为。
- 前端测试：ChatInputBar 三动作与默认高亮、修订摘要/报告消息渲染。
- 既有测试迁移：`part_06.rs` 等固化 "Accept→CrossReview→ReviewDecision→HumanConfirm" 的用例按新状态机调整（仅改预期，不删覆盖）。

## 9. 决策记录

| 决策点 | 选择 | 备选与理由 |
|---|---|---|
| review 开关粒度 | 配置=默认值，确认时可临时偏离 | A（能力开关）锁死无自由度；B（废弃配置）破坏老用户习惯 |
| 反馈形态 | 纯自由文本 | 结构化标签无消费方（YAGNI）；分区结构化负担大 |
| Reject 去留 | 移除出口，协议保留兼容 | 重写意图可由反馈表达；推倒重来丢历史与方向矛盾 |
| 收尾结构 | 单确认点，HumanConfirm/ReviewDecision 退役 | 两段式冗余；review 自动定稿违背"agent 不自批" |
| 范围 | 仅交互链，daemon 链不动 | daemon 链为 non-interactive 定位，自动循环是合理兜底 |
| 版本可视化 | author 输出改动摘要 | 逐行 diff 后续按需（YAGNI） |
| 临时送审 reviewer 来源 | 创建快照 provisional 恢复；无则报错 | ProviderDefaults 仅为展示类型未落 session；硬编码回退不可达（decisions.rs:663）；会话内改 provider 被锁定（controls.rs:181-184） |
