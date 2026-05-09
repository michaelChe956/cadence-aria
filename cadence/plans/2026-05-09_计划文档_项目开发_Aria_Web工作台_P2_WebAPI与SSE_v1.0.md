# Aria Web 工作台 P2 Web API 与 SSE Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 `aria web` 后端 HTTP API、SSE 事件流、CLI 启动入口、静态资源托管和 resource/provider output endpoints。

**Architecture:** P2 在 P1 runtime core 上增加 axum server，不改前端工作台 UI。API 契约必须与 design 的 Web API 契约保持一致，SSE event taxonomy 必须完整覆盖 design 事件表。

**Tech Stack:** Rust 1.95、axum、tower-http、tokio、serde/serde_json、futures-util、tokio-stream。

---

## Design Coverage

P2 覆盖：

- `GET /api/health`
- `GET /api/projection?task_id=`
- `GET /api/tasks`
- `POST /api/tasks`
- `POST /api/tasks/{task_id}/advance`
- `POST /api/tasks/{task_id}/confirm`
- `POST /api/tasks/{task_id}/stop`
- `POST /api/tasks/{task_id}/rollback/preview`
- `POST /api/tasks/{task_id}/rollback`
- `GET /api/artifacts/{artifact_ref}`
- `GET /api/files/content?path=`
- `GET /api/files/diff?base_checkpoint=&path=`
- `GET /api/events`
- provider output backend stream
- stop signal backend route and `stop_requested` event
- provider authorization/command diagnostics
- `aria web --workspace <PATH> [--host HOST] [--port PORT] [--check]`

## Source Tasks From Master Plan

| Master Task | Scope |
|------|------|
| Task 6 | axum handlers |
| Task 7 | SSE event hub |
| Task 7.1 | complete event taxonomy |
| Task 7.5 | tasks/artifact/file/diff resource APIs |
| Task 7.6 | provider output stream, backend diagnostics and stop signal |
| Task 8 | `aria web` CLI and static asset serving |

## Files

| Path | Responsibility |
|------|------|
| `Cargo.toml` | axum/tower/futures dependencies |
| `Cargo.lock` | dependency lock |
| `src/web/app.rs` | router and server |
| `src/web/handlers.rs` | HTTP handlers |
| `src/web/events.rs` | event hub and taxonomy |
| `src/web/error.rs` | API error response mapping |
| `src/web/static_assets.rs` | `web/dist` serving |
| `src/web/runtime.rs` | handler-facing runtime methods |
| `src/web/types.rs` | response DTOs |
| `src/web/mod.rs` | module exports |
| `src/cli.rs` | `aria web` parser and async route |

## Tasks

### Task P2.1: Axum API Handlers

- [ ] **Step 1: Execute master Task 6**

Run:

```bash
cargo test --test web_api_handlers --locked
```

Expected: PASS for create、advance、confirm、projection.

- [ ] **Step 2: Commit**

```bash
git add Cargo.toml Cargo.lock src/web/app.rs src/web/handlers.rs src/web/error.rs src/web/mod.rs tests/web_api_handlers.rs
git commit -m "feat: add aria web api handlers"
```

### Task P2.2: SSE Event Hub And Event Taxonomy

- [ ] **Step 1: Execute master Task 7**

Run:

```bash
cargo test --test web_events --test web_api_handlers --locked
```

Expected: PASS.

- [ ] **Step 2: Execute master Task 7.1**

Run:

```bash
cargo test --test web_event_taxonomy --test web_events --test web_api_handlers --locked
```

Expected: PASS and all design event types are present.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock src/web/events.rs src/web/state.rs src/web/runtime.rs src/web/handlers.rs src/web/app.rs src/web/mod.rs tests/web_events.rs tests/web_event_taxonomy.rs
git commit -m "feat: add aria web sse event taxonomy"
```

### Task P2.3: Resource APIs

- [ ] **Step 1: Execute master Task 7.5**

Run:

```bash
cargo test --test web_resource_handlers --locked
```

Expected: PASS for task list、artifact content、file content、file diff.

- [ ] **Step 2: Commit**

```bash
git add src/web/types.rs src/web/runtime.rs src/web/handlers.rs src/web/app.rs tests/web_resource_handlers.rs
git commit -m "feat: add aria web resource endpoints"
```

### Task P2.4: Provider Output Backend And Diagnostics

- [ ] **Step 1: Execute backend portions of master Task 7.6**

Run:

```bash
cargo test --test web_provider_output_events --locked
```

Expected: PASS for `provider_output` payload、`stop_requested` event and provider auth/command diagnostics.

- [ ] **Step 2: Commit**

```bash
git add src/web/types.rs src/web/events.rs src/web/runtime.rs src/web/handlers.rs src/web/app.rs src/interactive/models.rs src/interactive/web_projection.rs tests/web_provider_output_events.rs
git commit -m "feat: add aria web provider output backend"
```

### Task P2.5: CLI And Static Serving

- [ ] **Step 1: Execute master Task 8**

Run:

```bash
cargo test --test web_cli --locked
cargo check --locked
```

Expected: PASS.

- [ ] **Step 2: Commit**

```bash
git add src/cli.rs src/web/app.rs src/web/static_assets.rs src/web/mod.rs tests/web_cli.rs
git commit -m "feat: add aria web cli server"
```

## P2 Exit Criteria

Run:

```bash
cargo test --test web_api_handlers --test web_events --test web_event_taxonomy --test web_resource_handlers --test web_provider_output_events --test web_cli --locked
cargo check --locked
```

Expected: all listed tests PASS.

## Self-Review

- [x] P2 implements every API endpoint from design.
- [x] P2 uses SSE, not WebSocket, for first version.
- [x] P2 keeps static assets local and single-workspace scoped.
