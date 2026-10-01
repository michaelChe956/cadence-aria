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
