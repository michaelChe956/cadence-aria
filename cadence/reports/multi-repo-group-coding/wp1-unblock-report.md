# WP1 解禁报告：mixed-target 解析分流化（T1 / REQ-MTG-01，REQ-COD-04 RENAMED+MODIFIED）

- **change**: `openspec/changes/multi-repo-group-coding/`
- **计划**: `cadence/plans/2026-09-19_计划文档_阶段4-C2_多仓group编码_v1.0.md`（v1.1）Task 1
- **基线**: 计划锚点基于 `b856de99`；实施时 HEAD=`6eb88e51`，五处锚点实读复核全部未漂移（`group.rs` :288-364/:376-473、`coding_attempt_repository.rs` :235-285、`workspace_repository.rs` :96-123/:256-271、`builder.rs` :293-341）。
- **边界**: T2 冻结面 `advance.rs`/`group_initialization.rs` 零触碰；`validate_group_single_target`（per-attempt 单值完整性校验）保持不动——分流后每 target-attempt 仍单 target，该校验语义原样适用。

## 一、改造清单（三处校验点 + 两路由消费面 + 分组辅助）

### 1. 创建面（`src/web/handlers/coding/group.rs`）

新增多值解析面（既有单值函数 `group_target_snapshot`/`resolve_group_repository` 原样保留，创建编排仍走单值路径——多 target 建组全流程在 T2 落地，本 Task 后属预期中间态）：

| 函数 | 行为 |
|---|---|
| `split_group_targets`（新增，私有前置） | routing 三态 + `source_draft_error`→Inconsistent fail-closed + `validate_logical_group_selection` + units target 收敛 + **0-target focus 唯一回落**（focus 不唯一→TargetMissing） |
| `group_target_snapshots`（新增） | `-> ApiResult<Option<BTreeMap<LogicalRepositoryId, AttemptTargetSnapshot>>>`；**≥2 target 不再拒绝**，逐 target 过 selection 成员校验（TargetUnknown fail-closed）后产出冻结快照；单 target 退化单条目（语义与单值面零变化）；Legacy→`None` |
| `resolve_group_repositories`（新增） | 同构多值变体，逐 target 解析 `RepositoryRecord`；前置/错误码语义与上者一致 |

两函数标注 `#[allow(dead_code)]`——T2（WP2 分流创建循环）消费，WP1 只落解析面（计划 Interfaces 钉死）。

### 2. 恢复/replay 面（`src/product/coding_attempt_repository.rs`）

| 函数 | 行为 |
|---|---|
| `logical_repository_for_group_attempt` | **零改动**（fix round 1 P3：曾加的首行快照优先分支经 k3 审定为不可达——调用方入口 `resolve_coding_attempt_repository` :50-92 对带快照 attempt 已先行短路并按快照路由；该保证**由调用方入口既有行为提供**，函数内重复分支已删）。无快照保持现行收敛（`1`→唯一 / `0`→focus 唯一回落，否则 TargetMissing / `_`→TargetAmbiguous）——D2.1 A3 锚：无快照存量退化走原路 |

### 3. 评估上下文面（`src/product/coding_evaluation_context/builder.rs`）

| 函数 | 行为 |
|---|---|
| `schema_v2_evaluation_context_repository_id` | 签名改 per-attempt（`(paths, attempt)`，project/issue 从 attempt 取）；Logical 分支在 selection 校验后**快照优先**：有快照按快照 target 解析（快照 target 仍须在有效 selection 成员内 TargetUnknown fail-closed + `validate_snapshot_fields` 权威身份逐字段校验 Inconsistent fail-closed——fix round 1 P2 补齐，与恢复面 `resolve_coding_attempt_repository` 同形态先例，漂移快照不得静默路由），**不再经 selection focus 收敛**；无快照走现行 focus match 原文不动 |

### 4. plan 会话路由面（`src/product/workspace_repository.rs`）

| 函数 | 行为 |
|---|---|
| `plan_session_repository_target`（新增 helper，WorkItemPlan 分支接线） | `0` target→TargetMissing（**保持现行，focus 唯一也不回落**——双审定案）；`1`→唯一（现行零变化）；**≥2→回落 selection focus 唯一**（与创建面 0-target focus 语义对称），focus 不唯一→TargetAmbiguous（现行错误码/文案原样） |

WorkItem 分支（按 `work_item.target_repository_id` 单值）天然分流，未动。

### 5. 分组辅助（`src/product/coding_attempt_store/group_validation.rs`）

| 项 | 行为 |
|---|---|
| `UnitsByTarget` + `units_by_target`（新增 pub） | units 按 `target_repository_id` 分桶；无归属 unit 入 `unattributed` 不入任何桶（交调用方按 0-target/focus 规则处置）；纯分组不判定——`source_draft_error` 前置 fail-closed 校验仍由调用方完成（语义不变）。T2 分流创建消费 |

## 二、行为/错误码语义变化表（TargetAmbiguous 退役 vs 保留）

| 消费面 | 场景 | 改前 | 改后 |
|---|---|---|---|
| 创建面（多值 `group_target_snapshots`/`resolve_group_repositories`） | units 多 target | `repository_routing_ambiguous`（单值面 `_` 分支拒绝） | **成功**：逐 target 快照/仓 map（TargetAmbiguous 退役） |
| 恢复/replay 面（快照 attempt 路由） | attempt 带快照 | 调用方入口 `resolve_coding_attempt_repository` :50-92 已按快照路由（含成员+身份校验） | **零变化**——该保证由调用方入口既有行为提供（fix round 1 P3 删除函数内不可达重复分支）；无快照多 target 仍 ambiguous（A3 存量退化保持） |
| 评估上下文面 builder | attempt 带快照 + selection 多 focus | `repository_routing_ambiguous`（focus match） | **成功**：按快照路由（ambiguous 在该输入形态退役） |
| **selection focus 面** builder（无快照 attempt） | selection 多 focus | `repository_routing_ambiguous` | **保持** `repository_routing_ambiguous`（focus 面保留，测试钉死） |
| **plan 会话面** WorkItemPlan | 多 target + focus 唯一 | `repository_routing_ambiguous` | **成功**：focus 回落（对称解禁） |
| **plan 会话面** WorkItemPlan | 多 target + 多 focus/无 focus | `repository_routing_ambiguous` | **保持** `repository_routing_ambiguous`（focus 面保留） |
| **plan 会话面** WorkItemPlan | 0 target（含 focus 唯一） | `repository_routing_target_missing` | **保持** `repository_routing_target_missing`（fail-closed 红线，双审定案） |
| 创建面（单值 `group_target_snapshot` 原函数，T2 前仍被建组编排调用） | 全场景 | 三分支原样 | **零变化**（单 target（含 0-target focus 唯一）走既有代码路径） |
| 无归属 fail-closed（各面） | units 无 target 且 focus 不唯一/无 focus | `repository_routing_target_missing` | **保持**（REQ-COD-04 scenario「无唯一 target 归属仍 fail-closed」） |
| TargetUnknown / Inconsistent / SelectionInvalidated | selection 外 target / draft 断链 / 失效 | 稳定码拒绝 | **保持**（fail-closed 纪律不随解禁放松） |

## 三、REQ-COD-04 delta scenario 逐条对应

| Scenario | 对应测试（全绿） |
|---|---|
| 尝试 mixed-target group（分流解析成功，不再多目标歧义拒绝） | `group_target_snapshots_resolves_mixed_targets_per_target`、`resolve_group_repositories_resolves_mixed_targets_per_target`、`schema_v2_repository_id_routes_by_frozen_snapshot_over_multi_focus`（真实权威快照）、`schema_v2_repository_id_rejects_drifted_snapshot`（漂移快照 fail-closed——身份校验生效证明）、`plan_session_target_multi_target_falls_back_to_unique_focus`；恢复面快照路由由入口既有测试 `repository_resolver_resolves_new_lc_attempt_from_lc_subtree` 等承接 |
| 无唯一 target 归属仍 fail-closed | `group_target_snapshots_zero_target_without_unique_focus_fails_closed`（TargetMissing 稳定码）、`plan_session_target_zero_targets_stays_target_missing_even_with_unique_focus`、`group_target_snapshots_target_outside_selection_fails_closed`（TargetUnknown） |
| 同 target 的 group 行为零变化 | `group_target_snapshots_single_target_keeps_single_entry`、`plan_session_target_single_target_resolution_unchanged`、`group_target_snapshots_zero_target_unique_focus_falls_back_to_focus`（0-target focus 唯一回落原样保留）+ Step 3 五组定向回归全绿 |

## 四、TDD 红绿证据

1. **编译红（新 API 面）**：红测先行写入后 `cargo test --locked --lib` 17 errors——`group_target_snapshots`(×6)/`resolve_group_repositories`(×1)/`plan_session_repository_target`(×5)/`units_by_target`(×2) 未定义 + `schema_v2_evaluation_context_repository_id` 签名 E0061(×3)。
2. **运行时红（三个行为点，B1=签名重构不改行为中间态）**：14 passed / 3 failed——
   - `plan_session_target_multi_target_falls_back_to_unique_focus` → `repository_routing_ambiguous: work_item_plan_0001 has multiple logical repository targets`（现行拒绝行为在多 target+focus 唯一场景必红）
   - `schema_v2_repository_id_routes_by_frozen_snapshot_over_multi_focus` → `repository_routing_ambiguous: issue codebase selection has multiple focus repositories`
   - `logical_repository_for_group_attempt_routes_by_frozen_snapshot_first` → `repository_routing_inconsistent: schema-v2 group target cannot be resolved from the authoritative plan`（落入现行收敛路径）——**fix round 1 P3 已删该测试与对应不可达分支**：入口 `resolve_coding_attempt_repository` :50-92 既有短路已提供同语义，函数内重复分支不可达
3. **绿（B2 行为落定后）**：定向 17/17 passed（`cargo test --locked --lib` 过滤本报告全部测试名）。

## 四-bis、fix round 1（k3 审 1×P2+1×P3）

- **P2（builder 快照分支缺身份校验）**：快照优先分支补 `validate_snapshot_fields`（`logical_codebase::snapshot_validator`，对照恢复面同形态先例）——漂移快照（如伪造 `git_dir_identity`）不再静默路由，Inconsistent fail-closed；正向测试夹具换 `build_attempt_target_snapshot` 真实权威产物，并新增 `schema_v2_repository_id_rejects_drifted_snapshot` 负向（校验生效证明）。夹具补 `AggregatePolicyArtifactStore::ensure_bootstrap`。
- **P3（`logical_repository_for_group_attempt` 快照分支不可达）**：按 controller 定案删除该分支及直调私有函数的测试（`coding_attempt_repository.rs` 回退至与本 Task 前逐字节一致）；语义变化表该行修正为「由调用方入口既有行为提供」（§二）。
- **验证**：隔离 worktree（`c16e497c`+fix diff，排除兄弟并行编辑干扰）定向 20/20 passed（WP1 测试族 17+漂移负向，另含既有 `repository_resolver_*`/`schema_v2_group_detection`）+ `cargo clippy --locked --lib -- -D warnings` 全绿；`rustfmt --edition 2024` 限两文件。

## 五、定向回归（Step 3，单 target 零变化锁）

| filter | 结果 |
|---|---|
| `group_target_snapshot` | 6 passed |
| `resolve_group_repository`（子串不含复数形，由 17 绿批次覆盖 `resolve_group_repositories_resolves_mixed_targets_per_target`） | 17 绿批次覆盖 |
| `logical_repository_for_group_attempt` | 1 passed |
| `workspace_repository` | 7 passed（含 2 既有路由分类测试零改） |
| `evaluation_context` | 17 passed（含既有 fail-closed 测试改签名调用后语义零改） |

## 六、遗留与移交

- 多 target 建组**完整创建编排在 T2**（本 Task 后 mixed-target 建组仍不能走通全流程——`group_initialization.rs:617` per-attempt 单值校验按设计保留，属预期中间态，T1 验收口径=分流解析成功）。
- `units_by_target`/`group_target_snapshots`/`resolve_group_repositories` 为 T2 消费面；快照优先路由（恢复/评估面）对 T2 产出的 per-target attempt 即时生效。
- 实施期间同工作树有兄弟 agent 编辑 `admission.rs`/`coding_ws_handler/*`/`work_item_plan_compiler/*`；本 Task 提交显式列文件隔离。

## 七、fix round 后修订说明

§一/§二/§三/§四 中恢复/replay 面相关行已按 fix round 1 P3 修正；初版中该面「首行快照优先」表述作废，以本版为准（红绿证据保留历史并标注删除）。
