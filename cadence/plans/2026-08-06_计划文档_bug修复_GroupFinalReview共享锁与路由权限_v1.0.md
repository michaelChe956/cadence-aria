# GroupFinalReview 共享锁与路由权限 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 消除 GroupFinalReview 失败后遗留的 issue shared-worktree 锁，并让分片与归约 Provider 都获得且只能使用快照级权威路由信息。

**Architecture:** 终态锁清理统一以 `attempt_id` 为所有权键，复用 `LifecycleStore::release_issue_worktree_lock_by_owner`，不再把 Work Item 游标当作释放前置条件；预检仍负责阻止非终态写入越过其他 attempt 的锁。组级材料把 `routing_targets` 从 1,200-byte `UnitCrossReviewRecord` 拆到由权威 Binding 编译、参与快照哈希的 routing authority index；Prompt 构建器按分片投影索引，而归约携带完整索引。解析后的分片与归约 finding 在 CAS 落库前使用该快照索引进行语义校验。

**Tech Stack:** Rust 2024、Tokio、Serde、现有 JSON Product/Lifecycle Store、Cargo 内置单元与集成测试。

## Global Constraints

- 对应 OpenSpec change：`scalable-group-final-review`，工作包 2.3、3.3、4.1、5.1、5.3、6.5、8.3、8.5；不得扩展契约定义的范围。
- `MAX_UNIT_RECORD_BYTES` 固定为 `1_200`；不得通过增大上限、静默裁剪 identity、reason_code、目标 Revision 或 contract ref 绕过材料预算。
- 锁释放只在持久化 `current_lock_owner_id == attempt_id` 时清除锁；锁不存在或 owner 不同是幂等 no-op，不能改动另一 attempt 的锁。
- 对已经确定的 GroupReview 失败，清理/释放异常只能产生诊断，不能替换材料或 Provider 的原始错误；dirty shared worktree 仍保留现有人工清理保护。
- 分片 Prompt 只见本分片 Unit 及跨片边所需权限；归约 Prompt 必须见 active snapshot 的完整权限索引；每一段都纳入 UTF-8 完整 Prompt 字节度量。
- `src/product/coding_workspace_engine/tests/group_review_material.rs`、`tests/it_product/product_coding_workspace_engine/part_01.rs` 现有未提交的回归测试是本次红灯基线：保留其覆盖意图并在实现后使其通过；不触碰 `HANDOFF.md` 和 `.pi-subagents/`。
- Rust 定向单测使用 `cargo test --locked --lib <过滤名>`；集成测试使用对应 `--test` target；禁止 `-j 1`。

---

## 文件结构与边界

| 文件 | 本次职责 |
|---|---|
| `src/product/coding_workspace_engine/gates.rs` | 提供按 attempt owner 的终态共享锁释放入口；保留非终态的 owner 预检。 |
| `src/product/coding_workspace_engine/handoffs.rs` | 在 failed、abort、delete、final confirm、group final-review completion 路径调用统一终态清理，且不让清理覆盖已确定的业务错误。 |
| `src/product/coding_workspace_engine/group_review_types.rs` | 定义快照级 `RoutingAuthorityEntry`，移除单位记录内的完整 routing targets，并新增 routing-authority Prompt 段与预算字段。 |
| `src/product/coding_workspace_engine/group_review_material.rs` | 从排序后的权威 Binding 编译稳定的权限索引，写入 draft、content hash 和最终 snapshot。 |
| `src/product/coding_workspace_engine/group_review_budget.rs` | 把 routing-authority 段纳入 `join()` 与 `PromptBudgetBreakdown::total`。 |
| `src/product/coding_workspace_engine/group_review_prompts.rs` | 渲染 shard 局部权限与 reduction 全量权限，并把结论契约的 authority 引用从 unit record 改为该索引。 |
| `src/product/coding_workspace_engine/plan_defect_routing.rs` | 提供以 snapshot authority index 校验 finding 的纯函数，保留原有 Binding 校验给其它调用方。 |
| `src/product/coding_workspace_engine/group_review_orchestrator.rs` | 分片 CAS 前按 shard authority 校验；归约 CAS/分诊前按完整 authority 校验；无效结果仍以现有 `*_output_invalid` 失败关闭。 |
| `src/product/coding_workspace_engine/tests/group_review_material.rs` | 覆盖 record 体积、索引稳定性与完整 routing authority 不再挤占单位记录。 |
| `src/product/coding_workspace_engine/tests/group_review_prompts.rs`、`tests/group_review_budget.rs` | 覆盖 Prompt 投影范围与字节统计。 |
| `src/product/coding_workspace_engine/tests/group_review_orchestrator.rs`、`tests/group_review_reduction.rs`、`tests/group_review_e2e.rs` | 覆盖分片/归约语义校验、CAS 前拒绝与 Provider 未被调用的门禁。 |
| `tests/it_product/product_coding_workspace_engine/part_01.rs`、`src/product/coding_workspace_engine/tests/group_terminal.rs` | 覆盖失败、取消、删除、确认/完成的 owner-based 锁释放以及原始 GroupReview 失败保留。 |

### Task 1: 为终态 attempt 建立 owner-based 锁释放回归基线

**Files:**

- Modify: `tests/it_product/product_coding_workspace_engine/part_01.rs:217-345`
- Modify: `src/product/coding_workspace_engine/tests/group_terminal.rs:53-145`
- Modify: `src/product/coding_workspace_engine/gates.rs:473-488`
- Modify: `src/product/coding_workspace_engine/handoffs.rs:15-235, 312-337`

**Interfaces:**

- Consumes: `LifecycleStore::release_issue_worktree_lock_by_owner(project_id, issue_id, owner_id) -> Result<IssueSharedWorktree, ProductStoreError>`.
- Produces: `CodingWorkspaceEngine::release_issue_shared_worktree_lock_for_attempt(project_id, issue_id, attempt_id) -> Result<(), CodingWorkspaceEngineError>`，仅按 attempt owner 清理锁。
- Invariant: 非终态生产入口继续调用 `validate_attempt_issue_shared_worktree_lock_if_present`；终态释放入口不能要求 `work_item_id` 匹配。

- [ ] **Step 1: 写出终态 owner 清理的失败测试**

  保留已有的 `failed_group_attempt_releases_transferred_shared_worktree_lock_by_owner`，再在同一 fixture 中增加 abort、delete 和 group final-review completion 的转移锁场景。每个测试先把锁从 `work_item_0001` 转移到 `work_item_0002`，随后只断言 owner 行为：

  ```rust
  assert_eq!(shared.current_active_work_item_id, None);
  assert_eq!(shared.current_lock_owner_id, None);
  assert_eq!(shared.status, IssueSharedWorktreeStatus::Ready);
  ```

  在 `group_terminal.rs` 增加材料失败路径断言：`finalize_group_review_failure(..., CodingWorkspaceEngineError::GroupReviewMaterial("unit_cross_review_record_exceeds_size_limit".into()))` 返回的错误文本仍包含 `group_review_material_error`，而不是 `issue_worktree_lock_owner`。

- [ ] **Step 2: 运行终态测试，确认目前失败**

  Run:

  ```sh
  cargo test --locked --test it_product failed_group_attempt_releases_transferred_shared_worktree_lock_by_owner
  cargo test --locked --lib group_terminal
  ```

  Expected: 第一个测试在锁已转移到 `work_item_0002` 时出现 `issue_worktree_lock_owner`；材料失败路径显示清理错误覆盖了 `GroupReviewMaterial`。

- [ ] **Step 3: 实现不依赖 Work Item 的终态释放入口**

  在 `gates.rs` 用 owner-only 生命周期 API 替换现有带 `work_item_id` 的 helper：

  ```rust
  pub(crate) fn release_issue_shared_worktree_lock_for_attempt(
      &self,
      project_id: &str,
      issue_id: &str,
      attempt_id: &str,
  ) -> Result<(), CodingWorkspaceEngineError> {
      let lifecycle = LifecycleStore::new(self.store.paths());
      if lifecycle.get_issue_shared_worktree(project_id, issue_id)?.is_some() {
          lifecycle.release_issue_worktree_lock_by_owner(project_id, issue_id, attempt_id)?;
      }
      Ok(())
  }
  ```

  在 `handoffs.rs` 的 `handle_attempt_failed`、`handle_abort`、`handle_delete_attempt`、`handle_final_confirm` 的 group 分支和 `complete_group_attempt_after_final_review` 中调用此入口。保留状态转换、完成门禁与非终态的 owner 预检；把 failure 路径的 `ensure_issue_shared_worktree_clean`/释放错误改为诊断分支，使已经选定的 `CodingWorkspaceEngineError` 仍由调用方返回。dirty worktree 时不清锁，但不能改写原始 GroupReview 错误。

- [ ] **Step 4: 运行终态测试，确认通过且不释放他人锁**

  Run:

  ```sh
  cargo test --locked --test it_product failed_group_attempt_releases_transferred_shared_worktree_lock_by_owner
  cargo test --locked --test it_product final_confirm_owner_conflict_does_not_complete_attempt
  cargo test --locked --lib group_terminal
  ```

  Expected: 转移锁全部清空；既有 foreign-owner 预检测试仍拒绝非本 attempt 的写入且另一 attempt 的锁保持不变；材料失败保持原错误类别。

- [ ] **Step 5: 提交原子锁生命周期修复**

  ```sh
  git add src/product/coding_workspace_engine/gates.rs src/product/coding_workspace_engine/handoffs.rs src/product/coding_workspace_engine/tests/group_terminal.rs tests/it_product/product_coding_workspace_engine/part_01.rs
  git commit -m "fix(group-review): release terminal worktree locks by owner"
  ```

### Task 2: 将 routing authority 从单位记录移至快照索引

**Files:**

- Modify: `src/product/coding_workspace_engine/group_review_types.rs:7-88`
- Modify: `src/product/coding_workspace_engine/group_review_material.rs:20-105, 179-274, 1061-1085`
- Modify: `src/product/coding_workspace_engine/tests/group_review_material.rs:564-724`

**Interfaces:**

- Consumes: `AuthoritativeGroupReviewerBinding`、`ReviewerWorkItemProjection::blocker_routing`、`normalize_blocker_route`。
- Produces: `RoutingAuthorityEntry { source_unit_run_id, source_logical_work_item_id, source_work_item_revision_id, reason_code, allowed_route, required_target_kind, target_contract_refs }` 与 `GroupReviewMaterialSnapshot::routing_authority_index: Vec<RoutingAuthorityEntry>`。
- Invariant: `UnitCrossReviewRecord` 不再有 `routing_targets`；其序列化字节必须不超过 `MAX_UNIT_RECORD_BYTES == 1_200`。

- [ ] **Step 1: 将材料回归测试改成新契约的红灯**

  将 `material_preserves_routing_targets_while_trimming_non_authoritative_fields` 重命名为 `material_moves_routing_authority_out_of_trimmed_unit_record`，并断言：

  ```rust
  assert!(record.contract_interfaces.len() < 16);
  assert_eq!(result.routing_authority_index.len(), 1);
  assert_eq!(result.routing_authority_index[0].reason_code, "verification_command_failed");
  assert!(serde_json::to_vec(record).unwrap().len() <= 1_200);
  ```

  将 9 条真实 routing rule 的测试改为断言记录仍小于上限而 index 完整保留 9 条；将“routing targets alone 超限”改为断言长 authority 不再导致 `unit_cross_review_record_exceeds_size_limit`，但其后的 Prompt 硬上限测试属于 Task 3。

- [ ] **Step 2: 运行材料单测，确认当前实现不满足新接口**

  Run:

  ```sh
  cargo test --locked --lib material_moves_routing_authority_out_of_trimmed_unit_record
  cargo test --locked --lib material_keeps_realistic_full_routing_authority_without_exceeding_record_limit
  ```

  Expected: 编译或断言失败，因为 `UnitCrossReviewRecord` 仍携带 `routing_targets`，而 snapshot 尚无 `routing_authority_index`。

- [ ] **Step 3: 定义并确定性编译快照 authority index**

  在 `group_review_types.rs` 新增 index 字段与 `RoutingAuthorityEntry`，并从 `UnitCrossReviewRecord` 移除 `CompactRoutingTarget`。在 `group_review_material.rs` 增加 `routing_authority_index(&bindings)`：对每个排序后 Binding 的每条 blocker rule，写入 source unit/run/revision、`normalize_blocker_route(rule.route)` 得到的 route 和 target kind、排序去重后的 contract refs；以 `(source_unit_run_id, reason_code, allowed_route, target_contract_refs)` 稳定排序。

  把 index 写入 `GroupReviewMaterialSnapshotDraft`、repartition 时使用的 draft、`finalize` 返回值和 `draft_for_hash` 所序列化的内容。`compact_record` 只压缩身份、依赖、scope、contract 与 evidence；`trim_record` 仅在这些字段均不可再缩小时对真实超限 identity 返回既有 `unit_cross_review_record_exceeds_size_limit`。

- [ ] **Step 4: 运行材料单测，确认 hash 与 1,200-byte 边界**

  Run:

  ```sh
  cargo test --locked --lib material_
  ```

  Expected: routing authority index 在输入逆序时仍得到同一 content hash；9 条真实 rule 不再挤爆 unit record；超长 unit identity 仍失败关闭。

- [ ] **Step 5: 提交不可变 authority 索引与材料边界修复**

  ```sh
  git add src/product/coding_workspace_engine/group_review_types.rs src/product/coding_workspace_engine/group_review_material.rs src/product/coding_workspace_engine/tests/group_review_material.rs
  git commit -m "fix(group-review): move routing authority into snapshot index"
  ```

### Task 3: 将权限投影计入 shard 与 reduction Prompt

**Files:**

- Modify: `src/product/coding_workspace_engine/group_review_types.rs:7-27`
- Modify: `src/product/coding_workspace_engine/group_review_budget.rs:38-77`
- Modify: `src/product/coding_workspace_engine/group_review_prompts.rs:15-187`
- Modify: `src/product/coding_workspace_engine/tests/group_review_prompts.rs:1-240`
- Modify: `src/product/coding_workspace_engine/tests/group_review_budget.rs:13-96`
- Modify: `src/product/coding_workspace_engine/tests/group_review_e2e.rs:89-104`

**Interfaces:**

- Consumes: `GroupReviewMaterialSnapshot::routing_authority_index`、`GroupShardSpec::ordered_unit_run_ids` 与 `GroupPartitionResult::cross_shard_edges`。
- Produces: `PromptSegments::routing_authority` 和 `PromptBudgetBreakdown::routing_authority`；`build_shard_prompt` 的局部 JSON 段与 `build_reduction_prompt` 的全量 JSON 段。
- Invariant: `PromptSegments::join()` 与 `measure().total` 包含同一个 routing-authority 字节串，任何 `> GROUP_REVIEW_HARD_CAP_BYTES` 输入在 Provider 调用前失败。

- [ ] **Step 1: 为 Prompt 范围和预算写红灯测试**

  在 `group_review_prompts.rs` fixture 增加两个不同 Unit 的 authority entries 和一条跨片边。断言 shard prompt 只包含本 shard Unit 与跨片边端点所需条目，而 reduction prompt 包含二者：

  ```rust
  assert!(shard_prompt.routing_authority.contains("reason_run_a"));
  assert!(!shard_prompt.routing_authority.contains("reason_run_b"));
  assert!(reduction_prompt.routing_authority.contains("reason_run_a"));
  assert!(reduction_prompt.routing_authority.contains("reason_run_b"));
  assert_eq!(segments.measure().total, segments.join().len());
  ```

  同时把现有 `PromptSegments` fixture 补齐新字段并断言 `breakdown.routing_authority == segments.routing_authority.len()`。

- [ ] **Step 2: 运行 Prompt 与预算测试，确认新段尚未进入 Prompt**

  Run:

  ```sh
  cargo test --locked --lib group_review_prompts
  cargo test --locked --lib group_review_budget
  ```

  Expected: 编译错误或断言失败，因为 `PromptSegments` 与 `PromptBudgetBreakdown` 尚不存在 `routing_authority`，归约文本也没有合法 reason_code 集合。

- [ ] **Step 3: 实现局部/全量 authority 渲染及统一结论协议**

  为 `PromptSegments` 和 `PromptBudgetBreakdown` 增加 `routing_authority: String/usize`，在 `join` 与 `measure` 以相同顺序插入该段。新增一个纯 helper：

  ```rust
  fn authority_for_shard(
      snapshot: &GroupReviewMaterialSnapshot,
      shard: &GroupShardSpec,
  ) -> Vec<&RoutingAuthorityEntry>
  ```

  它取 shard member 加上所有 cross-shard edge 两端的 `source_unit_run_id`；`build_shard_prompt` 以 `render_json_section("routing_authority", &authority_for_shard(...))` 渲染。`build_reduction_prompt` 以同一标题序列化整个已排序 index。把协议中的“从相关 unit record 的 `routing_targets` 选择”改为“从提供的 `routing_authority` entry 选择”，并明确 implementation finding 必须保持 plan-defect 字段为空。

- [ ] **Step 4: 运行 Prompt/预算/E2E fixture 测试，确认 hard cap 仍生效**

  Run:

  ```sh
  cargo test --locked --lib group_review_prompts
  cargo test --locked --lib group_review_budget
  cargo test --locked --lib group_review_e2e
  ```

  Expected: 三个 Provider 的共用协议不变；分片不泄露无关 authority；归约 prompt 明确含完整合法 reason_code/route/ref 集；测量值与实际发送文本字节一致。

- [ ] **Step 5: 提交 Prompt authority 投影与预算计量**

  ```sh
  git add src/product/coding_workspace_engine/group_review_types.rs src/product/coding_workspace_engine/group_review_budget.rs src/product/coding_workspace_engine/group_review_prompts.rs src/product/coding_workspace_engine/tests/group_review_prompts.rs src/product/coding_workspace_engine/tests/group_review_budget.rs src/product/coding_workspace_engine/tests/group_review_e2e.rs
  git commit -m "fix(group-review): provide routing authority to shard and reduction prompts"
  ```

### Task 4: 在分片和归约持久化前校验快照 authority

**Files:**

- Modify: `src/product/coding_workspace_engine/plan_defect_routing.rs:165-252`
- Modify: `src/product/coding_workspace_engine/group_review_orchestrator.rs:176-320, 428-626`
- Modify: `src/product/coding_workspace_engine/internal_pr_review.rs:669-688`
- Modify: `src/product/coding_workspace_engine/tests/group_review_orchestrator.rs:430-670`
- Modify: `src/product/coding_workspace_engine/tests/group_review_reduction.rs:520-665`
- Modify: `src/product/coding_workspace_engine/tests/group_review_e2e.rs:900-1015`

**Interfaces:**

- Consumes: `ReviewFinding`、`RoutingAuthorityEntry`、shard member Unit Run IDs。
- Produces: `validate_group_review_finding_against_snapshot_authority(finding, authority, allowed_source_unit_run_ids) -> Result<(), PlanRepairError>`。
- Invariant: implementation finding 仅由既有普通 finding 规则校验；plan-defect finding 的 `reason_code`、`recommended_route`、`repair_target.kind` 与 `contract_refs` 必须匹配当前 prompt 所投影的某一 entry；失败前不得写成功 shard/reduction CAS，也不得继续分诊。

- [ ] **Step 1: 为 shard 与 reduction authority 校验写红灯测试**

  在 orchestrator fixture 构造合法的 snapshot authority 和一个 provider JSON，再新增以下两个输出：

  ```rust
  // shard: reason_code 属于另一个 shard，必须失败且没有可完成 report
  "{\"verdict\":\"request_changes\",\"findings\":[{\"defect_class\":\"verification_incomplete\",\"reason_code\":\"reason_run_b\",\"recommended_route\":\"verification_retry\",\"contract_refs\":[],\"repair_target\":null,\"confidence\":\"high\"}], ... }"

  // reduction: 路由或 contract_refs 不属于完整 index，必须 reduction_output_invalid
  ```

  断言 `execute_shards` 返回 `GroupReviewOrchestrationError::ShardOutputInvalid` 且 `list_group_review_shard_reports` 没有成功 report；断言 invalid reduction 不创建 `InternalPrReview`。

- [ ] **Step 2: 运行定向 orchestrator/reduction 测试，确认错误输出当前可能进入持久化路径**

  Run:

  ```sh
  cargo test --locked --lib group_review_orchestrator
  cargo test --locked --lib group_review_reduction
  ```

  Expected: shard 测试无法在 shard CAS 前拒绝跨 shard reason_code；reduction 测试只能依赖全 Binding 的后置校验，不能证明 prompt projection 与存储校验使用同一个 snapshot authority。

- [ ] **Step 3: 实现 snapshot-based 语义校验并接入 CAS 前路径**

  在 `plan_defect_routing.rs` 实现 authority-only validator：普通 implementation finding 复用普通 finding 结构校验；plan-defect finding 必须唯一匹配 `reason_code`、标准化 `recommended_route`、required target kind 和 contract-ref 子集，且 shard 调用传入的 Unit Run allowlist 必须包含匹配 entry 的 source Unit。无匹配、多个语义不同匹配或任一字段越权均返回 `PlanRepairError::InvalidFinding`。

  在 `build_and_store_shard_report` 解析并检查 finding 数量之后、`write_group_review_shard_report_cas` 之前调用该 validator；在 `execute_reduction` 对 provider payload 的 reduction findings 进行同一校验，然后才 `merge_findings`、写 reduction report 和创建 InternalPrReview。无效时延用当前的 `shard_output_invalid` 或 `reduction_output_invalid` 失败报告与 lease 释放逻辑。`internal_pr_review.rs` 不再为组级最终结论重新导出可漂移的 projection bindings；以 snapshot authority 作为同次运行的唯一校验输入。

- [ ] **Step 4: 运行语义校验与回归测试**

  Run:

  ```sh
  cargo test --locked --lib group_review_orchestrator
  cargo test --locked --lib group_review_reduction
  cargo test --locked --lib group_review_e2e
  cargo test --locked --lib group_review_compat
  ```

  Expected: 只有合法 authority 的 plan-defect finding 可落库；普通 implementation finding 仍可通过；无效 shard/reduction 均落现有 output-invalid 门禁且不生成成功 `InternalPrReview`。

- [ ] **Step 5: 提交 authority 语义校验**

  ```sh
  git add src/product/coding_workspace_engine/plan_defect_routing.rs src/product/coding_workspace_engine/group_review_orchestrator.rs src/product/coding_workspace_engine/internal_pr_review.rs src/product/coding_workspace_engine/tests/group_review_orchestrator.rs src/product/coding_workspace_engine/tests/group_review_reduction.rs src/product/coding_workspace_engine/tests/group_review_e2e.rs
  git commit -m "fix(group-review): validate findings against snapshot authority"
  ```

### Task 5: 全链路验证、OpenSpec 工作包回填与服务重启

**Files:**

- Modify: `openspec/changes/scalable-group-final-review/tasks.md`
- Verify: `src/product/coding_workspace_engine/tests/group_review_material.rs`
- Verify: `src/product/coding_workspace_engine/tests/group_review_prompts.rs`
- Verify: `src/product/coding_workspace_engine/tests/group_review_orchestrator.rs`
- Verify: `src/product/coding_workspace_engine/tests/group_review_reduction.rs`
- Verify: `tests/it_product/product_coding_workspace_engine/part_01.rs`

**Interfaces:**

- Consumes: Task 1-4 的 owner cleanup、authority index、Prompt 与语义校验。
- Produces: 可复现的 Rust 验证证据、完成的 OpenSpec 工作包勾选，以及使用新 aria 二进制的新 GroupFinalReview 手工验证。

- [ ] **Step 1: 执行格式、定向测试和全量 Rust 测试**

  Run:

  ```sh
  cargo fmt --check
  cargo test --locked --lib group_review_
  cargo test --locked --test it_product product_coding_workspace_engine::part_01::failed_group_attempt_releases_transferred_shared_worktree_lock_by_owner
  cargo test --locked
  ```

  Expected: 所有命令成功；若全量测试遇到 Provider 503/504 或 worker 超时，仅重试失败的本地测试命令并记录该环境错误，绝不把它伪报为测试通过。

- [ ] **Step 2: 检查变更范围与 OpenSpec 可追溯性**

  Run:

  ```sh
  git diff --check HEAD~4..HEAD
  openspec validate scalable-group-final-review --strict
  ```

  Expected: 无空白错误，OpenSpec 严格校验通过；仅在以上测试和手工验证均为绿色后，将 tasks 2.3、3.3、4.1、5.1、5.3、6.5、8.3、8.5 标为完成。

- [ ] **Step 3: 重启后端，确保运行的是新二进制**

  停止现有 aria 进程并以本 worktree 的 `feat-b-0730-add-pi` 二进制重新启动后端；不要依赖 `cargo watch` 自动换进程。用健康检查确认监听 `4317` 的 PID 在重启后变化，前端按既有 `pnpm dev` 流程重启到 `5173`。

- [ ] **Step 4: 用新 attempt 手工验证 GroupFinalReview**

  在前端创建全新的 group Coding Attempt（不能复用失败 attempt），推进到 GroupFinalReview 并触发。验证：

  ```text
  1. 不出现 coding_start_failed/product_store_conflict/issue_worktree_lock_owner。
  2. Provider 若返回合法 plan defect，归约可识别合法 reason_code；若返回非法 reason_code，显示 reduction_output_invalid 而不产生错误路由。
  3. 人为或已知材料失败后，同一 issue 可以新建 attempt；共享 worktree 中 current_active_work_item_id 和 current_lock_owner_id 均为空。
  ```

- [ ] **Step 5: 提交验收产物并准备代码审查**

  ```sh
  git add openspec/changes/scalable-group-final-review/tasks.md
  git commit -m "test(group-review): verify lock and routing authority regressions"
  ```

  完成后调用 `verification-before-completion` 读取新鲜证据，再调用 `requesting-code-review`；审查通过后才执行 OpenSpec sync/archive。
