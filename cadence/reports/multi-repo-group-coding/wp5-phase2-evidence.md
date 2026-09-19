# WP5 二期证据留档与 defer 登记（T5 / REQ-MTG-05，design D5）

- **change**: `openspec/changes/multi-repo-group-coding/`
- **登记对象**：跨 target-attempt 依赖就绪门自动编排（REQ-GCE-01 的 attempt 间扩展）——二期范围，**本 change 零落地**。

## 一、defer 登记（决议 Q2(c) 原文承接）

| 项 | 内容 |
|---|---|
| defer 项 | 跨 attempt 依赖门/自动编排/编排策略（按 plan 内 units 依赖映射跨 attempt 启动判定等任何形态） |
| 触发条件 | 一期交付后**真实多仓使用证据**齐备：①增殖审计分布（`split-audit` 记录的真实 plan/target/attempt 增殖形态与频度）②聚合终态/partial failure 分布（真实多 target plan 的 AllDelivered/Partial/NotStarted 与失败阻塞原因分布） |
| 证据来源 | ①审计面：`issue_lifecycle_root/{project}/{issue}/split-audit/{attempt_id}.json`+检索 `get_split_audits_for_plan`（本期落地）；②聚合视图：`PlanGroupProjection`（WP3 `plan_group_projection.rs`+issue_lifecycle API+前端 PlanGroupProjectionPanel，报告见 wp3-aggregate-report.md） |
| 处置 | 凭证据**另行立项**（独立 change 显式定义编排语义与门判定；不预埋、不隐式、不随 advance 形态落地） |
| 红线依据 | spec REQ-MTG-03 一期红线句（MUST NOT 自动拉起跨 target attempts；StartCoding 唯一入口）；决议 Q2 两期边界 |

## 二、本 change 无跨 attempt 编排落地自查

1. **行为面**：`split_advance_leaves_attempts_unorchestrated_at_created_prepare_context`
   （T2）——拆分创建后 N attempts 全停 (Created, PrepareContext)；本期
   `split_advance_records_proliferation_audit_for_every_target_attempt` 复核同判据。
2. **编译面 grep**：`split_audit.rs` 及其消费面零调度调用（start/spawn/queue/
   schedule/transition 族均无）；审计写入点（advance.rs 分流循环）仅追加 durable
   记录，无任何状态转换副作用。
3. **attach/恢复面**：断连重连门（`sc_advance_restart_blocked`）仅对集内 attempt
   放行 runner 重启，不主动拉起 Created 态 attempt（`resumed_attempt_needs_runner`
   判定 Running+WorktreePrepare/Coding 才动作，Created 不动作——REQ-ADV-05 句②锚）。
4. **检索面**：`get_split_audits_for_plan` 纯只读派生（list+filter+确定性排序），
   无第二状态机。

## 三、台账边界声明

**不修改既有 defer 台账**（`cadence/reports/workitem-conversational-gate-advance/defer-ledger.md`
等）——本 change 无 DEF 行销账义务（D5 defer 登记以本报告+spec REQ-MTG-03 defer 句
为准）。该台账 DEF-5「多仓 coding（后续 change）」即本 change 所在域：一期交付
（解禁+拆分+聚合+矩阵）完成后，二期依赖门凭本报告 §一 证据条件另行立项。

## 四、二期立项时的证据读取口径（备忘）

- 审计分布：按 plan 聚合 `get_split_audits_for_plan` 计数/去重 target 集；trigger
  枚举区分 advance 与建组入口（`group_creation` 词汇位一期预留，仅 advance 面接线）。
- 终态分布：`PlanGroupOverall` 三值（`all_delivered`/`partial`/`not_started`）+
  `PlanTargetEntry`（状态/stage/push/review_request/blocked_reason）——partial failure
  不伪装全局成功（REQ-COD-06 plan 级投影）。
