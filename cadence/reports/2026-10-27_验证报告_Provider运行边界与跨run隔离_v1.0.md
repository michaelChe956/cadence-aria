# 验证报告：Provider 运行边界与跨 run 隔离 v1.0 —— Task 11 贯穿验证与零改动核对

- **日期**：2026-10-28（验证执行日；文档族统一沿用 change 日期前缀 2026-10-27）
- **工作树**：`.worktrees/feat-b-0808-add-monorepo`，分支 `feat-b-0808-add-monorepo`，HEAD `e4d683d6fcd9d4cf0ba2dcffbe158f8a59c2c1b3`
- **验证基线（change 起点）**：`840e0c4c8cbc06a949dba3e6d756c332b5513303`（本 change 首个提交，docs/notes）的父提交 `9ffbd1ccfa203011228033753686228b95f982b0`
- **span 构成**：`9ffbd1cc..HEAD` 共 22 个提交 = 本 change 18 个提交 + 4 个并行政策链提交（`d48597a9`、`85497679`、`c1941606`、`f0bde9e9`，只触碰 `logical_codebase/policy_*` 与 `tests/it_web/**/policy_dependency.rs`，不触碰任何 provider 路径与红线文件）。零改动核对按整个 span 进行，结论不受混入提交影响。
- **契约**：`openspec/changes/add-provider-run-boundary/specs/provider-run-boundary/spec.md`（REQ-PRB-01..12）
- **结论先行**：**6 项通过，2 项不通过（行数红线守卫、clippy）**。其中 clippy 为已知既有问题（非本 change 引入）；行数红线守卫发现的 **21 项超限里 20 项为基线即红的既有债务，本 change 新增 1 项：`prompts.rs` 1665→1669 行，超出登记上限 1665**，需 Main 裁定处置（详见 §3）。**在该缺口处置前，本 change 不宜宣称"未触碰行数红线文件"。**

---

## 1. 七件事逐项结论

### 1.1 完整回归（✅ 通过）

命令与实际输出：

```
$ flock /tmp/aria-cargo.lock cargo test --locked --lib
test result: ok. 5501 passed; 0 failed; 7 ignored; 0 measured; 0 filtered out; finished in 97.64s
```

```
$ flock /tmp/aria-cargo.lock cargo check --locked --tests
   Compiling cadence-aria v0.0.9 (/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 57.13s
```

- lib 单测 **5501 passed / 0 failed**（含既有 prompt 预算断言 `f60_exit_contract_matrix.rs:512`（`MARKDOWN_AUTHOR_PROMPT_MAX_BYTES = 6000`，断言上限未被本 change 修改）与内存态 engine 既有用例，未经修改即通过）。
- **集成 target 编译通过**（`--tests` 覆盖 `tests/it_core`、`tests/it_web/**` 全部编译目标，`Finished` + 退出码 0）——今日曾因漏改 `tests/it_web/**` 导致的"单测绿、集成 target 挂"未再发生。

### 1.2 provider 侧零改动核对（✅ 通过）

证明方式：对 `9ffbd1cc..HEAD` 全 span 做 `git diff --stat`（输出为空 = 零字节改动）与 `git log`（输出为空 = 零提交触碰）双重核对。

命令与实际输出（两者均无任何输出行，即空）：

```
$ git diff --stat 9ffbd1cc..HEAD -- \
    src/cross_cutting/codex_provider \
    src/cross_cutting/claude_code_provider \
    src/cross_cutting/pi_provider \
    src/cross_cutting/kimi_code_provider \
    src/cross_cutting/streaming_provider \
    src/product/logical_codebase/provider_gateway.rs
（空输出）

$ git log --oneline 9ffbd1cc..HEAD -- \
    src/cross_cutting/codex_provider \
    src/cross_cutting/claude_code_provider \
    src/cross_cutting/pi_provider \
    src/cross_cutting/kimi_code_provider \
    src/cross_cutting/streaming_provider \
    src/product/logical_codebase/provider_gateway.rs
（空输出）
```

逐项结论（定义位置所在文件均在上面六个路径内，零改动即零字节变化）：

| 契约对象 | 定义位置 | 本 change 是否改动 |
|---|---|---|
| codex / claude / pi / kimi provider 适配层全部 | `src/cross_cutting/{codex,claude_code,pi,kimi}_provider/**` | 未改（diff/log 双空） |
| gateway envelope（含 `writable_roots`/`readable_roots`）与 `SessionResumeFingerprint` | `src/product/logical_codebase/provider_gateway.rs:539` | 未改 |
| `ProviderToolPolicy` 及其 token/digest | `src/cross_cutting/streaming_provider/mod.rs:107`（结构）、`:807`（`tool_policy_digest`）、`:828`（`deny_file_write_builtins_tokens`）、`:1317`（`has_any_tokens`） | 未改 |
| `StreamingProviderInput` 结构 | `src/cross_cutting/streaming_provider/mod.rs:919` | 未改（未新增字段） |
| codex sandbox / 审批策略、T13 FileChange 判失败语义 | `src/cross_cutting/codex_provider/session.rs:46-51`、`:199`、`:577`、`:707` | 未改 |

补充核对：
- `TMPDIR` / `CODEX_HOME` / `HOME` 未被覆盖：对本 change 全部落点文件 grep `TMPDIR|CODEX_HOME|"HOME"`，仅命中 `run_boundary.rs:132`、`:549` 的 `std::env::var_os("HOME")` **只读**（用于定位 `~/.codex/sessions` 等历史目录表），无任何 `set_var`。
- scratch 未进入 envelope / 指纹 / `StreamingProviderInput`：三处定义文件零改动即结构性排除。
- `PRODUCT_DATA_SCHEMA_VERSION` 未 bump：`src/product/product_data_schema.rs` 在 span 内零提交，仍为 v2（`:6`）。
- `Cargo.toml` / `Cargo.lock` 在 span 内零提交（未引入 `regex` 等任何新依赖）。

### 1.3 行数红线文件核对（❌ 6 个中 5 个未触碰，`prompts.rs` +4 行越线）

命令与实际输出（起点 vs HEAD 行数）：

```
$ for f in <六个红线文件>; do echo "$f: baseline=$(git show 9ffbd1cc:$f | wc -l) head=$(wc -l < $f)"; done
src/product/workspace_engine/prompts.rs:                    baseline=1665 head=1669   ← 超登记上限 1665
src/product/workspace_engine/lifecycle.rs:                  baseline=2167 head=2167
src/product/lifecycle_store/plan.rs:                        baseline=1683 head=1683
src/product/lifecycle_store/workspace.rs:                   baseline=1730 head=1730
src/cross_cutting/streaming_provider/mod.rs:                baseline=2062 head=2062
src/web/workspace_ws_handler/run/provider_run.rs:           baseline=1677 head=1677   ← 未到上限（登记 1678）
```

逐文件是否被本 change 修改（`git log --oneline 9ffbd1cc..HEAD -- <file>`）：

| 红线文件 | 行数变化 | 触碰提交 | 判定 |
|---|---|---|---|
| `workspace_engine/prompts.rs` | **1665 → 1669（+4）** | `6dfa715c`（feat(run-boundary)，本 change） | ❌ **越线**：登记上限 1665（`tests/it_core/large_file_guard.rs:201`），增长即报红 |
| `workspace_engine/lifecycle.rs` | 2167 → 2167 | 无 | ✅ 未触碰 |
| `lifecycle_store/plan.rs` | 1683 → 1683 | 无 | ✅ 未触碰 |
| `lifecycle_store/workspace.rs` | 1730 → 1730 | 无 | ✅ 未触碰 |
| `streaming_provider/mod.rs` | 2062 → 2062 | 无 | ✅ 未触碰 |
| `web/workspace_ws_handler/run/provider_run.rs` | 1677 → 1677 | 无 | ✅ 未触碰 |

`prompts.rs` 的 +4 行内容（`6dfa715c` 引入，`#[cfg(test)]` 可见性 re-export，非行为改动）：

```rust
// run-boundary 注入面测试（`tests/part_09.rs` 等非 `prompts` 子树）需以 crate 全路径
// 引用同一标题常量，故仅在测试构建下 re-export（纯可见性，无行为改动）。
#[cfg(test)]
pub(crate) use baseline_teaching::RUN_BOUNDARY_HEADING;
```

**与计划文档的冲突**：实施计划自审结论第 3 条明确写过"`prompts.rs` 已到行数红线不能加 `pub(crate) use`——改为在测试文件内复写规范逐字标题常量"，但实现仍加了 re-export 并被 `tests/part_09.rs` 消费（`crate::product::workspace_engine::prompts::RUN_BOUNDARY_HEADING`）。守卫登记表注释亦规定"确需新增项须用户裁决，不在实现侧自行放宽"。**处置需 Main 裁定**（选项见 §3）。

守卫测试实际结果（计划 Task 11 Step 4 期望 PASS，实际 FAILED）：

```
$ flock /tmp/aria-cargo.lock cargo test --locked --test it_core large_file_guard
test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 188 filtered out; finished in 0.17s
```

失败明细共 **21 项**超限，逐项甄别（方法：文件是否出现在 `git diff --name-only 9ffbd1cc..HEAD`；不在者再取基线行数对照）：

- **20 项为基线即红的既有债务**（文件不在本 change diff 内，基线行数与 HEAD 完全相同，即基线 `9ffbd1cc` 上守卫已是红的）：`coding_attempt_store/{admission_tests,gate,group_initialization}.rs`、`issue_automation_store{,/tests}.rs`、`lifecycle_store/worktree.rs`、`logical_codebase/{bootstrap_tests,provider_admission_preflight_tests,provider_gateway_tests}.rs`、`workspace_engine/tests/advance_handler.rs`、`web/{autopilot_orchestrator.rs,coding_start_parts/tests.inc.rs,handlers/automation_enrollment.rs,handlers/lifecycle_tests_d2_2.inc.rs,handlers/workspace_sc_recovery.rs}`、`web/src/` 下 4 个 ts/tsx 文件。
- **1 项由本 change 新增**：`src/product/workspace_engine/prompts.rs: 1669 行（登记上限 1665 行）`。
- **1 项与并行政策链提交相关但非本 change**：`logical_codebase/policy_tests.inc.rs` 基线 1204（>1200 且无登记，基线已红）→ HEAD 1254，增量 +50 来自 `f0bde9e9`（test(policy)）。
- `GRANDFATHERED` 表本身未新增条目（`large_file_guard.rs` 在 span 内零提交）。

### 1.4 clippy（❌ 不通过，仅已知既有 1 处，非本 change 引入）

命令与实际输出：

```
$ flock /tmp/aria-cargo.lock cargo clippy --locked --all-targets -- -D warnings
error: this function has too many arguments (8/7)
   --> src/product/entity_deletion_cascade.rs:841:1
    |
841 | / fn classify_shared_worktree_ownership(
    = note: `-D clippy::too-many-arguments` implied by `-D warnings`
error: could not compile `cadence-aria` (lib) due to 1 previous error
```

- 全部输出仅此 **1 处 error**（`entity_deletion_cascade.rs:841`，与任务简报"已知既有"一致）；该文件不在本 change 的 31 个改动文件内（基线即有），照实报告、不修。
- 本 change 新增/修改代码（`run_scratch.rs`、`run_boundary.rs`、`baseline_teaching.rs`、`runs.rs` 接线、`work_item_plan.rs` 漏斗等）**零 clippy 告警**（若有会在该 error 之前/之后并列列出）。
- 注意：因 lib 编译被该既有 error 阻断，clippy 未能完整走完所有 target 的 lint 阶段；lib 本体（含新代码）已通过编译前 lint 检查（错误仅上述 1 处）。

### 1.5 格式（✅ 通过）

```
$ cargo fmt --all -- --check
（无 diff 输出，退出码 0）
```

### 1.6 契约逐条落地核对：REQ-PRB-01..12 对照表（✅ 12/12 有实现 + 测试证据）

证明方式：逐条以 `grep`/`read` 定位实现 file:line 与测试名，测试全部包含在 §1.1 的 5501 个通过用例内。

| REQ | 要求摘要 | 实现位置 | 对应测试 |
|---|---|---|---|
| **REQ-PRB-01** | 边界块对作者侧 prompt 无条件注入；聚合/LC、无基线、无 `repository_path` 同样注入；位于最终合同尾之前；内存态 engine 省略且字节不变 | `baseline_teaching.rs:123-133`（`append_author_run_boundary`，`lifecycle_store.is_none()` 整块省略）；`:141`（挂在 `append_author_baseline_teaching` 最前，先于基线解析）；漏斗调用点 `prompts.rs:377`、`prompts.rs:509`、`prompts/revision.rs:69`（均在 `markdown_author_output_contract_for_round` 合同尾之前，`prompts.rs:392-394`） | `author_run_boundary_injected_for_aggregate_session_without_baseline`（`baseline_teaching.rs:701`）、`author_run_boundary_injected_for_all_four_author_types`（:680）、`in_memory_engine_prompt_omits_run_boundary_block`（:786）、`in_memory_engine_prompt_bytes_unchanged_by_run_boundary_injection`（:830） |
| **REQ-PRB-02** | 标题逐字；白名单/禁读清单/落盘规则三类条目；非 `/tmp` 整目录禁令；不披露状态路径；体积 ≤1.5 KiB | `baseline_teaching.rs:38-39`（`RUN_BOUNDARY_HEADING` 逐字）、`:44-66`（`run_boundary_block` 三类条目 + scratch 条件条目）、`:42`（`RUN_BOUNDARY_BUDGET_BYTES=1536`） | `run_boundary_block_carries_heading_and_whitelist_semantics`（:592，断言无"禁止访问 /tmp"、无 `issue-work-item-plans`）、`run_boundary_block_stays_within_budget`（:646）、`run_boundary_block_omits_scratch_when_absent`（:625） |
| **REQ-PRB-03** | 覆盖四种作者类型的初次/续轮/修订 + 自动续写重试；reviewer 不注入；同一 prompt 至多一次 | 漏斗三面（`prompts.rs:377`、`:509`、`revision.rs:69`）+ 重试面 `provider_drive/artifact_retry.rs:261`（整体替换后重注入一次）；reviewer 面 `baseline_teaching.rs:152-161` 不调用注入 | `author_run_boundary_injected_once_with_prior_session_history`（:723）、`revision_face_carries_run_boundary_block_exactly_once`（:811）、`artifact_retry_prompt_carries_run_boundary_block_once`（`tests/run_boundary_prompt.rs:11`）、`artifact_retry_resume_prompt_still_carries_run_boundary`（:36）、`reviewer_prompt_omits_run_boundary_block`（:792）、`build_work_item_plan_streaming_input_uses_splitter_role`（`tests/part_09.rs:226`，改造后断言块恰一次） |
| **REQ-PRB-04** | scratch 位于状态根 `run-scratch/<sid>/`、0700、不在仓库/cwd；路径显式告知并声明运行结束删除；目录缺失时省略条目；不 bump schema | `run_scratch.rs:30-32`（路径）、`:37-63`（prepare：清空→重建→0700）；`app_paths.rs`（分区方法 +13 行）；`baseline_teaching.rs:114-121`（`author_scratch_dir`：`symlink_metadata` 真目录才告知）、`:59-64`（条目文案）；`product_data_schema.rs` 零改动仍 v2 | `run_scratch_dir_lives_under_state_root_not_repo`（`run_scratch.rs:276`）、`prepare_run_scratch_sets_0700_permissions`（:406）、`gitignore_covers_aria_state_root`（:294）、`run_boundary_block_announces_existing_scratch_dir`（`baseline_teaching.rs:874`）、`run_boundary_block_omits_scratch_when_absent`（:625） |
| **REQ-PRB-05** | 注册后尽力准备（清空重建）；session 存活准入，purging/deleted 不复活；失败仅告警；同 run 内重启不清空；supersede 与 claim-if-idle 一致、覆盖全部 run 种类 | `run_scratch.rs:37-63`（`with_workspace_session_live_write` 存活准入）；manager 接线 `runs.rs:165/:182`（supersede 路径）、`runs.rs:408/:424`（claim-if-idle 路径）、`:456-469`（`prepare_run_scratch_best_effort`，Err 只 warn）、`:495`（prepare seam）；迟到清空竞态防护 `e88c07f6`（锁内复核 active run） | `prepare_run_scratch_clears_previous_content`（:391）、`prepare_run_scratch_refused_for_purging_session`（:420）、`restart_within_run_keeps_scratch_content`（`runs.rs:861`）、`start_run_from_attachment_prepares_run_scratch`（:827）、`try_start_run_if_idle_prepares_run_scratch`（:844）、`late_prepare_of_superseded_run_keeps_new_scratch`（:1021）、`late_prepare_of_superseded_auto_claim_keeps_new_scratch`（:1060） |
| **REQ-PRB-06** | 最后一个活动 run 收口时删除（锁内判定、释放 engine 后执行）；新 run 已在位不误删；覆盖取消/abort 迟到收口；幂等 NotFound 成功；失败仅告警 | `run_scratch.rs:66-71`（`cleanup_run_scratch`）；manager `finish_run` 唯一收口接线 `runs.rs:228` + `:471-481`（best-effort） | `finish_run_removes_scratch_when_no_active_run`（`runs.rs:881`）、`aborted_run_late_finish_run_removes_scratch`（:899）、`late_finish_run_of_superseded_token_keeps_new_scratch`（:916）、`finish_run_cleanup_failure_does_not_block_completion`（:940）、`cleanup_is_idempotent_and_reports_absent`（`run_scratch.rs:361`） |
| **REQ-PRB-07** | bind 之后一次孤儿回收；三类孤儿（记录不存在/终态/mtime 超 TTL）；TTL=6h 具名常量；失败不阻塞启动、单项失败不影响其余；活 run 保留 | `run_scratch.rs:93-140`（`sweep_orphan_run_scratch`）、`:18`（`RUN_SCRATCH_ORPHAN_TTL = 6h` 具名常量）；启动接线 `web/app.rs:777-783`（bind 成功后、与 reconcile 同侧，失败仅 `eprintln!`） | `sweep_orphan_run_scratch_removes_three_orphan_classes`（:564）、`sweep_orphan_run_scratch_keeps_live_run_scratch`（:584）、`sweep_orphan_run_scratch_skips_illegal_names_and_keeps_processing`（:601）、`sweep_orphan_run_scratch_does_not_trigger_session_read_spy`（:685，走 `find_workspace_session_path` 不触发读 spy）、`run_scratch_orphan_ttl_exceeds_provider_idle_watchdog`（:708，TTL>watchdog 用生产常量断言，`watchdog.rs:35`）、`orphan_run_scratch_sweep_runs_after_bind`（`app.rs:1884`） |
| **REQ-PRB-08** | 所有删除带护栏：只删分区下经 id 校验的直接子目录；分区根不删；漂移不删并告警；符号链接/非目录不删不跟随；内部链接只删链接本身 | `run_scratch.rs:74-85`（唯一删除入口 `remove_run_scratch_dir`：`validate_relative_id`）、`:119-174`（`remove_run_scratch_target` + `run_scratch_root_usable` + `is_run_scratch_orphan`） | `remove_run_scratch_dir_removes_only_partition_child`（:304）、`remove_run_scratch_dir_refuses_illegal_and_symlinked_target`（:327）、`remove_run_scratch_dir_unlinks_inner_symlink_without_following`（:468）、`remove_run_scratch_target_skips_path_drift`（:491）、`run_scratch_partition_root_replaced_by_symlink_is_not_followed`（:510）、`run_scratch_partition_root_replaced_by_regular_file_is_not_followed`（:650）、`sweep_orphan_run_scratch_skips_symlinked_entry_without_following`（:623） |
| **REQ-PRB-09** | 四类稳定码分类；codex 历史目录覆盖 + 其余来自可配置表；第一级名称判定不依赖 cwd；写形态词元；手写切词不引依赖；合法访问与 `/tmp` 下仓库不误报 | `run_boundary.rs:27-30`（四码常量）、`:74`（`scan_event`）、`:328`（`classify`）、`:131-141`（`default_history_dirs`：codex/kimi/pi 已按 local_usage 源码核实、claude `~/.claude/projects` 经 `7d63b0b6` 真机核实）、`:213`（`tokenize` 手写切词，无 `regex`） | `boundary_scan_matches_incident_vectors`（:406，事故四向量全命中）、`boundary_scan_flags_each_code_table`（:565）、`boundary_scan_flags_write_forms_table`（:594）、`boundary_scan_ignores_legitimate_access`（:449）、`boundary_scan_does_not_flag_repo_under_tmp`（:468）、`boundary_scan_uses_configurable_history_dirs`（:475）、`default_history_dirs_includes_codex_sessions`（:546） |
| **REQ-PRB-10** | 事件形状（`kind=Provider`/`status=Completed`/id=`run_boundary_{code}_{path_hash8}`/标题/detail 含 `path`+`access`+`source_event`/`command`/`cwd`/`output` 空）；原事件先持久化且不被修改；同 node 按 id 去重、至多 8 条；只扫 `command`+`detail` 前 4 KiB；reviewer 不扫；告警不再触发扫描；不 panic 不返回 `Result` | `run_boundary.rs:169-192`（`run_boundary_alert_event`：id/形状/空字段）、`:158`（`path_hash8`）、`:14-15`（`MAX_SCAN_BYTES=4096`、`MAX_RUN_BOUNDARY_ALERTS_PER_NODE=8`）、`:144-155`（`is_author_side_node` 白名单）；漏斗 `work_item_plan.rs:465-505`（先持久化 :473-477 → 扫描 :492 → 广播原事件 :493-500 → 追加告警 :501-504）、`:563-611`（去重 + 上界 + `let _ =` 吞错）；扫描器纯函数无 `Result` | `command_event_keeps_original_and_appends_alert`（`tests/run_boundary_detection.rs:37`）、`alerts_are_deduped_and_capped_at_eight`（:87）、`reviewer_node_is_not_scanned`（:126）、`alert_event_does_not_rescan`（:146）、`boundary_scan_is_bounded_and_never_panics`（`run_boundary.rs:494`） |
| **REQ-PRB-11** | 只告警不阻断：不改 turn 成败/审批/产物确认/auto_if_valid；检测内部失败不向 run 传播；不拦截读历史/读状态 | `work_item_plan.rs:489-504`（扫描结果 `Option` 吞错、追加 `let _ =`）、`:511-535`（`run_boundary_scan_request` 返回 `Option`）、`:540-556`（`run_boundary_context` 无 store 整段跳过）；全链路无 cancel/fail 调用 | `detection_failure_does_not_affect_run`（`run_boundary_detection.rs:179`）、`resume_reading_provider_history_warns`（`run_boundary.rs:486`，只告警不拦截） |
| **REQ-PRB-12** | provider 侧与既有契约零改动；既有测试保持绿色 | 证据见本报告 §1.2（六路径 diff/log 双空 + 结构定义逐条）；既有预算断言 `f60_exit_contract_matrix.rs:512` 未修改且通过 | §1.1 全量 lib 5501 passed（含既有 prompt 预算与内存态 engine 用例）+ `in_memory_engine_prompt_bytes_unchanged_by_run_boundary_injection` |

**如实披露（不算未覆盖，但需读者知悉）**：

1. **真机验收项未执行**（需 owner 另行授权 provider 调用，不以 fake provider 冒充）：①事故同形态会话 prompt 含边界块 ②run 结束后 scratch 被清理 ③`/tmp` 诱饵文件存在时 node detail 出现稳定码告警。对应 tasks.md 1.2 / 2.4 / 3.4 的真机部分——**未完成前不得宣称跨 run 污染已被治理**。
2. **一处既有断言被改写**：`tests/part_09.rs` 的 `build_work_item_plan_streaming_input_uses_splitter_role` 由「prompt 逐字等值」改为「前缀等值 + 边界块恰一次」（`6dfa715c`）。该测试非内存态 engine 用例（夹具带临时 store）、非预算上限断言；REQ-PRB-12 场景 3 的字面约束（"不修改断言上限"）未被违反，既有预算上限（6000/262144 字节等）全部未动。但按"零改动"精神如实列出。
3. REQ-PRB-09 中 claude/pi/kimi 历史目录已由 `7d63b0b6`「真机核实」补入（`run_boundary.rs:135-140` 注明核实来源），不再是"表空即不报"的未核实状态。

### 1.7 残余风险核对（✅ 三条均未被悄悄"解决"或"放大"）

对照技术方案 §7/§9 与实现现状：

| 残余风险 | 现状核对 | 证据 |
|---|---|---|
| 提示词是软约束，模型可忽略；检测层是唯一把静默污染变成可观测事件的一层（只告警不阻断） | **未被解决也未被放大**：无任何拦截/阻断路径加入；检测仅追加 `kind=Provider` 告警事件，不改 turn 成败 | `run_boundary.rs:169-192`（纯事件）；`work_item_plan.rs:489-504`（无 cancel/fail）；`detection_failure_does_not_affect_run` 通过 |
| resume 场景禁令无效：已读入原生历史的内容无法撤回，只有再次读取时告警 | **维持原状**：无撤回/改写原生历史的代码；再次读取仅产生 `run_boundary_provider_history_read` 告警 | `resume_reading_provider_history_warns`（只告警断言）；span 内零触碰 provider 历史读写面 |
| codex 原生 `FileChange` 即使写 scratch 仍被 T13 判失败，本 change 不开豁免 | **维持原状**：`codex_provider/session.rs:199/:577/:707` 零改动；边界块文案如实披露"原生文件变更…不要使用"（`baseline_teaching.rs:56`），未教绕过、未开豁免 | §1.2 codex_provider 目录 diff/log 双空 |
| 事故现场既有 `/tmp` 残留不由本 change 清理 | **维持原状**：sweep 只遍历状态根 `run-scratch/` 分区直接子目录 | `run_scratch.rs:93-140`；全 span 无 `/tmp` 清理代码 |
| 孤儿 TTL（6h）> watchdog 零活动阈值（1800s）由断言钉住；后者上调须重新核对 | **已钉住且修正过断言漂移**：`abaf4c47` 把字面量断言改为对 `PROVIDER_IDLE_WATCHDOG_TIMEOUT_PRODUCTION`（`watchdog.rs:35`）的链接期断言 | `run_scratch_orphan_ttl_exceeds_provider_idle_watchdog`（`run_scratch.rs:708`）通过 |
| 真机验收未完成前不得宣称治理 | **如实保留**：见 §1.6 披露 1 | — |

§9「明确不做的事」逐条复核（均未违反，除红线一条）：不改 sandbox/policy token/digest ✅、不改 envelope/指纹/`StreamingProviderInput` ✅、不删 `/tmp` 残留 ✅、不动历史版本产物（`lifecycle_store/plan.rs` 零改动）✅、不做定时清理常驻任务（仅 bind 后一次）✅、不 bump schema ✅、无任意路径递归删除入口（唯一入口 + id 校验 + 护栏）✅、不引 `regex`（Cargo.toml/lock 零改动）✅、**不改动红线文件 ❌（prompts.rs +4，唯一违例）**、不向 reviewer 注入/不扫 reviewer ✅、不接线 `ProviderRunResourceScope`（`provider_resources.rs` 零改动）✅。

---

## 2. 未覆盖 / 覆盖不足条目清单

| 条目 | 状态 | 原因 |
|---|---|---|
| 真机验收 3 项（prompt 含块 / scratch 清理 / 诱饵告警） | **未覆盖** | 需 owner 另行授权 provider 调用；不以 fake provider 冒充。tasks.md 1.2、2.4、3.4 因此不勾 |
| REQ-PRB-12「引入前后契约摘要一致」场景的逐字节比对 | 覆盖方式为结构性证明 | 用「定义文件零改动」（diff/log 双空）替代前后摘要 diff；两者等价（文件零字节变化 ⇒ 摘要必然一致），但未做运行态摘要快照比对，如实说明 |
| clippy 全 target 完整走完 | 部分受阻 | 既有 `entity_deletion_cascade.rs:841` error 阻断 lib 编译后 lint 阶段未能对全部 target 完成；本 change 新代码无告警（唯一 error 为既有） |

## 3. 需 Main 裁定的缺口

**`prompts.rs` 超 4 行（1665→1669，登记上限 1665），由本 change 提交 `6dfa715c` 引入**。两个处置选项：

1. **回缩（推荐，符合计划原文）**：删掉 `prompts.rs` 的 4 行 `#[cfg(test)]` re-export，`tests/part_09.rs` 改为在测试文件内复写规范逐字标题常量（计划自审结论第 3 条的原始裁定）；既有测试 `author_run_boundary_tests` 等位于 `prompts` 子树内可继续直接引用。改动约 2 个文件、<10 行，回缩后 `prompts.rs` 回到 1665。
2. **登记上限上调至 1669**：`large_file_guard.rs` 的登记表注释规定"确需新增项须用户裁决"——需用户明示批准，且与"不得继续增长"的收紧方向相悖。

注意：守卫测试在基线上本就有 20 项既有超限（红），本 change 只是**没有保持住**"不新增"这一条；即使处置完 `prompts.rs`，守卫测试仍会因既有 20 项而红（属既有债务，不由本 change 修复）。

**其余无必须改代码的缺口**：REQ-PRB-01..12 的实现与测试证据完整；provider 侧零改动成立；回归全绿。

## 4. tasks.md 勾选情况

已勾（证据充分）：**1.1、2.1、2.2、2.3、3.1、3.2、3.3、4.2**（共 8 项）。
未勾（证据不足，4 项）：

- **1.2**：真机核对事故同形态会话 prompt 含边界块——未执行（需 owner 授权）。
- **2.4**：真机核对 run 结束后 scratch 被清理——未执行（同上）。
- **3.4**：真机核对诱饵文件存在时 node detail 出现稳定码告警——未执行（同上）。
- **4.1**：「确认未触碰任何已到行数红线的文件」不成立（`prompts.rs` +4 越线，见 §3），且「完成独立审查」尚无独立审查产物。

## 5. 验证命令清单（可复现）

```
git rev-parse HEAD                          # e4d683d6…
git log --oneline 9ffbd1cc..HEAD            # 22 提交（18 本 change + 4 政策链）
git diff --stat 9ffbd1cc..HEAD -- <provider 六路径>   # 空
git log --oneline 9ffbd1cc..HEAD -- <provider 六路径> # 空
for f in <六红线文件>; do … done             # prompts.rs 1665→1669，其余不变
flock /tmp/aria-cargo.lock cargo test --locked --test it_core large_file_guard   # FAILED（21 项，20 项基线既有）
flock /tmp/aria-cargo.lock cargo check --locked --tests                          # Finished, 退出码 0
flock /tmp/aria-cargo.lock cargo test --locked --lib                            # 5501 passed, 0 failed
flock /tmp/aria-cargo.lock cargo clippy --locked --all-targets -- -D warnings   # 仅既有 entity_deletion_cascade.rs:841
cargo fmt --all -- --check                                                # 无 diff，退出码 0
```
