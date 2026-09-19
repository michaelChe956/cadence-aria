# Stage 4 C2 Task 5 报告：WP5 恢复矩阵+增殖审计收口（REQ-MTG-05，D4/D5）

- **change**: `openspec/changes/multi-repo-group-coding/`
- **计划**: `cadence/plans/2026-09-19_计划文档_阶段4-C2_多仓group编码_v1.0.md` Task 5（v1.1）
- **状态**: **全量交付**——Step 1 审计面+挂点接通 / Step 2 恢复矩阵（7×2 全量+3-target 抽样+半启动+断连重连）/ Step 3 二期证据留档+defer 登记 / Step 4 关闸自查（附录 B 对照+范围铁律）。
- **依赖**: T1/T2/T3/T4 已落地（HEAD=`023f5892`）。

## 一、交付清单

### 产码（`split_audit.rs` 新模块+advance.rs 接线）

| 项 | 内容 |
|---|---|
| `SplitAuditTriggerKind{Advance, GroupCreation}` | serde snake_case（`"advance"`/`"group_creation"`——计划 Interfaces 定案词汇；一期仅 advance 面接线，`group_creation` 为记录词汇位） |
| `SplitAuditTrigger{kind, command_id: Option}` | `command_id` 从 `AdvanceInput` 透传（advance 面） |
| `SplitAuditRecord` | `{id(=attempt_id), project_id, issue_id, plan_id, target_repository_id, attempt_id, bound_plan_revision_id, dependency_graph_revision_id, trigger, created_at}`——全字段蛇形 additive 新记录类型 |
| 落点 | `issue_lifecycle_root/{project}/{issue}/split-audit/{attempt_id}.json`（json_store durable 先例，`coding-attempts` 目录族同根；一 target-attempt 一条 ⇒ per-(plan,target) 唯一 ⇒ 审计唯一） |
| `record_split_audit`（store） | 幂等首写定档：身份一致（分流事实字段全等，command_id/created_at 不参与——恢复可携异 command_id）命中 → Ok 不重写；身份冲突（同 attempt 异 target 等）→ fail-closed Conflict；id≠attempt_id/created_at 缺失 → InvalidRecord |
| `get_split_audits_for_plan`（store） | per-(plan,target) 检索（消解「取最早」歧义）：list+filter+身份校验+target 升序确定性；异 plan 过滤；身份漂移 IdentityMismatch |
| advance.rs 接线（T2 占位真实现） | `initialize_advance_split` per-target 循环内 journal 获取后逐 target 落审计——**创建/恢复/重放三入口同点覆盖**（R7：中断于 journal 创建后审计写入前的缺口由重放幂等补齐）；T2 占位自由函数替换为真实现（R10 交接：签名按定案结构从 T2 编译期接口演化为逐 target 调用——定案需 bound_plan_revision_id/dependency_graph_revision_id/trigger 三类 T2 占位未承载的字段，演进在 T2 占位 doc comment 预告的「真实现替换」范围内） |

### 计划 Files 偏差登记（诚实呈报）

- 计划列 Modify `group_initialization.rs`——**实施核查后未改动**：分流创建/恢复/重放三入口全部流经 `initialize_advance_split` 的 per-target 循环（`prepare_group_initialization_with_admission_for_target(Some)` 的唯一调用方），该单点接线即全覆盖；`group_initialization.rs` 内部函数零改动（审计保持纯事实记录消费面，不在存储初始化路径内嵌副作用）。
- 「与既有 operationAuditStore 面一致」（任务上下文措辞）核查：`operationAuditStore` 为前端 WS 操作生命周期 zustand store（`web/src/state/operation-audit-store`），与计划定案的 product 层 durable 审计面（split-audit JSON 记录+检索）分属两面；T5 计划 Files/Steps 无任何 web/前端文件——按计划定案实施（durable 记录+json_store 先例同构），不新增 WS 事件面（ additive 纪律：无 wire 变更）。

### 测试（新文件 `advance_split_recovery_matrix.rs` 7 项+扩展 2 处）

| 族 | 测试 |
|---|---|
| 审计 #1 创建留痕全字段 | `split_advance_records_proliferation_audit_for_every_target_attempt`（含 #4 行为面无编排断言） |
| 审计 #2 幂等首写定档 | `split_audit_store_write_is_idempotent_first_writer_wins`（+矩阵主格 created_at 不漂移断言） |
| 审计 #3 检索消解 | `split_audit_retrieval_resolves_per_plan_target_and_filters_foreign_plans` |
| 矩阵主格 2t×7 | `split_recovery_matrix_all_checkpoints_resume_same_attempt_set`（逐格 durable 中断态+同套恢复+审计稳定+replay） |
| 矩阵抽样 3t×2 | `split_recovery_matrix_three_targets_samples_two_checkpoints` |
| 半启动×2 | `split_semi_started_resume_backfills_head_per_attempt_only` / `split_semi_started_unmaterialized_demotes_per_attempt_only` |
| 断连重连门 | `resumption::tests::sc_advance_restart_gate_accepts_split_target_attempt_set_membership` |
| 单 target 7 checkpoint | `advance_handler::advance_initialization_replay_resumes_same_record_attempt_and_units` 扩展（+GroupAttemptPersisted 第 7 格含 attempt 文件在场判据+单 target 零审计断言） |
| 夹具泛化 | `split_advance_fixture_with_target_count(2|3)`（2-target 映射不变=T2 行为锁定）+`clear_split_dependency_edges`（3-target 依赖图手术，见 §三） |

## 二、TDD 红绿证据

- **红**（实现前）：编译红 11 errors——`SplitAuditRecord/SplitAuditTrigger/SplitAuditTriggerKind` E0432 未定义+`record_split_audit`/`get_split_audits_for_plan` E0599×8（T2 Batch 1/2 同款编译红先例）。
- **绿**：实现后 `advance_split*` 15/15（新 7+既有 8——T2 既有 8 项含幂等/失败标记/嵌套/无编排全数回归绿）。
- **3-target 格真红**：首跑 `order split units failed: coding_group_dependency_graph wi_core`——种子依赖边 `wi_core→wi_registration`（seed.rs:355）在「每桶恰一 unit」形态下成跨桶依赖，per-target 拓扑排序按一期语义 fail-closed（跨桶依赖=跨 attempt 编排越权）。修正=夹具手术 `clear_split_dependency_edges`（graph.edges 与 plan projection 两侧 edge 清单一致清空——合法 plan 形态；一致性约束=group_validation「plan projection dependencies do not match the bound dependency graph」）后绿。该红恰证一期「无跨桶依赖」fail-closed 语义在矩阵路径上真实生效。

## 三、关键语义决策（留档）

1. **审计写入点=per-target 循环 journal 获取后**（非成功返回点）：真新建即落（attempt 身份分配时刻），恢复/重放过同点幂等命中（首写定档）——中断于「journal 已写、审计未写」的窗口由重放补齐，R7「审计覆盖创建/恢复/重放入口」三入口同点闭环。
2. **command_id/created_at 不参与身份比对**：恢复可能携异 command_id（Initializing record 续走不限同 command），首写记录是权威（溯源=首次分流事实）。
3. **单 target 零变化**：审计为分流专属（增殖动作）——单 target 路径不落任何 split-audit 记录（矩阵单 target 列逐格断言）；`prepare_group_initialization*`（None 变体）与既有单 target 全路径零改动。
4. **3-target 抽样组合的依赖图约束**：一期 per-target 拓扑排序要求依赖边桶内自洽（跨桶 fail-closed）——矩阵报告 §二注记。

## 四、终验（隔离树，T2 逃逸教训承接：必含 it_web）

前置：干净 checkout HEAD=`023f5892`+仅本任务 patch（8 文件）+独立 target dir（`/tmp/wp5-target`）+从主树拷同版本 `web/dist`（rust-embed 编译期嵌入、gitignored）。

- `cargo test --locked --lib`：**3387 passed / 0 failed / 3 ignored**（41.79s）
- `cargo test --locked --test it_web`：**329 passed / 0 failed / 1 ignored**（60.08s）
- `cargo clippy --locked --lib --all-targets`：**0 error**；仅 3 条 pre-existing warning（`issue_delivery.rs` unused import CodingAttemptScope——T4 落地面；`contract_autorepair.rs` collapsible_if+unused line——T2 报告 §九已登记，本任务未触碰子系统）。
- fmt：`cargo fmt --all --check` 幂等（隔离树复验 FMT_CLEAN）。
- 主树定向（并行兄弟在途，仅作过程证据）：workspace_engine 1189/1189、coding_attempt_store 179/179、coding_ws_handler 112/112、it_web mixed_target_group 3/3。

## 五、Step 4 关闸自查（5.4）

### 附录 B 决议承接对照（design 附录 B 15 行逐行——本 Task 相关行加粗）

| 决议原文 | 承接位置 | 本 Task 佐证 |
|---|---|---|
| Q2：多仓 group 模型 → (b) 每 target 一 attempt | D1/D2；REQ-MTG-01/02；WP1/WP2 | 矩阵主格同套 attempt 集断言（T2 建组+T5 恢复锁死） |
| Q2：(c) 一期=解除 REQ-COD-04 拒绝 | REQ-MTG-01；WP1 | 3-target 抽样格（多 target 创建/恢复全通） |
| Q2：一期=按 target 拆分独立 attempt | D2；REQ-MTG-01/02 | 半启动 per-attempt 独立等价两测试 |
| Q2：一期=手动推进 | REQ-MTG-03；WP2 守卫 | 矩阵恢复全程无自动启动（attempts 停 Created；attach 门 Created 不动作） |
| Q2：一期=group 级聚合只读视图/终态 | D3；REQ-MTG-04；WP3 | 二期证据留档引用 T3 聚合面为证据来源（wp5-phase2-evidence §一） |
| Q2：（聚合视图必须同 WP 交付） | D3 同批交付句 | WP3 已同批交付（wp3-aggregate-report）——WP5 留档引用完成闭环 |
| Q2：REQ-COD-02 单 target 快照不变式保全 | D1 红线 | 单 target 零变化：矩阵单 target 列 7 格+零审计断言 |
| Q2：二期=跨 attempt 依赖就绪门自动编排，凭一期证据再立项 | **D5；REQ-MTG-03 红线句+defer 登记；Non-Goals 首条** | **wp5-phase2-evidence.md defer 登记（触发条件+证据来源+另行立项）+§二无编排四点自查** |
| oracle Q2：保全 load-bearing 不变式；100% 复用单仓机制；最小化解洞 | D1/D3 | 审计面纯新增（additive 记录类型）；恢复矩阵全部复用既有 failpoint/journal 基建 |
| oracle Q2 风险：两形态并存→显式路由 | D2；R3 | 单/多 target 矩阵分列（单=回归锁死/多=等价语义） |
| oracle Q2 风险：attempt 增殖 UI 面必须同 WP 交付 | D3；R6；WP3 | T3 前端 per-target 呈现已交付（本 Task 无 UI 面） |
| Q5 change ②：底座复用（REQ-COD-05/06 已有实现） | 事实基线 #8；WP4 | 矩阵运行于 WP4 核查后的底座（HEAD=023f5892） |
| ter 面 A 工时带与依赖序 | tasks WP 划分；R9 | A-WP5=恢复矩阵+审计收口（本期兑现） |
| 决议顺序：①→③→② | 归档时序说明；R8 | 实施序承接（③已归档基座上实施） |
| 决议风险登记：Q2 二期工时待一期证据 | D5 触发条件；R9 | defer 登记不预定价（wp5-phase2-evidence §一处置行） |

（决议下一步行=流程项，非实施承接——T6 关闸核对。）

### 范围铁律自查

- **无越界项**：改动面=split_audit.rs（新）+advance.rs 接线+测试 4 文件+报告 3 份；`group_initialization.rs` 零改动（偏差登记 §一）；无 web/frontend/API/wire 改动。
- **无二期预埋**：Step 1 #4 断言（行为面）+编译面 grep（split_audit.rs 及消费面零调度调用）+defer 登记显式另行立项；`GroupCreation` trigger 词汇位为计划定案记录结构原文（非预埋功能——无任何产码分支消费它）。
- **REQ-MTG-05 四 scenario 逐条**：分流创建留审计→审计族 #1；恢复矩阵等价语义→矩阵全格+半启动+断连；审计不承载编排→#4 双面断言；检索 per-(plan,target)（要求正文）→#3。

## 六、结论

WP5 四 Step 全量闭环：增殖审计面（durable 记录+幂等首写定档+per-(plan,target) 检索）落地并接线 T2 挂点（R10 交接完成）；恢复矩阵 7×2 全量+3-target 抽样+半启动+断连重连全绿；二期 defer 登记留档；附录 B 对照+范围铁律自查通过。C2 一期（WP1-WP5）就此集齐，待 T6 关闸。
