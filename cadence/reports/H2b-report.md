# H2b 报告:执行轮协调清理(advance_store / group_initialization)

- 设计:`cadence/designs/2026-10-14_方案设计_Coding工作区整组删除与重启_v1.0.md`(A 节完整重置闭包/零成员判据/「journal 在而 attempts 亡」,§E H2 存储子片)
- Worktree:`.worktrees/feat-b-0808-add-monorepo`(分支 feat-b-0808-add-monorepo)
- 独占写面:`src/product/advance_store.rs`、`src/product/coding_attempt_store/group_initialization.rs`(含内联 tests;advance_store.rs 内先在的 SG6a.3 fmt 越界遗留随本提交入库,Main 已登记)
- 接口冻结:H2a 已按本报告 API 面回执消费(IRC 双向确认);`list_plan_artifacts`/`purge_plan_artifacts`(D1.2c)零改动

## 问题(基线缺口)

1. advance 侧无「旧 Ready 指向已死全集」的识别面:事故形态(旧 Ready record+外层 journal 指向四个已删 attempts)只能被上层重放成 `coding_attempt_not_found`,零成员闭包没有交叉核对输入。
2. group 初始化 journal 无 plan 级清理面:`delete_group_initialization_for_attempt` 只随单 attempt 删除走,attempts 已亡的残留 journal(清理中断/存量残留)无闭包清理入口。
3. `ensure_group_initialization_attempt` 对「journal 已达 AttemptPersisted 而 attempt 文件缺失」仍会用旧 snapshot 重建 attempt——复活已删旧 ID,违反设计「已持久化不复活旧 ID」。

## 修正(全部加法式,既有 API 语义零变化)

- `advance_store.rs`
  - `AdvancePlanExecutionReferences{records, initialization_journal}` + `AdvanceStore::plan_execution_references(p,i,plan)`:全 plan records(含 legacy 多 record)+ 外层 journal 只读事实;journal 归属/错 join fail-closed(同 §0.5),purging/deleted 亦可枚举。
  - `ready_bound_attempt_ids()`/`journal_bound_attempt_ids()`/`classify_stale_references(现存集) -> AdvanceStaleReferenceVerdict{NoReferences|Intact|AllTargetsDead|PartiallyDead}`:断裂集=Ready 绑定 ∪(journal 已越 AttemptPersisted 时的绑定);未越 AttemptPersisted 的 journal 绑定是初始化前缀不计断裂(设计 line 74);PartiallyDead 不自动清,交清剩余出口。
  - 零成员闭包复用 D1.2c `purge_plan_artifacts`(lease 消费,retry→journal→record)——不立墓碑(coding 重置≠Plan 删除),purge 后新 command/新 attempt IDs 正常绑定新轮(`put_record`+`advance_is_ready_for_attempt` 存储面直证)。
- `group_initialization.rs`
  - `ensure_group_initialization_attempt_inner` 不复活守卫:journal.phase 已越 AttemptPersisted 且 attempt 文件缺失 → fail-closed(`must not be revived`),不重建;Prepared 前缀首物化续做零变化。
  - `CodingGroupJournalReference{InitializationPrefix|BoundToAliveAttempt|BoundToDeadAttempt}` + `classify_group_initialization_references(p,i,plan,现存集)`:per-journal 三形态分类(前缀/存活/断裂),供闭包决定清理集合。
  - `purge_group_initialization_journals_with_lease(&EntityDeletionLease)`:清 plan 全部 journal(legacy 原路径+per-target 子目录,含 attempts 已亡残留);Plan-scope 租约校验(非 Plan 拒 `coding_group_initialization_purge_scope`);身份/归属错配 fail-closed;不删公共根与 arbitration 锁;missing 幂等。

## 证据(红→绿→回归;隔离树=HEAD `3bfe8ed7`+本片两文件,避开并行切片在途编译噪声)

| 命令 | 红(改前) | 绿/回归(改后) |
|---|---|---|
| `cargo test --locked --lib h2b`(主树) | 编译失败(bg_1019):新增测试引用尚未实现的 API(`plan_execution_references`/`classify_stale_references`/`purge_group_initialization_journals_with_lease`/`CodingGroupJournalReference` 等,git diff 可证 API 全系本片引入);期间主树另有并行切片在途错误,已逐一归位非本片 | —(主树绿由 D21c/H2a 收口后统一回归) |
| `cargo test --locked --lib h2b`(隔离树) | — | **11 passed; 0 failed**(h2b_incident 全删判定/h2b_partially_dead/h2b_prefix_journal_below_attempt_persisted/h2b_journal_past_attempt_persisted/h2b_verdicts_for_alive/h2b_zero_member_purge_then_new_round/不复活守卫/前缀续做钉/三形态分类/legacy 清理/per-target 清理) |
| `cargo test --locked --lib`(隔离树全量) | — | **4658 passed; 0 failed; 7 ignored**(含 advance_handler/advance_initialization/advance_split_*/group_split_target/group_uniqueness 等既有消费面回归,守卫与加法式 API 零破坏) |
| `cargo clippy --all-targets --all-features --locked -- -D warnings`(隔离树) | — | 本片两文件零 error/warning;基线 `3bfe8ed7` 自带他人文件 unused-import -D 债(gateway_sync/kimi/lifecycle_worktree/compile/prompts/web_state 等,列表在案,非本片引入不代修) |
| `rustfmt --edition 2021 --check <两文件>` | — | FMT_OK(fmt 限本片文件) |

首绿前的唯一测试侧返工:fixture 笔误两处(`save_manifest` 漏 `&project.id` 首参;per-target purge fixture 的 plan_0002 unit WI 与 input.current_work_item_id 不一致),均测试自纠,实现代码零返工。

## 边界与交接

- H2a 闭包(coding_cleanup/coding_plan_run)消费面:`plan_execution_references`+`classify_stale_references`+`classify_group_initialization_references`+`purge_group_initialization_journals_with_lease`+D1.2c 两个既有 API;锁序按设计步骤 4→5(journal→advance→run),我的两个 purge 均只消费传入租约不重取 scope。
- 初始化前缀协议:未 AttemptPersisted 续做(引擎/H3 消费面零变化,本片只加存储面守卫);已 AttemptPersisted 不复活旧 ID 由 ensure 守卫+BoundToDead 分类双面承载。
- 退役围栏/生命周期元数据/generation(H2a 面)不在本片;HTTP 409 映射(H1/H4)不在本片;run 侧 purge(N2 既有)不在本片。
- 基线 clippy 债(他人文件 unused-import)与主树并行在途状态不在本片修复范围;主树全量红绿由 Main 在 D21c/H2a 收口后统一调度。
