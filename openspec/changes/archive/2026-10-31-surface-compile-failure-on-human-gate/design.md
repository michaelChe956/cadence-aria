# Design: surface-compile-failure-on-human-gate

## 0. 现场核清的事实（逐条给结论与位置）

| # | 问题 | 结论 | 证据 |
|---|---|---|---|
| 1 | `human_confirm` 之后的 Final Compile 是必经还是可选？ | **必经**。`close_human_gate(Approve)` 只做 close CAS（`Running`）→ 立刻 `enter_policy_valid_work_item_plan_compile_trust_ready()` 重跑编译；只有 durable 落 `Confirmed` + phase `Completed` 才算确认成功，否则 `Err(single_candidate_gate_close_failure_error())` 并回到门。**没有「跳过编译直接放行」的分支** | `conversational_gate.rs:769-836`（CAS→compile→终态复核）、`compile.rs:594-620`（三个 compile 入口）、`single_candidate.rs` 内 `prepare_initial_plan_compile` 失败即 `?` 上抛 |
| 2 | `final_compile_prepare_failed` / `dependency_cycle` 现在会不会触发教学重驱？ | **不会**。REQ-WSC-09 的教学重驱只覆盖 generate 段 parse 语法类（`missing_section`/`invalid_ears`/`invalid_work_item_id`）与 3.6 的 IR 校验失败；Final Compile 段的失败不经过那条通道 | `openspec/specs/work-item-plan-single-candidate/spec.md` REQ-WSC-09；`run/single_candidate.rs:1211-1219`（`is_teachable_parse_failure` 白名单闸门）、`:1343-1374`（IR 失败分支；非白名单直接终态） |
| 3 | 界面上「还可自动修复 3 次」是什么语义？ | **还能提交几次门内反馈让 author 重写**，不是「还能点几次确认」。初始值 = `RunBudgets::default().max_manual_repairs = 3`；每次反馈 turn 预留时 durable 减 1；扣到 0 后新的反馈 turn 被 `human gate budget exhausted` 拒收。确认（approve）**不消耗**这个预算 | `work_item_plan_policy/types.rs:221-230`、`compile.rs:1083-1113`（开门时预算构造）、`lifecycle_store/human_gate.rs:181-200`（turn 预留扣 1） |
| 4 | 为什么现场会话的 `policy_diagnostics` 与门快照 findings 都是空？ | `final_compile_prepare_failed` 是 plain `?` 站点：只有 `sg3b_failure(...)` 站点才写 `policy_diagnostics`；而 SC initial compile 的返回边界只做 seal + 内存 stash，没有 durable 诊断落点 | `single_candidate.rs:139-213`（返回边界）、`:205-244`（`sg3b_failure`/`sg3b_seal_with_diagnostic`）、现场 durable `policy_diagnostics=[]`、`human_gate_snapshot.findings=[]` |
| 5 | 为什么候选能过候选门（SG3a.1）、却在 Final Compile 炸环？ | 候选门的 SG1 有效图只从 `item.logical_target`（IR `intent_contract.target == LogicalCodebase`）出发建 change_order 自动边；该候选的 target 是 `single_repository`，`logical_target` 为 None → 自动边为空 → 看不到环。Final Compile 的 `apply_change_order_cross_repo_depends_on` 按 projected draft 的 `target_repository_id` 建边，才与既有 `depends_on` 撞成环 | `plan_projection.rs:996-1016`（`logical_target` 仅来自 LogicalCodebase intent）、`plan_projection.rs:1057-1063`（`physical_by_logical` 为空时清空 target）、`compile_support.rs:533-547`（SG1 ④ 按 `logical_target` 分组）、`compile_support.rs:794-818`（Final Compile 按 `target_repository_id` 分组） |

## 1. 语义裁定：候选编译不过时「确认当前版本」

**裁定：fail-closed（禁用 + 原因 + 出路），且判据在平台侧。**

理由（全部来自上面的既有链）：

1. 确认与编译是同一动作的前后两段（事实 1），候选自身无效时「确认」不可能成功——不是「有风险」而是「必然失败」。放行等于让 owner 反复点同一颗必定失败的按钮，这正是现场 `_014`→`_015`→`_016`→`_017` 的死路。
2. 既有契约里没有「跳过编译放行」的语义，因此不能把「确认」改写成「跳过校验的强制确认」——那是新语义，且会让无效计划进入下游 Work Item 生成。
3. 判据必须能证明「这次失败属于当前候选」：用 source revision ref + candidate IR ref 双侧与会话 durable 值逐字相等。任一侧缺席即无法证明 → fail-open。宁可放过一次（它会立即再失败并重新落诊断），也不能因诊断缺失把门永久锁死。
4. 编译成功必须覆盖清除该事实，否则一次瞬态失败（store/IO 类）恢复后仍会被过期失败挡住——那又会变成新的死路。

**未采用**：(b) 可点但声明后果——仍是死路，owner 明确要求「不应出现在可确认的集合里」；
**未采用** 平台自动发起 author 重生成——既有契约没有这条通道（事实 2、Non-Goals），新造一条属于
状态机级改动，且 owner 的 `manual_repairs_remaining` 预算语义（事实 3）本就以「owner 反馈驱动重写」
为唯一驱动源。

## 2. 数据与投影

```
CandidateCompileFailure { failure_id, code, message, detail, recorded_at }
CandidateCompileStatus  { source_revision_ref?, plan_candidate_ir_ref?, failure?, recorded_at }
```

- 落点：`workspace-sessions/{session_id}/candidate-compile-status.json`（单文件覆盖写）。与
  `human-dispositions.json`、`human-gate-rejections/{turn}.json` 同目录、同一
  `with_optional_entity_live_write` 准入（不新造 store 家族）。
- 写入口：`WorkspaceEngine::run_single_candidate_initial_plan_compile` 的 Ok/Err 两侧（SC initial
  compile 的唯一返回边界）。Err 侧先 seal（拿 failure_id/refs）再落；Ok 侧写「无失败」覆盖。
- 为什么不用 `policy_diagnostics`：它只有 `{code,message,field}`、无 `detail`，且按诊断累积追加；
  本需求要「完整原文 + 单一当前事实（可清除）」，语义不同。既有 `policy_diagnostics` 站点一字不动。
- 读判据：`LifecycleStore::current_candidate_compile_failure(&record)`（单源）。门卡投影用内存
  session 镜像的 refs，确认判据用 durable record 的 refs，二者走同一
  `CandidateCompileStatus::belongs_to_candidate`。

## 3. Wire

`WsOutMessage::SessionState` 新增
`candidate_compile_failure: Option<CandidateCompileFailureDto>`：

```
candidate_compile_failure?: {
  code: string,
  message: string,          // 完整人话
  detail?: string,          // 完整诊断原文
  owner_hint?: string,      // 候选无效 + 让 author 重新生成 + design 可能需重写
  rewrite_guidance?: string[]  // 重写要点（与 author 教学同一组事实）
}
```

`serde(default, skip_serializing_if = "Option::is_none")` —— 缺席整体省略，旧 wire 字节不变。
`owner_hint` / `rewrite_guidance` 由 `code` 纯派生（不在 durable 里冗余存储），理由：二者是同一事实的
呈现函数，存盘会引入第二份可漂移副本；且已与前端确认这两字段并入同一载体而不是拆顶层字段
（拆开会出现第二个「当前候选」判据）。

## 4. 人话措辞的推导依据

- 环诊断由 `apply_change_order_cross_repo_depends_on` 产出，事实源是 projected work items
  （`title` / `target_repository_id` / `depends_on`）与 `change_order`。人话里的每个成分都能对上真实数据：
  - 环链：复用 SG1 既有的 `find_cycle_witness`（确定性 DFS，回边闭合），**不是** Kahn 残余节点集合
    （后者会把环下游节点也画进环，REQ-PLN-10 明确禁止）。
  - 「谁把谁写成 provider」：witness 跳 `(provider → consumer)` 中 `repo_index(provider) > repo_index(consumer)`
    的那一跳——即违反「仓序自动边只允许后仓依赖前仓」的显式依赖。这正是现场成因。
  - 仓序位次：`change_order.iter().position(repo) + 1`，真实位次，不猜仓名。
  - Work Item 名：`title`（空标题回落 id），由 owner 认得的标题表达，不拿哈希 id 当主语言。
- 机器 id 段留在 `detail`（`CYCLE_IDS_MARKER` 分隔），既有 blocker 语义
  （错误串仍以 `dependency_cycle:` 开头、仍含环上 id）逐字保留，旧断言与消费方不受影响。

## 5. author 教学面

`build_sc_manual_revision_prompt` 增可选 `compile_failure_teaching` 段，由
`build_sc_manual_revision_prompt_for_turn` 注入（门内反馈 → author 修订的唯一重建入口；frozen
structural prompt 路径不二次组装，保持既有冻结语义）。要点与 `rewrite_guidance` 同源：
provider 只能指向仓序在前的成员仓 WI、同仓顺序只能靠 `depends_on`、依赖图必须无环、根因在 design 时
必须同步改 design。无编译失败时该段不出现，既有提示词逐字不变。

## 5.5 自动重写的**执行**（REQ-WSC-21）

判定与执行分开：`compile_failure_surface::decide_route` 只出裁定，执行在
`compile.rs::route_candidate_compile_failure_to_author_rewrite`：

- 入口：Final Compile 失败分支（`enter_work_item_plan_compile_with_auto_confirmation` 的
  `Err` 臂），在补建门快照与 `enter_human_confirm` **之前**调用；返回 `true` 即不再开人工确认门。
- 通道：`route_single_candidate_contract_prerevision(verdict)`——与 F5-A 机械返修完全同一条
  ingestion（`start_review` 建 ReviewerRun 轮 → `complete_review` → 策略路由
  `TriggerAggregateRepair` → `request_provider_run(WorkItemPlanSingleCandidateAuthor)`），
  **不新造通道、不改状态机**。`start_review()` 无相位前置（只做 stage→CrossReview + 建评审节点），
  因此 post-Approval 可复用。
- verdict 形状：`auto_rewrite_verdict`（`Revise` + `RequiresRevision` + 一条
  `MustFix`/`ContractGap`/`Repairable` finding）；finding 的 `message` 只含**结构身份**
  （稳定码 + 环结构指纹），保证同一环每次都落到既有的「重复指纹」闸门；`required_action` 才写
  具体重写要点，且**不要求 author 改 design**。
- 递归：该链路经 `apply_policy_route` 回到 compile 入口，构成类型级 async 递归 → 以
  `Box::pin(...)` 打断（`E0733`）。

## 5.6 两个预算池的关系（owner 口径）

| 池 | 常量/字段 | 默认 | 谁消费 | 语义 |
|---|---|---|---|---|
| author 重写次数预算 | `max_manual_repairs` / `manual_repairs_remaining` | 3 | **平台自动重写** + **owner 反馈触发的重写**（同一池） | 「还能让 author 重写几次」（owner 第 3 条） |
| 机械返修池 | `max_repairs` / `repairs_used` | 1 | 指纹闸门那类**不重跑 author** 的机械 repair | 机械修复预算，**不**用于自动重写 |

两条判据互相独立：预算管「总共能重写几次」，结构指纹管「这次重写有没有意义」——同一环重复即使
预算仍有余也提前停（REQ-WSC-22）。

## 5.7 结构指纹判据（REQ-WSC-22）

`cycle_structure_fingerprint` = `sha256` over 规范化 JSON `{members, edges}`：

- `members` = 环 witness 上全部节点，按 `cycle_node_key` 去重排序；`edges` = 每跳
  `provider_key>consumer_key`，排序；
- `cycle_node_key` = `{编译序序号}@{target 逻辑仓 uuid}`——**刻意排除编译运行 id**
  （`work_item_<compile_id>_003` 每次尝试都变），否则同一环每次都被算成新环、白耗预算；
- 不含标题/文案/行号：作者换措辞、改标题不影响判据；
- 指纹行随 cause 落进 durable `detail`（`CYCLE_FINGERPRINT_MARKER`），与 `repeat_count`、
  `auto_rewrite{phase,remaining,repeated}` 同记录。

## 6. 已知缺口（本 change 不做，另开票据）

候选门 SG1 看不到跨仓环（事实 5）。正确修法是让 SG1 的 change_order 自动边不再只依赖
`intent_contract.target == LogicalCodebase`，而按 `physical_by_logical` 反查逻辑仓再定位 `change_order`
位次。风险：这改变候选门判定与冻结结构报告 digest（`context_digest` / `rule_version` 同代检查），
会让一批既有候选在新口径下被拒；且当前 SG1 的 `PlanningFailureKind::DependencyCycle` 修复通道
（SG5/SG6）尚未接线，提前拦下会把「落门可操作」变成「author run 终态失败」。因此本 change 只保证
「编译不过的候选在门上不可确认 + 原因与出路完整可读」，结构门口径收窄单独立项。

## 7. 验证策略

- 纯函数：环人话（真实 witness/仓序/逆行依赖）、`owner_message`/`detail_text`、
  `owner_hint`/`rewrite_guidance`/`author_teaching_block`、`approve_rejected_message`。
- store：完整诊断逐字落盘（长文本不截断）、候选绑定判定（命中/不命中/缺 refs）、成功后覆盖清除。
- 引擎：`session_state` 投影三要素齐全、旧候选不投影、确认 fail-closed（状态/阶段不变）、
  修订提示词含/不含教学段的两个方向、compile 失败边界确实落了完整 durable 诊断。
- 回归：既有 `compile_rejects_change_order_cycle`（稳定码仍在）、既有修订提示词逐字断言、全量门禁。

## 8. Risks

- **SG1 缺口（未修，另开票据）**：候选门 SG1 的 `change_order` 自动边只从
  `item.logical_target`（= IR `intent_contract.target == LogicalCodebase`）出发；`single_repository`
  target 的候选因此**看不到跨仓环**，要等到 Final Compile 的
  `apply_change_order_cross_repo_depends_on` 才炸。后果：本 change 的「自动重写」是**事后**纠错，
  不能阻止这类无效候选先被生成一轮。收窄该口径会改变候选门判定与冻结 `context_digest`，且 SG1 的
  `DependencyCycle` 修复指令通道（SG5/SG6）尚未接线，因此单独立项，不在本 change 内改。
- **自动重写的可达性**：`route_single_candidate_contract_prerevision` 走
  `start_review()` + `complete_review`；Fake reviewer 快速路径会直达
  `enter_human_confirm`（既有语义），此时自动重写退化为开门——真实 provider 会话不受影响，但
  Fake/无 reviewer 场景的自动化重写不可用（既有约束，未改）。
- **`auto_rewrite.remaining` 的口径**：本 change 用 durable `auto_rewrites_used` 记平台侧已消耗
  次数（初值 = `max_manual_repairs`）；它与门快照 `manual_repairs_remaining`（owner 反馈 turn
  预留时扣减）是**同一语义池的两处账本**，二者暂未合并为单一计数器——展示层数字以 `remaining`
  为准，owner 反馈路径仍按既有 gate turn 预算扣减。
