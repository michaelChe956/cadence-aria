# 人工门三个额度的真实代码结构（v1.0）

- 目标 worktree：`/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`
- 调查日期：2026-11-03
- 范围：只读核实；未修改实现代码、未 git add。
- **总裁决：** `max_repairs` 是每评审周期机械/政策返修配额；`max_manual_repairs` 是 author 重写与人工反馈重写应共用的 3 次预算口径；平台自动重写账本字段为 `CandidateCompileStatus.auto_rewrites_used`，人工反馈账本为 `HumanGateSnapshot.manual_repairs_remaining`，人工说明回合观测为 `accepted_feedback_turns`。但当前两种写入没有同步：**REQ-WSC-21 声称共享预算，代码实际上是两处不相交账本；这属于契约实现缺口，不只是文案问题。**

## Q1｜三个额度的真身

| 业务名称 | 字段 / 原文 | 谁扣 / 写入点 | 扣至何处 | 默认值 |
|---|---|---|---|---|
| ① 平台自动重写（编译失败后 author 自动重写） | `src/product/models/human_gate.rs:263-268`：`/// 已用掉的自动重写次数（与 CandidateAutoRewrite.remaining 同一账本）。` `compile_failure_surface.rs:133-135`：自动重写上限取 `RunBudgets::default().max_manual_repairs`。 | `src/product/workspace_engine/compile_failure_surface.rs:301-325`：只在 `CandidateCompileRoute::AutoRewrite` 时 `used.saturating_add(1)`，Escalate 保持不变；`cleared_candidate_compile_status:363-366` 成功编译清 0。存储于 candidate compile status，不更新 run history。 | 上限 `max_manual_repairs`；`remaining = max_auto_rewrites - auto_rewrites_used`（`compile_failure_surface.rs:153-175`），同结构环重复可提前停。 | `max_manual_repairs=3`（`types.rs:224-230`）。该上限和人工池相同，但计数账本不共享。 |
| ② 人工返修（owner 提交反馈后的一次修订 turn） | `types.rs:86-93`：`HumanGateSnapshot.manual_repairs_remaining`。 | `src/product/lifecycle_store/human_gate.rs:181-185` 为 0 拒绝；`:195` `snapshot.manual_repairs_remaining -= 1`；`:199-200` 在同一 CAS 内给已知 `accepted_feedback_turns` 加 1。CAS 说明 `:203-205`。 | 剩余至 0 即拒绝新 turn；新快照从 `max_manual_repairs - manual_repairs_used` 算出（`policy_routing.rs:100-107`），同 logical gate 恢复则接续快照 (`:116-130`)。 | `max_manual_repairs=3`；可用余额按运行历史扣减。 |
| ③ 人工说明回合（接受为修订 turn 的 owner 反馈数） | `types.rs:88-93`：注释定义“本 logical gate 内被接受为修订 turn 的反馈轮次”，字段为 `Option<u32>`。 | 同一 CAS：`human_gate.rs:196-200`，被接受/预留一次反馈才对 `Some(turns)` 饱和 +1；它不代表 provider 已成功启动，也不是返修配额。 | `u32::MAX` 饱和；不是触发自动返修或拒绝修订的 gate。 | 新逻辑门 `Some(0)`（`policy_routing.rs:110-112`）；旧 JSON 缺失保持 `None`，不补写（`types.rs:90-93`）。 |

**Q1 一句话结论：owner 看到的“自动修复次数已用完”并非 `max_repairs` 的自动重写定义（`max_repairs` 是每评审周期机械返修池）；真正的平台自动重写字段是 `auto_rewrites_used` / `auto_rewrite.remaining`，而“还可自动修复 N 次”当前 gate UI 显示的是 `manual_repairs_remaining`。**

### `max_repairs` 到底是谁在用？

- `types.rs:171-180`：`ReviewCycleState.repairs_used` 明确是每个被审 artifact 的 automatic repair consumption；`types.rs:192-199` 的 `RunHistory.repairs_used` 是 session 计数字段，注释 `:195-196` 说明 session totals 仅观测、预算执法 exclusively 用 `review_cycles`。
- `evaluate.rs:355-375`：取当前 cycle 的 `cycle.repairs_used`；只有 `phase == Initial && cycle_repairs_used < max_repairs` 才返回 `Repairable`，否则将可返修 finding 路由为 `HumanReason::RepairBudgetExhausted`。
- `evaluate.rs:465-472`：分别校验 `manual_repairs_used <= max_manual_repairs`，以及每个周期 `cycle.repairs_used <= max_repairs`。
- 生产消费点是 `workspace_engine/review/routing.rs:623-638`：`TriggerAggregateRepair` 分支用 `repairs_used_delta: 1` 与当前周期 `ReviewCycleState { repairs_used: 1 }` 合并。这个是 policy repair/cycle 账，不是 compile failure candidate 的 `auto_rewrites_used`。
- `evaluate.rs:98-114`：`RunHistoryDelta` 文档说明 evaluator 不扣修复；durable route handler 仅在相关协议成功后增加。

## Q2｜两处账本是否真共用额度池

### 契约原文

- `openspec/specs/work-item-plan-single-candidate/spec.md:820`：预算 MUST 是 author 重写预算，初值 `max_manual_repairs`、门卡显示同一口径；平台自动重写和 owner feedback 重写“扣同一个池”，每轮消耗 1；`max_repairs` 机械返修池 MUST NOT 作为自动重写预算。
- `compile_failure_surface.rs:123-135` 注释同样说“这是同一个预算池”，两类是平台自动重写与 owner 反馈触发重写；`max_repairs` 默认 1 是“不重跑 author 的机械 repair”池。

### 实际实现：同一上限，不是同一账本

- `types.rs:224-230` 默认：`max_repairs: 1`、`max_transitions: 12`、`max_manual_repairs: 3`。所以机械返修池 1 与 author 重写池 3 初值明确不同；平台重写上限和人工返修上限则共用 `max_manual_repairs=3` 这个**配置口径**。
- 平台消耗：`compile_failure_surface.rs:301-325` 将 `previous.auto_rewrites_used` 单独累加到新的 candidate compile status；`models/human_gate.rs:263-268` 将它定义为与 `CandidateAutoRewrite.remaining` 同一账本。
- 人工消耗：`review/policy_routing.rs:100-107` 从 `max_manual_repairs - history.manual_repairs_used` 构造 gate 余额；同文件 `:116-130` 已存在快照时以其余额和反馈 turn 计数 carry-forward；`human_gate.rs:195-200` 接受反馈时只扣 snapshot 余额/加反馈数。
- **同步点核查：没有。** 对目标 `src/product` 搜索所有 `auto_rewrites_used`、`manual_repairs_used` 写入引用：生产候选字段写入是 `compile_failure_surface.rs:313-325` 自增、`:365` 成功清零；生产人工历史计数只在 `RunHistoryDelta.manual_repairs_used_delta` 的 merge 中有类型/应用逻辑（`evaluate.rs:106-107,133-138`），人工 feedback CAS 在 `human_gate.rs:195-200` 更新 snapshot 而没有更改此 run history 字段。全仓生产路径未见任何 `auto_rewrites_used` 与 `manual_repairs_used`/`manual_repairs_remaining` 同时更新或互读的代码。`rg` 命令和完整搜索模式列于报告末尾。
- **生命周期不同：** 编译成功 `compile_failure_surface.rs:363-366` 把 `auto_rewrites_used` 清 0；`manual_repairs_used` 是 session `RunHistory` 字段（`types.rs:190-213`），未在该成功路径清零。前者候选编译成功轮清账，后者属于会话计数。
- `review/routing.rs:605-610` 中的 `attempts_used` 汇总 `history.repairs_used.saturating_add(history.manual_repairs_used)`，而 `manual_repairs_remaining` 只计算 `max_manual_repairs - history.manual_repairs_used`。`compile.rs:1081-1089` 对等注释说这来自既有 durable 计数，“不臆造新预算常量”。前者是累积使用统计值，不是可用额度。它也没有包含另一个 candidate status 的 `auto_rewrites_used`。

### `attempts_used` 代码与上下文注释原文

`src/product/workspace_engine/review/routing.rs:589-613`：

```rust
// 在 repairs_used/cycle 计数合并之前改道原人工门（Interactive
// → EnterHumanGate / AutoIfValid → StopNeedsHuman；L2 退役同
// 语义）：零 author start、零 single_candidate_author 预领键、
// 零 repair_reservation、零 repairs_used/cycle 增量。
let action = if matches!(action, RoutingAction::TriggerAggregateRepair { .. })
    && self.is_single_candidate_plan()
    && self.durable_structural_grant_present()
{
    ...
    attempts_used: history
        .repairs_used
        .saturating_add(history.manual_repairs_used),
    manual_repairs_remaining: RunBudgets::default()
        .max_manual_repairs
        .saturating_sub(history.manual_repairs_used),
    trigger: HumanReason::RepairBudgetExhausted,
    ...
};
```

另一个相同公式的原文注释 `workspace_engine/compile.rs:1081-1089`：

```rust
/// 预算口径与 routing_scope.rs `single_candidate_approval_gate` /
/// policy_routing.rs `human_gate_snapshot` 等价构造一致：
/// `manual_repairs_remaining = RunBudgets::default().max_manual_repairs
/// − run_history.manual_repairs_used`（saturating）、`attempts_used =
/// repairs_used + manual_repairs_used`——从既有 durable 计数接续扣减，
/// 不臆造新预算常量。
```

### 两个扣减点

`human_gate.rs:195-200` 是 owner 反馈轮 CAS 扣 `manual_repairs_remaining` 并增 `accepted_feedback_turns`；`review/routing.rs:627-638` 是 policy repair action 更新 `RunHistory.repairs_used`/cycle；平台 compile rewrite 更新 `CandidateCompileStatus.auto_rewrites_used`（`:301-325`）。**它们不是同一可同步账本。** 注释/契约称共池，但 durable 扣减分散且互不感知。

## Q3｜“只显示一个余量数字”是否如实

**明确结论：当前不如实。** 虽然两种 author rewrite 在契约中必须共池，实际代码没有原子共享账本，`auto_rewrites_used` 与 `manual_repairs_remaining` 可呈不同余量；只展示 gate `remaining_budget` 会隐藏 candidate compile 自动重写余额，反向也会隐藏人工返修余额。

而且两句能同时出现不是推测：在 code state `auto_rewrites_used=3`、`manual_repairs_used=0` 时，平台账本已用满（自动余量 0），而 gate 构造公式仍给 `manual_repairs_remaining = 3 - 0 = 3`。二者独立维护，故**可以同时成立、不是偶发**。注意当前 `auto_rewrites_used=3` 对应自动重写自身已耗尽的文本/状态，`repair_budget_exhausted` trigger 本身实际另有含义（Q4）。

相关代码：

- candidate 权威余量：`models/human_gate.rs:227-237` `CandidateAutoRewrite.remaining`；`compile_failure_surface.rs:153-175,301-325`。
- gate 余量：`review/policy_routing.rs:100-107`，既有快照 carry-forward `:116-130`；人工 CAS 消耗 `human_gate.rs:181-200`。
- UI 分别取 trigger 与预算：`web/src/components/chat-workspace/entries/gate-next-step.tsx:93-102` 定义 trigger 文案；`GatePromptEntry.tsx:152-153,561-570` 取值并渲染；`gate-card-metadata.ts:41-56` 分别读 `entry.metadata.gate_trigger` 和 `entry.metadata.remaining_budget`。
- `entry.metadata.remaining_budget` **不是 render 时从 `auto_rewrites_used` 动态重算的实时值**，而是 gate 投影给出的快照余额：`web/src/state/workspace-cockpit-gate-projection.ts:331-376`；turn 在场时使用 typed turn 的 `remaining_budget`，除非新 snapshot 身份取代它（`:343-365`）；没有 turn 时直接从 `humanGateSnapshot.manual_repairs_remaining` 取值（`:410-430`）。此值随 WS state/projection 更新而变化，不是前端独立账本。历史 ChatEntry metadata 则是创建时投影值；需更新/重建该条 entry 才更新，旧卡不能仅因 candidate status 写入而自动得到新池余额。
- `accepted_feedback_turns` 不从 entry metadata 读。`GatePromptEntry.tsx:210-217` 从当前 workspace store `humanGateSnapshot?.accepted_feedback_turns` 读取；后端字段定义 `types.rs:88-93`，CAS 在 `human_gate.rs:199-200` 更新。门卡的“你已回复 N 轮”因此与 gate snapshot 同一实时 state，不是门条 metadata 的旧值。

## Q4｜`trigger=repair_budget_exhausted` 到底对应哪个池？

- `evaluate.rs:349-375`：对自动 repair candidate，先取当前 `cycle.repairs_used`；`Initial && cycle_repairs_used < max_repairs` 才返 Repairable，否则返回 `HumanRequired { reason: RepairBudgetExhausted }`。故评估器的 trigger 对应**每周期机械/政策 repair 配额**（`max_repairs`），不是 author 自动重写预算，也不是人工反馈预算。
- 它不是所有同码 trigger 的唯一触发条件：`routing.rs:502-509` 若 `cycle.initial_count == 1 && cycle.verification_count >= 1`，直接合成 `HumanRequired { RepairBudgetExhausted }`，history delta 还是 `Default::default()`。这条由 cycle 已走过初评与复评次数阻断，不是在数值 `max_repairs` 已用尽时才触发。因此前端把这个枚举无条件翻成“自动修复次数已用完”，语义并不总成立。
- `routing_platform_rescue.rs:82-125` 对纯平台 compile failure verification repair 尝试救援：先要求 phase Verification 且 outcome 是 `RepairBudgetExhausted`，再要求所有 actionable finding 带平台前缀；最后如果 `cycle.repairs_used >= max_repairs` 即保留耗尽结论交人工门。此处确实检验机械返修池。
- gate 现场余额仍独立按 `max_manual_repairs - manual_repairs_used`（`routing.rs:605-612`），实际可以保留 3。
- UI 文案位置：`gate-next-step.tsx:93-102` 将 `repair_budget_exhausted` 硬编码为“自动修复次数已用完，需要你来定”；`GatePromptEntry.tsx:564-570` 同行分别拼 trigger 与 `还可自动修复 ${remainingBudget} 次`。

**Q4 一句话结论：owner 看到的“自动修复次数已用完”来自错误过度概括的 `RepairBudgetExhausted` trigger（主要是周期 `max_repairs=1` 耗尽，也可能是初评/复评 cycle 状态封闭），而“还可自动修复 3 次”来自独立门快照 `manual_repairs_remaining=max_manual_repairs(3)-manual_repairs_used(0)`；这两句字段不同、状态能够并存，且 trigger 不证明平台 `auto_rewrites_used` 已耗尽。**

## Q5｜改动落点和哪个安全

### A. 合并账本、兑现契约共池

需要把 `max_manual_repairs` author 重写池做成共同权威，在“平台真正委托 author”以及“人工 feedback turn reservation CAS”两入口恰好扣一次；编译成功清账、重试/replay、candidate/session 两份 durable 事实恢复也须统一。不要将 `max_repairs` 变成平台自动重写额度。

- 自动 rewrite 路由与写入：`workspace_engine/compile_failure_surface.rs:292-348,623-670`。
- feedback CAS：`lifecycle_store/human_gate.rs:110-115,175-205`。
- gate 新建/恢复与余额投影：`workspace_engine/review/policy_routing.rs:95-130`、`review/routing.rs:601-613`、`workspace_engine/compile.rs:1090-1117`。
- UI/wire 状态：`workspace_ws_types/out.rs` CandidateCompileFailure/auto rewrite projection、`web/src/state/workspace-cockpit-gate-projection.ts:331-430`、`GatePromptEntry.tsx:152-170,561-570`。
- `work_item_plan_policy/evaluate.rs:355-375,465-472` 的机械返修周期池与文案/预算逻辑保持独立。

**风险大**：candidate status 与 session snapshot 分开持久化，跨载体更新存在非原子窗口、幂等及恢复风险；不能只在其中一个入口硬减另一个字段。

### B. 两个真实数都显示（短期更安全）

- `gate-next-step.tsx:101` 更正 trigger 人话名：明确“评审周期机械返修/政策返修资格耗尽或周期关闭，需人工判断”，不能说平台自动重写池已用完。
- `GatePromptEntry.tsx:561-570` 与 `gate-card-metadata.ts:41-56` 已提供独立 trigger 和余额位置；让人工 feedback 余量标作“人工返修剩余”。
- 若要展示真正的平台自动重写剩余，走 candidate compile failure wire 状态里的 `auto_rewrite.remaining`（目前 UI candidate failure 同状态已在 `GatePromptEntry.tsx:160-170` 消费）；不要从 attempts_used 之和推算。
- 这样做最少触碰已有 durable 扣账语义，避免在未设计 CAS/recovery 的情况下伪造共池。可作为近期更安全选项。若 owner 最终坚持共池，则须先补共同权威状态与一致性语义，再收敛 UI 到一个数。

## 检索范围与命令

以下均以指定工作树为目标；工具读取使用 `$W` 绝对路径。未执行 build/test/lint/formatter。

```sh
W=/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02
cd "$W" && pwd

git grep -n -E 'attempts_used|manual_repairs_remaining|accepted_feedback_turns|repairs_used_delta|RepairBudgetExhausted|auto_rewrites_used|max_auto_rewrites|REQ-WSC-21' -- "$W/src/product/work_item_plan_policy" "$W/src/product/workspace_engine" "$W/src/product/lifecycle_store" "$W/cadence/notes" "$W/openspec/specs/work-item-plan-single-candidate/spec.md"

rg -n 'auto_rewrites_used\\s*=|auto_rewrites_used:|manual_repairs_used\\s*=|manual_repairs_used:|manual_repairs_used_delta' "$W/src/product"

rg -n 'auto_rewrites_used|manual_repairs_remaining|manual_repairs_used' "$W/src/product"

rg -n '自动修复次数已用完|还可自动修复|remaining_budget|manual_repairs_remaining|repair_budget_exhausted|accepted_feedback_turns' "$W/web/src"
```

定点读取：
`src/product/work_item_plan_policy/types.rs`、`evaluate.rs`；`src/product/models/human_gate.rs`；`src/product/workspace_engine/compile.rs`、`compile_failure_surface.rs`、`review/policy_routing.rs`、`review/routing.rs`、`review/routing_platform_rescue.rs`；`src/product/lifecycle_store/human_gate.rs`；`src/web/workspace_ws_handler/decisions.rs`；`web/src/components/chat-workspace/entries/gate-next-step.tsx`、`GatePromptEntry.tsx`、`gate-card-metadata.ts`；`web/src/state/workspace-cockpit-gate-projection.ts`；`openspec/specs/work-item-plan-single-candidate/spec.md`；`cadence/notes/2026-11-03_审查_块1change一致性裁决_v1.0.md`。
