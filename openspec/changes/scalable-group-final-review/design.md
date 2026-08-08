## Context

See [proposal.md](proposal.md) for motivation. The active flow currently enters `InternalPrReview` after the final per-unit review request and invokes a Provider-backed shard/reduction pipeline. The flow already owns authoritative facts needed for a non-semantic group completion check: unit/run completion, completion commits, per-unit review reports, handoff revisions, review request, and plan binding.

## Goals / Non-Goals

**Goals:**

- Make the human, not an additional Provider, the owner of the final cross-item decision.
- Retain a deterministic server-side proof that the group has the records needed for a meaningful human review.
- Preserve existing per-unit semantic review and Plan Repair before final confirmation.
- Keep historical Group Final data readable and fail safely on identity inconsistencies.

**Non-Goals:**

- No replacement AI summary, cross-item semantic reviewer, shard/reduction fallback, or automatic repair from the final step.
- No new server-side `write_policy` or file-path scope gate.
- No migration that rewrites historical attempts, UnitRuns, review reports, or Group Final artifacts.

## Decisions

### 1. Use a deterministic Group Final Readiness snapshot

After the final Work Item completes its independent review, the server builds and persists a read-only readiness snapshot. It lists every logical Work Item with its authoritative UnitRun, `start_commit`, terminal completion commit, complete commit range and ordered commit references, latest successful independent review including its finding/summary and raw-output evidence references, resolved handoffs and bound plan revision. It also records failures such as missing records, stale binding or unresolved dependency.

The snapshot is a data-integrity check, not another semantic code review and not a new write-scope gate. Commit, file and diff evidence for a Work Item MUST be derived from `start_commit..completion_commit`, never from the tail commit alone. An unchanged head is recorded as an empty observed range, not as the parent commit's change set. The snapshot determines whether the user can make an informed final confirmation; it never creates findings, routes Coder rework or starts Plan Repair.

Alternative considered: omit all server checks and always show a confirm button. Rejected because a missing completion commit or handoff would make the final decision unrecoverably ambiguous and would weaken existing terminal invariants.

### 2. Transition directly from group readiness to FinalConfirm

For new attempts, the runner replaces the `InternalPrReview` provider stage with readiness snapshot generation. A complete snapshot changes the attempt to the existing human-confirmable final state. The final panel renders the snapshot and requires an explicit user confirmation before terminal completion.

If the snapshot is incomplete, the attempt remains blocked with a precise consistency diagnostic and existing recovery/termination controls. It does not receive an AI retry action because no Provider has failed. A complete snapshot enables an explicit Final Confirm but does not waive retained terminal invariants: Final Confirm still runs the existing completion binding, range-scope and shared-worktree-clean checks. Failures from those existing checks remain deterministic diagnostics or manual-cleanup actions, not new Group Reviewer findings.

### 3. Preserve preceding review and repair boundaries

The final step consumes, but does not reinterpret, each Work Item's independent review. Normal implementation findings still use Coder rework before the Work Item can be completed. Valid Draft/contract/dependency findings still enter Plan Repair before final readiness. Human final confirmation cannot silently mark an incomplete readiness snapshot as complete and cannot emit a new repair request.

### 4. Keep legacy artifacts readable but disable new provider starts

Legacy attempts may contain shard/reduction reports or be recovered at an old internal-review stage. Their data remains renderable for audit. Recovery converts an attempt that has not reached a terminal final decision to the human-final path using the same readiness snapshot; it must not create a new shard or reduction call. If the legacy identity chain is inconsistent, recovery fails closed with a diagnostic rather than guessing how to map artifacts.

### 5. Keep out-of-scope residue visible rather than silently cleaning it

When a Coder correctly leaves an untracked or modified path outside its `write_policy`, Group Final displays the Coder evidence and does not add, commit or delete that path. A dirty shared worktree can therefore still trigger the pre-existing manual-cleanup requirement during Final Confirm. This is intentional: the user, rather than the orchestrator or an unrelated Coder, decides how to dispose of the residue before the lock is released.

## Risks / Trade-offs

- [Human review is less automatic] → The final panel presents complete commit ranges, independent review findings and raw evidence, handoffs and plans in one place; the user retains the final decision already intended by this flow.
- [Out-of-scope residue remains dirty] → It is deliberately not committed or deleted; Final Confirm surfaces the existing manual cleanup action before releasing the shared-worktree lock.
- [Deterministic snapshot misses semantic cross-item defect] → Semantic defects should be caught by the per-Work-Item Reviewer and Plan Repair before final confirmation; optional advisory analysis, if ever needed, must be a separate non-blocking change.
- [Legacy state variation] → Read legacy artifacts only for display and validate identities before deriving readiness; never infer missing authoritative records.
- [Removing code leaves stale user actions] → Remove automatic retry controls for new attempts and map recovery to human confirmation explicitly.

## Migration Plan

1. Add a readiness snapshot and update the group runner to enter it instead of invoking Internal Group Review.
2. Render the snapshot in the final confirmation panel and require the existing explicit final confirmation action.
3. Stop creating shard/reduction provider runs, prompts and retry gates for new attempts; retain decoders/readers for historical artifacts.
4. Verify fresh attempts, attempts that undergo Coder rework or Plan Repair, and recovery of legacy attempts with or without existing Group Final artifacts.
5. Rollback re-enables the old runner only for new attempts if needed; persisted readiness snapshots and historical review artifacts remain additive/readable.
