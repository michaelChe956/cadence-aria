# F2c 实施报告：projection typed 继承与系统故障

- Plan：`cadence/plans/2026-10-13_实施计划_确定性结构治理_v1.1.md` Task F2c（REQ-SG-01/02/03）
- Change：`openspec/changes/fix-error-experience-structural-governance/tasks.md` F2c
- 设计依据：`cadence/designs/2026-10-13_方案设计_五阶段错误体验_确定性结构治理_v1.2.md` §3/§3.1（所有内容修复经 author 重跑；operational 沿原安全等待，不借 L1 启动 author）、§4.1（Kahn 残余集合不是有向 witness，REQ-PLN-10）
- Worktree：`.worktrees/feat-b-0808-add-monorepo`（基线 dfbdd410 = F2b）

## 1. 接口与分支→kind 映射（同提交闭包）

`ProjectionValidationFinding`（model.rs）新增可选 typed 字段，沿原模型不动旧字段：

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub kind: Option<PlanningFailureKind>,
```

| finding 族 | kind | 依据 |
|---|---|---|
| `plan.dependency_graph`（`dependency_graph_projection_findings`） | **原样继承 canonical `finding.kind`**（F2a typed 族：DependencyCycle witness、UnknownProvider/RequiredContractMissing 三元 ID、RequiredCapabilityMissing 四元 ID） | "继承 canonical 结构信息"：同 dependency 失败保 IDs/kind，投影层不重新判型、不从 message 拆码；canonical 未 typed 分支（如 duplicate edge）保持 None（缺事实不猜） |
| 其余全部 projection 内分支（`push()`：projection_* / plan_projection_* / human_projection_invalid_flags 等） | `Operational { stable_code: code, cause: message }` | "内部 projection invariant 仍 Operational"：编译器对 canonical 输入的确定性投影漂移是系统/工程内部错，不驱 author（REQ-SG-01/03）；stable_code 取分支 code 常量单源，cause 仅诊断，classification 不读 |
| `plan.rs::validation_error`（stable_topological_order 三分支：IDs differ / unknown edge / Kahn cycle） | `Operational { stable_code: code, cause: message }` | Kahn 残余节点集合不是有向 witness（REQ-PLN-10），不编造 DependencyCycle payload；真实 cycle witness 由 canonical typed 族经继承路径提供。"拒绝结果不 Display 重 parse"：拒绝经 typed report 携带稳定码，消费面不解析 Display 文案 |
| 附录 A 三处旧式 test literal（initial_publication ×2、part_22 ×1） | `kind: None`（永久） | 模拟旧 wire 记录/未 typed 诊断：None 按 Unknown 折影，绝不产生 L1 资格 |

`planning_failure.rs` 仅更新 `PlanningFailureOrigin::Projection` 文档（F2c 继承/Operational 语义），无行为改动——分类核心 F1.1 已天然支持。

## 2. TDD 证据（cwd=worktree 根，绝对路径）

- 红（Step 2）：`cargo test --locked --lib sg_f2c` → **3 failed / 0 passed**，过滤命中 3（0 tests 不计红已规避：红态字段已建但全 producer 写 None，测试运行期失败）。典型失败：
  - `left: None, right: Some(RequiredCapabilityMissing { provider: "wi_provider", consumer: "wi_consumer", contract_id: "contract.shared", capability: "capability.missing" })`
  - `left: None, right: Some(Operational { stable_code: "plan_projection_edge_mismatch", ... })`
- 绿（Step 4）：同命令 → **3 passed / 0 failed**：
  - `sg_f2c_projection_origin_inherits_canonical_dependency_kind_and_ids`
  - `sg_f2c_projection_origin_internal_invariants_are_operational_without_author_drive`
  - `sg_f2c_projection_origin_old_records_fold_unknown_with_byte_contract`

注：红绿期间并行 worker（FixPark1）in-flight 文件两度使 lib test 目标编不过（E0252→E0433），经 peer 消息协同收敛后恢复；非本包引入。

### 验收场景对应（tasks.md F2c 验收逐条）

1. **同 dependency 保 IDs/kind**：capability 缺口经 projection 校验的 finding kind 与 canonical `validate_dependency_contract_graph` 输出逐字段相等（四元 ID 到值）；两节点环的 DependencyCycle witness（真实有向路径、`is_complete_cycle_witness` 通过、每跳端点为真实逻辑 WI ID）原样继承；两 fixture 在活动 SC 授权下整批 `StructuralAuthor`（IDs/kind 完整即保 L1 资格，透过投影层不漂移）。
2. **projection 内部错不驱 author**：coder 丢边（`plan_projection_edge_mismatch`）、human 成员弹出（`plan_projection_work_item_mismatch`）、compile 缺 work item projection、`stable_topological_order` 直调 Kahn 环阻断（断言不编造 DependencyCycle、typed Operational）——活动授权下整批 `OperationalWait`；混入 L1 依赖失败仍 `OperationalWait`（不部分自动，F1.1 优先级复验）。
3. **Unknown 旧 record 只折叠**：序列化去 `kind` 字段→解析 None→再序列化字节不变（旧 PlanValidationReportArtifact 哈希兼容，plan §0.5）；None 折影 Unknown 后活动授权下 `UnknownStop`，不补资格。
4. **逻辑/物理投影不一致拒绝**：三处独立 fixture（validate 漂移、成员不同构、compile 闭包缺口）均产出 findings 且 compile 返回 `Err(Validation(...))`，物理投影永不被静默放行。
5. **不从 Display 拆码**：`assert_kind_codes_match_findings` 对全部报告扫描 `kind.stable_code()==finding.code`（身份单源）；stable_code 恒为分支常量，cause/文案不参与分类。
6. **无遗漏 caller**：全仓 `ProjectionValidationFinding` 构造点仅 validation.rs `push`/`dependency_graph_projection_findings`、plan.rs `validation_error` 与三处 legacy test literal，同提交全部适配；无 shim、无旁路。

## 3. 回归

- `cargo test --locked --lib work_item_projection` → 49 passed（旧 46＋新 3，旧 fixture 断言全保留）。
- 前序包：`sg_f1_` 17 passed；`sg_f2a_` 5 passed；`sg_f2b_` 5 passed。
- 附录 A 适配文件：`initial_plan_publication` 5 passed；`plan_repair_awaiting`（part_22）16 passed。
- rustfmt：本包 7 文件中 6 个 clean；tests/plan.rs 已格式化后复跑 sg_f2c 3 passed、模块 49 passed；part_22.rs 仅 +1 行（该文件既有历史格式漂移不属本包，未整体 restyle）。
- `cargo test --locked --lib`（全量）→ 4471 passed / 2 failed / 7 ignored（56–63s 区间多次运行失败集在 5→3→2 间随并行 worker 提交动态变化）。两次残余失败 `task_3_4_single_candidate_crash_boundaries_reuse_durable_reservation`、`campaign_stage3_advance_confirmed_plan_is_ready_without_provider_start`：两文件对本包类型（ProjectionValidationFinding/projection_validation/planning_failure）引用为 0，且均位于 FixPark1 未释放写面（part_03/*、workspace_ws_handler/tests/*）——按 plan §0.4"并行 worker 写面未释放不运行检查"归属该 worker 收敛，非 F2c 回归。

## 4. 独占文件面（提交闭包）

同一提交 **7af71977**（基于 dfbdd410）：`model.rs`/`validation.rs`/`plan.rs`/`tests/plan.rs`/`planning_failure.rs`/`work_item_revision_store/tests/initial_publication.rs`/`workspace_engine/tests/part_22.rs`。仅显式 stage 上述 7 路径；工作区内 FixPark1 的 160 个修改/未跟踪文件未触碰、未 stage；未 stash、未 push；本报告不入代码提交。

## 5. 移交

- W0 收官：F2c 完成，`planning_failure.rs` 写面释放。
- SG1（compile_support 独占，依赖 F2c）与 SG11.1（role-run 模型，可与 SG1 并行）可开工（plan §0.3 DAG）。
- SG3b/Final Compile 后续消费 projection typed report 时沿 `kind` 单源读取，不新增 Display 解析路径。
