# Coding Attempt 全局唯一身份与作用域路由修复

Plan: cadence/plans/2026-07-16_计划文档_实施计划_CodingAttempt全局唯一身份与作用域路由修复_v1.0.md
Plan commit: e4d99c4
Task 1: complete (commits e4d99c4..6f982da, review clean)
Task 2: complete (commits 6f982da..8fc0972, review approved)
  Minor: `invalid_coding_attempt_scope` is not explicitly mapped to HTTP 400; current registered routes cannot produce a half-scope request.
  Minor: legacy REST unique-match success lacks a direct regression test; ambiguity compatibility is covered.
Task 3: complete (commits 8fc0972..667f6c6, review approved after two fix waves)
  Minor: Product Store duplicate-ID negative assertions do not symmetrically cover quality audit, test plan, review request, internal review, and raw artifact absence in the other Issue.
Task 4: complete (commits 667f6c6..a1d394a, review approved after two fix waves)
  Minor: single Work Item address tests inject a mocked `work_item` card; production aggregation currently exposes only `work_item_group`, so direct single-item UI reachability remains a separate UX concern.
Task 5: complete (commits a1d394a..12ebe10, review approved)
  Minor: legacy success test uses the same URL and response attempt ID, so it would not catch a future regression that trusts the URL over the response identity.
  Minor: legacy redirect cancellation tests do not directly cover stale rejection or StrictMode single-navigation behavior.
Task 6: complete (verification and service readiness approved; no code commit)
  Minor: branch history contains review-loop fix commits rather than the plan's simplified one-commit-per-task shape; boundaries are documented and history was not rewritten.

# WorkItemPlan 执行期修订与三投影 Workspace 协同

Plan suite: cadence/plans/2026-07-17_计划文档_实施计划_WorkItemPlan执行期修订与三投影Workspace协同_总览_v1.0.md
Plan commit: c99b914
Execution mode: subagent-driven-development
P1 Task 1: complete (commits c99b914..515278d, review clean)
P1 Task 2: complete (commits 515278d..d195768, review clean)
P1 Task 3: complete (commits d195768..ade3aca, review clean after three fix waves)
P1 Task 4: complete (commits ade3aca..983b25c, review approved)
  Minor: Publication Journal update APIs name `amendment_id` although the value is `journal.id`; consider renaming to `journal_id` before P4 consumers are added.
  Minor: focused tests do not directly assert successful phase advancement changes `updated_at`.
  Minor: focused tests do not deterministically cover `PlanPublished -> mark_failed` preserving the terminal phase.
P2 Task 1: complete (commits 983b25c..a641c05, review clean)
P2 Task 2: complete (commits a641c05..d78759b, review approved after one fix wave)
  Minor: finding sort key omits severity; current findings are all Error, but future mixed Warning/Error duplicates may not become adjacent before exact dedup.
  Minor: no direct clean regression for terminal output present only in `output_contracts` and absent from `provided_contract_refs`.
P2 Task 3: complete (commits d78759b..d586b37, review approved after two fix waves)
  Minor: namespaced projection findings concatenate logical IDs with `.`, without a formal escaping rule for a future relaxed ID character set.
  Minor: Store `projection_artifacts` fixture references a Work Item bundle compiled from a different contract context than the Plan bundle candidate.
P2 Task 4: complete (commits d586b37..4d709f0, review approved after one fix wave)
  Minor: `ProjectionRenderError::Serialization` does not include the failing normative section ID/title.
P3 Task 1: complete (commits 4d709f0..a0be408, review approved after two fix waves)
P3 Task 2: complete (commits a0be408..98331ec, review approved after two fix waves)
  Minor: Initial publication journal retains a full recovery copy of publication artifacts; define retention/compaction/archive policy in a future Store lifecycle task.
P3 Task 3: complete (commits 98331ec..9693560, review approved after two fix waves)
P3 Task 4: complete (commits 9693560..17389a7, review approved)
  Cross-task dependency: provider-specific Codex/Claude Code/Fake rendered previews require real P5 Coder/Reviewer Execution Envelopes; Task 4 shows provider-neutral projections and an explicit P5 waiting state without synthetic runtime facts.
P3 Task 5: complete (commits 17389a7..249c393, review approved after one fix wave)
P4 Task 1: complete (commits 249c393..f9c3a02, review approved after one fix wave)
P4 Task 2: complete (commits f9c3a02..da6bb5f, review approved after one fix wave)
  Cross-task dependency: P4 Manifest and P6 DTO must expose `added_capability_associations` and `removed_capability_associations` with the same serialized shape as `ContractDelta`.
P4 Task 3: complete (commits da6bb5f..04d7136, review approved after five fix waves)
  Cross-task dependency: Task 4 must produce and persist the immutable validation/projection/review provenance required by `PlanRepairAwaitingConfirmationPackage` before entering HumanConfirm.
P4 Task 4: complete (commits 04d7136..a55cf07, review approved after two fix waves)
  Cross-task dependency: P5 must consume only the final typed `PlanAmendmentManifest`, preserve the Active Amendment Lock through Coding Binding/Resume application, and release it only after the application journal completes.
P5 Task 1: complete (commits a55cf07..034061e, review approved after five fix waves)
  Decision: `Failed` is unrecoverable terminal; recoverable CodeReview/provider interruption uses `Blocked` and only exact `retry_review` may resume through the recovery journal.
  Cross-task dependency: Task 2 must bind real provider-rendered execution context to the active `CodingUnitRun`; Task 4 owns amendment application/recovery above the minimal journal Store.
P5 Task 2: complete (commits 034061e..f1190f1, review approved after four fix waves)
  Decision: all canonical validation errors fail closed before fixed route priority; non-Implementation Coder/Tester/Reviewer routes safe-stop without generic gates.
  Decision: Group completion/review consumes only latest authoritative UnitRuns and deterministic immutable HandoffRevisions; dirty CompletedRetry creates one manual Gate and does not advance.
  Cross-task dependency: Task 3 now owns PlanRepairRequest, pause state, counters and duplicate-request behavior; Task 4 owns Amendment application/resume.
P5 Task 3: complete (commits f1190f1..1bf8955, review approved after five fix waves)
  Decision: failed-review recovery and Plan Repair use one per-attempt arbitration; advanced rollback prefixes converge and journal deletion remains last.
  Decision: Completed recovery ownership transfers only after a durable ProviderStart event; persistence failure enters provider-interruption compensation, creates a new recovery gate, and can recover again before Plan Repair.
  Decision: Coder, CoderRework, Reviewer and real Tester provider paths create Plan Repair only for exact StartPlanRepair; Task 4 apply/resume remains out of scope.
  Minor: Task 3 implementation report says all added tests use `coding_plan_repair_`, but one valid DTO test uses the allowed `coding_amendment_` prefix; product behavior is unaffected.
P5 Task 4: complete (commits 1bf8955..1a2de10, review approved after four fix waves)
  Decision: Amendment Journal persists the authoritative materialization-time Attempt HEAD; Completed replay accepts only controlled runtime evolution and never regresses a progressed Attempt stage.
  Decision: delivery is socket-write-confirmed at-least-once with a stable event ID; writer abort, receiver drop, channel close and outstanding permits all converge to Pending/non-runnable failure and release arbitration for same-event recovery.
  Cross-task dependency: Task 5 owns Handoff Revision resolution and runtime impact propagation; P6 owns client ACK/dedup and Repair Session UI consumption.
P5 Task 5: complete (commits 30306f8..4b0ca81, review approved after four fix waves)
  Decision: runtime propagation compares the exact authoritative old/new Handoff transition; both Handoffs bind to the current Attempt, logical Unit, latest Completed UnitRun and completion commit, while historical/orphan/alias replays fail closed before writes.
  Decision: Runtime UnitRun materialization uses the fixed Attempt/Amendment/logical item/Work Item Revision/resolved Handoff tuple; only the latest execution may synchronize ExecutionUnit state, and older tuple replay is a stable zero-write no-op without mutating Completed runs.
  Decision: stable Handoff contract hash includes only sorted/deduplicated provided contracts and capabilities; explicit revalidation precedes resume, and Unchanged multi-input resume requires every incoming edge capability to be satisfied.
  Cross-task dependency: P6 consumes the shared real UnitRun/HandoffRevision History artifacts and linked Repair Session state; client ACK/dedup and inline Repair UI remain P6 scope.
P6 Task 1: complete (commits ed66abf..1b66375, review approved after three fix waves)
  Decision: the parent Coding store is the sole active Repair state source; complete request/link/trigger/return-context identity is fail-closed, empty amendment identity is invalid, and parent/child reconnect cannot replace a different durable Repair.
  Decision: authoritative snapshots win same-ID timeline nodes while watermark-new current-only events survive out-of-order delivery; richer stage, history and terminal cleanup cannot regress on reconnect.
  Decision: Repair Child History/Manifest artifacts are persisted and broadcast through the real Workspace wire before session state; History always derives from the latest canonical non-Repair WorkItemPlan parent plus authoritative UnitRun/Handoff facts, never from historical Child sessions or empty/runtime-only fallback.
  Cross-task dependency: Task 2 consumes the approved linked Repair state for the inline Repair Center; Task 3 owns Story/Design/Work Item shared relation and subgraph protocol coverage.
P6 Task 2: complete (commits 4492ad4..b6fa93c, review approved after two fix waves)
  Decision: Coding Workspace embeds the Repair Center on the parent route and opens the complete Work Item Workspace only through an explicit safe link; all mutations use the real Child Generic Workspace transport.
  Decision: stale Child sockets and mismatched session snapshots fail closed; stable semantic Repair generation preserves exact-once pending across equivalent reconnect snapshots, while a matching authoritative Child snapshot generation clears pending and scoped local send errors.
  Decision: Semantic classification is single-source with actual affected > conditional-only > unaffected, and the unified timeline consumes only visible Repair projection without copying Provider streams.
  Cross-task dependency: Task 3 owns Subgraph Replan plus Story/Design/Work Item shared upgrade relations and protocol coverage; Task 4 owns non-E2E recovery fixtures and final gates.
P6 Task 3: complete (commits 00e7860..7586481, review approved after two fix waves)
  Decision: Subgraph Replan is two-phase; incomplete expanded scope is explicitly non-publication-ready, while publication-ready output is rebuilt from authoritative active contracts and replacements and must pass the existing typed dependency graph validator.
  Decision: distributed split/merge rewires typed boundaries deterministically, persists replacement Logical Work Items and bindings through the existing amendment publication journal, and advances LogicalWorkItem active revisions before the Plan active revision with replay-safe checkpoints.
  Decision: Story/Design upgrades are explicit canonical Repair Child operations with durable semantic-link uniqueness, persisted identity recovery, real target validation, typed WebSocket/client consumption and an inline adjust-scope selector that preserves the five root actions and Coding route.
  Minor: linked amendment protocol errors currently use pending-state serialization but do not additionally compare the backend target context; current single-pending socket flow is safe, and final review should triage adding defensive stale-target context matching.
  Cross-task dependency: Task 4 owns non-E2E fault/recovery fixtures, schema-v2 fixtures and final gates; browser/E2E execution remains user-owned.
P6 Task 4: complete (commits 98000bf..b09de47, review approved after three fix waves)
  Decision: duplicate and genuinely overlapping same-semantic findings converge to one authoritative Request, Amendment and Repair Child; stale base revision remains the typed AmendmentConflict path.
  Decision: Workspace start plus Link, Snapshot and authoritative Request reads and identity validation share one failed-review arbitration view; pause reconciliation and event delivery remain outside the guard to avoid flock re-entry.
  Decision: recovery evidence verifies all nine durable fault points, raw Request/Amendment/Manifest uniqueness, unrelated active revision stability, Story/Design/WorkItem recovery, and real production Runner paths for four roles across Codex, ClaudeCode and Fake scripted adapters.
  Decision: linked protocol errors fail closed unless the complete Story/Design target context matches the pending identity; Work Item repair stays on its Plan Repair Child path.
  Verification boundary: browser E2E/Playwright and real external Provider CLIs were intentionally not run per user instruction; browser acceptance remains user-owned.

# 代码库初始化真实进度

Change: add-repository-initialization-progress
Plan: cadence/plans/2026-07-22_计划文档_代码库初始化进度_v1.0.md
Execution mode: subagent-driven-development
Base: b0dedb9
Baseline: web pnpm test PASS (89 files / 724 tests); Rust cargo test blocked before tests because build.rs requires missing web/dist/index.html.
Task 1: complete (commits b0dedb9..d240efa, review approved)
  Minor: 缺少 mark_step_running 同一步、不同时间戳下保持 started_at/updated_at 不变的直接幂等回归测试；最终全分支审查时复核。
Task 2: complete (commits d240efa..72458da, review approved)
  Minor: 全仓 Clippy 仍被 Task 1 既有 `result_large_err` 阻断（types.rs progress trait 和 operation tests helper）；非本任务引入，后续拥有类型/API 范围的任务或最终修复波需解决。
Task 3: complete (commits 72458da..ac0de30, review approved after one fix wave)
  Minor: legacy `new` with custom RepositoryPersistence may use UUID temp operation store; Task 4 must use explicit new_with_operations with same ProductAppPaths.
  Minor: coordinator order test records only generic initializer, not four distinct command names.
  Minor: no coordinator-level regression for finalization operation-store write failure/non-terminal recovery.
Task 4: complete (commits ac0de30..f3d59ba, recovery audit and final review clean)
  Recovery: retained and audited the interrupted candidate diff; comparison against HEAD with the acceptance test established actual 201-vs-202 RED, then fresh Task 4 verification passed.
  Verification: cargo fmt --check; cargo test --locked --lib repository_initialization_run_registry; cargo test --locked --test it_web repository_initialization_; cargo check --locked.
Task 4: complete (commits ac0de30..d671cab, review approved after two fix waves)
  Decision: operation result uses a dedicated RepositoryDto projection with path/runtime_root `<path>`; ordinary repository GET remains unchanged.
  Decision: operation completed/failed changed_paths use exact whole-path sanitization, preserving relative repository paths.
  Minor: worker-panic behavior is not injected end-to-end; RAII lease drop plus inactive stale-operation recovery are covered.

# 初始化命令顺序调整（rule_config 优先）

Plan: cadence/plans/2026-07-23_计划文档_初始化命令顺序调整_v1.0.md
Plan base commit: 7203884b
Execution mode: subagent-driven-development
Task 1: complete (commits 7203884b..513039b6, review clean)
  Minor: registration 测试辅助函数名 running_pre_check_operation/failed_pre_check_operation 语义已变为 RuleConfig 场景，命名有歧义，留待最终审查裁定。
  Note: Plan Task 1 定向命令 `--lib repository_initialization` 仅匹配 3 个测试，实际取证用 `--lib repository_store`。
Task 2: complete (commits 513039b6..5ae3dbce, review clean)
Task 3: complete (commits 5ae3dbce..HEAD, doc-only)
  Caveat: cargo test --locked 全量在本宿主因既有 ETXTBSY flake（btrfs 写后 exec，前次 task-8 已取证）无法全绿；失败测试隔离单跑全过，与本改动无因果。有效验证：--lib repository_store 53/53、it_web web_repository_initialization 10/10、前端 732/732、fmt/clippy/tsc/openspec validate 全绿。
Final review: approved (7203884b..0f60a118, Ready to merge=Yes); Minor 修复 ca6dab7（辅助函数改名 + 旧 Plan 冻结接口节同步）

# 回退 rule-config 优先顺序（恢复 pre-check 先跑）

Reason: 真机测试添加代码库时 /rule-config 作为第一步失败（Claude API 429 配额耗尽 + 5min/命令超时 + 目标仓库 eza 别名噪声），用户决定撤销顺序调整。
Action: 代码/测试/web 还原到 7203884b；OpenSpec（已归档 change + 主 specs）与 2026-07-22 Plan 全部恢复 pre-check 先跑；删除 2026-07-23 顺序调整 Plan；tasks.md 移除第 5 节。

# WorkItemDraftPrompt 开销瘦身与上限重论证

Plan: cadence/plans/2026-07-25_计划文档_基线修复_WorkItemDraftPrompt开销瘦身与上限重论证_v1.0.md
Plan commit: 5ccd91fd
Execution mode: subagent-driven-development
Task 1: complete (commits 5ccd91fd..f4a92f79, review approved)
  Minor: 投影测试改用 QUALITY_BUDGET 断言、QUALITY_BUDGET 常量加 #[cfg(test)]（controller 修复，避免 clippy dead_code）。
  Minor: 提交 8f514702 混合归因（含前序 Plan 的依赖投影与测试改写），原子性已论证，commit body 未注明。
Task 2: complete (commits f4a92f79..e175f732, review approved after controller adjudication)
  Decision: fixture 对齐 session_0003 实测锚点（outline JSON 1,891 B / 投影 1,119 B），阈值=实测 10,941+800=11,741；12,000 质量预算断言未动；实际节省 980 B（估值 2,690 B 系计划高估）。
  Minor: verification_plan.checks 精确复制约束从 hard_rules 降为 self_check 提示（plan-mandated，断言已锁定）。
  Minor: 两条断言更新为兼容子串（方向合理，语义由保留段落覆盖）。
Task 3: complete (commits e175f732..06c3af2a, review approved)
  Note: commit 裹挟并行 Plan（文件大小守卫）的纯移动产物（provider_run.rs 696 行、part_01 改名），全文 diff 证明除 3 行修复外零漂移；配套 run.rs 瘦身与 include 接线仍未提交（并行 Plan 范围）。
  Minor: 新分支附带 transition_stage(PrepareContext) 副作用（plan-mandated 复用 helper 的必然结果）。
Task 4: complete (design.md 双层模型修订；fmt/clippy/lib 1361/it_web serial 19/openspec validate/git diff --check 全绿；3.3/4.1 保持未勾选待 Case A/B 授权)
Final review: With fixes → 修复波 b24b0ad0（batch 分支同类悬挂一行修复 + batch 回归 11/11）。Minors 留档：draft 循环其余 Err 分支悬挂属既存行为（后续统一审计）；clear_active_run_if_token 不对称属既存。
Case A/B 真实验证（操作者授权）：Case A backend_session 10/10 pass（0 inconclusive）；Case B compact_duration 10/10 pass（2 次非连续 inconclusive 超时）。3.3/4.1 已勾选；pnpm tsc -b + pnpm test 736/736 补证。

# 末端 WorkItem 交接契约引用可空

Change: relax-terminal-handoff-contract-refs
Plan: cadence/plans/2026-07-26_计划文档_基线修复_末端WorkItem交接契约引用可空_v1.0.md
Plan commit: c7c99403
Execution mode: subagent-driven-development
Base: a7192b50
Task 1: complete (commits a7192b50..66c53630, review approved)
  Minor: Plan Task 1 Step 2 命令为双位置参数（cargo 拒绝），implementer 改用等价 --lib 精确过滤命令；后续 Plan 命令模板应只写单个 TESTNAME 过滤器。
  Note: 并行会话的 task-brief 会覆盖 .superpowers/sdd/task-N-brief.md 固定路径；本 Plan 后续 brief 使用唯一文件名。
Task 2: complete (commits 66c53630..9607f860, review approved)
  Minor: schema 行仍用记号 `str+`（非空 string），hard_rules 行措辞为"非空白"，两处对元素约束的记号轻微不对称；留待最终审查裁定。
  Note: 并行会话并发 cargo 构建曾致 3 个无关瞬态失败（[cadence_project_rules] 断言），stash 对照实验证明与本任务无关，重跑全绿。
Task 3: complete (commit 8066e065, review approved; 父提交为并行会话 a2976742，diff 仅 tasks.md)
  ⚠️ 已消解：1.1–2.2 勾选依据由 Task 1/2 审查结论覆盖。
Final review: approved (66c53630/9607f860/8066e065, Ready to merge=Yes)。Minors 留档：str+ vs 非空白记号不对称（保留）；Plan Step 2 命令模板已订正（本次提交）；spec「两项链路通过 Final Compile」场景由 tasks 4.1 Case A/B 真实验证兜底（归档前必须完成）。

# Provider Prompt 项目规则引用

Change: use-project-rules-in-prompts
Plan: cadence/plans/2026-07-26_计划文档_功能开发_ProviderPrompt项目规则引用_v1.0.md
Execution mode: subagent-driven-development
Task 1/2: complete (commits 9607f860..a2976742, review approved)
  Minor: RED 阶段后半部分的逐条失败尾部因并发 Cargo 锁与输出截断未留存；实施报告已披露，且不影响最终契约或 GREEN 证据。
Task 3: pending (全量质量门禁、OpenSpec 收尾与文档提交)。
