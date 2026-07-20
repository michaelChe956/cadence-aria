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
