# Task 3.1 报告：cwd≠target 专用回归锁 fixture（含覆盖盘点）

**状态：complete**（零生产代码改动；3 个真缺口测试落地，其余契约点盘点确认已覆盖）

## 1. 覆盖盘点结论（先行，非硬造）

对 tasks.md 3.1 验证口径（root cwd+member target 放行 / root·target 指纹变化拒绝 / 普通缺根 session 零 spawn）逐点盘点 1.2/2.1-2.8 已建分离形态测试：

| 契约验证点 | 层 | 既有覆盖 | 形态/缺口判定 | 处置 |
|---|---|---|---|---|
| root cwd+member target 放行 | gateway validate+spawn | `cwd_outside_authority_or_target_git_identity_drift_zero_spawns`(d)、`provider_specific_writable_evidence_does_not_expand_coding_root`（2.2） | root=临时目录、target=普通目录；无真实 Git 成员、无空格/symlink | **真缺口** → 新增 `logical_root_cwd_member_target_fixture_is_allowed` |
| root 指纹变化拒绝 | gateway 双工厂 | `dual_gateway_factories_reject_canonical_root_mismatch`（2.2；空格+symlink，登记/receipt/envelope-cwd 三投影 fail-closed） | 已覆盖 | 零补 |
| root/target 指纹变化拒绝（resume） | gateway `resume_or_start` | 既有 supersede 测试全部同 cwd 旧形态（version/policy 维度）；cwd 维度仅 2.5 字段级 digest（engine_gateway_guard.rs）+ 2.3 引擎级 E2E（Revision 族）；target 维度仅 2.8 resolver 记录级 | gateway 层分离形态处置（StartNew+supersede 审计）无直测 | **真缺口** → 新增 `logical_root_or_target_fingerprint_drift_is_rejected` |
| target 指纹变化拒绝 | resolver planning snapshot | `checkout_identity_change_triggers_fingerprint_drift`、`manifest_root_change_triggers_fingerprint_drift`、`resume_with_matching_fingerprint_reuses_context_and_mismatch_rebuilds`（2.8） | 已覆盖 | 零补（planning_context_resolver_tests.inc.rs 未动） |
| target Git 身份拒绝 | 生产 target resolver | `coding_target_rejects_*`、`*_worktree_gitfile_pointing_outside_main_git_dir`（真实 git、同 cwd 形态） | 已覆盖；分离形态由新 fixture 测试经 `ProductionPolicyTargetResolver::for_lc` 全链顺带锁 | 零补 |
| 普通缺根 session 零 spawn | admission | `bootstrap_phase_only_waives_missing_root_rules` Normal 对照、`missing_member_language_rule_waits_before_provider_spawn`（1.2） | 全部同 cwd、aggregate-root target；admission 从未以「root cwd+成员 checkout target」驱动过 | **真缺口** → 新增 `ordinary_missing_root_session_zero_spawns` |
| cwd 越界/symlink 逃逸/Git identity 漂移 zero spawn | gateway | task28 (a)(b)(c)（2.2） | 已覆盖 | 零补 |
| 写证据不随 cwd 扩大 | envelope | `provider_specific_writable_evidence_does_not_expand_coding_root`（2.2） | 已覆盖 | 零补 |
| cwd 快照冻结/根迁移 Stale | resolver | `planning_launch_has_no_write_roots_and_cwd_is_aggregate_root`、`resolver_produces_single_snapshot_cwd_and_inventory_for_all_artifacts`（2.8） | 已覆盖 | 零补 |

盘点结果与派工预期一致：resolver 层零剩余；真缺口集中在 gateway 分离形态 fixture / gateway resume 分离形态处置 / admission 分离形态缺根。

## 2. 新增测试（3 个，均真缺口）

### `logical_root_cwd_member_target_fixture_is_allowed`（task28_root_authority.inc.rs）
brief Step 3 的 deterministic fixture：**非 Git canonical LC root（路径含空格 `lc root`，登记/manifest 投影经 symlink alias 字面）+ 成员主仓真实 Git checkout（.git 目录）+ 真实 `git worktree add` 链接工作树（.git 文件形态）**，gateway 注入生产 `ProductionPolicyTargetResolver::for_lc`（三层身份+git-dir identity 复验走真实生产路径，无成员 fallback）。断言：
- validate 放行：envelope 冻结 canonical cwd（root≠target 显式 `assert_ne`）与经生产 resolver 复验的 canonical member target；
- spawn 放行：input cwd=冻结 root → 真实启动（start_count=1）；
- spawn input cwd 回退 member target → `TargetMismatch{field:"cwd"}` fail-closed、零新增启动（REQ-ENV-01 禁止回退 member cwd）。

### `logical_root_or_target_fingerprint_drift_is_rejected`（task28_root_authority.inc.rs）
gateway `resume_or_start` 分离形态维度隔离锁：
- 仅 root cwd 漂移（authority 内 legacy root → root，target 不变）→ StartNew+supersede（session id 透传；审计 reason=`resume_fingerprint_mismatch`）；
- 仅 member target 漂移（cwd 不变，member a→b）→ StartNew+supersede；
- 对照全维度一致 → Resume、零新增 supersede；全程 `registry_start_count()==0`。

### `ordinary_missing_root_session_zero_spawns`（provider_admission_preflight_tests.inc.rs）
admission 首次以分离形态驱动（复用 admission_fixture 的真实 Git 成员）：cwd=root 内自引用 symlink alias 字面（钉 admission 步骤 6 复验判据 canonical 相等而非字面相等）+ target=成员 checkout：
- Normal 相位+成员缺 `.claude/rules/language.md` → `Waiting{member_language_rules_missing}` + `start_count()==0`（reason_code 断言同时锁定 cwd alias 复验未被破坏——若破坏会以 `spawn_revalidation_drift` 出现）；
- 对照：补齐根规则后同一分离形态请求 ready——分离形态本身不是阻断源。

## 3. G 边界（旧同 cwd fixture 保留）

- provider_gateway_tests.rs:70 `resume_fingerprint_changes_when_provider_version_or_snapshot_drifts`、:868 `logical_provider_entrypoints_use_gateway_for_sync_and_streaming_stacks` 未迁移、未重名、断言原样，运行确认仍绿。
- 新测试与既有测试零重名重复（三个名称全库唯一）。

## 4. 与 brief Files 的偏差（均有据）

| brief 列文件 | 实际处置 | 理由 |
|---|---|---|
| provider_gateway_tests.rs（主文件） | 未改（新测试进其 include 的 task28_root_authority.inc.rs） | 主文件 1174/1200 守卫余量仅 26 行；task28 文件本就是 cwd≠target 专区（2.2 建） |
| audit.inc.rs | 未改 | 审计断言（supersede_count/last_supersede_reason）已内联在 task28 新测试，无独立审计缺口 |
| planning_context_resolver_tests.inc.rs | 未改 | resolver 层盘点零缺口（见矩阵），按「不硬造重复测试」不补 |
| （brief 未列）provider_admission_preflight_tests.inc.rs | 新增 1 测试 | 缺根 admission 的契约落点在 admission 层（契约原文「缺根 admission fixture」），fixture 与断言 helper 均在该文件 |

## 5. TDD 说明

brief Step 2 预期 FAIL 的前提（既有 fixture 将 cwd/target 绑定同一路径）已在 2.2（错误等式拆除）/2.5（cwd 指纹维度）红→绿拆除；本任务为回归锁（生产逻辑 Phase 2 已全落地，brief 明言「只扩充测试 helper」），三测试对现行实现直接绿即锁成立，无生产代码改动。

## 6. 验证（实际执行）

| 命令 | 结果 |
|---|---|
| `cargo test --locked --lib logical_root_cwd_member_target_fixture_is_allowed` | 1 passed |
| `cargo test --locked --lib logical_root_or_target_fingerprint_drift_is_rejected` | 1 passed |
| `cargo test --locked --lib ordinary_missing_root_session_zero_spawns` | 1 passed |
| `cargo test --locked --lib logical_provider_entrypoints_use_gateway_for_sync_and_streaming_stacks` | 1 passed（G 边界） |
| `cargo test --locked --lib resume_fingerprint_changes_when_provider_version_or_snapshot_drifts` | 1 passed（G 边界） |
| `cargo test --locked --lib task28_gateway_root_authority` | 5 passed（3 既有+2 新） |
| `cargo test --locked --lib provider_admission` | 9 passed（8 既有+1 新） |
| `cargo test --locked --lib resume_or_start` | 4 passed |

全量 lib 验证按协作纪律留给 controller 收口（队友并行，避免中途全量误报）。

## 7. 提交

- `src/product/logical_codebase/provider_gateway_tests/task28_root_authority.inc.rs`（420→846 行：fixture+2 测试+追加 imports）
- `src/product/logical_codebase/provider_admission_preflight_tests.inc.rs`（1033→1108 行：+1 测试）
- `.superpowers/sdd/2026-10-01_实施计划_LC根初始化_v1.0/task-3-1-report.md`（本报告）

commit：`test: add logical root cwd target separation fixtures`
