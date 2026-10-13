# N1 报告：split 与运行 gate 依赖同源（全图校验 + 桶内排序）

- Change：`openspec/changes/fix-multi-repo-group-coding-start/`（Task 1.1，REQ-CS-03/08）
- Plan：`cadence/plans/2026-10-13_实施计划_多仓分组coding启动_v1.0.md` §2
- Worktree：`.worktrees/feat-b-0808-add-monorepo`（分支 feat-b-0808-add-monorepo）
- 状态：DONE（红→绿→回归全绿；group_validation.rs 零修改、advance_store 零修改）

## 问题（基线缺陷）

A1→B1→A2 形态的合法跨仓计划下，创建面与运行面按不同来源判定依赖：

1. 创建面：`advance_split`/`prepare_group_initialization_with_admission_for_target`/
   `validate_group_attempt_structure`（group_validation.rs:349）对 **target 桶** 调用全集语义的
   `topologically_order_unit_bindings`——桶外合法端点被当 unknown 拒绝（`mixed_target_group_rejected`
   /IdentityMismatch 误拒）。
2. 运行面：`select_next_sc_group_unit` 仅以本 attempt units 验证全图边——桶外合法端点误报
   `SC_GROUP_DEPENDENCY_UNKNOWN` 失败关闭，而非 Waiting。

## 修正（同源语义：全图校验 → 桶内排序 → 外部 depends 保留）

- `group_initialization.rs`
  - `topologically_order_unit_bindings`：端点不在输入集内的依赖为**合法外部边**——保留在返回
    binding、不参与入度/排序；重复 WI、声明列表内重复依赖、self 边、集内环仍失败关闭。
  - 新增 `topologically_order_target_unit_bindings(all, target)`：先显式核 **真 unknown**（端点不在
    权威全集内）再复用全图 helper 校验（重复/self/全局环，含跨桶环，不按仓缩图），随后仅以桶内边
    排 target 桶（同一 helper、同一并列打破规则），空桶失败关闭，返回 binding 保留全部原
    `dependency_logical_work_item_ids`（plan gate 消费）。
  - `prepare_group_initialization_with_admission_for_target`：ScAdvance + `journal_target=Some(t)` 时
    `unit_bindings` 期望权威全集——内联消费上述 target 排序；`None` 保持全集拓扑序旧语义（对既有
    调用方零变化）。
- `advance_split.rs`：per-target 桶排序改走 `topologically_order_target_unit_bindings(&authoritative.units,
  target)`；prepare 传权威全集（store 内同源收窄）。全局拓扑序定位（:109）与 advance 事实模型不动。
- `group_dependency_gate.rs`：全图身份/边合法性按 bound plan 全 units 核
  （`plan_revision.work_item_bindings` 即权威全集）——attempt units 逐一必须在全集内、图边端点必须
  在全集内、declared 与权威图全等（含外部 depends）、**全图判环**（不按仓缩图）。候选只筛本桶；
  桶外合法上游一律 **Waiting**（audit 携带 bound `dependency_work_item_revision_id`，`dependency_unit_id/
  handoff_id=None`），不得以「查不到本桶上游」判 Ready 或 Unknown——完整跨 attempt handoff 解析仍属
  S1。真 unknown/self/环失败关闭。
- **不改文件**：`group_validation.rs`（N2 串行窗口）、`advance_store.rs`（D1.2c 禁写）。完整性校验
  （group_validation.rs:349）经容外部边的基线 helper 自然获得同一桶内排序语义（同算法同并列规则，
  与创建序一致）。

## 证据（cargo test --locked --lib，红→绿→回归）

| 命令 | 红（改前） | 绿（改后，rustfmt 后复验同） |
|---|---|---|
| `plan_coding_n1_` | 2 失败（业务红）：创建面 `Io("mixed_target_group_rejected")` 误拒合法 A1→B1→A2；运行面桶外依赖返回 FailedClosed 非 Waiting。`plan_coding_n1_global_invalid_graph_fail_closed` 改前即绿（失败关闭钉子） | 3 passed |
| `group_split_target` | — | 11 passed |
| `sc_group_dependency_gate` | — | 9 passed |
| `campaign_stage3_dependency` | — | 3 passed（含 invalid_graph_and_handoff_mismatch_fail_closed） |
| `advance_split_targets` | — | 8 passed |
| `advance_split_recovery_matrix`（N4 文件，未动） | — | 8 passed |
| `advance_initialization` | — | 14 passed |
| `group_uniqueness` | — | 2 passed |

新增测试（沿现 test modules，无新挂载文件）：

- `plan_coding_n1_cross_target_edges_preserved_on_initialize_and_replay`（group_split_target.rs）：
  A 桶只物化 {A1,A2} 且桶内排序、A2 保留对 B1 的外部 depends；B 桶同理；重放命中同一 journal
  （同集合/顺序/冻结快照）。
- `plan_coding_n1_global_invalid_graph_fail_closed`（group_split_target.rs）：真 unknown/重复 WI/
  self 边/跨桶环在初始化失败关闭——桶内排序合法化不放宽全图合法性。
- `plan_coding_n1_external_dependency_waits_without_unknown_or_ready`（sc_group_dependency_gate.rs）：
  全图 A1→B1→A2、本桶 {A1,A2}，A2 依赖桶外 B1 → Waiting（`SC_GROUP_DEPENDENCY_HANDOFF_PENDING`，
  message 具名外部依赖，audit 带 bound revision 证据），非 Unknown 非 Ready。

## 边界与交接

- 单 target 旧语义零变化（`journal_target=None` 全集拓扑序；单桶=全集时容外部边≡旧严格序）。
- 创建/恢复/运行三面分别有证据；`advance_store` 零修改、未切换 advance 事实模型。
- `group_validation.rs` 本片零修改，N1 提交后释放交 N2（pause 合法性接管）。
- 完整跨 attempt handoff 解析（外部上游完成后跨 attempt 判 Ready）属 S1，本片一律 Waiting。
