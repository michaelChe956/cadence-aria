# Design: fix-local-delivery-and-chain-truth

## Context

动机与范围见 `proposal.md`，行为合同见 `specs/` 五份 delta。本设计转录已由 owner 裁定、经 DesignPushLocal2 终稿确认的方案，并以当前工作区源码核对落点；不新增能力，也不重新选择方案。接地时间 2026-10-14，工作区含并行实施者的未提交改动，下表区分「已在途」与「尚未动」。

### 已接地的落点

| 区域 | 符号与位置（相对 worktree，行号以符号为准） | 现状 |
|---|---|---|
| 推送状态模型 | `src/product/coding_models/review.rs`：`PushStatus`、`is_delivered()` | 在途：已含 `SkippedNotConfigured` 与 `is_delivered()`（`Pushed \| SkippedNotConfigured`） |
| 远端是否配置 | `src/product/git_workspace_service.rs`：`git_remote_configured` | 在途：用 `git remote` 名单判定，不用 `get-url`/`ls-remote` 退出码 |
| 评审请求推送决策 | `src/product/coding_workspace_engine/internal_pr_review.rs`：`execute_review_request`、timeline 节点收口 | 在途：未配置远端时置于 `remote_head`/`git_push` 之前走 `SkippedNotConfigured`；节点标 Completed |
| ReviewRequest 提示 | `src/product/coding_workspace_engine/git_operation.rs`：`manual_instructions` | 在途：本地交付写「未配置远端，未推送」，不再写「基于远端」 |
| 已交付聚合 | `src/product/coding_attempt_store/plan_group_projection/mod.rs`（`delivered`）、`issue_delivery.rs`（`AllPushed`） | 在途：两处都改为 `is_delivered`；`maybe_complete_issue_delivery`（`handoffs.rs`）经 `AllPushed` 受益 |
| 重开推送 | `src/product/coding_attempt_store/git_operation.rs`：`reopen_failed_review_git_operation` | 只接受 `PushStatus::Failed`，**不改**，本地交付不可重推 |
| 推送状态文本 | `src/web/handlers/dto.rs`：`push_status_text` | 在途：新增 `skipped_not_configured` |
| 指针发布 | `src/product/logical_codebase/pointer_publication_coordinator.rs`：`git_push` 后只认 `PushStatus::Pushed` | **不动**，见 Non-Goals |
| plan 聚合进度 | `src/web/handlers/coding/plan_start.rs`：`build_plan_coding_progress`；`src/web/types.rs`：`PlanCodingTargetDto`、`PlanCodingUnitProgressDto` | 在途：九字段与 attempt 状态守卫已落工作区 |
| runner 死亡转 AMR | `src/web/coding_ws_handler/runner/task.rs`（`coding_runner_failed_while_running`）；`src/product/coding_attempt_store/role_run.rs`：`fail_running_role_runs_for_attempt`、`supersede_residual_running_role_runs_for_attempt`；`src/web/coding_ws_handler/socket.rs`（RecoverCoding 分支） | 在途：死亡收尾与恢复清场已接线 |
| prompt 声明 | `src/product/coding_workspace_engine/prompts.rs`：`platform_directives_section`；追加点 `coding.rs`、`rework.rs`、`provider_retry.rs` | 在途：已在各路径追加；`worktree_discipline_section` 仍仅 LC 注入，不承载本声明 |
| 前端类型与标签 | `web/src/api/types/coding.ts`；`web/src/state/coding-execution-labels.ts`（新）；`PlanGroupProjectionPanel.tsx`、`DeliveryStatusPanel.tsx`、`CodingWorkspaceReports.tsx` | 在途：类型联合、统一标签表、交付判定 |
| 逐仓面板、仪表盘覆盖、AMR 恢复按钮 | `web/src/pages/CodingWorkspacePage.tsx`（header 区）、`web/src/pages/plan-coding-progress.ts`、`web/src/state/coding-dashboard-projection.ts`、`web/src/hooks/useCodingWorkspaceWs.ts` | **尚未动**；`recover_coding` 目前只有消息类型（`coding.ts`），无发送路径 |
| 平台摘要载体 | `src/product/coding_attempt_store/context.rs`：`append_manual_recovery_diagnostic`（SystemEvent 尾帧先例）；`git_workspace_service.rs`：`git_commit_range_changed_files` | **尚未动**，可选包 |

已核实：`gates.rs`、`group_final_readiness.rs`、`group_terminal.rs` 均不读取推送状态，所以 FinalConfirm 链路对本地交付天然放行；真正读取「已推送」的只有 `issue_delivery.rs` 与 `plan_group_projection/mod.rs` 两处，其余口径经它们间接传递。

## Goals / Non-Goals

**Goals：**
- 无远端仓库的交付成为合法终态，且不影响有远端仓库的任何既有行为。
- 页面对链路状态的呈现与 durable 事实一致：AMR 不显示执行中，role run 不永久运行中，多仓交付逐仓可见。
- 为平台与前端冻结一份可并行实施的数据契约。
- Coder 汇报与提交责任由平台声明约束；平台摘要提供不可被叙事改写的交付事实。

**Non-Goals：**
- proposal 所列非目标全部适用。设计层补充：不为展示便利给 unit 或 attempt 增加持久化字段；不新增 HTTP 端点（扩展既有 plan 聚合进度端点）；不引入第二套标签表。

## Decisions

### D1. 「未配置」只看远端名单，且判定前置

**决策**：以 `git remote` 名单是否含目标远端名判定「未配置」。该判定位于评审请求阶段 `phase==PushStarted` 之后、任何远端探测与推送之前。

**理由**：DesignPushLocal2 在无 origin 的临时仓库实测（该次会话报告）：`git remote get-url origin` 返回 2，`git ls-remote origin` 与 `git push origin` 返回 128。退出码无法区分「未配置」与「已配置但不可达」，会把真失败伪装成跳过，因此不用。前置位置使两种进入方式同样收敛：全新进入，以及恢复一个停在 `PushStarted` 的 journal——后者正是两个 AMR 现场之一的形态，该形态下不前置则恢复后仍会先探测远端头并再次死亡。

**否决的替代**：①把推送失败一律降级为本地交付——会掩盖鉴权与网络故障；②在 runner 外层吞掉探测错误——不改变 journal 停在中途的事实，且与「Indeterminate 停等」的既有语义冲突。

**红线**：远端已配置时行为字节级不变。回归锚的 fixture 均配置 origin，走原路径。

### D2. 本地交付是终态，不可重推

`skipped_not_configured` 的 ReviewRequest 不进入 `reopen_failed_review_git_operation`（仅接受 `Failed`），补配远端后不自动补推。**理由**：自动补推需要决定推哪个远端、是否覆盖、失败如何呈现，属独立能力；owner 已把它划为非目标。**代价**：补配远端的用户需要另走手工推送，界面文案应说明本地交付的含义（「本地交付(无远端)」）。

### D3. 已交付谓词单点定义

`PushStatus::is_delivered()` 是唯一谓词，`issue_delivery.rs` 与 `plan_group_projection/mod.rs` 必须调用它；前端 `PlanGroupProjectionPanel` 的行级 delivered 同口径。Issue 级聚合取值 `all_pushed` 名称不改——它是 REST 与前端既有取值，改名会破坏旧客户端；语义在 spec 中显式扩为「全部已交付」。

**否决的替代**：新增 `all_delivered` 取值——需要 DTO、前端联合类型与旧客户端回落三处同时迁移，收益只是名称。

### D4. 投影真实性：只在读取层屏蔽

`provider_started` 增加 `attempt.status == Running` 合取；新增四个 `execution_state` 取值，优先级为：`completed` > `invalidated` > `awaiting_manual_recovery` > `failed` > `aborted` > `waiting_human` > `waiting_dependency` > `running` > `paused` > `initialized` > `waiting_slot`（尾部沿既有顺序）。

**理由**：AMR 下 unit 保持 Running 是恢复「原位继续」的前提（`recover_attempt_from_manual_recovery` 不改 stage、unit、`active_unit_id`），改写底层事实会破坏恢复。因此真实性只能靠投影层守卫。

**后果（需要同步修改的既有断言）**：POST 启动回执时刻 attempt 仍为 Created（启动 claim 已推进但 admission 尚未跨过），旧断言 `execution_state=="running"` 在该时刻不再成立，应回落 `initialized`——这恰是 REQ-CS-04「已接受首启不等于 provider 实际进度」的字面要求。

**否决的替代**：①在 `start_claim` 上做相位降级——写 durable 事实；②前端自行用 `attempt_status` 修正——违反「后端是唯一事实源」，且各面板口径会漂移。

### D5. 仪表盘：AMR 与 Failed 画成失败，覆盖该 attempt 下全部未完成 unit

仅在聚合绑定 ≥2 target-attempt 时，`selectCodingDashboardUnits` 路径按 unit 所属 attempt 状态覆盖六态；单 target 与 v1 单 attempt 路径零变化。 [ASSUMPTION：「覆盖全部未完成 unit」取自 owner 的「AMR 显示失败」裁定与「按 unit 所属 attempt 状态覆盖」的设计表述；Pending unit 在 AMR 下无法推进，标「待开始」会误导。若 owner 只要求标记当前 active unit，改为仅覆盖 active unit 即可，不影响其它契约。]

### D6. 逐仓面板数据源：扩展既有端点，不另拉投影

**决策**：扩展 `PlanCodingTargetDto`（页面已在轮询的 plan 聚合进度端点），而非另读 `group_projection`。**理由**：前者天然带 `worktree_path`、`lease_state`，且免一次请求；后者没有 worktree。**代价**：两处各自派生交付字段，必须以 D3 的共享谓词与 `push_status_text` 保证口径一致。

### D7. 冻结的 S3→FE 数据契约（九字段）

`target_attempts[]` 每项在既有五字段之外增加（snake_case，类型均 `string | null`）：

```json
{
  "target_repository_id": "…", "attempt_id": "…", "attempt_status": "awaiting_manual_recovery",
  "worktree_path": "…", "lease_state": "held",
  "repository_name": "busi",
  "branch_name": "aria/…", "base_branch": "main",
  "stage": "review_request",
  "head_commit": "20b298be…",
  "push_status": "skipped_not_configured",
  "push_error": null,
  "review_request_id": "…",
  "manual_recovery_reason": "coding_runner_failed_while_running"
}
```

字段来源：`repository_name` 取 checkout 路径末段（回落目标仓标识，与 `resolve_plan_target_repository_name` 同口径，经 issue 归属 codebase 解析）；`branch_name`/`base_branch`/`stage`/`head_commit`/`manual_recovery_reason` 取最新绑定 attempt，`stage` 经 `coding_execution_stage_text`；`push_status`/`push_error`/`review_request_id` 取该 attempt 最新 ReviewRequest，`push_status` 经 `push_status_text`。成员 attempt 已删时 attempt 派生项为 null，`attempt_status` 沿既有「已中止」投影。

`units[]`：`execution_state` 值域 additive 增 `awaiting_manual_recovery | waiting_human | failed | aborted`；`waiting_reason` 在 AMR 时透传 `manual_recovery_reason`，故前端类型由三值联合放宽为 `string | null`（三值保留为「编排等待」子集）。前端统一标签表 `ATTEMPT_STATUS_LABELS`、`STAGE_LABELS`、`PUSH_STATUS_LABELS` 唯一来自 `web/src/state/coding-execution-labels.ts`，未知值回落原文。

**契约冻结的含义**：实现两端各自以此为准，不得再商议字段名与类型；新增字段需走本 change 的修订。

### D8. role run 收尾：死亡时 Failed，恢复时 Superseded，两处各一

runner 死亡转 AMR 时同批把 Running role run 记 Failed 并带原因码；显式恢复时再把残留 Running 记 Superseded（先例：`recovery.rs` 的 stale role run 收尾）。恢复侧清场的理由是：修复前已遗留脏数据的现场（两个 AMR attempt）无法由死亡路径回溯修复。收尾失败只告警不吞没，也不阻止转 AMR——转入人工恢复是安全网，不能被收尾的副作用拖垮。**不动** unit、`start_claim`、`active_unit_id`。

### D9. AMR 恢复动作：前端补发送路径，后端不新增

后端 WS 通路已完整：`RecoverCoding` 是 AMR 的唯一合法放行消息，处理链为恢复 worktree 锁、`recover_attempt_from_manual_recovery`（CAS 回 Running 并清原因）、再 `spawn_coding_runner`；非 AMR 状态重发得到 `attempt_not_awaiting_manual_recovery`，无副作用。缺口只在前端：`recover_coding` 仅有消息类型，没有发送与按钮。因此 FE-2 补发送与按钮，并展示人工恢复原因。没有 HTTP 恢复入口，也不新增。

### D10. prompt 声明：最终组装后追加，不触碰投影

`commit_responsibility` 是 Work Item 投影的一部分，改动会牵动 `renderer_version` 与执行上下文哈希，让在途 run 校验失败。因此新增 `platform_directives_section()`，在各 prompt 路径最终组装后、哈希计算后追加；一段承载提交优先声明（REQ-CWC-04）与汇报契约（REQ-CWC-05）。与 `worktree_discipline_section` 的区别：后者仅 LC attempt 注入，本段对全部 attempt 无条件注入。已知追加点：`coding.rs`（首次与增量）、`rework.rs`（返修与增量）、`provider_retry.rs`；新增 prompt 路径必须同样追加，由 spec 的「各路径一致」场景守护。

### D11. 平台确定性摘要（可选包）

载体沿用 `append_manual_recovery_diagnostic` 先例：SystemEvent 尾帧、独立 id 号段（不占 `coding_chat_entry_NNNN`，避免被同号备注覆盖）。变更文件清单用既有 `git_commit_range_changed_files` 读取 `start_commit..completion_commit`；验证证据取已记录的运行证据，缺失显示「未记录」。生成失败只告警、不阻断完成。前端展示载体（SystemEvent 在对话流的渲染形态）需先确认，故该包排在末位，且不阻塞解堵。

## Risks / Trade-offs

- **[旧二进制无法读取本地交付记录]** 写入 `skipped_not_configured` 的 ReviewRequest 与 journal，旧二进制反序列化失败 → 按只进不退部署；回滚前须确认无该状态记录，proposal 已声明。
- **[exhaustive match 遗漏]** 新增枚举变体会让所有穷尽匹配编译失败 → 以编译错误定位；已知 `dto.rs` 的 `push_status_text`，指针发布仅用 `==` 比较，无编译影响。
- **[无远端时指针发布仍失败]** 指针发布只认 `pushed`，无 origin 时会走失败记录 → 已声明为非目标；是否影响 issue 完成需 Main 现场确认（未核）。
- **[远端名写死 `origin`]** 多处调用点以 `"origin"` 为交付远端名，未配置按同名判断 → 沿用，不改；若仓库只配置了别名远端，会被判为未配置，属已知边界。
- **[两处派生口径漂移]** `group_projection` 与 `target_attempts[]` 各自派生交付字段 → 共享 `is_delivered` 与 `push_status_text`，并以一致性测试守护。
- **[在途改动与契约偏离]** 实施先于本契约启动 → 复核时以契约为准，偏差回改实现。
- **[跨 attempt handoff 前置]** 现场 WI-002 的死因是 `authoritative_resolved_handoff_revision_ids` 只在本 attempt 内解析依赖，这是对 REQ-GCE-01/REQ-CS-03 的缺陷修复，不属本 change 但是恢复演练的前置。若它未先合入，恢复 WI-002 会再次死亡并回到 AMR，无害但多写一条诊断。

## Migration Plan

按风险分三层合入，顺序与可并行性见 `tasks.md`。部署后现场恢复（假定跨 attempt handoff 缺陷修复与本 change 实现均已部署并重启服务）：

0. **只读预检（部署前）**：确认 WI-001 journal 停在 `PushStarted`；WI-001 unit run 已 Completed 且提交一致；WI-002 的 unit 为 Running、`active_unit_id` 对齐、无遗留 Running 占位 run；两 attempt 的 lease 状态。任一不符，下面步骤不适用，回到分析。
1. **先恢复 WI-002**（stage=coding）：发送 RecoverCoding；runner 回到 coding，经修复后的跨 attempt 解析创建 run 并启动 provider。
2. **再恢复 WI-001**（stage=review_request）：发送 RecoverCoding；重入评审请求阶段，探测到远端未配置，收敛为本地交付，写 ReviewRequest，进入 FinalConfirm（等待人工最终确认）。
3. 两步互不依赖，顺序可换；WI-002 的依赖只看 unit 级 handoff，不看 WI-001 是否推送。
4. 启动 reconcile 与 attach 自动恢复均不处理 AMR，所以部署重启不会提前放行；此条由 REQ-CRO-08 的「无自动放行」场景守护，演练时实际观察确认。
5. **退路**：若修复后恢复仍失败，退路为 abort 加 restart（REQ-CRO-03）。这是破坏性操作，必须 owner 明确同意后才执行。

## Open Questions

- 平台摘要的前端展示载体（SystemEvent 在对话流的渲染形态）是否沿用现有 `execution_event` 分类，需在 REQ-CWC-06 的实施包开始前确认，不影响其它包。
- 指针发布在无远端时的行为是否阻塞 issue 完成，需现场核实；不影响本 change 契约，只决定是否另立项。
