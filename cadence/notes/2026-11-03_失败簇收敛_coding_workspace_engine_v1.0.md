# 失败簇收敛:coding_workspace_engine::tests(235)v1.0

- 日期:2026-11-03
- 工作者:FixCWECluster2(接手;首任 FixCWECluster 因模型通道故障中止,其已落地改动按「方向正确、接着做」原则沿用,未推倒重来)
- 工作树:`/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`
- 范围:`coding_workspace_engine::tests` 全部失败(派工单口径 235;首任基线实测 249,含 FailureTriage 迁移 `running_attempt_with_worktree` 后新暴露的红)

## 0. 接手时盘上状态(14:27 核对)

首任(FixCWECluster,ds-task,14:05–14:23)及其子 agent 已按「gateway 夹具迁移」方向落地如下改动,**方向与三步改法一致**(①`seed_complete` 建 LC+issue ②attempt 带 `logical.snapshot(paths)` ③provider 经 `engine_with_fixture_gateway` validated 启动),本 worker 全部沿用:

| 簇 | 接手时状态 | 证据 |
|---|---|---|
| code_review_triage(14)| **已迁移且已验证**:首任窄跑 33 passed / 0 failed(bg_773) | `code_review_triage.rs`(+74/-61)、`verification_triage_gates.rs` |
| group_completion_recovery(14,与 authority 同模块计数)| 已迁移未验证 | `group_completion_recovery.rs`(+78/-51):`group_completion_fixture_at_stage` → seed_complete + snapshot;`lock_group_fixture_with_other_owner` → 仓维(`upsert_repo_worktree` + `try_acquire_repo_worktree_lock`);owner 冲突断言 kind 同步为 `repo_worktree_lock_owner` |
| group_completion_authority(34)| 夹具在 recovery 文件,已随上条连带迁移;文件本身零 diff | 用例全部经 `group_completion_fixture` → 已迁移的 `at_stage` |
| provider_failure_recovery(27,基线实测 44)| 已迁移未验证 | `provider_failure_recovery.rs`(+213/-80)+ 4 子文件:裸 engine+provider → `engine_with_fixture_gateway`(fake adapter 同时是行为面与 registry spawn 面);issue 维锁 → 仓维(`UpsertRepoSharedWorktreeInput`/`release_repo_worktree_lock`/`try_acquire_repo_worktree_lock`,维度取 `snapshot.logical_repository_id`) |
| provider_idle_stall(10)| 已迁移未验证 | `provider_failure_recovery/provider_idle_stall.rs`(+148/-33) |
| provider_start_persistence(11,基线 10)| 已迁移未验证 | `provider_start_persistence.rs`(+235/-142)+ 2 子文件:`engine_with`(注入 gateway 的 engine)+ `prepare_coder_validated_launch` 产 `validated_input` 塞进 invocation(`validated_input: Some(..)`) |
| tests.rs 共享夹具 | `running_attempt_with_worktree()` 已是 seed_complete + snapshot(130 处用例共用);`start_attempt_enters_running_through_admission_ticket` 同;`delivery_fixture` 的 issue 补 `logical_codebase_id` | tests.rs(+41) |
| logical_test_fixture.rs | 首任 14:05 扩充:成员 checkout 移入聚合根之下(`aggregate-root/repository_0001`,满足 `scope_member_prefix`) | 14:05 版为最新,本 worker 沿用 |
| provider_gateway_validated_input.rs | `build_gateway_with_registry` 复用已播种 manifest(避免 `ensure_bootstrap` `IdentityMismatch`)、authority root 恒取 workspace root;新增 `ValidatedPassthroughAdapter`(桥接裸 `start` 的 fake adapter 到 validated 通道,trait 默认不回落)/`register_fixture_adapter`(5 名注册)/`engine_with_fixture_gateway` | 13:33 版 |
| 外围零散簇(gates_scope_domain 9 / coder_resume_recovery 7 / resume_worktree_lock 6 / cross_attempt_handoff 6 / coding_operational_blocker 6 / internal_review_triage 5 / group_terminal 5 / group_review_identity_snapshot 5 / gate_rework 5 / group_failure_same_attempt 4 / plan_amendment 19 / campaign_* / advance_ready_only 3 / …)| 首任子 agent 14:04–14:23 已批量迁移(约 35 文件),未编译未窄跑 | git diff --stat(见 §3 行数门禁表) |

**不属于失败簇(零 diff,跳过)**:`provider_failure_recovery/logical_legacy_fallback.rs`、`group_review_idle_stall.rs`(provider_idle_stall 10 条全部在 `provider_failure_recovery/provider_idle_stall.rs`)。

**生产文件核查**:`advance_ready_only.rs` 14:20:44 的改动经查为纯测试夹具(该文件为测试辅助文件);gates.rs / worktree_lock.inc.rs / group.rs / handoffs.rs / cross_target_check.rs / issue_cross_repo_review.rs 的 diff 为本分支既有实现改动(上午时段,非本簇 worker 所改)。

## 1. 接手后的增量工作

### 1.1 第一次全量窄跑(基线核对后)

```bash
flock /tmp/aria-cargo.lock -c 'cargo test --locked --lib coding_workspace_engine 2>&1 | tee /tmp/cwe-final.log | tail -8'
```
**693 passed / 49 failed**(首任基线 249 → 49;修复 206 条,新增 6 条暴露的既有欠账,见 §1.2)。编译检查(bg_835)先行通过(1m51s,仅 2 个 dead_code warning)⇒ 首任半成品无断裂。

### 1.2 残留 49 条逐簇归因与修复(全部夹具/期望公式级,除 2 条报 Main)

| # | 残留簇(条数) | 首条 panic 归因 | 处置 |
|---|---|---|---|
| A | group_completion_authority 17 + runtime_handoff_impact 1 | 完成链 `mark_repo_worktree_completed_item` 读仓维 `shared-worktrees/<logical_id>.json` ⇒ `Store(Io)`:夹具只建 LC+快照,未建仓维 worktree 记录 | `group_completion_recovery.rs::group_completion_fixture_at_stage` 与 `runtime_handoff_impact.rs::runtime_handoff_fixture` 各补 `upsert_repo_worktree`(**无锁**记录;owner 冲突/锁转移用例自行在其上演出,清洁记录下 acquire 必然成功) |
| B | d4_member_baseline 10 + provider_gateway_validated_input 2 + campaign_stage3_amendment 2 | `save_manifest: IdentityMismatch`:已播种 manifest(随机 lc_id 全局唯一)后再 save 新 manifest 被拒 | ① `campaign_amendment_fixture` 改**复用** part_09 建仓链权威(load_manifest + load_member + 首成员主 checkout);② `seed_logical_codebase_checkout` 改 load-or-create + 追加 nil 成员(membership_revision+1);③ `seed_d4_member_codebase` 改 load-or-create + **替换**成员集(D4 全员基线按 manifest active 成员采样) |
| C | coding_output_teaching_reredrive 3 | `ProviderStream("logical_provider_gateway_required")`:文件完全未被首任覆盖 | 三处裸 engine+provider → `engine_with_fixture_gateway`(fake adapter 经 `ValidatedPassthroughAdapter` 桥接 validated 通道) |
| D | handoffs::termination_finalize_tests 2(612 既有欠账,基线未跑至此) | ① `product_store_not_found: issue issue_0001`(夹具无 issue 记录);② `Io("target_snapshot_identity_drifted")`(夹具 attempt 无快照 + issue 维 worktree 记录) | `handoffs.rs::terminated_work_item_attempt_with_dirty_issue_worktree`(#[cfg(test)] 模块)迁移:`seed_complete` + 快照 + 仓维 worktree 记录/锁。断言零改动——dirty gate 对照锚(`shared_worktree_dirty_manual_gate`)在仓维路由下同样生效(gates.rs:465-467) |
| E | group_amendment_chain 5(基线 2 + 新暴露 3) | `get_attempt_for_work_item_group(.., None)` NotFound:attempt 带快照后 target bucket=Some(member),None 只匹配无快照 legacy | `publish_amendment_manifest` 增 `member_id` 参数,按桶检索传 `Some(member_id)`(夹具辅助查询参数,非断言) |
| F | group_review_executor 2 | ① `tool policy guard: policy role reviewer must carry DenyFileWriteBuiltins`:无窗路径生产不伪铸 ReviewerVerify 合同(由真实 provider 启动检查拒绝),fake adapter 场景下被通用 spawn 前守卫拦;② 同根因连锁(`Elapsed`) | 两处 `RealGroupReviewExecutor::new(..).with_review_window_id("w-group-review-executor-fixture")`(照 `group_review_identity_snapshot.rs:874` 既有先例) |
| G1 | start_attempt_enters_running_through_admission_ticket 1 | `Io("mixed_target_group_rejected")`:Logical 路由下 drafts 无 `target_repository_id` | 用例内补 drafts target 绑定(照 `advance_ready_only.rs::bind_single_target_drafts` 同款,`Some(logical.member_id)`) |
| G2 | provider_execution_context 1 | LC attempt coder prompt 在 write_scope 后追加 `[worktree_discipline]` 段(r54 越界根修,coding.rs:225-229),期望公式未含 | **断言更新**:期望拼接公式在 write_scope 与 host_check 之间插入 `worktree_discipline_section(&worktree)`(与生产同一单源构造;reviewer 侧无此段,断言不受影响)。迁移标准对照 `cadence/notes/2026-11-03_失败簇收敛_workspace_engine_v1.0.md` §3.1(旧断言前提「Legacy 单仓派生 ⇒ target=None」随单仓入口删除作废 ⇒ 按新契约同源构造迁移,新断言更严不弱化,未删任何断言)——本行同判据 |
| G3 | validated_streaming_input_for_role_returns_none_for_legacy_attempt 1 | 共享夹具已带快照 ⇒ 「legacy(无快照)attempt → None」分支现场消失 | 用例内显式剥掉 `target_snapshot`(与 `with_target_snapshot` 相对的现场构造),断言零改动 |
| G4 | prestart_zero_output::modern_start_io 1 | **实现缺陷**(详见 §5.1):`provider_stream/launch.rs::provider_adapter_error_from_gateway` 把 `ProviderGatewayError::Adapter(inner)` 拍平为 `code=ProviderUnavailable/details=Display`,丢弃 inner 的 stdout/stderr/exit_code ⇒ "Text file busy" 不再出现在 message ⇒ `classify_transport_message` 无法识别 StartIo ⇒ 误判 `non_retryable(provider_unavailable)` | **未改,报 Main**。生产影响:LC 路径一切依赖 stderr 关键词的 transport 分类(connection/text file busy 等)同样失效,不止本用例 |
| G5 | group_terminal::group_review_material_failure 1 | **实现缺陷候选**(详见 §5.2):`handoffs.rs::handle_attempt_failed` group 分支(373-378)从 **issue 维** `get_issue_shared_worktree` 读 active 指针,而两态化后 group attempt 的 worktree 记录只在**仓维** ⇒ 恒 None ⇒ fallback 与仓维记录 active(wi2)不一致 ⇒ `ensure_issue_shared_worktree_clean` 跳过 dirty 检查 ⇒ `release_repo_worktree_lock_by_owner` 清空锁 ⇒ 用例「material failure 不得破坏转移锁」的断言(left None / right Some(wi2))失败 | **未改,报 Main**。维度不对称:同函数的 clean 检查(gates.rs:465-467)已按 route 走仓维 |

### 1.3 与首任改动的相容性核查

- `pi_rework_permission.rs` 为 Pi coder 自建本地 gateway 装配(共享 gateway 的 availability gate 只登记 ClaudeCode/Codex)——用例内局部装配,非第二套共享夹具,合理保留。
- `StaticCapabilitySource` 对全部 dialect(ClaudeCode/Codex/Pi/KimiCode)返回 Confirmed;`codex_danger_full_access_unsupported` 为历史诊断码(当前启动路径不再产生)⇒ 首任未覆写 provider 名的做法经核安全。
- 首任子 agent 摸过的 ~15 个外围文件 diff 逐一抽审(gate_coder_feedback / campaign_stage3_amendment / group_review_identity_snapshot / pi_rework_permission / coder_resume_recovery / group_terminal / provider_failure_recovery 系 / provider_start_persistence 系 / tests.rs),模式一致无断裂;`advance_ready_only.rs`(生产路径下测试辅助文件)14:20:44 改动经查为纯夹具。


### 1.5 第 3 次窄跑暴露的回归与追加修复(17:44-17:47)

第 3 次窄跑(bg_922,编译快照含 §1.2 修复但不含绑锁修正)部分日志(706 ok / 34 failed,另 2 用例挂死)暴露 §1.2-A 首版「**无锁**仓维记录」不充分:

- **根因**:租约分类(`classify_worktree_lease_for_attempt`)对「有快照 + 仓维记录在场但无人持锁」判 `coding_lease_state_unknown` 停等 ⇒ 完成链既到不了 preflight(zero-write 用例 12 条新红:报错错位)也到不了 mutation pause 点(2 条 pause 用例永等挂死)。对比:记录**缺失**时分类为「无租约事实」反而放行——「有记录无锁」比「无记录」更糟。
- **追加修复**:① `group_completion_fixture_at_stage` / `runtime_handoff_fixture` 改仓维记录 + `bind_repo_lock_to_attempt`(自持活跃租约 ⇒ Allowed,与生产 acquire+bind 语义一致);② `lock_group_fixture_with_other_owner` 先 `release_repo_worktree_lock_by_owner(attempt)` 再以 `coding_attempt_other` acquire(冲突现场保持,断言不变);③ `git_operation_reconcile::abort_reconciles_unjournaled_worktree_side_effect`(49 名单内漏修项):`handle_abort` 终止链解析 issue 归属且 clean 检查内部按 route 对无快照 fail-closed ⇒ 补 `seed_complete` + 快照;④ `campaign_amendment_fixture`(final3 仍红 2 条):amendment 恢复链进 Running 必经 admission,同缺仓维锁 ⇒ 补仓维记录 + `bind_repo_lock_to_attempt("wi_a", attempt)`。
- `try_acquire_repo_worktree_lock` 对「同 active + 同 owner」幂等(acquired=true)⇒ n5_second_unit 后半段自 acquire 与 at_stage 预绑锁不冲突(已核对 repo_locks.rs:173-216)。

### 1.4 两条实现缺陷(Main 已授权修复,2026-11-03 16:40 裁定)

#### 缺陷1(核心路径,优先):gateway Adapter 错误拍平丢结构化字段

- **位置**:`src/product/coding_workspace_engine/provider_stream/launch.rs::provider_adapter_error_from_gateway`
- **机制**:`ProviderGatewayError::Adapter(inner)` 被拍平为 `{code: ProviderUnavailable, details: Display, stdout/stderr/exit_code 全空}`,而 `ProviderAdapterError` 的 Display 只有 `{code:?}: {details}`(不含 stderr)⇒ transport 分类(`provider_retry::classify_transport_message` 的 message 拼接 stdout/stderr)拿不到 "text file busy" 等关键词 ⇒ LC 路径 spawn 失败被误判 `non_retryable(provider_unavailable)`。
- **生产影响**:去单仓入口后 LC 主链路的 transport 分类整体失能——一切依赖 stderr/stdout 关键词的分类(connection interrupted / text file busy / resume stalled…)在 gateway 路径下全部失效,不止被测的这一条。
- **修法**(已落地):Adapter 臂 `inner.clone()` 透传(保留 code/stdout/stderr/exit_code/timeout/duration);其余 gateway 自身 fail-closed 门(政策/复验/registry/可用性)保持既有映射。未动 `classify_transport_message` 分类规则、未动 `ProviderAdapterError` Display(经核 `classify_launch_trust_failure` 对非 `ProviderUnavailable` 变体原样返回 details,透传后 details 不被改写)。

#### 缺陷2:handle_attempt_failed 的 group 分支维度不对称

- **位置**:`src/product/coding_workspace_engine/handoffs.rs::handle_attempt_failed`(group 分支 active 指针读取)
- **机制**:group 分支从 **issue 维** `get_issue_shared_worktree` 读 active 指针,而两态化后 group attempt 的 worktree 记录只在**仓维** ⇒ 恒 None ⇒ 回退 attempt 自身指针 ⇒ 与仓维记录 active(如已转移的 `work_item_0002`)不一致 ⇒ `ensure_issue_shared_worktree_clean` 因 active 不匹配跳过 dirty 检查(gates.rs:472)⇒ `release_repo_worktree_lock_by_owner` 清空他人/已转移的锁。决定性证据:同函数下方 clean 检查(gates.rs:465-467)**已**按 `route_issue_shared_worktree` 走仓维——一个函数两个维度。
- **生产影响**:group attempt 失败收尾误释放共享仓维锁(dirty worktree 的保留语义被绕过),`group_terminal::group_review_material_failure_keeps_business_error_when_terminal_lock_was_transferred` 钉住的「material failure 不得破坏已转移锁」不变量被破坏。
- **修法**(已落地):该分支改按 `route_issue_shared_worktree(&updated)` 的仓维记录读 active 指针(与 465-467 对齐;无仓维记录时仍回退 `attempt_work_item_id`,记录缺失语义不变)。改动仅此一处,未扩大。

## 2. 窄跑证据

- **第 1 次**(§1.1):**693 passed / 49 failed**(首任基线 249 → 49;编译检查先行通过,首任半成品无断裂)。
- **第 2 次**:被并行 web worker(FixWebCluster)编辑中途的暂态编译错误挡住(`workspace_ws_handler/tests` 两个 E0433/E0599,与本簇无关),未取得用例结果。
- **第 3 次**(bg_922;含 §1.2 修复、不含 §1.5 绑锁修正):部分结果 706 ok / 34 failed + 2 挂死(挂死与 12 条 zero-write 新红均系 §1.5 根因);挂死令 harness 未打印失败详情与汇总。34 条 = 49 − 13(§1.2 B/C 类修复生效)− 2(§1.4 实现修复生效),账目吻合。
- **第 4 次**(含 §1.4 实现修复 + §1.5 追加修复;排队于 Main 统一全量之后):待补。

## 3. 行数门禁(wc -l,2026-11-03 16:35 实测)

| 文件 | 改后行数 | 说明 |
|---|---|---|
| tests/group_completion_recovery.rs | 1656 | 本 worker +12(仓维 worktree 记录) |
| tests/group_completion_authority.rs | 1207 | 零改动(随夹具连带) |
| tests/runtime_handoff_impact.rs | 822 | 本 worker +11 |
| tests/campaign_stage3_amendment.rs | 1137 | 本 worker 复用权威改写 |
| tests/provider_gateway_validated_input.rs | 816 | 本 worker load-or-create + 剥快照用例 |
| tests/provider_gateway_validated_input/d4_member_baseline.rs | 1052 | 本 worker load-or-create |
| tests/coding_output_teaching_reredrive.rs | 308 | 本 worker gateway 迁移 |
| tests/group_amendment_chain.rs | 967 | 本 worker +9(桶检索参数) |
| tests/group_review_executor.rs | 212 | 本 worker +5(评审窗口) |
| tests/provider_execution_context.rs | 764 | 本 worker 期望公式 +worktree_discipline 段 |
| tests.rs | 1131 | 本 worker +16(drafts 绑定) |
| handoffs.rs(#[cfg(test)] 夹具 + 实现 group 分支) | 1092 | 夹具迁移 +14;实现修复 +12 |
| provider_stream/launch.rs | 517 | 实现修复 +11(缺陷1) |
| code_review_triage.rs / provider_failure_recovery.rs / provider_start_persistence.rs | 1204 / 1689 / 1384 | 首任改动,本 worker 未触碰 |

均未触及 1200 硬线(provider_failure_recovery.rs 1689 / group_completion_recovery.rs 1656 / provider_start_persistence.rs 1384 为首任迁移即如此,登记数字由 Main 统一处理);`large_file_guard.rs` 未改。

## 4. 共享夹具是否需要扩能力

**不需要**。本簇全部修复使用的能力(`seed_complete`/`snapshot`/`upsert_repo_worktree`/`bind_repo_lock_to_attempt`/`init_member_git_repo`)与 gateway 装配面(`build_gateway_with_registry`/`engine_with_fixture_gateway`/`ValidatedPassthroughAdapter`/`register_fixture_adapter`)均为现成资产。「复用已有权威」的 load_manifest/load_member 读法为 store 公开 API 的直接调用,已在用例内就地使用,无需共享层扩展。`pi_rework_permission.rs` 的 Pi 本地 gateway 为用例内局部装配(共享 availability gate 只登记 ClaudeCode/Codex),不构成第二套共享夹具。
## 5. 仍未解 / 需 Main 裁定

| # | 用例 | 归因 | 状态 |
|---|---|---|---|
| ① | `abort_during_running_group_completion_prewrite_pause_is_stable` | 夹具预绑仓维锁后 handle_abort 的 clean 检查对 dirty worktree 触发 `SharedWorktreeDirtyManualGate`(生产正确语义);用例验证的是 pause 点 abort 生效,dirty 非其变量 | **已修**(fixture dirty 参数 true→false,断言零改动),待下次窄跑确认 |
| ② | `abort_during_completed_retry_prewrite_pause_is_stable` | abort 正确释放仓维锁(owner=attempt)后,resume 的 completed-retry completion 先撞 `Conflict{repo_worktree_lock_owner}` 而非断言期望的 `group_completion_status_not_ready`——失败形态/检查顺序问题(锁动作先于 status 复核),夹具层无法在不伪造现场的前提下修 | **已按 Main 裁定迁移断言**。契约核查(2026-11-03):`group_completion_status_not_ready` 在 `openspec/` 零命中,仅 `group_completion.rs:656` 实现消息文本与本测试自约——**无契约出处** ⇒ 断「fail-closed + 零写」不变量族(`repo_worktree_lock_owner` 或 `group_completion_status_not_ready` 均为「abort 后不得写」的拒绝形态,一并接受),用例内注释留痕「错误码因锁动作先于 status 复核而变更」 |
| ③④ | `campaign_stage3_amendment_returns_to_same_attempt_via_original_plan_session` / `..._incompatible_revision_fails_closed_without_path_switch` | approve 链 final compile 报 `work_item_target_missing: wi_a has no intent target and no deterministic fallback repository`——amendment 运行时产出的 amended drafts 不带 `target_repository_id`,与 FinishWSEngine 所修 55 条「outline/draft target 不同源」同源根因(amended draft 落盘点在 workspace/plan_amendment 流程内,超出本簇夹具面) | **Main 已裁定归并 FinishWSEngine**(其已在改同一根因),本 worker 不再触碰 |

### 逐簇剩余数(第 4 次窄跑口径,249 → 4)

| 簇 | 接手基线 | 第 4 次后剩余 |
|---|---|---|
| code_review_triage(含 verification_triage_gates) | 12 | 0 |
| provider_failure_recovery(含 provider_idle_stall 10) | 44 | 0 |
| provider_start_persistence | 10 | 0 |
| group_completion_authority / recovery(合并口径) | 34 | 2(§5 ①②;①已修待验) |
| provider_gateway_validated_input(含 d4_member_baseline 10) | 17 | 0 |
| 外围零散(gates_scope_domain / coder_resume_recovery / resume_worktree_lock / cross_attempt_handoff / coding_operational_blocker / internal_review_triage / group_terminal / gate_rework / group_failure_same_attempt / plan_amendment 系 / coding_output_teaching_reredrive / runtime_handoff_* / schema_v2_runtime / runner_fallback_commit / git_operation_reconcile / start_attempt / advance_ready_only / write_scope_prompt / worktree_discipline / reviewer_configuration_gate / provider_execution_context / provider_driven / parser_prompt / gates_worktree_routing / group_final_readiness 系 / group_amendment_chain / campaign_stage3_dependency / campaign_stage3_provider_failure / gate_coder_feedback / sc_group_admission_isolation / handoffs::termination_finalize_tests) | ~132 | 2(§5 ③④) |
