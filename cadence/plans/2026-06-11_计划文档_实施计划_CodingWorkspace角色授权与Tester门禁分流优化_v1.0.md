# CodingWorkspace 角色授权与 Tester 门禁分流优化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 Coding Workspace 角色级授权配置、Tester JSON repair，以及 Testing 结果分流，避免 Tester 被权限/契约问题阻塞后自动进入 Analyst 返修。

**Architecture:** 在 `CodingRoleProviderConfigSnapshot` 中扩展 role-level permission mode，并让 Coding Workspace 每个 provider role 按配置构造 `StreamingProviderInput`。Tester 两段式流程新增一次 repair turn，Testing blocked 由 blocked gate 暂停，只有有有效测试证据的 failed report 才进入 Analyst。

**Tech Stack:** Rust 1.95.0、Cargo、Axum WebSocket、React/Vite、pnpm、Cadence Aria Coding Workspace。

---

## 参考文档

- 设计文档：`cadence/designs/2026-06-11_技术方案_CodingWorkspace角色授权与Tester门禁分流优化_v1.0.md`
- Rust 命令规则：`cadence/project-rules/build-test-commands.md`
- 强制约束：本计划所有 `cargo test` 命令均不得使用 `-j 1`。

## 文件结构

- Modify: `src/product/coding_models.rs`
  - 新增 Coding role permission mode 数据类型。
  - 扩展 `CodingRoleProviderConfigSnapshot`，保留旧 JSON 默认兼容。
- Modify: `src/product/coding_attempt_store.rs`
  - 读取旧 `role-provider-config.json` 时补默认 permission modes。
  - 更新 role provider config 持久化。
- Modify: `src/product/coding_workspace_engine.rs`
  - 所有 provider run 按 role 读取 permission mode。
  - 记录 auto approval 事件。
  - Tester `plan_tests` / `execute_test_plan` 增加 repair。
  - 暴露 Testing report 分流 helper。
- Modify: `src/web/coding_ws_handler.rs`
  - 新增 `permission_mode_select` WebSocket 入站消息。
  - Session snapshot 返回扩展后的 role config。
  - Testing blocked 不自动进入 Analyst。
- Modify: `src/web/test_controls.rs`
  - Controlled fake provider 支持 repair 场景 fixture。
- Modify: `web/src/api/types.ts`
  - 增加 `CodingProviderPermissionMode`、role config 字段和 WS 消息类型。
- Modify: `web/src/hooks/useCodingWorkspaceWs.ts`
  - 增加 `sendPermissionModeSelect`。
- Modify: `web/src/components/coding-workspace/CodingProviderConfigPanel.tsx`
  - 显示和修改每个 role 的 permission mode。
- Modify: `web/src/pages/CodingWorkspacePage.tsx`
  - 传递 permission mode 更新 handler。
- Modify: `web/src/pages/CodingWorkspacePage.test.tsx`
  - 覆盖前端权限模式 UI、blocked gate 文案。
- Modify: `web/src/hooks/useCodingWorkspaceWs.test.tsx`
  - 补齐 session snapshot 中的 `permission_modes` 测试数据。
- Modify: `web/src/state/coding-workspace-store.test.ts`
  - 补齐 role provider config fixture 的 `permission_modes`。
- Modify: `web/src/api/types.test.ts`
  - 补齐 API 类型 fixture 的 `permission_modes`。
- Modify: `tests/it_product/product_coding_workspace_engine.rs`
  - 覆盖 role permission mode、Tester repair、Testing routing。
- Modify: `tests/it_web/web_coding_ws_handler.rs`
  - 覆盖 WS permission mode select、blocked 不进 Analyst。

---

### Task 1: Role-Level Permission Mode 数据模型与持久化

**Files:**

- Modify: `src/product/coding_models.rs`
- Modify: `src/product/coding_attempt_store.rs`
- Test: `src/product/coding_models.rs`
- Test: `tests/it_product/product_coding_workspace_engine.rs`

- [ ] **Step 1: 写失败测试，证明旧 config 反序列化时补默认 permission modes**

在 `src/product/coding_models.rs` tests 中新增：

```rust
#[test]
fn role_provider_config_deserializes_legacy_json_with_default_permission_modes() {
    let legacy = r#"{
      "coder": "codex",
      "tester": "claude_code",
      "analyst": "claude_code",
      "code_reviewer": "codex",
      "internal_reviewer": "claude_code",
      "review_rounds": 1
    }"#;

    let snapshot: CodingRoleProviderConfigSnapshot =
        serde_json::from_str(legacy).expect("legacy role config");

    assert_eq!(snapshot.permission_mode_for_role(&CodingProviderRole::Coder), CodingProviderPermissionMode::Supervised);
    assert_eq!(snapshot.permission_mode_for_role(&CodingProviderRole::Tester), CodingProviderPermissionMode::Auto);
    assert_eq!(snapshot.permission_mode_for_role(&CodingProviderRole::Analyst), CodingProviderPermissionMode::Auto);
    assert_eq!(snapshot.permission_mode_for_role(&CodingProviderRole::CodeReviewer), CodingProviderPermissionMode::Supervised);
    assert_eq!(snapshot.permission_mode_for_role(&CodingProviderRole::InternalReviewer), CodingProviderPermissionMode::Supervised);
}
```

- [ ] **Step 2: 运行测试确认 RED**

```bash
cargo test --locked --lib role_provider_config_deserializes_legacy_json_with_default_permission_modes
```

Expected: 编译失败，提示 `CodingProviderPermissionMode` 或 `permission_mode_for_role` 不存在。

- [ ] **Step 3: 新增 permission mode 类型和默认配置**

在 `src/product/coding_models.rs` 增加：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingProviderPermissionMode {
    Auto,
    Supervised,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingRolePermissionModes {
    pub coder: CodingProviderPermissionMode,
    pub tester: CodingProviderPermissionMode,
    pub analyst: CodingProviderPermissionMode,
    pub code_reviewer: CodingProviderPermissionMode,
    pub internal_reviewer: CodingProviderPermissionMode,
}

impl Default for CodingRolePermissionModes {
    fn default() -> Self {
        Self {
            coder: CodingProviderPermissionMode::Supervised,
            tester: CodingProviderPermissionMode::Auto,
            analyst: CodingProviderPermissionMode::Auto,
            code_reviewer: CodingProviderPermissionMode::Supervised,
            internal_reviewer: CodingProviderPermissionMode::Supervised,
        }
    }
}
```

扩展 `CodingRoleProviderConfigSnapshot`：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingRoleProviderConfigSnapshot {
    pub coder: ProviderName,
    pub tester: ProviderName,
    pub analyst: ProviderName,
    pub code_reviewer: ProviderName,
    pub internal_reviewer: ProviderName,
    pub review_rounds: u32,
    #[serde(default)]
    pub permission_modes: CodingRolePermissionModes,
}
```

在 `From<&ProviderConfigSnapshot>` 中补：

```rust
permission_modes: CodingRolePermissionModes::default(),
```

同步修复当前仓库内已有 `CodingRoleProviderConfigSnapshot { ... }` 字面量。执行者必须在以下文件中给每个结构体字面量补 `permission_modes: CodingRolePermissionModes::default(),`，并在对应 `use cadence_aria::product::coding_models::{ ... }` 中加入 `CodingRolePermissionModes`：

- `tests/it_product/product_coding_workspace_engine.rs`
- `tests/it_product/product_coding_attempt_store.rs`
- `tests/it_product/product_coding_models.rs`

补齐后的字面量形状：

```rust
CodingRoleProviderConfigSnapshot {
    coder: ProviderName::Codex,
    tester: ProviderName::Fake,
    analyst: ProviderName::Codex,
    code_reviewer: ProviderName::Fake,
    internal_reviewer: ProviderName::Fake,
    review_rounds: 1,
    permission_modes: CodingRolePermissionModes::default(),
}
```

在 `impl CodingRoleProviderConfigSnapshot` 中增加：

```rust
pub fn permission_mode_for_role(
    &self,
    role: &CodingProviderRole,
) -> CodingProviderPermissionMode {
    match role {
        CodingProviderRole::Coder => self.permission_modes.coder,
        CodingProviderRole::Tester => self.permission_modes.tester,
        CodingProviderRole::Analyst => self.permission_modes.analyst,
        CodingProviderRole::CodeReviewer => self.permission_modes.code_reviewer,
        CodingProviderRole::InternalReviewer => self.permission_modes.internal_reviewer,
    }
}

pub fn set_permission_mode_for_role(
    &mut self,
    role: &CodingProviderRole,
    mode: CodingProviderPermissionMode,
) {
    match role {
        CodingProviderRole::Coder => self.permission_modes.coder = mode,
        CodingProviderRole::Tester => self.permission_modes.tester = mode,
        CodingProviderRole::Analyst => self.permission_modes.analyst = mode,
        CodingProviderRole::CodeReviewer => self.permission_modes.code_reviewer = mode,
        CodingProviderRole::InternalReviewer => self.permission_modes.internal_reviewer = mode,
    }
}
```

- [ ] **Step 4: 写 store 持久化测试**

在 `tests/it_product/product_coding_workspace_engine.rs` 新增：

```rust
#[test]
fn role_permission_modes_are_persisted_with_role_provider_config() {
    let root = tempfile::tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create attempt");

    let mut snapshot = store
        .get_role_provider_config_snapshot("project_0001", "issue_0001", &attempt.id)
        .expect("default role config");
    snapshot.set_permission_mode_for_role(
        &CodingProviderRole::CodeReviewer,
        CodingProviderPermissionMode::Auto,
    );
    store
        .update_role_provider_config_snapshot(
            "project_0001",
            "issue_0001",
            &attempt.id,
            snapshot,
        )
        .expect("save role config");

    let saved = store
        .get_role_provider_config_snapshot("project_0001", "issue_0001", &attempt.id)
        .expect("saved role config");
    assert_eq!(
        saved.permission_mode_for_role(&CodingProviderRole::CodeReviewer),
        CodingProviderPermissionMode::Auto
    );
}
```

- [ ] **Step 5: 运行 Task 1 测试确认 GREEN**

```bash
cargo test --locked --lib role_provider_config_deserializes_legacy_json_with_default_permission_modes
cargo test --locked --test it_product role_permission_modes_are_persisted_with_role_provider_config
```

Expected: 两个测试通过。

- [ ] **Step 6: Commit Task 1**

```bash
git add src/product/coding_models.rs src/product/coding_attempt_store.rs tests/it_product/product_coding_workspace_engine.rs
git commit -m "feat: add coding role permission modes"
```

---

### Task 2: Provider Run 按角色使用 Permission Mode

**Files:**

- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `src/cross_cutting/approval_bridge.rs`
- Test: `tests/it_product/product_coding_workspace_engine.rs`
- Test: `src/cross_cutting/approval_bridge.rs`

- [ ] **Step 1: 写失败测试，证明 Tester 使用 role permission mode**

在 `tests/it_product/product_coding_workspace_engine.rs` 的 `cadence_aria::product::coding_models` import 中加入 `CodingProviderPermissionMode`。

在 `tests/it_product/product_coding_workspace_engine.rs` 的 `SessionInputCapturingProvider` 相关区域新增：

```rust
#[tokio::test]
async fn coding_tester_uses_role_permission_mode_auto() {
    let root = tempfile::tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    let mut role_config = store
        .get_role_provider_config_snapshot("project_0001", "issue_0001", &attempt.id)
        .expect("role config");
    role_config.set_permission_mode_for_role(
        &CodingProviderRole::Tester,
        CodingProviderPermissionMode::Auto,
    );
    store
        .update_role_provider_config_snapshot("project_0001", "issue_0001", &attempt.id, role_config)
        .expect("save role config");
    store
        .update_attempt_status("project_0001", "issue_0001", &attempt.id, CodingAttemptStatus::Running)
        .expect("running");

    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let provider = SessionInputCapturingProvider::with_outputs(
        [
            r#"{"summary":"unit","steps":[{"id":"unit","title":"Unit","intent":"verify unit","required":true,"tool":"provider_managed","risk_level":"low","command_or_tool_input":{},"evidence_expectation":"provider evidence"}]}"#,
            r#"{"step_results":[{"step_id":"unit","status":"passed","evidence_refs":["unit.log"],"provider_analysis":"ok"}]}"#,
        ],
        [None, None],
    );

    engine
        .execute_testing_with_provider(
            &attempt,
            &provider,
            &CodingExecutionContext {
                work_item_markdown: Some("Work Item".to_string()),
                verification_commands: Vec::new(),
            },
            &[],
            TesterAgentOptions::default(),
        )
        .await
        .expect("testing");

    let inputs = provider.inputs.lock().expect("inputs");
    assert_eq!(inputs[0].permission_mode, ProviderPermissionMode::Auto);
    assert_eq!(inputs[1].permission_mode, ProviderPermissionMode::Auto);
}
```

- [ ] **Step 2: 运行测试确认 RED**

```bash
cargo test --locked --test it_product coding_tester_uses_role_permission_mode_auto
```

Expected: 失败，当前 input 仍是 `ProviderPermissionMode::Supervised`。

- [ ] **Step 3: 实现 role -> ProviderPermissionMode 映射**

在 `src/product/coding_workspace_engine.rs` 替换固定函数：

```rust
fn coding_provider_permission_mode(mode: CodingProviderPermissionMode) -> ProviderPermissionMode {
    match mode {
        CodingProviderPermissionMode::Auto => ProviderPermissionMode::Auto,
        CodingProviderPermissionMode::Supervised => ProviderPermissionMode::Supervised,
    }
}
```

增加 helper：

```rust
fn role_permission_mode_for_attempt(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    role: CodingProviderRole,
) -> Result<ProviderPermissionMode, CodingWorkspaceEngineError> {
    let snapshot = store.get_role_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    Ok(coding_provider_permission_mode(
        snapshot.permission_mode_for_role(&role),
    ))
}
```

所有 `StreamingProviderInput { permission_mode: ... }` 改为按 role 传入：

```rust
permission_mode: role_permission_mode_for_attempt(
    &self.store,
    &attempt,
    CodingProviderRole::Tester,
)?,
```

Coder、Analyst、CodeReviewer、InternalReviewer 对应替换为各自 role。

- [ ] **Step 4: 增加 auto approval 审计事件测试**

在 `src/cross_cutting/approval_bridge.rs` tests 新增：

```rust
#[tokio::test]
async fn approval_bridge_auto_emits_auto_approval_event() {
    let (event_tx, mut event_rx) = mpsc::channel(8);
    let bridge = ApprovalBridge::new(ProviderPermissionMode::Auto, event_tx);

    let decision = bridge
        .request_tool(
            "Bash",
            "cargo test --locked",
            RiskLevel::Medium,
            CancellationToken::new(),
        )
        .await
        .expect("auto decision");

    assert!(decision.approved);
    let event = tokio::time::timeout(TEST_TIMEOUT, event_rx.recv())
        .await
        .expect("auto approval event")
        .expect("event");
    match event {
        ProviderEvent::Execution(event) => {
            assert_eq!(event.title, "Auto approval");
            assert!(event.detail.as_deref().unwrap_or_default().contains("cargo test --locked"));
        }
        other => panic!("unexpected event: {other:?}"),
    }
}
```

- [ ] **Step 5: 实现 auto approval 审计事件**

在 `ApprovalBridge::request_tool` 的 `ProviderPermissionMode::Auto` 分支发送 `ProviderEvent::Execution`：

```rust
let _ = self
    .event_tx
    .send(ProviderEvent::Execution(ProviderExecutionEvent {
        event_id: format!("auto_approval_{id}", id = next_permission_id()),
        kind: ProviderExecutionEventKind::Provider,
        status: ProviderExecutionEventStatus::Completed,
        title: "Auto approval".to_string(),
        detail: Some(format!("{tool_name}: {description}")),
        command: None,
        cwd: None,
        output: Some(serde_json::json!({
            "auto_approved": true,
            "tool_name": tool_name,
            "description": description,
            "risk_level": risk_level,
        }).to_string()),
        exit_code: None,
    }))
    .await;
return Ok(PermissionDecision {
    approved: true,
    reason: Some("auto_approved".to_string()),
});
```

`ProviderExecutionEventKind` 当前没有专用 permission 类型，审计事件使用现有 `ProviderExecutionEventKind::Provider`，通过 `title="Auto approval"` 和 `output.auto_approved=true` 区分。

- [ ] **Step 6: 运行 Task 2 测试确认 GREEN**

```bash
cargo test --locked --test it_product coding_tester_uses_role_permission_mode_auto
cargo test --locked --lib approval_bridge_auto_emits_auto_approval_event
```

Expected: 两个测试通过。

- [ ] **Step 7: Commit Task 2**

```bash
git add src/product/coding_workspace_engine.rs src/cross_cutting/approval_bridge.rs tests/it_product/product_coding_workspace_engine.rs
git commit -m "feat: apply coding role permission modes"
```

---

### Task 3: WebSocket 与前端支持 Permission Mode 配置

**Files:**

- Modify: `src/web/coding_ws_handler.rs`
- Modify: `web/src/api/types.ts`
- Modify: `web/src/hooks/useCodingWorkspaceWs.ts`
- Modify: `web/src/components/coding-workspace/CodingProviderConfigPanel.tsx`
- Modify: `web/src/pages/CodingWorkspacePage.tsx`
- Test: `tests/it_web/web_coding_ws_handler.rs`
- Test: `web/src/pages/CodingWorkspacePage.test.tsx`
- Test: `web/src/hooks/useCodingWorkspaceWs.test.tsx`
- Test: `web/src/state/coding-workspace-store.test.ts`
- Test: `web/src/api/types.test.ts`

- [ ] **Step 1: 写后端 WS 失败测试**

在 `tests/it_web/web_coding_ws_handler.rs` 的 `cadence_aria::product::coding_models` import 中加入 `CodingProviderPermissionMode`。

在 `tests/it_web/web_coding_ws_handler.rs` 新增：

```rust
#[tokio::test]
async fn coding_ws_permission_mode_select_updates_role_config() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &CodingWsInMessage::PermissionModeSelect {
            role: "tester".to_string(),
            permission_mode: CodingProviderPermissionMode::Supervised,
        },
    )
    .await;

    assert_eq!(
        wait_for_provider_config_update(&mut ws).await,
        CodingWsOutMessage::CodingProviderConfigUpdated {
            role: CodingProviderRole::Tester,
            provider: ProviderName::Fake,
        }
    );

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState {
            role_provider_config_snapshot,
            ..
        } => {
            assert_eq!(
                role_provider_config_snapshot.permission_mode_for_role(&CodingProviderRole::Tester),
                CodingProviderPermissionMode::Supervised
            );
        }
        other => panic!("expected updated coding session state, got {other:?}"),
    }

    let snapshot = store
        .get_role_provider_config_snapshot("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("role config");
    assert_eq!(
        snapshot.permission_mode_for_role(&CodingProviderRole::Tester),
        CodingProviderPermissionMode::Supervised
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}
```

- [ ] **Step 2: 运行后端测试确认 RED**

```bash
cargo test --locked --test it_web coding_ws_permission_mode_select_updates_role_config
```

Expected: 编译失败，`PermissionModeSelect` 不存在。

- [ ] **Step 3: 实现 WS 入站消息和 handler**

在 `src/web/coding_ws_handler.rs` 的 `CodingWsInMessage` 增加：

```rust
PermissionModeSelect {
    role: String,
    permission_mode: CodingProviderPermissionMode,
},
```

在 WebSocket inbound handler 中增加分支：

```rust
} else if let CodingWsInMessage::PermissionModeSelect { role, permission_mode } = inbound {
    let Some(parsed_role) = parse_coding_provider_role(&role) else {
        let _ = send_coding_json(
            &mut socket_tx,
            &CodingWsOutMessage::CodingProtocolError {
                code: "coding_permission_mode_role_invalid".to_string(),
                message: format!("unknown coding role: {role}"),
            },
        )
        .await;
        continue;
    };
    let mut role_snapshot = coding_store.get_role_provider_config_snapshot(
        &current_attempt.project_id,
        &current_attempt.issue_id,
        &current_attempt.id,
    )?;
    let provider = role_snapshot.provider_for_role(&parsed_role).clone();
    role_snapshot.set_permission_mode_for_role(&parsed_role, permission_mode);
    coding_store.update_role_provider_config_snapshot(
        &current_attempt.project_id,
        &current_attempt.issue_id,
        &current_attempt.id,
        role_snapshot,
    )?;
    let _ = send_coding_json(
        &mut socket_tx,
        &CodingWsOutMessage::CodingProviderConfigUpdated {
            role: parsed_role.clone(),
            provider,
        },
    )
    .await;
    if let Ok(snapshot) = build_coding_session_state(&coding_store, current_attempt.clone()) {
        let _ = send_coding_json(&mut socket_tx, &snapshot).await;
    }
```

同步更新 `is_coding_ws_message_allowed`，允许 active 状态下的 `PermissionModeSelect`。

- [ ] **Step 4: 写前端失败测试**

在 `web/src/pages/CodingWorkspacePage.test.tsx` 修改 role panel 测试，扩展 snapshot：

```ts
roleProviderConfigSnapshot: {
  coder: "fake",
  tester: "fake",
  analyst: "fake",
  code_reviewer: "fake",
  internal_reviewer: "fake",
  review_rounds: 1,
  permission_modes: {
    coder: "supervised",
    tester: "auto",
    analyst: "auto",
    code_reviewer: "supervised",
    internal_reviewer: "supervised",
  },
},
```

在 `mockCodingWs()` 返回对象中新增：

```ts
sendPermissionModeSelect: vi.fn(),
```

在 `web/src/hooks/useCodingWorkspaceWs.test.tsx`、`web/src/state/coding-workspace-store.test.ts`、`web/src/api/types.test.ts` 中，将所有 `role_provider_config_snapshot` / `roleProviderConfigSnapshot` fixture 补齐相同的 `permission_modes` 字段：

```ts
permission_modes: {
  coder: "supervised",
  tester: "auto",
  analyst: "auto",
  code_reviewer: "supervised",
  internal_reviewer: "supervised",
},
```

新增 role panel 断言：

```ts
expect(screen.getByTestId("coding-provider-config-panel")).toHaveTextContent("Auto");
await userEvent.click(screen.getByRole("button", { name: "将 Tester 授权模式切换为 Supervised" }));
expect(api.sendPermissionModeSelect).toHaveBeenCalledWith("tester", "supervised");
```

- [ ] **Step 5: 实现前端类型和 Hook**

在 `web/src/api/types.ts` 增加：

```ts
export type CodingProviderPermissionMode = "auto" | "supervised";

export type CodingRolePermissionModes = {
  coder: CodingProviderPermissionMode;
  tester: CodingProviderPermissionMode;
  analyst: CodingProviderPermissionMode;
  code_reviewer: CodingProviderPermissionMode;
  internal_reviewer: CodingProviderPermissionMode;
};
```

扩展 `CodingRoleProviderConfigSnapshot`：

```ts
permission_modes: CodingRolePermissionModes;
```

扩展 `CodingWsInMessage`：

```ts
| {
    type: "permission_mode_select";
    role: CodingProviderRole;
    permission_mode: CodingProviderPermissionMode;
  }
```

在 `web/src/hooks/useCodingWorkspaceWs.ts` 增加：

```ts
const sendPermissionModeSelect = useCallback(
  (role: CodingProviderRole, permissionMode: CodingProviderPermissionMode) => {
    sendJson({ type: "permission_mode_select", role, permission_mode: permissionMode });
  },
  [sendJson],
);
```

并在返回对象暴露 `sendPermissionModeSelect`。

- [ ] **Step 6: 实现前端面板**

在 `CodingProviderConfigPanel.tsx` props 增加：

```ts
onPermissionModeSelect: (
  role: CodingProviderRole,
  permissionMode: CodingProviderPermissionMode,
) => void;
```

在每个 role card 中显示两个按钮：

```tsx
const mode = snapshot.permission_modes[role];
...
<div className="mt-2 flex min-w-0 flex-wrap gap-1">
  {(["auto", "supervised"] as const).map((permissionMode) => (
    <button
      key={permissionMode}
      type="button"
      disabled={locked || permissionMode === mode}
      onClick={() => onPermissionModeSelect(role, permissionMode)}
      aria-label={`将 ${label} 授权模式切换为 ${permissionMode === "auto" ? "Auto" : "Supervised"}`}
      className="inline-flex h-7 items-center rounded-md border border-[var(--aria-line)] px-2 text-[11px] font-semibold text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)] disabled:opacity-45"
    >
      {permissionMode === "auto" ? "Auto" : "Supervised"}
    </button>
  ))}
</div>
```

在 `CodingWorkspacePage.tsx` 传入：

```tsx
onPermissionModeSelect={api.sendPermissionModeSelect}
```

- [ ] **Step 7: 运行 Task 3 测试确认 GREEN**

```bash
cargo test --locked --test it_web coding_ws_permission_mode_select_updates_role_config
pnpm -C web test -- CodingWorkspacePage.test.tsx
pnpm -C web test -- useCodingWorkspaceWs.test.tsx
pnpm -C web test -- coding-workspace-store.test.ts
pnpm -C web test -- types.test.ts
```

Expected: 后端 WS 测试通过；前端页面、hook、store、API type 测试通过。

- [ ] **Step 8: Commit Task 3**

```bash
git add src/web/coding_ws_handler.rs web/src/api/types.ts web/src/hooks/useCodingWorkspaceWs.ts web/src/components/coding-workspace/CodingProviderConfigPanel.tsx web/src/pages/CodingWorkspacePage.tsx web/src/pages/CodingWorkspacePage.test.tsx web/src/hooks/useCodingWorkspaceWs.test.tsx web/src/state/coding-workspace-store.test.ts web/src/api/types.test.ts tests/it_web/web_coding_ws_handler.rs
git commit -m "feat: configure coding role permission modes"
```

---

### Task 4: Tester `plan_tests` 与 `execute_test_plan` Repair

**Files:**

- Modify: `src/product/tester_agent_loop.rs`
- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `src/web/test_controls.rs`
- Test: `src/product/tester_agent_loop.rs`
- Test: `tests/it_product/product_tester_agent_loop.rs`
- Test: `tests/it_web/web_coding_ws_handler.rs`

- [ ] **Step 1: 写 plan repair prompt 单元测试**

在 `src/product/tester_agent_loop.rs` tests 新增：

```rust
#[test]
fn tester_plan_repair_prompt_includes_raw_output_and_schema_error() {
    let prompt = build_tester_plan_repair_prompt(
        "## 最终测试报告\n无法执行 cargo",
        "missing_json_object",
    );

    assert!(prompt.contains("Phase: plan_tests_repair"));
    assert!(prompt.contains("missing_json_object"));
    assert!(prompt.contains("## 最终测试报告"));
    assert!(prompt.contains("\"summary\""));
    assert!(prompt.contains("\"steps\""));
    assert!(prompt.contains("只返回合法 JSON"));
}
```

- [ ] **Step 2: 实现 repair prompt builder**

在 `src/product/tester_agent_loop.rs` 增加：

```rust
pub fn build_tester_plan_repair_prompt(raw_output: &str, parse_error: &str) -> String {
    format!(
        "Tester Provider Runtime\n\
         Phase: plan_tests_repair\n\
         The previous plan_tests output could not be parsed as TestPlan JSON.\n\
         Parse error: {parse_error}\n\
         Return only one valid JSON object. Do not use Markdown fences. Do not explain.\n\
         Required shape:\n\
         {{\"summary\":\"...\",\"context_warnings\":[],\"assumptions\":[],\"steps\":[{{\"id\":\"...\",\"title\":\"...\",\"intent\":\"...\",\"required\":true,\"tool\":\"run_command|read_file|list_files|search_code|provider_managed\",\"risk_level\":\"low|medium|high\",\"command_or_tool_input\":{{}},\"evidence_expectation\":\"...\"}}]}}\n\
         Previous output:\n\
         {raw_output}"
    )
}
```

同步增加 execute repair builder：

```rust
pub fn build_tester_execute_repair_prompt(
    raw_output: &str,
    missing_required_steps: &[String],
) -> String {
    format!(
        "Tester Provider Runtime\n\
         Phase: execute_test_plan_repair\n\
         The previous execute_test_plan output did not provide valid step_results for every required step.\n\
         Missing required steps: {missing_required_steps:?}\n\
         Return only JSON: {{\"step_results\":[{{\"step_id\":\"...\",\"status\":\"passed|failed|blocked|skipped\",\"evidence_refs\":[\"...\"],\"provider_analysis\":\"...\"}}]}}\n\
         Previous output:\n\
         {raw_output}"
    )
}
```

- [ ] **Step 3: 写集成失败测试，证明 Markdown plan 会 repair 后通过**

在 `tests/it_product/product_tester_agent_loop.rs` 顶部把 import 改为包含 `VecDeque`：

```rust
use std::collections::VecDeque;
use std::fs;
use std::sync::{Arc, Mutex};
```

在同一文件新增测试：

```rust
#[tokio::test]
async fn tester_repairs_markdown_plan_output_before_blocking() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (event_tx, _event_rx) = mpsc::channel(64);
    let (_command_tx, mut command_rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    let provider = RepairingTesterProvider {
        outputs: Mutex::new(VecDeque::from([
            "## 最终测试报告\n无法执行测试".to_string(),
            r#"{"summary":"repaired plan","steps":[{"id":"unit","title":"Unit","intent":"verify unit","required":true,"tool":"provider_managed","risk_level":"low","command_or_tool_input":{},"evidence_expectation":"provider evidence"}]}"#.to_string(),
            r#"{"step_results":[{"step_id":"unit","status":"passed","evidence_refs":["unit.log"],"provider_analysis":"ok"}]}"#.to_string(),
        ])),
        captured_prompts: Arc::new(Mutex::new(Vec::new())),
    };

    let report = engine
        .execute_testing_with_provider_commands(
            &attempt,
            &provider,
            &CodingExecutionContext {
                work_item_markdown: Some("Work Item".to_string()),
                verification_commands: Vec::new(),
            },
            &[],
            TesterAgentOptions::default(),
            &mut command_rx,
        )
        .await
        .expect("testing report");

    assert_eq!(report.overall_status, TestingOverallStatus::Passed);
    assert_eq!(report.plan_summary.as_deref(), Some("repaired plan"));
    let raw_refs = store
        .list_testing_reports("project_0001", "issue_0001", &attempt.id)
        .expect("reports");
    assert_eq!(raw_refs.len(), 1);
    let prompts = provider.captured_prompts.lock().expect("prompts");
    assert!(prompts.iter().any(|prompt| prompt.contains("Phase: plan_tests_repair")));
}
```

在 `tests/it_product/product_tester_agent_loop.rs` 现有 `ScriptedTesterProvider` 下方新增独立 fixture，避免和当前文件已有 provider 重名：

```rust
struct RepairingTesterProvider {
    outputs: Mutex<VecDeque<String>>,
    captured_prompts: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for RepairingTesterProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.captured_prompts
            .lock()
            .expect("prompts")
            .push(input.prompt);
        let output = self
            .outputs
            .lock()
            .expect("outputs")
            .pop_front()
            .expect("scripted tester output");
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            if cancel.is_cancelled() {
                return;
            }
            if event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await
                .is_err()
            {
                return;
            }
            if cancel.is_cancelled() {
                return;
            }
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: output,
                    provider_session_id: None,
                })
                .await;
        });

        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}
```

- [ ] **Step 4: 实现 plan repair flow**

在 `execute_testing_with_provider_commands` 的 `parse_test_plan_payload` Err 分支前，增加一次 repair：

```rust
let plan = match parse_test_plan_payload(&attempt.id, &plan_id, &plan_output, Some(plan_raw_ref.clone())) {
    Ok(plan) => {
        self.store.save_test_plan(&plan)?;
        plan
    }
    Err(first_error) => {
        let repair_prompt = build_tester_plan_repair_prompt(&plan_output, &first_error.to_string());
        let repair_adapter_input = AdapterInput {
            provider_type: provider_type_for_name(&tester_provider),
            role: AdapterRole::Reviewer,
            worktree_path: Some(worktree_path.to_string_lossy().to_string()),
            prompt: repair_prompt,
            context_files: Vec::new(),
            output_schema: "coding_workspace_test_plan_json".to_string(),
            timeout: options.timeout.as_secs().max(1),
            max_retries: 0,
        };
        let repair_input = StreamingProviderInput {
            provider_type: repair_adapter_input.provider_type.clone(),
            role: repair_adapter_input.role.clone(),
            prompt: repair_adapter_input.prompt.clone(),
            working_dir: worktree_path.clone(),
            workspace_session_id: Some(attempt.id.clone()),
            resume_provider_session_id: None,
            permission_mode: role_permission_mode_for_attempt(
                &self.store,
                &attempt,
                CodingProviderRole::Tester,
            )?,
            env_vars: BTreeMap::new(),
            timeout_secs: repair_adapter_input.timeout,
        };
        let repair_output = self.run_provider_stream_to_completion(CodingProviderStreamRun {
            attempt: &attempt,
            node_id: &node.id,
            provider,
            legacy_input: &repair_adapter_input,
            input: repair_input,
            provider_name: &tester_provider,
            provider_role: CodingProviderRole::Tester,
            command_rx,
            allow_legacy_stream_fallback: false,
        }).await?;
        let repair_raw_ref = self.store.save_provider_raw_output(
            &attempt.id,
            CodingExecutionStage::Testing,
            "plan_tests_repair",
            &repair_output,
        )?;
        match parse_test_plan_payload(&attempt.id, &plan_id, &repair_output, Some(repair_raw_ref.clone())) {
            Ok(plan) => {
                self.store.save_test_plan(&plan)?;
                plan
            }
            Err(repair_error) => {
                return self.block_invalid_test_plan(
                    &attempt,
                    &node,
                    &repair_output,
                    repair_raw_ref,
                    "test_plan_repair_failed",
                    repair_error.to_string(),
                ).await;
            }
        }
    }
};
```

为避免函数过长，把现有 parse error blocked 逻辑抽成：

```rust
async fn block_invalid_test_plan(
    &self,
    attempt: &CodingExecutionAttempt,
    node: &CodingTimelineNode,
    provider_output: &str,
    raw_ref: String,
    reason_code: &str,
    error: String,
) -> Result<TestingReport, CodingWorkspaceEngineError>
```

- [ ] **Step 5: 实现 execute repair/rerun**

在 `execute_test_plan` 完成并保存 `execute_raw_ref` 后，把当前 report 构建改为可 repair 的两段式。先用 clone 构建初始 report，避免 `step_results` / `unplanned_commands` 被 move：

```rust
let mut report_raw_ref = execute_raw_ref.clone();
let provider_claim = serde_json::from_str(&full_output).ok();
let mut report = build_plan_based_testing_report(
    &report_id,
    &attempt.id,
    &report_plan,
    step_results.clone(),
    unplanned_commands.clone(),
    provider_claim,
    Some(report_raw_ref.clone()),
);
report.unplanned_evidence = unplanned_evidence.clone();
```

当初始 report 缺 required step 且 `blocked_summary.is_none()` 时，运行一次 execute repair：

```rust
if !report.missing_required_steps.is_empty() && blocked_summary.is_none() {
    let repair_prompt =
        build_tester_execute_repair_prompt(&full_output, &report.missing_required_steps);
    let repair_adapter_input = AdapterInput {
        provider_type: provider_type_for_name(&tester_provider),
        role: AdapterRole::Reviewer,
        worktree_path: Some(worktree_path.to_string_lossy().to_string()),
        prompt: repair_prompt,
        context_files: Vec::new(),
        output_schema: "coding_workspace_test_execution_json".to_string(),
        timeout: options.timeout.as_secs().max(1),
        max_retries: 0,
    };
    let repair_input = StreamingProviderInput {
        provider_type: repair_adapter_input.provider_type.clone(),
        role: repair_adapter_input.role.clone(),
        prompt: repair_adapter_input.prompt.clone(),
        working_dir: worktree_path.clone(),
        workspace_session_id: Some(attempt.id.clone()),
        resume_provider_session_id: None,
        permission_mode: role_permission_mode_for_attempt(
            &self.store,
            &attempt,
            CodingProviderRole::Tester,
        )?,
        env_vars: BTreeMap::new(),
        timeout_secs: repair_adapter_input.timeout,
    };
    let repair_output = self
        .run_provider_stream_to_completion(CodingProviderStreamRun {
            attempt: &attempt,
            node_id: &node.id,
            provider,
            legacy_input: &repair_adapter_input,
            input: repair_input,
            provider_name: &tester_provider,
            provider_role: CodingProviderRole::Tester,
            command_rx,
            allow_legacy_stream_fallback: false,
        })
        .await?;
    let repair_raw_ref = self.store.save_provider_raw_output(
        &attempt.id,
        CodingExecutionStage::Testing,
        "execute_test_plan_repair",
        &repair_output,
    )?;
    report_raw_ref = repair_raw_ref.clone();
    for provider_step_result in parse_testing_step_results_from_provider_output(&repair_output) {
        if !step_results
            .iter()
            .any(|existing| existing.step_id == provider_step_result.step_id)
        {
            step_results.push(provider_step_result);
        }
    }
    let repair_provider_claim = serde_json::from_str(&repair_output).ok();
    report = build_plan_based_testing_report(
        &report_id,
        &attempt.id,
        &report_plan,
        step_results.clone(),
        unplanned_commands.clone(),
        repair_provider_claim,
        Some(report_raw_ref.clone()),
    );
    report.unplanned_evidence = unplanned_evidence.clone();
}
```

保留 blocked summary 覆盖：

```rust
if let Some(summary) = blocked_summary {
    report.overall_status = TestingOverallStatus::Blocked;
    report.context_warnings.push(summary);
}
```

后续保存 report、chat entry、`TestingReportUpdate` 和 blocked gate 的代码继续使用 `report`；blocked gate 的 `raw_provider_output_ref` 使用 `Some(report_raw_ref)`。

实现约束：

- 只 repair 一次。
- repair raw output 保存为 `execute_test_plan_repair_0001.txt`。
- repair 仍缺 required step 时 `overall_status=Blocked`。
- 不把 repair 失败送 Analyst。

- [ ] **Step 6: 运行 Task 4 测试确认 GREEN**

```bash
cargo test --locked --lib tester_plan_repair_prompt_includes_raw_output_and_schema_error
cargo test --locked --test it_product tester_repairs_markdown_plan_output_before_blocking
cargo test --locked --test it_web coding_ws_start_coding_drives_full_happy_path_to_final_confirm
```

Expected: 三个测试通过。

- [ ] **Step 7: Commit Task 4**

```bash
git add src/product/tester_agent_loop.rs src/product/coding_workspace_engine.rs src/web/test_controls.rs tests/it_product/product_tester_agent_loop.rs tests/it_web/web_coding_ws_handler.rs
git commit -m "feat: repair tester provider JSON outputs"
```

---

### Task 5: Testing 结果分流，不让 Blocked 自动进入 Analyst

**Files:**

- Modify: `src/web/coding_ws_handler.rs`
- Modify: `src/product/coding_workspace_engine.rs`
- Test: `tests/it_web/web_coding_ws_handler.rs`
- Test: `tests/it_product/product_coding_workspace_engine.rs`

- [ ] **Step 1: 写失败测试，证明 blocked testing 不进入 Analyst**

在 `tests/it_web/web_coding_ws_handler.rs` 新增测试：

```rust
#[tokio::test]
async fn coding_ws_testing_blocked_does_not_start_analyst_automatically() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_full_chain_attempt_and_provider(
        root.path(),
        Arc::new(TestingBlockedProvider),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let mut blocked_gate = None;
    for _ in 0..120 {
        match recv_json(&mut ws).await {
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.kind == CodingGateKind::StageGate =>
            {
                if let Some(stage) = gate.stage.clone() {
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.kind == CodingGateKind::Blocked
                    && gate.stage.as_ref() == Some(&CodingExecutionStage::Testing) =>
            {
                blocked_gate = Some(gate);
                break;
            }
            CodingWsOutMessage::CodingSessionState { pending_gates, .. } => {
                if let Some(gate) = pending_gates.into_iter().find(|gate| {
                    gate.kind == CodingGateKind::Blocked
                        && gate.stage.as_ref() == Some(&CodingExecutionStage::Testing)
                }) {
                    blocked_gate = Some(gate);
                    break;
                }
            }
            CodingWsOutMessage::CodingTimelineNodeCreated { node }
                if node.stage == CodingExecutionStage::Rework =>
            {
                panic!("rework started after testing blocked");
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }

    let gate = blocked_gate.expect("testing blocked gate");
    assert_eq!(gate.stage, Some(CodingExecutionStage::Testing));
    assert_eq!(gate.reason_code.as_deref(), Some("test_plan_repair_failed"));

    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    assert_eq!(attempt.status, CodingAttemptStatus::Blocked);
    assert_eq!(attempt.stage, CodingExecutionStage::Testing);

    let nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("nodes");
    assert!(!nodes.iter().any(|node| node.stage == CodingExecutionStage::Rework));

    ws.close(None).await.expect("close ws");
    server.abort();
}
```

把现有 `app_with_full_chain_attempt` 提取为可注入 provider 的 helper，并保留原函数作为默认 happy path 包装：

```rust
fn app_with_full_chain_attempt(root_path: &Path) -> axum::Router {
    app_with_full_chain_attempt_and_provider(root_path, Arc::new(FullChainStreamingProvider))
}

fn app_with_full_chain_attempt_and_provider(
    root_path: &Path,
    provider: Arc<dyn StreamingProviderAdapter>,
) -> axum::Router {
    let repo = root_path.join("repo");
    let remote = root_path.join("remote.git");
    init_cargo_repo(&repo);
    run_git(root_path, &["init", "--bare", remote.to_str().unwrap()]);
    run_git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );

    let app_paths = ProductAppPaths::new(root_path.join(".aria"));
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: "project_0001".to_string(),
            name: "repo".to_string(),
            path: repo,
            default_policy_preset: Some("manual-write".to_string()),
            default_provider_mode: Some("fake".to_string()),
        })
        .expect("create repository");
    let lifecycle = LifecycleStore::new(app_paths.clone());
    lifecycle
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: repository.id,
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "实现爬楼梯".to_string(),
        })
        .expect("create work item");
    lifecycle
        .update_work_item_plan_status(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            WorkItemPlanStatus::Confirmed,
        )
        .expect("confirm work item");
    CodingAttemptStore::new(app_paths)
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create attempt");

    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::Fake, provider);
    build_web_router(WebAppState::with_provider_registry(
        root_path.to_path_buf(),
        WebRuntime::new_fake(root_path.to_path_buf()),
        registry,
    ))
}
```

在 `tests/it_web/web_coding_ws_handler.rs` 的 provider fixtures 区域新增：

```rust
struct TestingBlockedProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for TestingBlockedProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let output = if input.prompt.contains("Phase: plan_tests_repair") {
            "still not json".to_string()
        } else if input.prompt.contains("Phase: plan_tests") {
            "not json at all".to_string()
        } else {
            return Err(ProviderAdapterError::execution_failed(
                None,
                String::new(),
                "unexpected testing prompt",
                0,
            ));
        };

        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            if cancel.is_cancelled() {
                return;
            }
            if event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await
                .is_err()
            {
                return;
            }
            if cancel.is_cancelled() {
                return;
            }
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: output,
                    provider_session_id: None,
                })
                .await;
        });

        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        let (tx, rx) = mpsc::channel(8);
        match input.role {
            AdapterRole::Executor => {
                let worktree = input
                    .worktree_path
                    .as_ref()
                    .map(PathBuf::from)
                    .expect("worktree path");
                fs::write(worktree.join("src/lib.rs"), CLIMB_STAIRS_LIB).map_err(|error| {
                    ProviderAdapterError::incompatible_output(error.to_string(), "", "")
                })?;
                tx.try_send(StreamChunk::Done {
                    full_output: "implemented climb_stairs".to_string(),
                })
                .expect("send coding done");
            }
            AdapterRole::Reviewer
                if input.output_schema == "coding_workspace_analyst_verdict_json" =>
            {
                tx.try_send(StreamChunk::Done {
                    full_output: r#"{"verdict":"no_issue","summary":"testing ok"}"#.to_string(),
                })
                .expect("send analyst done");
            }
            AdapterRole::Reviewer => {
                tx.try_send(StreamChunk::Done {
                    full_output: r#"{"verdict":"approve","summary":"review ok","findings":[]}"#
                        .to_string(),
                })
                .expect("send review done");
            }
            _ => {
                tx.try_send(StreamChunk::Done {
                    full_output: "ok".to_string(),
                })
                .expect("send done");
            }
        }
        Ok(rx)
    }
}
```

- [ ] **Step 2: 写 product helper 测试**

在 `tests/it_product/product_coding_workspace_engine.rs` 的 `cadence_aria::product::coding_models` import 中加入 `TestCommandStatus`、`TestingReport`、`TestingStepResult`。

在 `tests/it_product/product_coding_workspace_engine.rs` 新增：

```rust
#[test]
fn testing_report_requires_evidence_before_analyst_rework() {
    let blocked = TestingReport {
        id: "testing_report_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        commands: Vec::new(),
        overall_status: TestingOverallStatus::Blocked,
        provider_claim: None,
        backend_verified: true,
        started_at: "2026-06-11T00:00:00Z".to_string(),
        completed_at: Some("2026-06-11T00:00:01Z".to_string()),
        plan_id: None,
        plan_summary: None,
        steps: Vec::new(),
        unplanned_commands: Vec::new(),
        unplanned_evidence: Vec::new(),
        missing_required_steps: Vec::new(),
        skipped_required_steps: Vec::new(),
        context_warnings: vec!["test_plan_parse_error".to_string()],
        raw_provider_output_ref: Some("provider-raw/testing/plan_tests_0001.txt".to_string()),
    };
    assert!(!testing_report_should_enter_analyst(&blocked));

    let mut failed_with_evidence = blocked.clone();
    failed_with_evidence.overall_status = TestingOverallStatus::Failed;
    failed_with_evidence.plan_id = Some("test_plan_0001".to_string());
    failed_with_evidence.steps = vec![TestingStepResult {
        step_id: "unit".to_string(),
        status: TestCommandStatus::Failed,
        evidence_refs: vec!["unit.stderr.log".to_string()],
        command: Some(vec!["cargo".to_string(), "test".to_string(), "--locked".to_string()]),
        provider_analysis: Some("unit failed".to_string()),
    }];
    assert!(testing_report_should_enter_analyst(&failed_with_evidence));
}
```

- [ ] **Step 3: 实现 routing helper**

在 `src/product/coding_workspace_engine.rs` 中新增 public helper，放在 `impl CodingWorkspaceEngine` 代码块之外：

```rust
pub fn testing_report_has_execution_evidence(report: &TestingReport) -> bool {
    (!report.steps.is_empty() && report.plan_id.is_some())
        || !report.commands.is_empty()
        || report
            .steps
            .iter()
            .any(|step| !step.evidence_refs.is_empty() || step.command.is_some())
        || report
            .unplanned_commands
            .iter()
            .any(|command| !command.stdout_ref.is_empty() || !command.stderr_ref.is_empty())
}

pub fn testing_report_should_enter_analyst(report: &TestingReport) -> bool {
    match report.overall_status {
        TestingOverallStatus::Failed => testing_report_has_execution_evidence(report),
        TestingOverallStatus::Blocked | TestingOverallStatus::SkippedByUserDecision => false,
        TestingOverallStatus::Passed | TestingOverallStatus::PassedWithWarnings => false,
    }
}
```

- [ ] **Step 4: 修改 WebSocket 主流程分流**

在 `src/web/coding_ws_handler.rs` 测试完成后、进入 Rework stage gate 前加入：

```rust
if !testing_report_should_enter_analyst(&testing_report) {
    return emit_current_session_state(event_tx, coding_store, &current).await;
}
```

导入 helper：

```rust
use crate::product::coding_workspace_engine::testing_report_should_enter_analyst;
```

保留 blocked gate 的 `send_raw_output_to_analyst` 人工入口；`handle_blocked_gate_response` 中该 action 仍可 `resume_blocked_attempt_at_stage(..., Rework)`。

- [ ] **Step 5: 运行 Task 5 测试确认 GREEN**

```bash
cargo test --locked --test it_product testing_report_requires_evidence_before_analyst_rework
cargo test --locked --test it_web coding_ws_testing_blocked_does_not_start_analyst_automatically
```

Expected: 两个测试通过。

- [ ] **Step 6: Commit Task 5**

```bash
git add src/product/coding_workspace_engine.rs src/web/coding_ws_handler.rs tests/it_product/product_coding_workspace_engine.rs tests/it_web/web_coding_ws_handler.rs
git commit -m "fix: keep blocked testing at testing gate"
```

---

### Task 6: Gate 文案、最终回归与 E2E 指南更新

**Files:**

- Modify: `web/src/pages/CodingWorkspacePage.test.tsx`
- Modify: `web/src/pages/CodingWorkspacePage.tsx`
- Modify: `cadence/plans/2026-06-10_计划文档_修复方案_CodingWorkspace真实ProviderTester两段式恢复_v1.0.md`
- Create: `cadence/reports/2026-06-11_进度报告_CodingWorkspace角色授权与Tester门禁分流优化验证_v1.0.md`

- [ ] **Step 1: 写前端文案测试**

在 `web/src/pages/CodingWorkspacePage.test.tsx` 新增：

```ts
it("renders tester contract blocked gate as blocked instead of failed test", async () => {
  mockCodingWs();
  useCodingWorkspaceStore.setState({
    attemptId: "coding_attempt_0001",
    status: "blocked",
    stage: "testing",
    pendingGates: [
      {
        gate_id: "gate_0001",
        kind: "blocked",
        title: "Testing blocked",
        description: "TestPlan parse failed",
        stage: "testing",
        role: "tester",
        reason_code: "test_plan_missing_json",
        evidence_refs: ["testing_report_0001.json"],
        raw_provider_output_ref: "provider-raw/testing/plan_tests_0001.txt",
        available_actions: [
          {
            action_id: "retry_test_plan",
            label: "重试测试计划",
            action_type: "retry_test_plan",
          },
        ],
      },
    ],
  });

  render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

  const gate = screen.getByTestId("coding-pending-gate");
  expect(gate).toHaveTextContent("Tester 未返回测试计划 JSON");
  expect(gate).toHaveTextContent("测试被阻塞");
  expect(gate).not.toHaveTextContent("测试失败");
});
```

- [ ] **Step 2: 实现 reason_code 文案映射**

在 `web/src/pages/CodingWorkspacePage.tsx` 的 pending gate 渲染区域加入：

```ts
const TESTING_BLOCKED_REASON_LABELS: Record<string, string> = {
  test_plan_missing_json: "Tester 未返回测试计划 JSON",
  test_plan_invalid_json: "Tester 返回的 JSON 无法解析",
  test_plan_schema_invalid: "Tester 测试计划字段不完整",
  test_plan_repair_failed: "Tester 测试计划修复失败",
  missing_required_steps: "缺少 required 测试步骤证据",
  high_risk_test_step_requires_permission: "高风险测试步骤需要人工确认",
};

function blockedGateDisplayTitle(gate: CodingGateRequired) {
  if (gate.stage === "testing" && gate.reason_code) {
    return TESTING_BLOCKED_REASON_LABELS[gate.reason_code] ?? gate.reason_code;
  }
  return gate.title;
}
```

渲染时对 testing blocked gate 显示：

```tsx
{gate.stage === "testing" ? <span>测试被阻塞</span> : null}
```

- [ ] **Step 3: 更新计划文档 checkbox**

在 `cadence/plans/2026-06-10_计划文档_修复方案_CodingWorkspace真实ProviderTester两段式恢复_v1.0.md` 中新增本轮后续任务引用，不覆盖已有 checkbox 记录。追加：

```markdown
## 2026-06-11 后续优化计划

- 角色级权限配置已转入 `cadence/plans/2026-06-11_计划文档_实施计划_CodingWorkspace角色授权与Tester门禁分流优化_v1.0.md` 执行。
- Tester JSON repair 与 Testing blocked 分流纳入同一计划。
```

- [ ] **Step 4: 创建验证报告草稿**

新增 `cadence/reports/2026-06-11_进度报告_CodingWorkspace角色授权与Tester门禁分流优化验证_v1.0.md`：

```markdown
# CodingWorkspace 角色授权与 Tester 门禁分流优化验证报告

## 基本信息

- 验证日期：2026-06-11
- 分支：`bugfix_test_branch`
- 设计文档：`cadence/designs/2026-06-11_技术方案_CodingWorkspace角色授权与Tester门禁分流优化_v1.0.md`
- 实施计划：`cadence/plans/2026-06-11_计划文档_实施计划_CodingWorkspace角色授权与Tester门禁分流优化_v1.0.md`

## 自动化验证

- `cargo fmt --check`：
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：
- `cargo check --locked`：
- `cargo test --locked`：
- `pnpm -C web test`：
- `pnpm -C web build`：

## E2E 验收

- Tester 默认 auto：
- Coder 默认 supervised：
- Markdown plan repair：
- repair 失败停在 Testing gate：
- Testing blocked 不进 Analyst：
- failed with evidence 进入 Analyst：

## 风险与遗留

- 无已知遗留；若验证失败，在对应命令行记录失败原因。
```

- [ ] **Step 5: 运行聚焦与全量验证**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
pnpm -C web test
pnpm -C web build
git diff --check
```

Expected:

- Rust fmt/clippy/check/test 全部通过。
- 前端 test/build 通过；如仅有 Vite chunk size warning，记录但不视为失败。
- `git diff --check` 无输出。

- [ ] **Step 6: Controlled E2E 验收**

启动服务：

```bash
cargo watch -w src -w Cargo.toml -w Cargo.lock -x "run --locked -- web --workspace . --host 127.0.0.1 --port 4317"
pnpm dev --port 5173
```

健康检查：

```bash
curl --noproxy '*' -sS http://127.0.0.1:4317/api/health
curl --noproxy '*' -sS -I http://127.0.0.1:5173/
curl --noproxy '*' -sS http://127.0.0.1:5173/api/health
```

Expected:

- 后端返回 `{"status":"ok"}`。
- 前端 `/` 返回 `200 OK`。
- 前端 `/api/health` 返回 `{"status":"ok"}`。

页面验收：

- 创建新的 coding attempt。
- 确认 Tester permission mode 默认 `auto`。
- 确认 Coder permission mode 默认 `supervised`。
- 使用 test controls 注入 Markdown plan，再确认 repair 成功或失败 gate 行为符合预期。
- Testing blocked 时 timeline 不新增 Analyst/Rework 节点。

- [ ] **Step 7: 填写验证报告**

把 Step 6 和 Step 7 的实际结果写入验证报告，不写“通过”除非命令输出已确认。

- [ ] **Step 8: Commit Task 6**

```bash
git add web/src/pages/CodingWorkspacePage.tsx web/src/pages/CodingWorkspacePage.test.tsx cadence/plans/2026-06-10_计划文档_修复方案_CodingWorkspace真实ProviderTester两段式恢复_v1.0.md cadence/reports/2026-06-11_进度报告_CodingWorkspace角色授权与Tester门禁分流优化验证_v1.0.md
git commit -m "test: verify coding tester gating workflow"
```

---

## 最终交付检查

- [ ] `git log --oneline -6` 显示每个 task 一个清晰提交。
- [ ] `git status --short` 只剩用户明确保留的无关改动。
- [ ] 设计文档、实施计划、验证报告都在 `cadence/` 下。
- [ ] 没有使用 `cargo test -j 1`。
- [ ] Testing blocked 不会自动进入 Analyst。
- [ ] Tester 默认 auto，Coder/Reviewer 默认 supervised。
- [ ] 用户可在页面按 role 调整 permission mode。
