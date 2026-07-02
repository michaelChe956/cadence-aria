# Cadence Aria

An AI-assisted software development workbench. Cadence Aria systematically breaks down requirements (Issues) into Story Specs, Design Specs, and Work Items, then drives them through unified Provider Workspace and Coding Workspace sessions for generation, cross-review, implementation, and verification/rollback — making AI a traceable, auditable, and reversible R&D collaborator.

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

---

## Table of Contents

- [Core Capabilities](#core-capabilities)
- [Workbench Overview](#workbench-overview)
- [Tech Stack](#tech-stack)
- [Quick Start](#quick-start)
- [Development Guide](#development-guide)
- [Project Structure](#project-structure)
- [License](#license)

---

## Core Capabilities

### 1. Issue Lifecycle Workbench

The product backbone is **Project → Repository → Issue**. The home page shows a four-column board:

- **Issue**: the source of requirements, bound to a repository context.
- **Story Spec**: user stories and split suggestions derived from an Issue.
- **Design Spec**: frontend / backend design documents derived from confirmed Stories.
- **Work Item**: executable tasks derived from both Story and Design Specs.

Clicking an Issue enters focus mode; the right three columns automatically filter to that Issue's full derivation chain, making it easy to trace the path from requirement to code.

![Issue Lifecycle Workbench](assets/readme/issue-lifecycle-workbench-overview.jpg)

### 2. Project / Repository / Issue Management

The left sidebar centrally manages Projects and Repositories:

- Create a Project as a collection of requirements.
- Add local repository paths to a Project; Issues are automatically bound to code context upon creation.
- Issues support Markdown descriptions, phases (clarification / development), and status tracking.

![Project and Repository Management](assets/readme/project-repository-management.jpg)

### 3. AI Provider Driver & Dependency Self-Check

Aria depends on **Claude Code** (required) and **Codex** (optional) as underlying executors. On first launch it automatically inspects the environment:

- Checks whether `node` / `npm` are available.
- Checks whether `claude` / `codex` commands are on `PATH`.
- If missing, guides the user through `npm install -g` (not the official one-liner), making it easier to sync through internal mirrors or package managers.

The provider architecture is extensible; OpenCode, Kimi Code, and other AI executors can be added with low friction.

![Issue Description: Provider Dependency Self-Check](assets/readme/provider-dependency-self-check.jpg)

### 4. Story / Design / Work Item Generation & Version History

Every artifact card supports:

- **AI generation**: trigger a provider to produce suggestions from an Issue or upstream artifact.
- **Cross-review**: author provider generates → reviewer provider reviews → author provider revises; after the configured number of rounds, it proceeds to human confirmation.
- **Version history**: every generation and revision is kept as a version (v1, v2, …) with full Markdown and review records.
- **Human confirmation**: confirmed artifacts can flow downstream.

![Story Spec Version History](assets/readme/story-spec-version-history.jpg)

![Story Spec Full Preview](assets/readme/story-spec-full-preview.jpg)

![Design Spec Version History](assets/readme/design-spec-version-history.jpg)

![Design Spec Full Preview](assets/readme/design-spec-full-preview.jpg)

### 5. Work Item Group & Execution Plan

Work Items are organized as Groups containing:

- Dependency graph
- Split options (frontend/backend split, include integration/E2E tests, require execution-plan confirmation)
- Child Work Item details
- Associated Coding Attempt status

After a Plan is generated and human-confirmed, the Work Item can enter the Coding Workspace for development, testing, review, rework, and final acceptance.

![Work Item Group Details](assets/readme/work-item-group-details.jpg)

### 6. Provider Workspace Dialog

A unified conversational workspace hosts the full lifecycle of Story, Design, and Work Item artifacts:

- Left track shows current node and state.
- Center chat area shows provider inputs, outputs, clarification Q&A, and user instructions.
- Right artifact area shows Markdown/JSON artifacts, version history, review comments, and confirmation records.

![Story Spec Review Flow](assets/readme/story-spec-review-flow.jpg)

![Design Spec Review Flow](assets/readme/design-spec-review-flow.jpg)

### 7. Coding Workspace

Once a Work Item is confirmed, it enters the Coding Workspace:

- Multi-role execution (Analyst, Planner, Coder, Tester, Reviewer) in sequence or parallel.
- Real-time streaming events, permission requests, Stage Gates, and human confirmations.
- Supports execution-plan changes, Abort, Diff viewing, and Rollback.

---

## Tech Stack

| Layer | Technology |
|-------|------------|
| Backend | Rust (Edition 2024), Axum, Tokio, rust-embed |
| Frontend | TypeScript, React 19, Vite, TanStack Router, Zustand, Tailwind CSS, Monaco Editor |
| Testing | Rust integration / unit tests, Vitest, Playwright E2E |
| Distribution | npm / npx (`@cadence-aria/cli`) |

---

## Quick Start

### Use via npx (recommended for end users)

```bash
npx @cadence-aria/cli
```

This opens the browser automatically and navigates to `http://127.0.0.1:<port>/workbench`.

### Run from source

```bash
# 1. Build frontend assets (the backend embeds web/dist at compile time via rust-embed)
pnpm -C web install   # first time only
pnpm -C web build

# 2. Start the backend + frontend dev server
cargo run --locked -- web --workspace . --host 127.0.0.1 --port 4317
# In another terminal, start the frontend dev server
cd web && pnpm dev --port 5173
```

In dev mode the frontend proxies `/api` to `http://127.0.0.1:4317`. Open `http://127.0.0.1:5173`.

### Requirements

- macOS or Linux (Windows is not currently supported)
- Rust (version pinned in `rust-toolchain.toml`)
- Node.js >= 18 and pnpm
- Claude Code (the app guides installation if missing on first launch)

---

## Development Guide

### Build & Check

```bash
# Any cargo build/test must be preceded by a frontend build
pnpm -C web build

# Formatting
cargo fmt --check

# Clippy
cargo clippy --all-targets --all-features --locked -- -D warnings

# Check
cargo check --locked

# Test
cargo test --locked

# Frontend type check
cd web && pnpm tsc -b

# Frontend unit tests
cd web && pnpm test
```

> Note: do **not** pass `-j 1` to `cargo test`; parallelism is managed by `.cargo/config.toml`.

### npx Local Smoke Test

```bash
pnpm -C web build
cargo build --release
node scripts/smoke-npx.mjs
```

See also:

- `cadence/readmes/2026-05-31_README_npx分发开发与发布指南_v1.0.md`
- `cadence/readmes/2026-05-30_README_sccache可选缓存_v1.0.md`
- `cadence/project-rules/build-test-commands.md`

---

## Project Structure

```text
.
├── src/                    # Rust backend
│   ├── cli.rs              # CLI entry
│   ├── cross_cutting/      # Provider adapters, process management, artifact validation
│   ├── daemon/             # Daemon and state machine
│   ├── interactive/        # REPL / interactive projections
│   ├── product/            # Product indexes: Project / Issue / Repository / WorkItem
│   ├── protocol/           # Protocol definitions
│   ├── runtime_units/      # Runtime execution units
│   ├── task_run/           # Task run orchestration
│   └── web/                # Web API, WebSocket, static assets
├── web/                    # React + TypeScript frontend
│   ├── src/
│   │   ├── components/     # Components (lifecycle, chat-workspace, coding-workspace)
│   │   ├── pages/          # Pages (Workbench, ChatWorkspace, CodingWorkspace)
│   │   ├── state/          # Zustand state
│   │   └── api/            # API client and types
│   └── e2e/                # Playwright E2E
├── npm/                    # npx distribution packages
│   ├── cli/                # JS launcher
│   ├── cli-darwin-arm64/
│   ├── cli-darwin-x64/
│   └── cli-linux-x64/
├── cadence/                # Project artifacts: designs, plans, PRDs, rules
│   ├── designs/            # Technical design docs
│   ├── plans/              # Planning docs
│   ├── prds/               # Product requirement docs
│   └── project-rules/      # Project-specific rules
├── tests/                  # Rust integration tests
├── scripts/                # Build and release scripts
└── README.md               # 中文 README
```

---

## How to Use the System

### 1. Launch & First-Time Dependency Check

#### For end users

```bash
npx @cadence-aria/cli
```

The launcher picks a free port, starts the backend, and opens the browser at `http://127.0.0.1:<port>/workbench`.

#### From source

```bash
pnpm -C web build
cargo run --locked -- web --workspace . --host 127.0.0.1 --port 4317
# In another terminal
cd web && pnpm dev --port 5173
```

Then open `http://127.0.0.1:5173`. The dev frontend proxies `/api/*` to the backend on port `4317`.

#### First-launch dependency self-check

On first launch Aria checks the environment:

- `node` / `npm` availability;
- `claude` (Claude Code, required) on `PATH`;
- `codex` (optional) on `PATH`.

If anything is missing, a dialog guides you to install the missing provider via `npm install -g`. Claude Code must be installed before you can continue; Codex can be skipped and installed later through the **Provider Config** button in the top-right corner.

---

### 2. The Workbench Main UI

The default route is `/workbench`, split into three areas:

- **Left sidebar**: Project list, repositories for the selected Project, and current Issue count.
- **Issues column**: all Issues under the selected Project.
- **Detail panel**: when an Issue is selected, shows Story Spec, Design Spec, and Work Item cards derived from it.

---

### 3. Create a Project

1. Click **+ New Project** at the top of the left sidebar.
2. Enter the Project name.
3. The new Project appears in the list and is auto-selected.

A Project is the container for requirements. All Issues, Repositories, Specs, and Work Items belong to a Project.

---

### 4. Add a Repository

An Issue must be bound to a code repository, because Story / Design / Work Item generation and execution all rely on that context.

1. In the left sidebar, under **Repositories**, click **+ Add Repository**.
2. Enter the absolute path of the local repository.
3. Save. The Repository appears in the list with its name and path.

If the path later becomes invalid, related Issues and derived cards enter a blocked state until you rebind or fix the path.

---

### 5. Create an Issue

1. Click **+ New Issue** near the Issues column.
2. Select the Repository to bind.
3. Enter a title and Markdown description (background, ideas, constraints, etc.).
4. Save. The Issue appears in the Issues list with status `draft`.

Click an Issue card to enter **focus mode**: the right panel filters Story Spec, Design Spec, and Work Item cards to only those derived from the selected Issue.

---

### 6. Generate a Story Spec

1. Select the target Issue in the Issues list.
2. Click **Generate Story Spec** on the Issue card or in the detail panel.
3. The **Provider Workspace dialog** opens:
   - **Left track**: fixed flow nodes and current status (context preparation, split suggestion, draft generation, cross-review, revision, human confirmation).
   - **Center chat**: the provider suggests splits based on the Issue description and repository context; you can add instructions or answer clarification questions.
   - **Right artifact pane**: generated Markdown, version history, review comments.
4. The provider may propose splitting one Issue into multiple Story Specs.
5. After reviewing, confirm to create the corresponding Story Spec cards.
6. Each Story Spec card shows its status, e.g. `confirmed` / `v1`.

---

### 7. Generate a Design Spec

A Design Spec can only be derived from a **confirmed** Story Spec.

1. Select a confirmed Story Spec in the detail panel.
2. Click **Generate Design Spec**.
3. In the Workspace dialog, the provider proposes:
   - Story-to-Design mapping (one-to-one or many-to-one);
   - Design kind: `frontend`, `backend`, or both.
4. Review the mapping and kinds, then confirm to create the Design Spec cards.
5. Each Design Spec also supports multiple versions, cross-review, and human confirmation.

---

### 8. Generate a Work Item Plan and Work Items

A Work Item must be derived from both a Story Spec and a Design Spec.

1. Select confirmed Story Specs and Design Specs.
2. Click **Generate Work Item** / **Prepare Work Item Plan**.
3. The **Work Item Plan options dialog** opens. Configure:
   - `include_integration_tests`
   - `include_e2e_tests`
   - `force_frontend_backend_split`
   - `require_execution_plan_confirm`
4. After confirming the options, the provider generates a Plan containing:
   - A list of child Work Items;
   - A dependency graph between Work Items;
   - Split findings / validator findings.
5. Review the Plan and confirm. A Work Item Group card is created.
6. The Work Item Group card shows child Work Items and associated Coding Attempt status.

**Key rule**: a Work Item must have a Plan and human confirmation before entering the Coding stage.

---

### 9. Working Inside the Provider Workspace Dialog

Whether for Story, Design, or Work Item, the Workspace dialog works the same way:

- **Top config bar**: view or adjust provider combination, review rounds, superpowers / OpenSpec constraints, repository context.
- **Left flow track**: fixed nodes showing where you are in the process.
- **Center chat area**:
  - View provider input summaries;
  - View streaming provider output;
  - Add instructions or answer clarifications;
  - View permission requests, review verdicts, stage changes.
- **Right artifact pane**:
  - View full Markdown / JSON artifacts;
  - Switch between versions (v1, v2, …);
  - View the review comment chain;
  - View confirmation records.

When the flow reaches a human confirmation node, it pauses until you confirm or request changes, which triggers another revision round.

---

### 10. Enter the Coding Workspace

1. On a Work Item Group or individual Work Item card, click **Open Workspace** / **Start Coding**.
2. The app navigates to `/workbench/coding/$attemptId`, the Coding Workspace.

Typical flow inside the Coding Workspace:

- **Analyst**: analyzes requirements and code context;
- **Planner**: produces an execution plan;
- **Coder**: changes code according to the plan;
- **Tester**: runs tests and verifies results;
- **Reviewer**: reviews the changes.

Each role streams events over WebSocket in real time. The left panel shows the Timeline, the center shows chat/output, and the right panel shows Artifacts / Diff / Test Reports.

During execution you may encounter:

- **Permission requests**: the provider asks before sensitive actions like writing files or running commands;
- **Stage Gates**: the flow pauses at key nodes for human confirmation, change request, or termination;
- **Execution-plan change requests**: you can ask the provider to revise the plan;
- **Abort**: stop the current Coding Attempt at any time;
- **Rollback**: revert to a previous state if the result is unsatisfactory.

---

### 11. Deleting Projects, Issues, and Artifacts

You can delete directly from the Workbench:

- **Project**: click the delete button next to the Project card in the left sidebar;
- **Repository**: click the delete button in the repository list;
- **Issue**: click the delete button on the Issue card;
- **Story Spec / Design Spec / Work Item**: click the delete button on the card in the detail panel.

Deletions are usually soft-deletes at the product index layer, so records can be reconstructed from runtime data if needed.

---

### 12. Refresh and Status Monitoring

- The page polls the current Project’s Issues, Repositories, and Lifecycle data automatically.
- Click the **Refresh** button to sync manually.
- Issue and artifact cards show current status, version, confirmation state, and last update time.
- The current Project’s Issue count is shown at the bottom of the left sidebar.

---

### 13. CLI Companion Commands

In addition to the web UI, Aria provides a CLI:

```bash
# Check daemon status
aria daemon status --workspace .

# Start the daemon
aria daemon run --workspace .

# Run a task directly
aria task run --workspace . "your requirement here"

# Start the web server
aria web --workspace . --host 127.0.0.1 --port 4317
```

However, most day-to-day operations are performed through the `/workbench` web UI.

---

## License

[MIT](LICENSE)
