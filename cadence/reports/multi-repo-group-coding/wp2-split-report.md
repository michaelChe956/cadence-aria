# WP2 拆分报告：按 target 拆分创建+per-attempt 快照绑定+advance 绑定扩展（T2 / REQ-MTG-01/02/03，REQ-ADV-01/02）

- **change**: `openspec/changes/multi-repo-group-coding/`
- **计划**: `cadence/plans/2026-09-19_计划文档_阶段4-C2_多仓group编码_v1.0.md`（v1.1）Task 2
- **状态**: **部分交付（分批纪律中途交接）**——Step 2/Step 4 全量落地 + Step 3 存储面落地（两批 commit）；Step 3 的 advance.rs 分流循环（k3 F3 必改点一）与 engine 级多 target 测试族**未实施**，见「六、剩余工作」。

## 一、已交付清单（按 commit）

### Batch 1 —— `4830bf03`：advance additive 集绑定+判定扩展+map 多 target（Step 2 + Step 4 全量）

| 项 | 内容 |
|---|---|
| `AdvanceTargetAttemptBinding`（新增，advance_store.rs） | `{target_repository_id: String, attempt_id: String}`（OQ1 定案形态） |
| `AdvanceRecord.target_attempts` | additive（serde default+空集 skip=存量零迁移；存量 JSON 反序列化→空集，序列化不落键） |
| `AdvanceInitializationJournal.target_attempt_ids` | additive（serde default；`attempt_id` 恒为拓扑序首个） |
| `advance_is_ready_for_attempt` | 判定=「`attempt_id` 精确匹配 OR 属于 `target_attempts` 集」；缺记录/未 Ready/双不匹配一律 false（fail-closed 零变化） |
| `AdvanceOutcome::Completed.target_attempts` | 载荷扩展（单 target 空） |
| WS `AdvanceCompleted.target_attempts` | additive（serde default+空集 skip——单 target wire 零变化，测试钉死序列化无该键） |
| `map_advance_outcome`（k3 F3 必改点二） | Replayed：多 target record（集非空）以「集合非空+workspace_entry 在」为完备判据；单 target 保持 (attempt_id, workspace_entry) 双在场零变化 |
| `load_or_prepare_advance_initialization_for_attempts` | 外层 journal 集绑定形态（集合一致性比对，顺序不敏感）——单 target 旧签名委托 |
| `start_attempt` 守卫接线 | 无需改动（:207-209 调 `advance_is_ready_for_attempt`，判定函数已扩展自动生效）；守卫族测试锚定 |

**测试（红→绿）**：`advance_ready_judgment_accepts_target_attempt_set_membership`、`advance_ready_judgment_fails_closed_when_not_ready_or_unbound`、`advance_ready_judgment_keeps_exact_match_semantics_for_single_target`、`legacy_advance_record_json_deserializes_without_target_attempts`（存量逐字节等价 #7）、`advance_initialization_journal_legacy_json_defaults_target_attempt_ids`、`multi_target_ready_replay_maps_to_completed_with_target_attempts_payload`、`multi_target_ready_replay_without_workspace_entry_stays_incomplete`、`single_target_replay_without_attempt_id_stays_incomplete`、`single_target_completed_wire_omits_target_attempts`（wire 零变化）。回归：守卫族+wire 序列化 16/16。

### Batch 2 —— `942a8373`：per-(plan,target) 唯一性+per-(issue,target) 单 active+per-target journal 子路径（Step 3 存储面）

| 项 | 内容 |
|---|---|
| `get_attempt_for_work_item_group(…, target: Option<LogicalRepositoryId>)` | 检索升级 per-(plan,target)（消解「取最早」歧义）；`None`=无快照桶（legacy 语义保持——routing Legacy 时全部 attempt 无快照，行为零变化） |
| `list_attempts_for_work_item_group`（新增） | plan 级全量列表（attempt_no 升序）——删除门禁/投影/amendment 路由消费 |
| `create_group_attempt` 双前置细化 | per-(plan,target) 唯一（A1：无快照 attempt 不参与判定）+ per-(issue,target) 单 active（A4：无快照 active 不计桶） |
| `prepare_group_initialization_with_admission_for_target`（新增） | OQ2 journal 路径规则：`Some(t)` → `group-initializations/{plan_id}/{t}.json` 子目录（journal id 追加 target UUID 限定）；`None`/旧签名 → 原路径 `{plan_id}.json` 零迁移。attempt 级前置按输入快照 target 桶细化 |
| `list_group_initialization_journals_for_plan`（新增） | 读取先试原路径再列子目录（按 target UUID 升序确定性） |
| `locate_group_initialization_journal_path`（新增私有） | mark/advance 相位函数与 delete 的 journal 定位规则（先原路径后子目录，按 id 匹配；delete 对原路径 mismatch 保持原 fail-closed 报错、子目录 sibling 跳过不误删） |
| `ensure_group_initialization_attempt` / `validate_materialized_group_initialization_attempt` | 「另一 active attempt」检查按 target 桶细化——异 target active 不阻塞（2.2 异 target 并行解禁） |
| `topologically_order_unit_bindings` | 导出 `pub(crate)`（advance.rs 分流循环消费——Batch 3 接线） |
| `group_initialization_journal_path_for_target`（paths.rs 新增） | 子路径 helper |

**测试（红→绿，`tests/group_split_target.rs` 7 项）**：`same_target_second_attempt_rejected_per_plan_target`（#4）、`per_target_retrieval_returns_only_matching_bucket`（A2）、`legacy_bucket_retrieval_keeps_snapshotless_attempts`（单 target 零变化）、`snapshotless_attempt_never_blocks_or_pollutes_target_buckets`（A1/A2/A4）、`single_active_refined_per_issue_target`（#5）、`split_journal_lives_in_target_subpath_and_lists_back`（含幂等重放）、`single_target_journal_stays_on_original_path`、`split_journal_phase_advance_locates_subpath`、`split_ensure_attempt_allows_other_target_active`。

## 二、消费面改造表（OQ4 留痕）

| 消费点 | 改造 |
|---|---|
| `group_initialization.rs`（prepare 内既有 attempt 前置） | per-(plan,target) 桶（输入快照键） |
| `coding_attempt_store/group.rs:109`（cfg(test) ensure_group_attempt_for_advance） | 同上 |
| `coding_attempt_store/group.rs:150`（create_group_attempt） | per-(plan,target) 唯一+per-(issue,target) 单 active 双细化 |
| `plan_amendment_context.rs:186` | 遍历 plan 全量 attempt 找 open/applying context（单 target 语义零变化；Ambiguous 检查保留） |
| `web/handlers/lifecycle/deletion.rs:42` 与 `:188` | 门禁改 plan 全量（任一 attempt 在即拒）；`purge_attempt_lock_residue` 增 `{plan_id}/` 子目录整目录清理 |
| `web/handlers/lifecycle.rs:169` | 投影/DTO 组装改全量迭代；`latest_attempt` 按 unit 归属 attempt（`unit.attempt_id` 定位） |
| 测试面调用点（随改随列） | `test_controls/plan_repair/{seed.rs×3, recovery.rs, topology.rs, provider_matrix/coding.rs}`、`workspace_ws_handler/tests/plan_repair_activation.rs`、`coding_workspace_engine/tests/{group_failure_same_attempt.rs, group_amendment_chain.rs}`、`workspace_engine/tests/advance_handler.rs×4`、`it_web/{web_coding_attempt_api/part_10.rs, web_work_item_plan_repair/part_04.rs×3}`（均传 `None`=legacy 桶，行为零变化） |

## 三、D2.1 四断言对应测试（存储面）

| 断言 | 测试 |
|---|---|
| A1 唯一性豁免 | `snapshotless_attempt_never_blocks_or_pollutes_target_buckets` |
| A2 不静默归属 | `per_target_retrieval_returns_only_matching_bucket`、`snapshotless_attempt_never_blocks_or_pollutes_target_buckets` |
| A3 fail-closed 路由 | T1 已钉死（`logical_repository_for_group_attempt_routes_by_frozen_snapshot_first` 等——本批无改动） |
| A4 单 active 桶豁免 | `snapshotless_attempt_never_blocks_or_pollutes_target_buckets`（active 不阻塞异 target）、`split_ensure_attempt_allows_other_target_active` |

## 四、REQ-ADV-01/02 对应（Step 2/4 已落）

- 守卫 fail-closed（#6）：`advance_ready_judgment_*` 三测试 + 既有守卫族（`sc_coding_start_without_advance_is_rejected` 等）16/16 绿。
- 存量兼容（#7）：`legacy_advance_record_json_deserializes_without_target_attempts`。
- 每 plan 恰一条 record：`put_record` 冲突语义未动（`advance_record_store_supports_both_idempotency_indexes` 等 3 既有测试绿）。

## 五、单 target 零变化验证

- 定向回归 125/125（advance_handler 全族+advance_checkpoint 恢复+advance_rejects/replay+coding_attempt_store::tests 全量+group_uniqueness+amendment chain 相关）。
- Batch 1 时另跑守卫族+wire 序列化 16/16。

## 六、剩余工作（Batch 3 交接——**未实施**）

1. **advance.rs 分流循环（k3 F3 必改点一）**：`initialize_advance_inner` 显式两分支——`units_by_target`（T1 辅助）键数 ≤1 走现行代码原样；≥2 进 per-target 循环（OQ2 命名 branch=`aria/issues/{issue_id}/{logical_id}`、worktree=`.worktrees/aria-issues/{issue_id}/{logical_id}`、base_branch=各仓当前分支、per-target 快照 `build_attempt_target_snapshot`）→ 各自 `prepare_group_initialization_with_admission_for_target(Some(t))`（**已就位**）→ `ensure_group_initialization_attempt` → repo 维三元键 worktree 三件套（`upsert_repo_shared_worktree`/`try_acquire_repo_worktree_lock`/`bind_repo_worktree_lock_to_attempt`——底座已就位）→ units 分组物化。外层 journal 用 `load_or_prepare_advance_initialization_for_attempts`（**已就位**）；断言 :508-515 升级集合一致性；`ready_record.attempt_id`=拓扑序首个+`target_attempts` 全集；失败标记 :357-386 适配（`list_group_initialization_journals_for_plan` 已就位——error 落每个 per-target journal+record，attempt 集身份保留）。
2. **worktree 嵌套共存检查**（OQ2 披露，T2S3 创建前检查父路径）：per-target worktree 父路径 `.worktrees/aria-issues/{issue_id}` 若已被 issue 级/既有 repo 级 worktree 记录占用（`get_issue_shared_worktree`/`get_repo_shared_worktree` 比对 worktree_path）→ fail-closed；通过后以 repo 维 upsert 登记。
3. **审计挂点占位**：`record_split_audit` 空实现（`#[allow(dead_code)]`+T5 注释），本 Task 不接调用（T5 落地真实现+接线）。
4. **engine 级多 target 测试族**（Step 1 #1/#2/#3/#8/#9/#10 的 engine 面）：多 target 建组产出 N attempts（各自快照+units 分组）/幂等（同/异 command_id 同 plan）/failpoint×多 target 恢复同套/record 绑定（首个+全集）/无自动编排（N attempts 全部停留 Created+PrepareContext、provider ledger 空）/半启动恢复 per-attempt 等价。夹具路线：PlanRepairFixture seed + 两 logical 仓（`with_logical_codebase_feature`+manifest+selection+bootstrap policy，T1 `split_resolution_fixture` 模式）+ 草稿 per-unit target 改写。
5. **隔离 worktree 终验**：主树并行兄弟中间态可能挡 `cargo test --lib`——C2T1 先例（干净 HEAD+仅本任务文件 diff 的隔离树复跑）。
6. wp2-split-report 本文件在 Batch 3 落地后补齐「2.1-2.4 证据清单全量+无自动编排断言+单值 schema 零改动自查（execution.rs :114-159 无 diff）」。

## 七、TDD 红绿证据

- Batch 1 红：编译红 11 errors（`target_attempts`/`target_attempt_ids`/`AdvanceTargetAttemptBinding`/`AdvanceOutcome::Completed`/WS 字段未定义）→ 绿 17/17+16/16。
- Batch 2 红：编译红 E0061×N+E0599（4 参签名/`list_attempts_for_work_item_group`/`prepare_..._for_target` 未定义）→ 绿 11/11（初版 1 failed=测试按创建序断言，实现在 target UUID 序确定性排序，修正断言为集合等价后绿）→ 回归 125/125。
