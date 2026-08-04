# Task 1 Report — Shared types, module registration, and identity snapshots

## What was implemented

- Added `coding_models::group_review` with all Task 1 persistent DTO declarations:
  - `UnitReviewConclusionSnapshot`, `CompactFindingDigest`, and `SnapshotRebuildError`.
  - Future shared shard/reduction DTOs, obligations, provenance, and `CasOutcome`.
- Added `coding_workspace_engine::group_review_types` with the complete `pub(crate)` shared material/compiler/prompt helper type contract for Tasks 2–5.
- Added `CodeReviewReport.unit_run_id: Option<String>` with `#[serde(default)]` for backward-compatible legacy report deserialization.
- Added snapshot store APIs for idempotent write/read and fail-closed deterministic rebuild.
  - Snapshot identity is keyed by attempt + unit run.
  - Rebuild resolves the authoritative unit and unit run; it does not infer identity from report round/order.
  - A legacy report without `unit_run_id` returns `SnapshotRebuildError::MissingUnitRunId`.
  - Finding-message and raw-report SHA-256 digests are calculated deterministically.
- Updated code-review persistence to create the report then snapshot; snapshot failure removes the just-written report before returning the error.
- Group-scope code-review report creation now records the active unit run ID; work-item scope keeps it absent for compatibility.

## TDD evidence

### RED

1. **Idempotent write test**
   - Command: `cargo test -p cadence-aria --lib group_review_identity_snapshot -- --nocapture`
   - Expected failure: unresolved `CompactFindingDigest` / `UnitReviewConclusionSnapshot` imports and missing `write_` / `get_` store methods.
   - Observed: compilation failed with those unresolved types and methods.

2. **Legacy rebuild test**
   - Command: `cargo test -p cadence-aria --lib group_review_identity_snapshot -- --nocapture`
   - Expected failure: missing `rebuild_unit_review_conclusion_snapshot`.
   - Observed: compilation failed with `no method named rebuild_unit_review_conclusion_snapshot`.

3. **Atomic rollback and success-path integration tests**
   - The tests were added before the final report/snapshot atomic persistence implementation. During implementation, the focused test initially surfaced fixture execution-context identity mismatch; the fixture was corrected to use authoritative bundle hashes/renderer versions, then the intended snapshot failure and rollback behavior was verified.

### GREEN

- Command: `cargo test -p cadence-aria --lib group_review_identity_snapshot -- --nocapture`
- Result: **5 passed, 0 failed**.
- Covers idempotent write/read, legacy missing-unit-run fail-closed rebuild, deterministic rebuild from persisted raw report and authoritative binding, rollback on snapshot write failure, and normal report+snapshot persistence.

## Files changed

### New

- `src/product/coding_models/group_review.rs`
- `src/product/coding_attempt_store/group_review_store.rs`
- `src/product/coding_workspace_engine/group_review_types.rs`
- `src/product/coding_workspace_engine/tests/group_review_identity_snapshot.rs`

### Updated

- `src/product/coding_models/mod.rs`
- `src/product/coding_models/review.rs`
- `src/product/coding_attempt_store/mod.rs`
- `src/product/coding_attempt_store/paths.rs`
- `src/product/coding_workspace_engine/mod.rs`
- `src/product/coding_workspace_engine/reports.rs`
- `src/product/coding_workspace_engine/code_review.rs`
- `src/product/coding_workspace_engine/tests.rs`
- Existing CodeReviewReport construction fixtures/tests in `src/` and `tests/`, updated with `unit_run_id: None` to preserve their legacy intent.

## Verification

- `cargo check -p cadence-aria` — passed.
- `cargo fmt --check` — passed after `cargo fmt`.
- `cargo clippy --all-targets --all-features --locked -- -D warnings` — passed.
- `cargo test -p cadence-aria --lib group_review_identity_snapshot -- --nocapture` — passed: 5/5.
- `cargo test -p cadence-aria` — passed on the final rerun: unit, integration, and doc-test targets green (312 integration passed; 12 ignored).

## Self-review

- Shared type ownership follows the brief: persistent DTOs are in `coding_models/group_review.rs`; non-persistent compiler/prompt types are in `coding_workspace_engine/group_review_types.rs`.
- The implementation preserves per-unit renderer text; `render.rs` was not changed.
- Atomic behavior is tested at the `execute_code_review_with_commands` integration boundary.
- Rebuild is fail-closed for missing unit identity and uses authoritative store binding rather than sequence/round inference.
- All current code quality gates except the unrelated full-suite timeout passed.

## Concerns

- Full suite passed after the final snapshot replacement correction. No residual test risk identified.
