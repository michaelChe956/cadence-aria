# park-1 修复报告:plan compile finalize 持久化 per-target LifecycleWorkItemRecord

- 日期:2026-10-13
- 分支/Worktree:`feat-b-0808-add-monorepo`(`.worktrees/feat-b-0808-add-monorepo`)
- 提交:`fix: plan finalize 持久化 per-target work item 记录(park-1)`

## 问题(8-19 验收记录已文档化的欠账)

compile finalize 只写 plan_update / runtime binding / child sessions,**不落
per-target `LifecycleWorkItemRecord`**(work-items/*.json 目录不存在)。coding
会话创建走 `workspace_repository.rs` WorkItem 臂的
`list_work_items → find(work_item.id == session.entity_id)` 落空,报
`product_store_not_found: work_item WI-xxx`,按钮无反应。

现场数据(`.aria/projects/project_0001/issues/issue_0001`):
`work_item_plan_compiles/040807f7….json` status=committed,
`outline_to_work_item_id` 五条映射齐、child_session_ids 0004-0008 齐,
work-item-revisions 下五条 canonical contract 均带
`intent_contract.target`(logical_codebase + per-WI logical_repository_id)——
唯独 work-items/ 缺失。

## 修法

### 1. finalizer.rs:`persist_compiled_work_item_records`(新步骤)

位置:`finalize_initial_plan_compile` 内 `plan_summary_prepared` 之后、
enrollment binding/child session 之前;新 step_cursor
`work_item_records_persisted` + 新 failpoint checkpoint
`WorkItemRecordsPersisted`(compile.rs),崩溃恢复(Continue)自动重放。

记录 **id = logical work item id(entity 语义)**,与
`plan.work_item_ids`、child session `entity_id`、WorkItem 臂 find 同域:

- title/kind/scopes/verification_plan_ref 从 compiled
  `work_item_revision.canonical_contract` 派生;
- depends_on 从 `outcome.dependency_graph_revision.edges`(logical 域)派生;
- source_outline_id 经 `tx.outline_to_work_item_id` 反查(legacy 值域 /
  SC 键域双兼容);
- `tx.created_work_item_ids` 记录 logical ids(可观测性)。

### 2. 归属解析:`work_item_record_attribution`(纯函数,pub(crate))

与 Final Compile 成员归属语义同源(lower.rs `resolve_item_target_repository_id`
/ ir_adapter `logical_target_for_ir_item`):

- Legacy(单仓,targets=None):repository_id=story 仓(与
  `work_item_plan_repository_id` 同源),target_repository_id=None;
- LC:`intent.target=logical_codebase` → 成员物理仓 + Some(logical);
  `single_repository` → 声明物理仓 + 反查唯一逻辑成员;
  无 intent → 确定性回落仓 + 反查逻辑成员;
  目标失效/非成员/回落缺失 → `work_item_target_missing` fail-closed。

LC 成员表复用 `plan_compile_member_targets`(session repository 经
`resolve_workspace_repository_context` 解析);Legacy 路径不解析 session 仓,
对无 issue.repo_id 的旧数据零回归。

### 3. lifecycle_store/work_item.rs:`upsert_compiled_work_item_record`

finalize 的唯一写点。已存在时仅覆盖 compile 派生归属字段,保留
execution_status/worktree_path/execution_plan_status/completion_*/created_at
——重放 finalize、恢复重放、同 plan 再编译均不重复建、不清执行进度;
with_exclusive_lock 保护读改写。

### 纪律红线

SC 链 finalize 禁读 legacy compile store(`panic_on_legacy_compile_reads`),
故 `source_draft_id` 不在 finalize 派生(消费面 group draft context 自有
tx 兜底)。

## TDD 证据

红→绿(park1_work_item_records.rs,8 个测试):

- 红:finalize 后 list_work_items 无 wi_a/wi_b、attribution 桩、upsert 桩
  ——7 failed;
- 绿:全部通过,含
  - E2E:records 五字段归属 + `workspace_repository_for_session` WorkItem 臂
    resolve 成功;
  - 幂等:failpoint 中断后恢复 Continue 重放,记录数不变、状态保留;
  - fail-closed:无效逻辑成员/非成员物理仓/回落缺失。
- 基线翻转:part_08 "must not create legacy facts" → "must persist
  per-target facts";part_15 游程序列基线补 `work_item_records_persisted`。

## 回归

| 范围 | 结果 |
|---|---|
| `workspace_engine::tests`(plan compile 全族) | 1268 passed, 0 failed |
| `lifecycle_store`(全族) | 105 passed, 0 failed |
| `single_candidate`(全族) | 172 passed, 0 failed |
| `workspace_repository` + `coding_evaluation_context`(直接消费面) | 32 passed, 0 failed |

## 关联

park-2(start_generation 路由)由 5a217fa8+3372bdbe 解决,与本修正交;
本修后 LC 链 coding 会话创建的 resolve 链已通(单测覆盖,实测以验收环境为准)。

## Fix Round(复审 P2):source_outline_id 回退取键域

- 提交:`fix: park-1 source_outline_id 回退取键域`
- 缺陷:回退分支 `tx.outline_to_work_item_id.get(logical_id)` 取的是
  **值**;SC 路径 map 形态为 {outline/logical id → work_item.id(物理)},
  Continue 恢复重放时落盘 source_outline_id 变成 `work_item_{compile_id}_{nnn}`。
- 修法(复审给定):键命中则取键——
  `contains_key(logical_id).then(|| logical_id.clone())`。
- TDD:红(sc_shaped_outline_map_persists_logical_source_outline_id,
  left `Some("work_item_phys_001")` / right `Some("wi_a")`)→绿。
- 回归(隔离基线树 /tmp,= 本包 HEAD + 修复 diff;当时主 worktree 被
  SG1 in-flight compile_support.rs 未编译态阻塞):workspace_engine 全族
  1269 passed / 0 failed(首轮 1 例 failpoint 并行 flaky,复跑全绿)、
  lifecycle_store 105 passed / 0 failed、single_candidate 172 passed /
  0 failed;park1 包 12 测试(含新 sc_shaped 红绿)全绿。

