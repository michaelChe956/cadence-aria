# F2b 实施报告：split/preflight typed 与授权排除

- Plan：`cadence/plans/2026-10-13_实施计划_确定性结构治理_v1.1.md` Task F2b（REQ-SG-01/02/02A）
- Change：`openspec/changes/fix-error-experience-structural-governance/tasks.md` F2b
- 设计依据：`cadence/designs/2026-10-13_方案设计_五阶段错误体验_确定性结构治理_v1.2.md` §3/§3.1（L1 不要求解唯一；确定性检出不授权任意删依赖/改归属；所有内容修复经 author 重跑）
- Worktree：`.worktrees/feat-b-0808-add-monorepo`（基线 36ee6209 = F2a）

## 1. 接口切换（接地 P2-1，同提交闭包）

`work_item_split_validator::utils` 的 `error`/`warning` 在本包从旧三参切为显式 kind 四参：

```rust
pub(crate) fn error(kind: PlanningFailureKind, code: &str, message: impl Into<String>, work_item_ids: Vec<String>) -> WorkItemSplitFinding
// warning 同参
```

- 旧三参签名与"恒写 kind=None"兼容层删除，无 shim。
- 原稳定 code 全部保持不变；kind 均由调用分支显式给出，不从 code/message/severity 反推。
- `types.rs` 公共校验入口签名未动（仅消费 validator 输出）。
- `mod.rs` 仅补 `PlanningFailureKind` import（子模块经 `use super::*` 可见）。

## 2. 分支→kind 映射（缺事实不猜）

| 分支族 | kind | 依据 |
|---|---|---|
| draft `unknown_provider_logical_work_item` | `UnknownProvider{provider,consumer,contract_id}` | outline 级供需缺口与 F2a canonical 同构，三元 ID 完整；唯一具 L1 资格的 split 预检 kind |
| draft canonical→split 映射 | finding.kind 原样传递；None 折影 `Unknown{code}` | "canonical 既有 kind 传递"；旧 JSON None 按 Unknown 投影 |
| `missing_trusted_verification_command_catalog`、`untrusted_required_verification_command`、`verification_command_{source_invalid,cwd_outside_repository,unsafe,needs_manual_review}` | `Operational{stable_code=code, cause}` | security operational 分支：可信目录/命令安全是用户确认的授权事实，沿原安全等待/人工门，机器不补权限、不驱 L1 |
| `dependency_cycle`（outline/plan）、`dependency_not_in_*`、`dependency_graph_mismatch` | `Unknown{code}` | 声明图 DFS 只有布尔事实，非有效图有向 witness（REQ-PLN-10），不编造 `DependencyCycle`（与 F1.2 compiler 同判例）；引用缺口无 contract_id 三元组，不冒充 UnknownProvider |
| options×items 三缺口（frontend_backend/integration/e2e required） | `Unknown{code}` | 携已确认 options 的业务混合裁决：不免费 L1，机器不得替用户开关已确认 options；消息保留选项键＋F-51 共享修复路径 |
| 其余 identity/预算/溯源/写域/verification plan 族 | `Unknown{code}` | 无封闭 L1 kind 事实或事实不完整，fail-closed |
| `draft_target_repository_mismatch` | `Unknown{code}` | 不强塞 TargetMismatch（其语义是授权成员/Plan Intent 解析归属，归 SG1） |

`lifecycle.rs` kind 字段注释、`planning_failure.rs` producer 清单同步更新（补 F2b split/preflight）。

## 3. TDD 证据（cwd=worktree 根）

- 红（Step 2）：`cargo test --locked --lib sg_f2b` → **5 failed / 0 passed**，过滤命中 5；失败均为 `kind: None` vs 期望 `Some(...)`（见会话记录；典型：`left: None, right: Some(Unknown { code: "unknown_done_when_ref" })`）。
- 绿（Step 4）：同命令 → **5 passed / 0 failed**：
  - `sg_f2b_required_item_gaps_carry_confirmed_options_without_machine_authorization`
  - `sg_f2b_required_command_authorization_is_operational_and_not_machine_granted`
  - `sg_f2b_acceptance_path_does_not_delete_acceptance_criteria_to_fake_pass`
  - `sg_f2b_mixed_operational_waits_whole_batch_without_partial_authorization`
  - `sg_f2b_all_split_findings_carry_explicit_kind_with_stable_code_identity`

### 验收场景对应

1. **required integration/e2e/frontend-backend 缺口携已确认 options**：断言三 finding 消息含选项键＋共享修复常量、kind=Unknown、Error；活动 SC facts 下整批 `UnknownStop`（≠StructuralAuthor）；选项关闭同 fixture 三码消失（原 options 权限不变）。
2. **intent/required command 缺授权不机器补权限**：`untrusted_required_verification_command`/`missing_trusted_verification_command_catalog`（draft）与 `verification_command_unsafe`/`cwd_outside_repository`（plan）kind=Operational 且 stable_code 逐分支保留；`needs_manual_review` 为 warning 仍显式 operational；整批 `OperationalWait`。
3. **acceptance_path 不能删 AC 假通过**：task→未知 AC 引用经 canonical→split 映射保留为 Error＋`Unknown{code}`，affected ids 保留 `ac_missing` 引用，`has_errors()` 阻断，分类 `UnknownStop`。
4. **mixed operational 先停**：L1 单独（UnknownProvider，三元 ID 断言到值）→ `StructuralAuthor`；混入 operational → `OperationalWait`；再混入 Unknown（identity 缺口）→ `UnknownStop`（F1.1 优先级在 split 预检层复验）。
5. **无遗漏 caller 扫描**：outline/draft/plan 三族违规 fixture 产生的每条 finding 均断言 `kind.is_some()` 且 `kind.stable_code()==finding.code`（身份单源，含未单测分支）。

## 4. 回归

- `cargo test --locked --lib work_item_split_validator` → 35 passed（旧 30＋新 5，旧 fixture 断言全保留）。
- `cargo test --locked --lib sg_f1_` → 17 passed；`sg_f2a_` → 5 passed（前序包不回归）。
- `cargo test --locked --test it_product -- product_work_item_split_validator` → 20 passed（plan 级真实 fixture）。
- `cargo test --locked --lib`（全量）→ **4462 passed / 0 failed / 7 ignored**（53.35s；rustfmt 本包文件后复跑 sg_f2b 5 passed、模块 35 passed、it_product 20 passed 均不回归）。

## 5. 独占文件面（提交闭包）

`outline.rs`/`draft.rs`/`plan.rs`/`utils.rs`/`mod.rs`/`models/lifecycle.rs`/`planning_failure.rs`/`tests.rs` 同一提交 **dfbdd410**（P2-1：utils 工厂切换＋全 caller 同包，无双写面；工作区内他人 rustfmt 重排/未跟踪文件未触碰、未 stage）。

## 6. 移交

- F2c 接管 `planning_failure.rs`（projection typed 继承与系统故障）。
- 本包未动 services、未 push、未 stash；报告文件不入代码提交（按 plan §0.4 仅 Files/Test stage）。
