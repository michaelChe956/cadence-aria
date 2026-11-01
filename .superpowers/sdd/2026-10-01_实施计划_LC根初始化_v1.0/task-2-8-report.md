# Task 2.8 报告：gateway 复验拆除 cwd==target 等式 + 双工厂 root assertion

状态：DONE　|　worktree: feat-b-0808-add-monorepo　|　基线 HEAD: 6e441e63（映射 tasks.md 2.2；派发上下文按 tasks.md 编号称「Task 2.2」，本报告按计划文件编号 2.8 与 brief 配对）

## 交付合同

| 面 | 变更 | 语义 |
|---|---|---|
| `provider_gateway.rs` `revalidate_before_spawn` 步骤 5 | **拆除** `canonical_cwd == canonical_target` 错误等式（原 1053-1067） | 改为独立 cwd 权威复验：(a) canonical(spawn cwd) == canonical(envelope.working_directory)（Task 2.5 冻结字段在此消费；漂移 ⇒ `TargetMismatch{field:"cwd"}`）；(b) canonical(冻结 cwd) 必须位于 canonical(authority_root) 允许范围内（REQ-ENV-11；symlink 逃逸/外来根 ⇒ `TargetMismatch{field:"cwd_authority"}`，零 spawn） |
| `provider_gateway.rs` `validate_inner` | 新增 cwd authority 词法早门 | envelope 冻结 cwd 词法前缀必须落在 authority_root 内（validate 时点 cwd 目录允许不存在；canonical 复验在 spawn 前强制，spawn 不可能绕过）。policy/capability/config/target/git-dir/worktree/availability 全链原样保留 |
| `production_policy_resolvers.rs` | 新增 `pub fn assert_canonical_lc_root_consistent(registration_root, aggregate_root, authority_root, envelope_working_directory) -> Result<PathBuf, ProviderGatewayError>` | 四路 canonical root 投影一致才放行并返回 canonical root；投影缺失（bootstrap 早期/legacy 无 receipt）≠ 不一致；在场投影损坏/漂移 ⇒ fail-closed（field：`lc_root`/`registration_root`/`aggregate_root`）。经 mod.rs re-export |
| `gateway_factory.rs` `build_for_lc` | 双工厂 root assertion 接线 | authority_root=manifest.provider_context_root 与登记工厂（record.json `aggregate_root`）、aggregate 生产 driver（`aggregate-recipe-receipts/*.json` 冻结的 canonical root，源自 preflight snapshot root）投影不一致 ⇒ gateway 不组装（zero spawn 边界前移到工厂；聚合 driver 每 turn 重建 gateway，全量拦截）。record 在场不可读 ⇒ fail-closed（不把损坏投影当缺失）；多 receipt 根彼此不一致 ⇒ fail-closed |

兼容边界：cwd==target 的既有形态（成员 worktree 在 authority 之下）在新门下继续放行——`logical_provider_entrypoints_use_gateway_for_sync_and_streaming_stacks`（:868，同 cwd/target fixture）不冒充新锁、保持绿。

## 三个新测试（provider_gateway_tests.rs `task28_gateway_root_authority` 模块，逐字命名）

- `dual_gateway_factories_reject_canonical_root_mismatch` ✅——纯断言四路投影（含空格目录+symlink 字面不同）canonical 相等才放行；登记/receipt/envelope-cwd 任一漂移 ⇒ 对应 field 错误；工厂层 record/receipt 漂移 ⇒ `build_for_lc` fail-closed，canonical 一致（经 symlink）⇒ 组装恢复
- `cwd_outside_authority_or_target_git_identity_drift_zero_spawns` ✅——(a) 外来根 cwd validate 即拒；(b) symlink 逃逸词法放行、spawn 前 canonical 拒（registry_start_count==0）；(c) git identity 漂移继续拒；(d) **cwd=authority root ≠ member target 合法分离形态真实 spawn（start_count==1）**——旧等式会错杀该形态
- `provider_specific_writable_evidence_does_not_expand_coding_root` ✅——coding cwd=root 时写证据仍恰为 [target worktree]；root 作写根 ⇒ envelope fail-closed；sync spawn 独立 `working_directory` 优先于 `worktree_path` 通过复验（evidence gate 只作证据，不猜 OS sandbox）

## 消费面 fixture re-pin（契约变更后的存量 fixture 拓扑修正）

真实 LC 拓扑=成员 checkout 位于聚合根（CommonNonGitParent）之下；下列 fixture 把成员/manifest 根摆成兄弟目录，在新 cwd authority 契约下越界，re-pin 为「manifest 根=成员所在真实公共父目录」：

| 文件 | 处置 |
|---|---|
| `web/handlers/automation_enrollment_test_support.rs` `seed_logical_codebase` | manifest 根 `root/aggregate-root` → `paths.root()`（成员 checkout 的真实父目录） |
| `product/coding_workspace_engine/tests/provider_gateway_validated_input.rs` `build_gateway_with_registry` | manifest/authority 根 `.aria` → `.aria` 父目录（workspace root） |
| `tests/it_web/provider_gateway_envelope.rs` | 同上两处 + 非默认 LC 用例 record/manifest 根统一为 root（record 与 manifest 保持一致，工厂断言可比） |

## 行数守卫（拆分波裁决）

`provider_gateway_tests.rs` 加测试后 1590 行超 1200 ⇒ 按仓库 `.inc.rs` 惯例拆出 `provider_gateway_tests/task28_root_authority.inc.rs`（420 行），主文件回到 1174 行；`it_core large_file_guard` 绿。

## 验证（实际执行）

| 命令 | 结果 |
|---|---|
| `cargo test --locked --lib dual_gateway_factories_reject_canonical_root_mismatch` | 先红（`assert_canonical_lc_root_consistent` 未解析 + 存量等式错杀合法形态）→ 后绿 |
| `cargo test --locked --lib task28_gateway_root_authority` | 3 passed |
| `cargo test --locked --lib resume_fingerprint_changes_when_provider_version_or_snapshot_drifts` | 1 passed |
| `cargo test --locked --lib` | **3962 passed / 0 failed**（3 ignored；基线 3959+3 新增。中途一次 2 failed 为负载 flaky，复跑绿） |
| `cargo test --locked --test it_web` | **354 passed / 0 failed**（修复前 2 failed=本任务引入的 fixture 越界，re-pin 后绿） |
| `cargo test --locked --test it_core` | 187 passed / 0 failed（含 large_file_guard） |
| `cargo test --locked --test it_provider / it_task_run / web_logical_codebase_entrypoints / aggregate_initialization_zero_git_side_effects` | 54/31/44/1 全绿 |
| `cargo test --locked --test it_product` | 208 passed / 2 failed——**预存在**（git stash 基线复现同败：`execute_code_review_persists_report…`、`completing_group_units_publishes…`，与工作区其他未跟踪残留属同一在途工作面，非本任务引入） |

## 范围边界与 carry

- `provider_admission_check.inc.rs` 步骤 6 仍以 `request.target.worktree` 作复验 cwd——当前全消费方（聚合 driver）cwd==target 零影响；Task 2.1-2.7 接线 root cwd 启动时需改为 `request.working_directory`（已在其 brief 语义内）。
- 工厂 receipt 扫描取「目录内全部 receipt 根一致」强于「最新一条」；LC 换根残留的旧 receipt 会持续 fail-closed（fail-closed 同向，登记留档）。
- 生产 wiring（2.1-2.7 调用方迁移到 root cwd）不在本任务；本任务交付门与复验语义。

## Fix round 1（审查 P1）：coding runner LC 作用域工厂组装错误 fail-closed

**Finding（P1）**：`src/web/coding_ws_handler/runner/task.rs:135-138`——LC 作用域 attempt 以 `.ok()` 静默吞掉 `factory.build_for_lc(...)` 错误：无日志无传播，把本任务「root 投影不一致 fail-closed」反转成 fail-open 窗口（gateway=None 时逻辑 attempt 依赖引擎第二道防线拒绝，死因退化为含糊的 gateway-missing，且错误不可见）。另两个消费方（workspace manager、aggregate driver）均正确 map_err 传播。

**修法**：
- `Ok(Some(lc_id))` 分支：build 错误转 `CodingWorkspaceEngineError::ProviderStream("logical gateway factory build failed: {error}")` 并短路 `execute_start_coding_flow`（**零 provider 启动**），经既有 F-14 失败路径双通道可见化（AwaitingManualRecovery + manual-recovery diagnostic 携带工厂失败原文 + `coding_start_failed` protocol error 事件）。
- `Ok(None)`（无 LC 作用域/单仓 legacy 直连契约）保持既有降级；resolver `Err` 分支原样（warn + 降级）。

**覆盖测试**：`runner_recovery.rs` 新增 `lc_gateway_factory_build_failure_fails_closed_before_provider_spawn`——record 根≠manifest 根（双工厂 root 漂移）的 LC 作用域 running attempt：断言 runner 零启动即退出、attempt 转 AwaitingManualRecovery（稳定 reason 码）、diagnostic 含「logical gateway factory build failed」+「registration_root」、protocol error 事件含失败原文。先红（旧行为 `.ok()` 降级）→ 后绿。

**验证**：新测试 1 passed；`runner_recovery` 8/8、`coding_ws_handler` 127/127；全量 lib **3963/0**；it_web **354/0**；it_core large_file_guard 绿（task.rs 318 行/runner_recovery.rs 860 行均低于上限）；it_product 仍为同 2 例**预存在**失败（基线 stash 已复现，非本修复引入）。

## Task 2.8 剩余交付：planning snapshot 贯穿 + Phase 2 收口核验（tasks.md 2.5/2.2 收尾；REQ-PLN-03/07）

状态：DONE　|　基线 HEAD 5dd7bd50（2.1-2.7 全落后）　|　brief 主体（root assertion）已由上文 2.2 章节覆盖，本轮=剩余范围审计→补全→收口。

### 剩余范围判定（审计发现两处真实缺口）

**缺口 1（plan author 链 cwd 断裂）**：`logical_planning_launch()`（lifecycle.rs）把 `session.repository_path`（成员 checkout）当 run cwd 供给 `LogicalPlanLaunch`，`planning_request()` 的 `working_directory` 因此=成员 checkout——gateway_start.rs 原 88-90 行注释自认「LC root≠target 的注入归 Task 2.2」，该迁移在 Phase 2 从未执行。全部 plan author 启动点（WorkItemPlan outline/auto-retry、SingleCandidate 首启/reredrive、draft/review follow-ups——均经 `resolve_plan_author_launch`→`start_work_item_plan_author` 单一咽喉）provider 以成员 checkout 为 cwd spawn，违反 REQ-PLN-01/07。**并发断裂**：B3 StaleContext 重建 run 的 input worktree=rebuilt.cwd=root 而 envelope cwd=成员 checkout → spawn 前复验 cwd 漂移 → 重建 run 启动即被拒。

**缺口 2（snapshot 未冻结 cwd）**：REQ-PLN-03 要求 snapshot 固化 canonical working directory 且「cwd、target、policy 或成员快照任一指纹不一致→拒绝沿用旧 session」；`PlanningContextSnapshot` 无 cwd 字段、`access_fingerprint_value` 不哈希 cwd → 聚合根迁移（manifest `provider_context_root` 变化）不触发 StaleContext。

### 交付合同

| 面 | 变更 | 语义 |
|---|---|---|
| `gateway_start.rs` `LogicalPlanLaunch` | `working_dir` 改义 canonical root（`gateway.authority_root()` 唯一来源，不读 repository_path）；新增 `target_worktree`（成员 checkout） | cwd/target 显式分离；`planning_request()`：cwd=root、target=checkout(ids, target_worktree)/aggregate_root(target_worktree)、readable_roots=[root]、空写根——成员 cwd fallback 消除（REQ-PLN-07） |
| `gateway_start.rs` `start_work_item_plan_author` | Logical 分支把 `input.working_directory` 重绑 envelope 冻结 root | 正常 run（worktree=成员）与 B3 重建 run（worktree=rebuilt.cwd=root）spawn cwd 恒为 root、worktree 保持 target 语义透传（与 2.1 author 链 provider_drive.rs:176 同型）——B3 漂移断裂随之消除 |
| `lifecycle.rs` `routing_reference_context`（拆至 `lifecycle/routing_reference.inc.rs`） | cwd=gateway.authority_root()，target=canonicalize(repository_path)，结构体字面量显式分离（不再用 cwd==target 的 `planning()` 构造器） | prompt 路由引用镜像真实 launch 形态；解析失败仍回落 Legacy（启动链 fail-closed 不受影响） |
| `planning_context.rs`+resolver | snapshot 新增 `working_directory`（`#[serde(default)]`，旧快照读取后与重算指纹不等→一次性 supersede，与 2.5 cwd 维度升级同语义）并纳入 `access_fingerprint_value` | 聚合根迁移→指纹漂移→StaleContext 重建（REQ-PLN-03「cwd 指纹不一致」场景闭合）；resolver 冻结 `snapshot.working_directory=manifest root` |

### 新测试（六个，先红后绿；红=行为断言失败非编译错）

- `logical_plan_launch_uses_root_cwd_not_member_repository_path`（gateway_start tests）——launch cwd=canonical root、target_worktree=成员、request cwd=root/target=成员/readable=[root]/空写根
- `start_work_item_plan_author_rebinds_input_cwd_to_envelope_root_for_rebuild`——重建形态启动成功且 spawn cwd=envelope root（修复前红：启动被 cwd 漂移拒绝）
- `start_work_item_plan_author_binds_normal_input_cwd_to_envelope_root`——正常形态 worktree=成员、spawn cwd=root（修复前红：cwd=None）
- `lc_plan_author_run_spawns_from_root_cwd_with_member_target`（lc_admission）——LC SC 全链 StartGeneration→gateway spawn 捕获 cwd=canonical root、worktree=canonical 成员（修复前红：cwd=None）
- `manifest_root_change_triggers_fingerprint_drift`（resolver tests）——root 迁移其余维度不变→StaleContext+snapshot.working_directory 冻结（修复前红：SameContext）
- `access_fingerprint_changes_when_working_directory_drifts`——指纹含 cwd 维度

### 行数守卫

lifecycle.rs 1208>1200 → 拆 `lifecycle/routing_reference.inc.rs`（纯移动、include! 同模块域、author_root_launch 同型）→ 1140 行；it_core large_file_guard 绿。

### Phase 2 收口核验（实际执行）

| 命令 | 结果 |
|---|---|
| `cargo test --locked --lib` | **3979 passed / 0 failed**（3 ignored；基线 3973+6 新增；lifecycle 拆分后终态复跑同绿）|
| `cargo test --locked --test it_web` | **354 passed / 0 failed**（1 ignored）|
| `cargo test --locked --test it_core` | 186 passed / 1 failed——`workspace_ws_idle_timeout_records_server_idle_connection_diagnostic` 负载时序 flaky，**单跑绿**（B6 已登记 flaky 家族同性质；idle 诊断面与 planning 链无交集，非本任务引入）|
| `cargo test --locked --test web_logical_codebase_entrypoints` | **44 passed / 0 failed** |
| 定向：`gateway_start`+`planning_resume`+`single_candidate_lc_admission`+`planning_context` 43/0；`provider_run_events`+`author_revision_loop`+`workspace_engine` 1361/0 |

### 贯穿链核验结论（tasks.md 2.5 验证口径）

- **context/cwd/prompt/audit/revision/resume/WS follow-up 全链 root cwd**：Author/ChoiceFollowup（2.1）、Revision（2.3）、ReviewOnly/plan review（2.4）、split sync（2.5 尾）、Coder/retry（2.6）、Group/InternalReviewer（2.7）已迁+本轮 plan author 链（WorkItemPlan/SC/follow-ups）补齐=全入口闭合；audit 由 gateway start_streaming 留痕+run-bound tool-policy sink（既有）；prompt 路由引用本轮对齐真实 launch；resume 双指纹（snapshot 指纹+envelope fingerprint）均含 cwd。
- **成员/checkout/policy/access 指纹漂移触发重建**：membership_revision/checkout_id/revision/dirty/available/policy_digest（既有）+cwd（本轮）全维度入指纹；invalidation 强制 StaleContext（既有）。
- **禁项核验**：`issue.repo_id`/first Story fallback 仅存在于 legacy 单仓链（resolver 显式 fail-closed「primary fallback forbidden」）；成员 cwd fallback 由本轮消除并经 `logical_plan_launch_uses_root_cwd_not_member_repository_path` 锁定。
- **边界**：target identity 不进 snapshot 结构——envelope resume fingerprint（2.5 已含 target/cwd/policy/version/capability）持有 target/git 身份，与 snapshot 指纹互补（resolver 注释既有分工），不另造同义字段。
