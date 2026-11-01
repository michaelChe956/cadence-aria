## Context

See [proposal.md](proposal.md) for motivation. The current group-unit completion path owns both Git staging and commit creation, while the streaming Provider path owns immediate failure side effects. The latter prevents a caller from distinguishing a retryable transport failure from a terminal failure before it creates a blocked gate.

Existing Coder rework and Plan Repair are separate business-flow decisions and remain authoritative after a Provider has completed normally.

## Goals / Non-Goals

**Goals:**

- Move commit selection from orchestration to the Coder that has the Work Item `write_policy`.
- Make transient Coder and Work Item Code Reviewer failures recover automatically and auditable.
- Preserve the existing manual recovery UX after the automatic budget is exhausted.
- Keep provider retry, implementation rework, and plan repair as distinct sequential transitions.

**Non-Goals:**

- No static path blacklist, server-side staging-scope enforcement, or server-generated fallback commit.
- No change to Plan Repair Workspace providers, Story/Design Workspace providers, or historical Group Final provider retries.
- No change to the semantic threshold or configurable limit of Coder rework.
- No automatic retry for a Provider that returned a normal but invalid business result.

## Decisions

### 1. Coder owns staging and commit creation

The Coder execution context will state the exact responsibility: inspect all Git state, select only paths allowed by the current `write_policy`, stage those paths precisely, commit the Work Item, and report status plus SHA. It will also prohibit broad cleanup of unknown content. This is prompt-level responsibility, deliberately not a new server gate.

Each UnitRun already captures an immutable `start_commit`. At group-unit completion the engine will replace its mutating staging/commit sequence with a read-only current-HEAD lookup and persist that SHA as the terminal `completion_commit`. The Work Item's normative Git evidence is the inclusive change range `start_commit..completion_commit`, not the tail commit by itself. This keeps an initial Coder commit and any later Coder-rework commit in the same Work Item evidence set. A head equal to the start commit is an explicitly empty observed range; it MUST NOT cause the parent commit's files to be attributed to the Work Item.

Existing completion scope checks continue to exist, but their changed-file and diff evidence source changes from `git show <completion_commit>` to the UnitRun's commit range. This preserves the existing completion invariant rather than adding a server-side staging or path gate. Group Final evidence consumes the same range and its ordered commit list. Crash-recovery continues to compare the recorded terminal SHA with HEAD, so a completed unit is still idempotent.

Coder-reported staged paths, SHA and post-commit status are retained as role-run raw-output evidence. Aria does not parse that natural-language report into a new commit-approval gate, and never creates a fallback commit if the observed range is empty.

Alternative considered: validate every staged path in the service before allowing the commit. Rejected because it would duplicate `write_policy` interpretation as a new enforcement gate and contradict the accepted Coder-owned responsibility model.

### 2. Centralize retry policy above provider failure side effects

A provider invocation will first produce a typed outcome: completed output, user cancellation, non-retryable protocol/business failure, or retryable transport failure. Only after the retry coordinator has exhausted its two automatic retries will it call the existing role-specific failure path that closes the timeline node and creates a Coder/Reviewer gate.

The classifier will be attached before errors are collapsed into display strings, allowing start errors, stream closure, execution timeout, connection/process interruption, and identified 5xx responses to be handled consistently. Protocol violations, structured-output parsing failures and normal review results bypass this coordinator as non-retryable results. A timeout caused while the Provider is waiting for a user permission or choice response is a human-interaction wait, not an execution timeout, and MUST bypass automatic retry.

Alternative considered: set the legacy adapter `max_retries` field to two. Rejected because the streaming execution path owns its own lifecycle and failure side effects, and the field currently cannot provide a cross-provider audit trail or correct gate timing.

### 3. Every provider invocation is a separate role run and timeline attempt

The initial call and each automatic retry will receive distinct role-run records, explicit trigger metadata, retry-cycle identity, cycle ordinal and reason codes. Failed automatic attempts retain their raw output and terminal event; a new invocation starts a new timeline attempt so streamed partial text is never visually conflated with the retry's output. The UI derives automatic-retry state from these persisted attempts rather than an in-memory counter.

Manual retry remains a user-authorized new invocation cycle. It links to the exhausted cycle for audit but receives its own initial-call-plus-two-retries budget.

### 4. Retried Coder calls use fresh full context on the same worktree

An interrupted Coder can have made partial changes before transport failure. Retrying a resumed delta conversation risks losing that context or repeating a stale tool state. Therefore each automatic Coder retry starts a fresh provider session with the full rendered context and the same worktree; the prompt directs it to inspect existing state first. A retried Code Reviewer likewise starts a fresh read-only review session against the current worktree. Provider-specific fresh-session recovery follows this same policy and consumes one retry slot. Existing inline provider-specific fresh retries are removed or routed through this coordinator; they MUST NOT remain as an unrecorded nested retry within one role run.

### 5. Business-flow precedence is explicit

Provider retry occurs only before a normal role result exists. A normal Reviewer request for implementation changes enters the existing Coder rework loop. A valid Plan Defect discovered by a normal Coder or Reviewer result takes precedence over the current rework/review cycle and enters existing Plan Repair. Retry bookkeeping never increments `rework_count`, and Plan Repair never treats its semantic findings as transport failures.

## Risks / Trade-offs

- [Coder omits a commit] → Aria will not create one. The full context requires explicit commit evidence; the persisted observation becomes an empty `start_commit..HEAD` range rather than misattributing the previous commit. This is the accepted responsibility trade-off.
- [Partial output is visible before retry] → Each invocation is displayed and retained separately, avoiding an apparently successful merged response.
- [Misclassified transient error] → Preserve the original typed cause and raw output; unknown failures fail closed to the existing manual gate rather than retrying indiscriminately.
- [Provider-specific resume recovery exceeds the cap] → Route it through the shared budget counter and cover it with a maximum-three-invocations regression test.
- [Manual retries loop indefinitely] → Only automatic retries are bounded; every additional cycle requires an explicit user action and remains auditable.

## Migration Plan

1. Add regression coverage around a dirty shared worktree, Coder-created multi-commit ranges including rework, empty observed ranges, and legacy completed-unit replay.
2. Introduce typed retry outcomes and persist retry attempts before enabling the automatic coordinator.
3. Update Coder prompt rendering and replace orchestration-owned Git mutation.
4. Enable the new behavior for newly started and manually resumed calls; existing persisted completed role runs and completion commits remain readable without migration.
5. Rollback is code-only: disabling the coordinator restores direct manual gates; it does not rewrite role-run history or Git commits.
