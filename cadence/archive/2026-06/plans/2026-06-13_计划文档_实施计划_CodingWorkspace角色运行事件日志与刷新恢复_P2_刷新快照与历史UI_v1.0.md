# CodingWorkspace 角色运行事件日志 P2 刷新快照与历史 UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让刷新后的 `CodingSessionState` 带上 role run 事件摘要和最近事件，并在 `RoleRunHistoryPanel` 中展示可读进度线索。

**Architecture:** 基于 P1 的 `list_role_run_events`，后端构建 `CodingRoleRunSnapshot` DTO，而不把 event summary 写回 role run 主 JSON。前端在现有 `CodingRoleRun` 类型上增加可选 `event_summary` 与 `recent_events` 字段，store 继续接收 `role_runs`，UI 在卡片内显示 event count、last event 和 recent events。

**Tech Stack:** Rust 1.95.0、Serde、Axum WebSocket、TypeScript、React、Zustand、Vitest。

---

## P1 Dependency Check

Before starting P2, confirm P1 exists:

```bash
rg -n "CodingRoleRunEvent|list_role_run_events|append_role_run_event" src/product
```

Expected:

- `CodingRoleRunEvent` and `CodingRoleRunEventType` are in `src/product/coding_models.rs`.
- `list_role_run_events` is in `src/product/coding_attempt_store.rs`.

## File Structure

- Modify: `src/product/coding_models.rs`
  - 新增 snapshot DTO：`CodingRoleRunEventSummary`、`CodingRoleRunEventPreview`、`CodingRoleRunSnapshot`。
- Modify: `src/web/coding_ws_handler.rs`
  - `CodingSessionState.role_runs` 从 `Vec<CodingRoleRun>` 改为 `Vec<CodingRoleRunSnapshot>`。
  - `build_coding_session_state` 读取 P1 事件日志并构建 summary/recent events。
- Modify: `tests/it_web/web_coding_ws_handler.rs`
  - 覆盖 snapshot 包含 event summary 和 recent events。
- Modify: `web/src/api/types.ts`
  - 增加 `CodingRoleRunEventType`、`CodingRoleRunEventSummary`、`CodingRoleRunEventPreview`。
  - 扩展 `CodingRoleRun`。
- Modify: `web/src/api/types.test.ts`
  - 覆盖新增字段类型。
- Modify: `web/src/state/coding-workspace-store.test.ts`
  - 覆盖 store 保留 role run event summary。
- Modify: `web/src/components/coding-workspace/RoleRunHistoryPanel.tsx`
  - 展示 event summary 与 recent events。
- Modify: `web/src/components/coding-workspace/RoleRunHistoryPanel.test.tsx`
  - 覆盖 UI 展示。

## Task 1: Backend Snapshot DTO And WebSocket State

**Files:**
- Modify: `src/product/coding_models.rs`
- Modify: `src/web/coding_ws_handler.rs`
- Test: `tests/it_web/web_coding_ws_handler.rs`

- [ ] **Step 1: Write failing WebSocket snapshot test**

In `tests/it_web/web_coding_ws_handler.rs`, add `CodingRoleRunEventType` to the `coding_models` import.

In `coding_session_snapshot_includes_role_runs`, replace the existing unbound `create_role_run` call with the `let run = store.create_role_run` expression shown below, then append two events:

```rust
    let run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0003".to_string()),
        )
        .expect("role run");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ProviderPrompt,
            serde_json::json!({
                "mode": "plan_tests",
                "prompt": "plan tests"
            }),
        )
        .expect("prompt event");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Task update",
                "status": "running",
                "detail": "No tasks found"
            }),
        )
        .expect("execution event");
```

Replace the existing `CodingSessionState` assertions with:

```rust
        CodingWsOutMessage::CodingSessionState { role_runs, .. } => {
            assert_eq!(role_runs.len(), 1);
            assert_eq!(role_runs[0].role, CodingProviderRole::Tester);
            assert_eq!(role_runs[0].run_no, 1);
            assert_eq!(role_runs[0].node_id.as_deref(), Some("coding_node_0003"));
            let summary = role_runs[0].event_summary.as_ref().expect("event summary");
            assert_eq!(summary.event_count, 2);
            assert_eq!(
                summary.last_event_type,
                Some(CodingRoleRunEventType::ExecutionEvent)
            );
            assert_eq!(summary.last_event_title.as_deref(), Some("Task update"));
            assert_eq!(summary.last_event_status.as_deref(), Some("running"));
            assert_eq!(role_runs[0].recent_events.len(), 2);
            assert_eq!(role_runs[0].recent_events[1].title.as_deref(), Some("Task update"));
        }
```

- [ ] **Step 2: Run test and confirm failure**

Run:

```bash
cargo test --locked --test it_web coding_session_snapshot_includes_role_runs
```

Expected:

- Compilation fails because `CodingRoleRun` has no `event_summary` or `recent_events`, or the enum variant still uses `Vec<CodingRoleRun>`.

- [ ] **Step 3: Add backend DTOs**

In `src/product/coding_models.rs`, add this block after `CodingRoleRunEvent`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodingRoleRunEventSummary {
    pub event_count: usize,
    pub last_event_at: Option<String>,
    pub last_event_type: Option<CodingRoleRunEventType>,
    pub last_event_title: Option<String>,
    pub last_event_status: Option<String>,
    pub terminal_event_type: Option<CodingRoleRunEventType>,
    pub terminal_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodingRoleRunEventPreview {
    pub sequence: u64,
    pub event_type: CodingRoleRunEventType,
    pub created_at: String,
    pub title: Option<String>,
    pub status: Option<String>,
    pub detail: Option<String>,
    pub truncated: bool,
    pub artifact_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodingRoleRunSnapshot {
    #[serde(flatten)]
    pub run: CodingRoleRun,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_summary: Option<CodingRoleRunEventSummary>,
    #[serde(default)]
    pub recent_events: Vec<CodingRoleRunEventPreview>,
}

impl std::ops::Deref for CodingRoleRunSnapshot {
    type Target = CodingRoleRun;

    fn deref(&self) -> &Self::Target {
        &self.run
    }
}
```

- [ ] **Step 4: Update WebSocket imports and enum type**

In `src/web/coding_ws_handler.rs`, add these imports from `coding_models`:

```rust
CodingRoleRunEvent, CodingRoleRunEventPreview, CodingRoleRunEventSummary,
CodingRoleRunEventType, CodingRoleRunSnapshot,
```

Change `CodingWsOutMessage::CodingSessionState`:

```rust
        role_runs: Vec<CodingRoleRunSnapshot>,
```

- [ ] **Step 5: Build role run snapshots**

In `build_coding_session_state`, replace:

```rust
    let role_runs =
        coding_store.list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
```

with:

```rust
    let role_runs = coding_role_run_snapshots(coding_store, &attempt)?;
```

Add these helper functions near `build_coding_session_state`:

```rust
fn coding_role_run_snapshots(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Result<Vec<CodingRoleRunSnapshot>, ProductStoreError> {
    coding_store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        .into_iter()
        .map(|run| {
            let events = coding_store.list_role_run_events(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &run.id,
            )?;
            let event_summary = role_run_event_summary(&events);
            let recent_events = recent_role_run_events(&events, 10);
            Ok(CodingRoleRunSnapshot {
                run,
                event_summary,
                recent_events,
            })
        })
        .collect()
}

fn role_run_event_summary(events: &[CodingRoleRunEvent]) -> Option<CodingRoleRunEventSummary> {
    let last = events.last()?;
    let terminal = events.iter().rev().find(|event| {
        matches!(
            event.event_type,
            CodingRoleRunEventType::MessageComplete
                | CodingRoleRunEventType::ProviderFailed
                | CodingRoleRunEventType::Timeout
                | CodingRoleRunEventType::Aborted
        )
    });
    Some(CodingRoleRunEventSummary {
        event_count: events.len(),
        last_event_at: Some(last.created_at.clone()),
        last_event_type: Some(last.event_type),
        last_event_title: role_run_event_title(last),
        last_event_status: role_run_event_status(last),
        terminal_event_type: terminal.map(|event| event.event_type),
        terminal_reason: terminal.and_then(role_run_event_reason),
    })
}

fn recent_role_run_events(
    events: &[CodingRoleRunEvent],
    limit: usize,
) -> Vec<CodingRoleRunEventPreview> {
    let start = events.len().saturating_sub(limit);
    events[start..]
        .iter()
        .map(|event| CodingRoleRunEventPreview {
            sequence: event.sequence,
            event_type: event.event_type,
            created_at: event.created_at.clone(),
            title: role_run_event_title(event),
            status: role_run_event_status(event),
            detail: event
                .payload
                .get("detail")
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned),
            truncated: event.truncated,
            artifact_ref: event.artifact_ref.clone(),
        })
        .collect()
}

fn role_run_event_title(event: &CodingRoleRunEvent) -> Option<String> {
    event
        .payload
        .get("title")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .or_else(|| {
            event
                .payload
                .get("mode")
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
        })
        .or_else(|| Some(format!("{:?}", event.event_type)))
}

fn role_run_event_status(event: &CodingRoleRunEvent) -> Option<String> {
    event
        .payload
        .get("status")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}

fn role_run_event_reason(event: &CodingRoleRunEvent) -> Option<String> {
    event
        .payload
        .get("reason_code")
        .and_then(|value| value.as_str())
        .or_else(|| event.payload.get("message").and_then(|value| value.as_str()))
        .map(ToOwned::to_owned)
}
```

- [ ] **Step 6: Run WebSocket test**

Run:

```bash
cargo test --locked --test it_web coding_session_snapshot_includes_role_runs
```

Expected:

- Test passes.

- [ ] **Step 7: Commit backend snapshot changes**

Run:

```bash
git add src/product/coding_models.rs src/web/coding_ws_handler.rs tests/it_web/web_coding_ws_handler.rs
git commit -m "feat: include role run event summaries in coding snapshots"
```

## Task 2: Frontend Types And Store

**Files:**
- Modify: `web/src/api/types.ts`
- Modify: `web/src/api/types.test.ts`
- Modify: `web/src/state/coding-workspace-store.test.ts`

- [ ] **Step 1: Write failing frontend type/store expectations**

In `web/src/api/types.test.ts`, extend the `coding_session_state` fixture to include one role run with:

```ts
      role_runs: [
        {
          id: "coding_role_run_0001",
          attempt_id: "coding_attempt_0001",
          stage: "testing",
          role: "tester",
          run_no: 1,
          status: "running",
          trigger: "initial",
          node_id: "coding_node_0003",
          started_at: "2026-06-13T00:00:00Z",
          completed_at: null,
          reason_code: null,
          raw_provider_output_refs: [],
          artifact_refs: [],
          event_summary: {
            event_count: 2,
            last_event_at: "2026-06-13T00:00:02Z",
            last_event_type: "execution_event",
            last_event_title: "Task update",
            last_event_status: "running",
            terminal_event_type: null,
            terminal_reason: null,
          },
          recent_events: [
            {
              sequence: 2,
              event_type: "execution_event",
              created_at: "2026-06-13T00:00:02Z",
              title: "Task update",
              status: "running",
              detail: "No tasks found",
              truncated: false,
              artifact_ref: null,
            },
          ],
        },
      ],
```

Add assertions:

```ts
    expect(outbound.role_runs?.[0].event_summary?.event_count).toBe(2);
    expect(outbound.role_runs?.[0].recent_events?.[0].detail).toBe("No tasks found");
```

In `web/src/state/coding-workspace-store.test.ts`, update `stores role runs from websocket session snapshots` so `roleRun()` includes the same `event_summary` and `recent_events`, then add:

```ts
    expect(useCodingWorkspaceStore.getState().roleRuns[0].event_summary).toMatchObject({
      event_count: 2,
      last_event_title: "Task update",
    });
    expect(useCodingWorkspaceStore.getState().roleRuns[0].recent_events?.[0]).toMatchObject({
      detail: "No tasks found",
    });
```

- [ ] **Step 2: Run frontend tests and confirm failure**

Run:

```bash
pnpm -C web test -- types.test.ts coding-workspace-store.test.ts
```

Expected:

- TypeScript fails because `event_summary`, `recent_events`, and `execution_event` role event type are not defined.

- [ ] **Step 3: Extend frontend API types**

In `web/src/api/types.ts`, after `CodingRoleRunTrigger`, add:

```ts
export type CodingRoleRunEventType =
  | "provider_prompt"
  | "provider_start"
  | "text_delta"
  | "execution_event"
  | "tool_call"
  | "tool_result"
  | "status_changed"
  | "permission_request"
  | "choice_request"
  | "message_complete"
  | "provider_failed"
  | "timeout"
  | "aborted"
  | "persistence_warning";

export type CodingRoleRunEventSummary = {
  event_count: number;
  last_event_at?: string | null;
  last_event_type?: CodingRoleRunEventType | null;
  last_event_title?: string | null;
  last_event_status?: string | null;
  terminal_event_type?: CodingRoleRunEventType | null;
  terminal_reason?: string | null;
};

export type CodingRoleRunEventPreview = {
  sequence: number;
  event_type: CodingRoleRunEventType;
  created_at: string;
  title?: string | null;
  status?: string | null;
  detail?: string | null;
  truncated: boolean;
  artifact_ref?: string | null;
};
```

Extend `CodingRoleRun`:

```ts
  event_summary?: CodingRoleRunEventSummary | null;
  recent_events?: CodingRoleRunEventPreview[];
```

- [ ] **Step 4: Run frontend type/store tests**

Run:

```bash
pnpm -C web test -- types.test.ts coding-workspace-store.test.ts
```

Expected:

- Both files pass.

- [ ] **Step 5: Commit frontend types/store tests**

Run:

```bash
git add web/src/api/types.ts web/src/api/types.test.ts web/src/state/coding-workspace-store.test.ts
git commit -m "feat: type coding role run event summaries"
```

## Task 3: RoleRunHistoryPanel Recent Events UI

**Files:**
- Modify: `web/src/components/coding-workspace/RoleRunHistoryPanel.tsx`
- Modify: `web/src/components/coding-workspace/RoleRunHistoryPanel.test.tsx`

- [ ] **Step 1: Write failing UI test**

In `web/src/components/coding-workspace/RoleRunHistoryPanel.test.tsx`, update the first `roleRun` fixture for `Tester #1` with:

```ts
            event_summary: {
              event_count: 3,
              last_event_at: "2026-06-13T00:00:03Z",
              last_event_type: "execution_event",
              last_event_title: "Task update",
              last_event_status: "running",
              terminal_event_type: "timeout",
              terminal_reason: "plan_tests_timeout",
            },
            recent_events: [
              {
                sequence: 2,
                event_type: "text_delta",
                created_at: "2026-06-13T00:00:02Z",
                title: "text_delta",
                status: null,
                detail: "No tasks found",
                truncated: false,
                artifact_ref: null,
              },
              {
                sequence: 3,
                event_type: "execution_event",
                created_at: "2026-06-13T00:00:03Z",
                title: "Task update",
                status: "running",
                detail: "Planning tests",
                truncated: true,
                artifact_ref:
                  "artifacts/role-run-events/coding_role_run_0001/0003_output.txt",
              },
            ],
```

Add assertions:

```ts
    expect(panel).toHaveTextContent("3 events");
    expect(panel).toHaveTextContent("Task update");
    expect(panel).toHaveTextContent("running");
    expect(panel).toHaveTextContent("No tasks found");
    expect(panel).toHaveTextContent("artifacts/role-run-events/coding_role_run_0001/0003_output.txt");
```

- [ ] **Step 2: Run component test and confirm failure**

Run:

```bash
pnpm -C web test -- RoleRunHistoryPanel.test.tsx
```

Expected:

- Test fails because the panel does not render event summary or recent events.

- [ ] **Step 3: Render event summary and recent events**

In `RoleRunHistoryPanel.tsx`, add this block after trigger/reason rendering and before `RefsSummary`:

```tsx
                <EventSummary run={run} />
                <RecentEvents run={run} />
```

Add these helpers before `RefsSummary`:

```tsx
function EventSummary({ run }: { run: CodingRoleRun }) {
  const summary = run.event_summary;
  if (!summary || summary.event_count === 0) return null;
  return (
    <div className="grid gap-0.5 text-[10px] text-[var(--aria-ink-muted)]">
      <div className="flex min-w-0 items-center gap-1">
        <span className="font-mono">{summary.event_count} events</span>
        {summary.last_event_title ? <span className="truncate">{summary.last_event_title}</span> : null}
        {summary.last_event_status ? (
          <span className="shrink-0 font-mono">{summary.last_event_status}</span>
        ) : null}
      </div>
      {summary.terminal_reason ? <div className="truncate">{summary.terminal_reason}</div> : null}
    </div>
  );
}

function RecentEvents({ run }: { run: CodingRoleRun }) {
  const events = run.recent_events ?? [];
  if (events.length === 0) return null;
  return (
    <div className="grid gap-0.5 border-t border-[var(--aria-line)] pt-1">
      {events.slice(-3).map((event) => (
        <div key={`${run.id}:${event.sequence}`} className="grid min-w-0 gap-0.5">
          <div className="flex min-w-0 items-center gap-1 text-[10px] text-[var(--aria-ink-muted)]">
            <span className="shrink-0 font-mono">#{event.sequence}</span>
            <span className="truncate">{event.title ?? event.event_type}</span>
            {event.status ? <span className="shrink-0 font-mono">{event.status}</span> : null}
          </div>
          {event.detail ? (
            <div className="truncate text-[10px] text-[var(--aria-ink-muted)]">{event.detail}</div>
          ) : null}
          {event.artifact_ref ? (
            <div className="truncate font-mono text-[10px] text-[var(--aria-ink-muted)]">
              {event.artifact_ref}
            </div>
          ) : null}
        </div>
      ))}
    </div>
  );
}
```

- [ ] **Step 4: Run component test**

Run:

```bash
pnpm -C web test -- RoleRunHistoryPanel.test.tsx
```

Expected:

- Test passes.

- [ ] **Step 5: Run frontend slice tests**

Run:

```bash
pnpm -C web test -- types.test.ts coding-workspace-store.test.ts useCodingWorkspaceWs.test.tsx RoleRunHistoryPanel.test.tsx
```

Expected:

- All listed test files pass.

- [ ] **Step 6: Commit UI changes**

Run:

```bash
git add web/src/components/coding-workspace/RoleRunHistoryPanel.tsx web/src/components/coding-workspace/RoleRunHistoryPanel.test.tsx
git commit -m "feat: show coding role run recent events"
```

## Task 4: P2 Final Verification

**Files:**
- Verify only.

- [ ] **Step 1: Run backend and frontend focused tests**

Run:

```bash
cargo test --locked --test it_web coding_session_snapshot_includes_role_runs
pnpm -C web test -- types.test.ts coding-workspace-store.test.ts useCodingWorkspaceWs.test.tsx RoleRunHistoryPanel.test.tsx
```

Expected:

- Both commands pass.

- [ ] **Step 2: Run format/check**

Run:

```bash
cargo fmt --check
cargo check --locked
pnpm -C web build
```

Expected:

- All commands pass.

- [ ] **Step 3: Inspect final diff**

Run:

```bash
git diff --check
git status --short
```

Expected:

- `git diff --check` produces no output.
- `git status --short` is clean after commits.
