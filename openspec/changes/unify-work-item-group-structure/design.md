# 设计记录：unify-work-item-group-structure

## 决策记录

### O1 先对齐两套图，而不是先瘦 prompt

v1.0 把「prompt 承载 31K 契约」当作主因、把「两套图不一致」列为待证伪假设。本轮用现场物证把假设证成：喂给 reviewer 的确定性结构报告里 5 条有效边全部是 `canonical_input`、`order_indices` 全空（`work-item-plan-sources/…/mechanical_report/report-956ec37e5e6777e7.json`），而 Final Compile 侧同一候选成环。
→ 「review 说没问题、确认后报错」是纯机械口径问题；先修它，收益/成本比最高，且不需要任何 provider 轮次。

### O2 `Plan Intent` 的 target 四键由平台派生，但**不是**"已有能力提上来"

范围：仅 `intent_target_kind/_repo/_codebase/_logical_repo`。`intent` / `provider_work_item_id` 仍由 author 写（`work_item_contract/model.rs:190-192` 明文「不得由系统自动补声明」，REQ-C1-PLAN-01）。
**前置工程量**：`PlanMemberTargets`（`lower.rs:47-53`）只有 `physical_by_logical` + `fallback_physical_id`，**没有成员 checkout 前缀表**；前缀表目前只在写范围校验期传入（`write_scope_bound.rs:110-148`）。因此必须先把前缀表接进 lowering 上下文，并对「跨成员 / 无前缀 / 不唯一」做 typed 拒绝。
**此外**：现状省略 `### Plan Intent` 段会返回 `None` → 落 `fallback_physical_id`（change_order 首仓），静默把所有 WI 归到首仓（`lower.rs:381-392`）。所以派生能力上线前 MUST NOT 允许省略该段，否则会制造新的静默错归属。

### O2b 托管仓不是唯一解

反例：本案 WI-004 写范围在 `gateway/src/test/…`，把 target 迁到 frontend 立即 `write_scope_outside_bound_repository`（`write_scope_bound.rs:110-148`）。因此「依赖闭包最靠后仓」只是**一个**合法解，平台必须给选项而不是给结论。

### O4 legacy 单仓路径零变化

`plan_projection.rs:1044-1052` 在成员表为空时清空 `logical_target` 是 legacy 兜底，保持不动；改动只发生在 `SingleRepository` 分支且仅当成员表在场。

## 架构与改动面

### 第一阶段（止血，对应 tasks 1-3）

1. `compile_support.rs` 新增单一反查：`ReverseMember = Unique(A) | Absent | Ambiguous`。
   - 输入：物理仓 ID + `physical_by_logical`；输出三态。
   - `finalizer.rs:924-937` 的本地闭包改为调用它（`Absent`/`Ambiguous` 复用既有 `work_item_target_missing` 文案）。
2. `plan_projection.rs:981-1014` `planning_structure_items_from_ir` 增加上下文参数：
   - `SingleRepository { repository_id }` → `Unique` 时置 `logical_target = Some`；
   - `Absent` / `Ambiguous` → 保持 `(None, None)`。
   - 调用点（4 处）同步：`plan_projection.rs:1044`、`conversational_gate_parts/revision_turn.inc.rs:1410`、`lifecycle_store/workspace_single_candidate_tests.rs:1125`、`workspace_engine/tests/single_candidate.rs:1456`。
3. `compile_failure_surface.rs:363-409` `owner_hint` / `rewrite_guidance`：删除「根因在 design」断言；witness 完整时渲染位次与选项。

### 第二阶段（形态一消失，对应 tasks 4-5）

4. `lower.rs` 的 `lower_plan_intent` / `resolve_item_target_repository_id`：`### Plan Intent` 缺席时按成员表 + 写范围前缀派生；冲突以派生为准 + 审计事件；写范围跨成员 → typed 拒绝。
5. `sc_author_teaching.rs:41` 与 `:253` 的互斥取值来源删除；被接管条款从 prompt 移除；新增计量测试锁定固定块体量与硬措辞数。

### 第三阶段（防回长，对应 tasks 6）

6. `draft_batch/compile_support.rs` 的 `project_effective_planning_graph` 与 `apply_change_order_cross_repo_depends_on` 共用同一「仓序边生成」纯函数，并以 parity 测试固定；清理 `PlanningStructureReport` 上已失真的「尚未接线」注释与 `#[allow(dead_code)]`。

## 与既有契约的关系

- `REQ-WSC-19`（确认不得形成死路）已在主干实现（`conversational_gate_parts/dispositions.inc.rs:95-110`），本 change 不改。
- `REQ-WSC-21`（首次即提示 + 继续自动重写）保留；本 change 只改「提示什么」（移除 `REQ-WSC-20`，改由 `REQ-WG-05`/`REQ-WG-06` 约束）与「重写指令带什么」。
- `REQ-WSC-18`（失败必须完整可读呈现）保留且被本 change 复用：它已经要求 `message` 点明「谁把谁写成 provider、为什么与仓序自动边合成环」，与 `REQ-WG-05` 的 witness 事实口径一致。
- **移除而非改写 `REQ-WSC-20` 的理由**：该条把「必须讲清是 design 的什么原因导致成环」「必须明确需要 owner 修改 design」写成**无条件**义务；而现场存在 design 仓序完全正常、根因在候选托管仓选择的反例。把这条留住并"软化措辞"会留下一个名字与内容冲突的场景（openspec 的 MODIFIED 语义要求保留原场景名），因此选择显式 REMOVED + 在新能力给出判据正确的替代条款。
- `REQ-WSC-23/24/25` 属于未归档的 v1.0 change（`mechanicalize-plan-compile-repair`），不在主干；本 change 以 `REQ-WG-01/05/06` 表达同一意图的更窄版本，两者若同时实施需去重（见「关键风险」）。

## 关键风险

1. **拦截率上升**：口径对齐后，此前"侥幸放行"的候选会在生成阶段失败。这是目的，但需要观测首次通过率是否显著下降；若下降过多，说明必须先落第二阶段（派生）再对齐图，tasks 顺序需可调。
2. **与 v1.0 change 的 requirement 重叠**：`REQ-WSC-23/24/25` 与 `REQ-WG-01/05/06` 语义相近。若两个 change 都要实施，必须先择一并删除另一份 delta，避免两套断言漂移。
3. **`Plan Intent` 退役的兼容面**：存量已持久 source 里带 `Plan Intent` 的候选仍需可编译（只校验一致性，不报错）。
4. **审计缺口**：冲突以派生为准时若不留审计，「平台替我改了什么」不可追溯；tasks 中必须有审计事件与落盘断言。

## 被否决的备选

- **平台自动删边消环**：删的是真实数据流（canonical Inputs），会让后续 coder 丢掉跨仓契约 → 否决。
- **平台自动把 Work Item 迁到"最靠后仓"**：写范围越界（O3）→ 否决。
- **只做 prompt 瘦身**：平台还没接住时先放大风险（v1.0 已论证）→ 否决。
- **把 `PlanningFailureKind` 扩成 lowering 全集**：污染 wire 枚举与 `classify_failure_set` 的关闭性 → 否决（v1.0 F16 已论证）。
