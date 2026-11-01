# Design

## 0. 现场与根因

- 多仓（LC 聚合根）plan 会话：聚合根目录本身不是 git 仓库，仅各成员仓（busi/gateway/frontend 等）各自入库；成员 checkout 是聚合根下的子目录。
- Work Item scope 以聚合根为基准书写（`busi/src/a.ts`）——见 `coding_workspace_engine/gates.rs` `scope_member_prefix` 与 `coding_work_item_context.rs` 的既有多仓口径；运行期匹配兼容「仓内相对」与「带成员前缀」两种写法（`scope_allows_member_path`）。
- 现场缺口：author 可产出 `scripts/dev.sh` 这类落在聚合根非成员区的 exclusive 写面——该路径不属于任何成员仓，执行期任何 worktree 都写不到；机械校验层无归属核对，只能靠 reviewer 人工发现。

## 1. 判定语义（write_scope_outside_bound_repository）

对每个 IR item（多仓会话，成员 checkout 前缀表在场）：

1. **被授权仓** = `item.target_repository_id`（lowering 已保证多仓会话下恰落一成员物理仓，聚合视图 ID 由 `work_item_target_missing` fail-closed 拒收）。
2. **前缀表查不到被授权仓** → Error finding（fail-closed，说明「无法核对归属」的理由）——与 `intent_target_bound_violation` 的映射缺失 fail-closed 惯例同构，不以跳过吞错。

3. 逐 scope 提取**固定路径锚点**（首个 `*` 前的固定段；顿号方言逐段展开；`*`/`**/x` 等无固定锚点 → 不判定）：
   - 锚点含 `..` 相对段 → fail-closed 出 finding（字面前缀与实际落点脱钩，机械不展开、不猜——F4）；
   - 被授权成员 checkout 前缀为空串（成员 checkout 即聚合根）→ 一切路径域都在其内 → 放行；
   - 锚点 == 被授权前缀或以 `<前缀>/` 开头 → 放行（**只判归属不判存在**，基线树不参与）；
   - 锚点落在**另一成员**的前缀内 → 越界（跨成员写按既有授权语义判：每 WI 恰落一被授权成员仓，其他成员的 checkout 不在本项授权面内）；
   - 锚点不落在**任何**成员前缀内 → 越界。该形态与「仓内相对写法」（运行期 `scope_allows_member_path` 兼容）语法不可区分（F2）——finding 文案**并列说明两种可能归属**并给出「加成员目录前缀」的改写示例（如 `busi/src/lib.rs`），不武断归类为「非成员区」；plan 期以聚合根为基准判归属与运行期兼容匹配是分工而非冲突。
4. 空 exclusive 列表（= 不限制可写范围，既有文档语义）不触发；每 WI 至多聚合一条 finding（声明序、去重、稳定排序）。

前缀比较语义与仓内既有前缀核对一致（`validate.rs` `scope_absent_from_baseline`、`root_recipe_receipt_store` 的 `path == prefix || path.starts_with("<prefix>/")` 口径）；锚点提取下沉到 `cross_cutting/worktree.rs`（`scope_fixed_path_anchors`），与 `ScopePattern::parse`/`normalize_scope_text`/顿号方言同源，不复制第二套归一逻辑。多仓 scope 的路径基准教学（`<成员目录>/<仓内路径>` 规范形）补入 SC author 的 LC 成员段（`sc_author_teaching.rs`，member_targets 在场才注入，单仓 prompt 逐字节不变）。

### 生效面（明确边界）

本规则的**生效面是权威落盘路径**（`complete_single_candidate_work_item_plan_author`
→ findings 留在机械报告 → `preflight_review_verdict` → 既有机械修订轮回灌）。
运行期预校验路径（web `prevalidate_plan_candidate_ir`）只消费
`validate_plan_candidate_ir` 的 `Err`（结构性失败），report 里的 preflight
findings 按该路径的既有语义丢弃——与 `check_target_forbidden` /
`acceptance_path_not_in_baseline` 全族同待遇，不为本规则新增行为。


## 2. 数据链路（成员 checkout 前缀表）

- `PlanCandidateValidationContext` 新增 `member_checkout_prefixes: Option<&BTreeMap<String, String>>`（物理成员仓 ID → checkout 相对聚合根目录，如 `"busi"`；空串 = 成员 checkout 即聚合根）。
- 装载器独立模块 `draft_batch/member_checkout_prefixes.rs`（`plan_member_checkout_prefixes` + 三路径共享门控 `validation_member_checkout_prefixes`；F3 行数门禁约束下的迁移与接线收敛，调用点只留一行调用 + 各自既有错误映射）：
  - issue → lc_id → `LogicalCodebaseStore::for_lc` → `load_manifest`；`RepositoryRouting::classify` Legacy → `Ok(None)`（单仓，无聚合根可越界）、FailClosed → `Err`（与 target 解析同口径）；
  - 仅 active 成员（Removed/Tombstoned 成员的原 checkout 目录按非成员区处理——其路径本就不在任何授权面内）；
  - 每成员 Main checkout 恰一（与 `cross_target_check::resolve_active_member_main_checkouts` 同窗口口径）；`canonical_path` 相对 `provider_context_root`（strip_prefix + canonicalize 兜底，与 gates `scope_member_prefix` 同源）；不在聚合根下 → `Err` fail-closed。
- 三条生产校验路径同源接入（与 `validation_member_targets` 同一门控：enrolled 绑定在场才装载；装载失败按各路径既有形态显式错误，不以 None 吞错）：
  1. `workspace_engine/single_candidate.rs`（author 权威落盘，Err → compile 失败）；
  2. `conversational_gate_parts/revision_turn.inc.rs`（门内人工修订，Err → operational 停点）；
  3. `web/workspace_ws_handler/run/single_candidate.rs`（运行期预校验，Err → 预检诊断回灌教学重驱）。

## 3. fail-closed 裁定表（任务要求的说明）

| 场景 | 裁定 | 依据 |
| --- | --- | --- |
| 无绑定（非 enrolled/Manual） | 不触发（前缀表传 None） | 既有惯例：enrollment target 缺席时 intent/绑定核对均不触发（`types.rs` 上下文契约、`validate.rs` 同款门控），Manual/legacy 零回归 |
| 单仓/无 LC（legacy 路由） | 不触发（装载器返回 None） | 单仓无聚合根，scope 即仓内相对，不存在「非成员区」 |
| 集合绑定且 LC 在场 | 正常核对（前缀表按 active 成员装载） | 集合绑定的授权面 = 成员集合；每 WI 仍恰落一被授权成员仓（lowering 铁律） |
| 前缀表装载失败 | 装载点显式错误（compile 失败/operational 停点/预检诊断） | 既有惯例：装载失败不以 None 吞错（`current_enrollment_binding_for_compile` 同款） |
| 表在场、被授权仓不在表内 | Error finding（fail-closed） | 与 `intent_target_bound_violation` 映射缺失同构：无法核对 ≠ 放行 |
| 跨成员写（另一成员 checkout 内） | Error finding（同稳定码） | 既有授权语义：WI 的写面只在被授权成员仓的 worktree 内生效，其他成员 checkout 本项执行期不可写 |

## 4. 与既有契约的关系（冲突排查）

- **`work_item_contract/validation.rs`（意图合同）零变化**：本核对只消费 lowering 已产出的 `target_repository_id` 与 write_policy，不重判意图；`intent_unexecutable`/`intent_undeclared` 语义不动。
- **`acceptance_path_not_in_baseline`（AC 路径×基线树）互补不重叠**：那条判「引用的既有路径存在性」，本条判「写面归属」，新建路径只被后者放行、不被前者拦截（AC 引用才受限）。
- **运行期 `scope_allows_member_path` 双写法兼容不变**：本核对在 plan 期强制聚合根基准的规范写法（成员前缀形式），运行期对历史/兼容写法的匹配语义零变化；被拦 scope 的修复即改写为成员前缀形式，恰与运行期主形态一致。
- **`work_item_check_target_forbidden` 正交**：那条判「检查目标 × 本项 forbidden」自洽性；本条判「写面 × 被授权仓」归属，互不替代。
- **preflight 族治理零新增机制**：新码进 `PREFLIGHT_FINDING_CODES`，经既有 `preflight_review_verdict` → F5-A 修订轮回灌；required_action 与共享常量逐字同源（REQ-WSC-06）。

## 5. TDD

红→绿五类用例（`work_item_plan_compiler/tests/write_scope_bound.rs`）：非成员区被拦（owner 现场原文形态）/ 跨成员被拦 / 新建路径不误伤 / 前缀表缺席零回归 / 被授权仓不在表内 fail-closed；另附 verdict 修复动作同源断言（`plan_preflight` 侧）。
