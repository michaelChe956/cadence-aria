# 阶段 4 C2 T1（WP1 解禁）执行报告

- **change**: `openspec/changes/multi-repo-group-coding/`（REQ-COD-04 RENAMED+MODIFIED=按 target 分流）
- **计划**: `cadence/plans/2026-09-19_计划文档_阶段4-C2_多仓group编码_v1.0.md`（v1.1）Task 1 / WP1.1-1.3
- **工作树**: `.worktrees/feat-b-0808-add-monorepo`（HEAD 基线 `6eb88e51`；计划锚点基于 `b856de99`，五处锚点实读复核零漂移）
- **详细留痕**: `cadence/reports/multi-repo-group-coding/wp1-unblock-report.md`（改造清单+行为/错误码语义变化表+scenario 对应+红绿证据+回归表）

## 交付摘要

三处校验点+两路由消费面分流化，mixed-target 解析不再一律拒绝：

1. **创建面** `group.rs`：新增 `split_group_targets`/`group_target_snapshots`/`resolve_group_repositories` 多值解析面（≥2 target 逐仓快照/仓 map；0-target focus 唯一回落原样；单值函数零变化，T2 消费）。
2. **恢复/replay 面** `coding_attempt_repository.rs`：**零改动**（fix round 1 P3——快照优先由调用方入口 `resolve_coding_attempt_repository` :50-92 既有行为提供，函数内曾加分支不可达已删，文件回退至与本 Task 前逐字节一致）；无快照保持现行收敛（D2.1 A3 锚）。
3. **评估上下文面** `builder.rs`：`schema_v2_evaluation_context_repository_id` 签名 per-attempt + 快照优先（多 focus 不阻断；快照受 selection 成员 TargetUnknown + `validate_snapshot_fields` 权威身份 Inconsistent 双 fail-closed 约束——fix round 1 P2 补齐身份校验，漂移快照不得静默路由）。
4. **plan 会话面** `workspace_repository.rs`：`plan_session_repository_target`——多 target+focus 唯一回落成功；0-target 保持 TargetMissing；多 focus 保持 TargetAmbiguous（双审定案全部落实）。
5. **分组辅助** `group_validation.rs`：`units_by_target`+`UnitsByTarget`（无归属 unit 不入桶，T2 复用）。

## 定案落实核对（双审钉死项）

| 定案 | 落实 |
|---|---|
| WorkItemPlan 面 0-target 保持 TargetMissing（fail-closed 红线） | `plan_session_target_zero_targets_stays_target_missing_even_with_unique_focus` 钉死 |
| 多 target+focus 唯一→回落成功 | `plan_session_target_multi_target_falls_back_to_unique_focus`（运行时红→绿） |
| 多 focus→TargetAmbiguous | builder/plan 两面 `stays_ambiguous` 测试钉死（focus 面不退役） |

## 验证证据

- **红**：编译红 17 errors（新 API 未定义+签名 E0061）→ 运行时红 3 failed/14 passed（现行 `repository_routing_ambiguous`×2+收敛 `inconsistent`×1 在目标场景必红，B1=签名重构不改行为中间态捕获）。
- **绿**：定向 17/17（主工作树）；隔离 worktree（干净 HEAD+仅本任务五文件 diff）复跑 17/17——排除兄弟 agent 并行编辑干扰。
- **单 target 回归零变化**（Step 3 五 filter）：`group_target_snapshot` 6 绿 / `logical_repository_for_group_attempt` 1 绿 / `workspace_repository` 7 绿 / `evaluation_context` 17 绿 / `resolve_group_repositories` 由 17 绿批次覆盖。
- **门禁**：`cargo fmt`（限定五文件）+ `cargo clippy --locked --lib -- -D warnings` 在隔离树全绿（主树瞬时被兄弟中间态挡，见 concerns）。
- **冻结面**：`advance.rs`/`group_initialization.rs`/`validate_group_single_target` 零触碰（git diff 可证）。

## Commits

- `c16e497c` feat(coding-ws): mixed-target group resolution unblocked via per-target split parsing（WP1 主体：src 五文件+两报告）。
- fix round 1 提交（本提交；基于 `32dc4de8`，含 builder.rs 身份校验+coding_attempt_repository.rs 回退+两报告修订——hash 见 `git log` 本行）。

## Concerns

1. 主工作树同批有兄弟 agent 编辑 `admission.rs`/`coding_ws_handler/*`/`work_item_plan_compiler/*`（及归属待查的 `workspace_engine/tests/single_candidate/contract_autorepair.rs`——已向 F16Fix 澄清非本任务文件）；crate 级全量验证以主 agent 收口为准，本任务以隔离树证据交付。
2. 多 target 建组全流程在 T2（本 Task 后 mixed-target 建组仍走不通创建编排——`group_initialization.rs:617` per-attempt 单值校验按设计保留，预期中间态，T1 验收口径=分流解析成功）。

## fix round 1（k3 审 1×P2+1×P3）

- **P2**：builder 快照分支补 `validate_snapshot_fields`（恢复面同形态先例）；正向测试换 `build_attempt_target_snapshot` 真实权威快照（夹具补 `ensure_bootstrap`），新增 `schema_v2_repository_id_rejects_drifted_snapshot` 负向（伪造 git_dir_identity→`repository_routing_inconsistent`——校验生效证明）。
- **P3**：删 `logical_repository_for_group_attempt` 不可达快照分支+直调测试（controller 定案选删除）；语义变化表该行修正为「由调用方入口既有行为提供」；`coding_attempt_repository.rs` 与本 Task 前逐字节一致。
- **验证**：隔离 worktree（`c16e497c`+fix diff）定向 20/20 passed + clippy `-D warnings` 全绿；`rustfmt --edition 2024` 限两文件（主树 `cargo fmt` 被 C2T2 兄弟的 advance_store.rs 中间态挡——整 crate 解析依赖）。
