# 设计：门卡收口态 / 在途态 / 中断态

## 决策 1：D1 的作用域 = 「已收口」门卡；活门卡一字不动

- **判据**：`entry.resolved === true`（既有字段，`isResolved`）。活门卡（`resolved !== true`）渲染路径逐字不变 → REQ-WSC-11 的「结论卡**与人工门卡主区**先呈现人话选项」约束对活门卡继续成立；本 change 的 REQ-WSC-26 显式声明「已收口门卡不在 REQ-WSC-11 呈现义务内」，两条 requirement 不冲突、不互相覆盖。
- **两类已收口卡**，按 `entry.resolution` 分：
  - **决定收口**（`confirm` / `request-change` / `terminate`）：关门决定已落（owner 点击、`human_gate_closed` 帧、或会话终态派生）。去行动化全套：陈述式标题、不渲染人话选项块、不渲染 meta 行、徽章旁说明。
  - **留档收口**（`superseded`，门内轮次切换后旧卡只读留档）：标题陈述式 + 不渲染人话选项块；**保留** meta 行——既有断言（`GatePromptEntry.card-refresh.test.tsx`「已留档的旧轮卡只读历史事实」）明确要求留档卡渲染「预算历史不可用」，那是「历史事实」的一部分，不是行动号召；既有 `archive-note` + 「已留档」徽章不变。
- **为什么 meta 行在决定收口时要隐藏**：meta 行（`gate-meta`）首段是 `GATE_HUMAN_TRIGGER_LABELS` 的「…需要你来定」（`gateQuotaPresentation` 的「为什么找你」），在已终止卡上读成「还要你定」。

### 文案（逐字，写入 spec）

| 场景 | 文案 |
|---|---|
| 收口标题（triage 门） | `本轮分诊已结束` |
| 收口标题（批次确认门） | `本轮发布确认已结束` |
| 收口标题（其余） | `本轮人工确认已结束` |
| 说明：`confirm` | `已按当前版本确认，这一轮无需再操作` |
| 说明：`request-change` | `已要求修改，这一轮已结束，无需再操作` |
| 说明：`terminate`（默认） | `这一轮已终止，之前的选择未生效，无需再操作` |
| 说明：`terminate` 且有被中断修订的证据 | `这一轮修订被中断，你的选择未生效，无需再操作` |
| 说明：`superseded` | 不新增（沿用既有 `archive-note`） |

- 「已终止」徽章仍是**独立 `<span>`**、文案逐字不变（`p1-entries.test.tsx` 用 `getByText("已终止")` 精确匹配）。说明是**另一个**元素（`data-testid="gate-resolution-note"`），文案不等于「已终止」，不会让精确匹配撞多个。
- 「被中断修订的证据」= 时间线里存在 `status=aborted` 且 `summary` 含 `修订被中断，你的选择未生效` 的节点（D3 写入；durable，刷新后仍在）。证据缺席时**不**宣称「修订被中断」（不猜）。
- **不引入 `closure_source` 字段**（brief 标注「可选」）：来源差异靠上述 durable 证据表达，比新增一个仅 live 路径可得、刷新后丢失的字段更稳。

## 决策 2：D2 选 (a) 前端派生，不做后端投影

**选择**：前端从已有 wire 字段派生在途态，**不新增后端 wire 字段**。

派生选择器 `selectGateInflightPhase(state)` 返回 `"revising" | "reviewing" | null`：

```
null 若：activeRunId == null
      或 stage !== "human_confirm"
      或 flowKind !== "single_candidate"
      或 门已收口（humanGateClosure.decision 或 terminalGateClosure）
      或 存在 open/busy/awaiting_confirm 的 live humanGateTurn（既有 liveTurnStatus 路径接管）
      或 候选编译自动重写 running（有它自己的文案与锁）
活动节点（activeNodeId，缺省取最后一个 status=active 节点）：
  node_type === "author_run"   && status === "active" → "revising"
  node_type === "reviewer_run" && status === "active" → "reviewing"
  其余 → null（不猜）
```

**理由**：

1. **后端 durable `HumanGateTurnStatus` 区分不了「修订」与「复评」**：SG6b 起有 reviewer 分支时 turn 在复评期间仍是 `Running`（`sc_manual_revision.rs` 注释「有 reviewer 分支 turn 仍 Running」），单靠 turn 投影无法产出「修订 vs 复评」文案；要做对必须再投影活动节点类型，等于把 (a) 的派生搬到后端，却多出 wire + 序列化 + 4 处 `SessionState` 构造点 + 前端类型的改动面。
2. **`active_run_id` + `timeline_nodes` 已在 `session_state` 里且是服务端权威**；`timeline_node_updated` 帧实时更新节点状态，所以派生值在 run 结束后会随节点终态**自动失效**，不会因 `activeRunId` 陈旧而永久锁住门卡（选择器要求活动节点 `status==="active"`）。
3. brief 的「更稳替代」(b) 优势是跨进程重启可见；但 `active_run_id` 本身是进程内内存字段（重启即 None），重启后在途 turn 由既有恢复链（`fail_human_gate_turns_lost_provider_at_restart`）收口为 `provider_err`，不属于本 change 的刷新 / 重连场景。**此处是 (a) 的已知边界，在 spec 与报告里如实写明**。

**锁的语义**（`inflightLocked = 选择器非 null && !isResolved && 本卡是当前门卡`）：

- 渲染「正在按反馈修订…」/「正在复评…」状态行（复用既有 `GATE_REVISION_RUNNING_NOTE` / `GATE_REVISION_REVIEWING_NOTE`，与 live-turn 路径同文案同 testid `gate-revision-status`）；
- **不渲染**反馈编辑器与确认按钮（与 brief「隐藏反馈框与确认按钮」一致）；**保留**「终止此门」（owner 此前正是靠它收口，是合法出口）；
- 顶部指引改用不指向「确认当前版本」的文案（新增常量，因为既有 `GATE_NEXT_STEP_REVISION_RUNNING` 末句「期间也可直接点『确认当前版本』」在锁态下失真）；
- **已收口卡不受影响**：`!isResolved` 守卫在先，收口卡不可能被选择器重新变成可操作。

**live-turn 路径（有 `humanGateTurn` 且 open/busy）不变**：既有断言（`GatePromptEntry.test.tsx`「gives way to the revision progress line after a successful submit」在 turn open 时仍键入并提交反馈、`GatePromptEntry.guidance.test.tsx`「修订进行中」要求指引含「确认当前版本」）钉死了该路径「编辑器可见、确认可点」的行为。本 change 只补**刷新后 turn 为 null** 的缺口，不改 live-turn 路径——该不对称在报告里列为「已知差异」。

### 「门卡重新追加在作者区块之后」（brief D2 末项）

`workspace-chat-rebuild.ts:280-283` 每次重建把门卡追加到末尾，是**既有排序契约**（门卡恒在对话流末尾）。D2 的目标行为只要求「在途态可表达 + 锁」，本 change **不改**排序；观感问题由 D1（已收口卡不再像待办）与 D2（在途时明说「正在修订」）共同缓解。作为诚实边界登记。

## 决策 3：D3 复用既有三件套，不造第二套中止语义

既有链路：

1. `finish_aborted_run`（`timeline.rs`）：取消 → 节点 `Aborted` + 「运行已中止」（REQ-WIGA-16）。
2. `fail_cancelled_human_gate_revision_turn`（`provider_run.rs`）：取消臂 → turn `Failed` + `failure_class=Cancelled` + `human_gate_turn_failed{failure_class:"cancelled"}` 帧。
3. `RunAbortOutcome`（`runs.rs`）三态：取消结果如实分支。

**缺口**：`drive_work_item_plan_provider_session_to_output` 在取消时先走 `finish_aborted_run` 再返回 `Err("provider run aborted")`；`sc_manual_revision.rs` 的 `Err(message)` 分支**不区分**这个哨兵，一律 `fail_human_gate_turn(ProviderErr)` + 帧 `provider_err`；`close_revision_author_node_after_turn_failure` 又无条件把节点覆写成 `Failed` + 「人工修订失败，本轮反馈未采纳」。结果：已 `Aborted` 的节点被改写成 `Failed`，对话流零提示。

**修法（只补分流，不加状态）**：

- 抽出哨兵常量 `PROVIDER_RUN_ABORTED_MESSAGE = "provider run aborted"`（两处 `return Err(...)` 与分类共用，避免字面量漂移）。
- 新增纯函数 `revision_drive_error_outcome(message) -> RevisionDriveErrorOutcome { failure_class, label, frame_message }`：消息等于哨兵 → `Cancelled` / `"cancelled"` / 中断文案；否则 → `ProviderErr` / `"provider_err"` / 原消息。真实错误路径（含 idle watchdog、choice 等待超时，它们的消息不是哨兵）**逐字不变**。
- `close_revision_author_node_after_turn_failure(failure_class)`：`Cancelled` → 节点落 `Aborted` + summary `运行已中止：修订被中断，你的选择未生效`（含 REQ-WIGA-16 要求的「已中止」表述 + 新增的明说句）；其它失败类 → 逐字不变（`Failed` + 「人工修订失败，本轮反馈未采纳」）。
- `fail_cancelled_human_gate_revision_turn`（`cancel_settle` 兜底路径）经同一 `fail_human_gate_turn(Cancelled)` 自动得到同样的节点收口；其帧文案与新句统一。

**对话流明说**：`buildChatEntries` 对「`status=aborted` 且 summary 含 `修订被中断，你的选择未生效`」的节点追加一条 `stage_change` 系统条目（紧随该节点已有条目之后），内容为该句。durable（来自节点 summary），刷新 / 重连后仍在。

**不动**：`RunAbortOutcome`、`abort_active_run_and_wait_completion` 的 30s 收口窗（现场 06:11:30 会话 `terminated` 的时序），后者属另一层问题，本 change 不扩面。

### 已知副作用（如实登记）

`recover_human_gate_turns` 会对 `Failed(Cancelled)` 且持有活动 `user_hold` 的 turn 做 `NonstructuralFailureCharge` 对账（`conversational_gate_recovery.rs`）。D3 之后，因终止导致的修订失败从 `ProviderErr` 改归 `Cancelled`，其预算对账路径随之改走既有的「取消」车道。这是复用既有语义的直接后果，不是新增语义；在报告 concerns 登记。

## 决策 4：对既有测试的调整（须逐条说明）

`src/web/workspace_ws_handler/tests/human_gate_revision_cancel.rs` 中 `cancelled_revision_turn_fails_durably_and_echoes_human_gate_turn_failed` 断言「取消同样收口修订 author 载体节点」为 `TimelineNodeStatus::Failed`。REQ-WIGA-16 明文「被中止的 run，其节点 `status` SHALL NOT 为 `failed`」，该断言恰好钉死了本缺陷 → 改为 `Aborted` 并追加 summary 断言。这是对一条**与已生效 requirement 相抵触的旧断言**的订正，不是削弱。

## 风险与回滚

- 前端改动纯渲染层 + 一个只读选择器；回滚 = revert。
- 后端改动局限在失败分类与节点收口，不碰 turn 状态机、不碰预算结算。
- `conversational_gate.rs` 已 1157 行（上限 1200），新增逻辑放入新文件 `conversational_gate_cancel.rs`，主文件净增 ≤ 15 行。
