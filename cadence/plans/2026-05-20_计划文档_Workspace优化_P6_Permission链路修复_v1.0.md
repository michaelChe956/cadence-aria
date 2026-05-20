# P6: Permission 链路修复 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 Permission 链路加全链路 trace log，让 unmatched id 显式报错（protocol_error），PendingPermissions 加 15min 超时清理；timeout 作为独立审计状态并中止当前 run，不伪装成用户拒绝；前端权限 tab 展示 permission_events 列表。

**Architecture:** 在 workspace_ws_handler → engine → approval_bridge 三个点各加 trace log；bridge unmatched id 时通过 event_tx 发 ProtocolError；新增后台任务扫 PendingPermissions 超时报 PermissionTimeout，engine 收到后把当前 run 标记为 permission_timeout 并中止；前端 NodeDetailPanel 权限 tab 渲染 events。

**Tech Stack:** Rust (tokio + tracing + serde_json) + TypeScript (React)

**前置依赖:** 弱依赖 P1（NodeDetail.permission_events 字段定义）

**后续 plan 消费点:**
- P7 E2E 消费 Permission 链路用例（G1-G5）

**文件结构总览:**

| 文件 | 操作 | 职责 |
|---|---|---|
| `src/web/workspace_ws_handler.rs` | 修改 | PermissionResponse 入口 trace log |
| `src/product/workspace_engine.rs` | 修改 | PermissionResponse 转发 trace log |
| `src/cross_cutting/approval_bridge.rs` | 修改 | trace log + unmatched protocol_error + 超时清理 |
| `web/src/components/workspace/NodeDetailPanel.tsx` | 修改 | 权限 tab 展示 permission_events |
| `web/src/hooks/useWorkspaceWs.ts` | 修改 | sendPermissionResponse 加 console.info |

---

### Task 1: 全链路 trace log

**Files:**
- 修改: `src/web/workspace_ws_handler.rs`
- 修改: `src/product/workspace_engine.rs`
- 修改: `src/cross_cutting/approval_bridge.rs`

- [ ] **Step 1: workspace_ws_handler.rs 入口 trace**

在 `WsInMessage::PermissionResponse` 处理分支（约 line 341）添加：

```rust
WsInMessage::PermissionResponse { id, approved, reason } => {
    tracing::info!(permission_id = %id, approved, "ws inbound permission response");
    let command_tx = { current_run.lock().await.as_ref().map(|run| run.command_tx.clone()) };
    if let Some(command_tx) = command_tx {
        let _ = command_tx.send(ProviderCommand::PermissionResponse { id, approved, reason }).await;
    }
}
```

- [ ] **Step 2: workspace_engine.rs 转发 trace**

在 `Some(command) =>` 分支（约 line 417）添加：

```rust
Some(command) => {
    if let ProviderCommand::PermissionResponse { id, .. } = &command {
        tracing::info!(permission_id = %id, "engine forwarding permission response");
    }
    if session.commands.send(command).await.is_err() {
        commands_open = false;
    }
}
```

- [ ] **Step 3: approval_bridge.rs trace + unmatched warning**

```rust
async fn listen_for_permission_commands(
    mut command_rx: mpsc::Receiver<ProviderCommand>,
    pending: PendingPermissions,
) {
    while let Some(command) = command_rx.recv().await {
        match command {
            ProviderCommand::PermissionResponse { id, approved, reason } => {
                tracing::info!(permission_id = %id, approved, "bridge received permission response");
                if let Some(decision_tx) = pending.lock().await.remove(&id) {
                    tracing::info!(permission_id = %id, "bridge dispatched decision to pending");
                    let _ = decision_tx.send(PermissionDecision { approved, reason });
                } else {
                    tracing::warn!(permission_id = %id, "bridge: no pending entry for id");
                }
            }
            ProviderCommand::Abort => {
                // ... 现有逻辑 ...
            }
        }
    }
}
```

- [ ] **Step 4: 跑测试确认编译通过**

在 `workspace_engine.rs` / `workspace_ws_handler.rs` 的事件转发处增加 `PermissionTimeout` 处理：

```rust
EngineEvent::PermissionTimeout { permission_id } => {
    tracing::warn!(permission_id = %permission_id, "permission timed out; aborting active run");
    self.mark_permission_timeout(&permission_id).await;
    self.finish_failed_run().await;
}
```

WebSocket 出站映射为：

```rust
WsOutMessage::ProtocolError {
    code: "PERMISSION_TIMEOUT".to_string(),
    message: format!("Permission request {permission_id} timed out"),
    context: Some(serde_json::json!({ "permission_id": permission_id })),
}
```

Run: `cargo check --locked`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/web/workspace_ws_handler.rs src/product/workspace_engine.rs src/cross_cutting/approval_bridge.rs
git commit -m "feat(permission): add full-trace logging across ws-handler, engine, bridge"
```

---

### Task 2: unmatched id 发 protocol_error

**Files:**
- 修改: `src/cross_cutting/approval_bridge.rs`
- 修改: `src/product/workspace_engine.rs`

- [ ] **Step 1: bridge 发 ProtocolError 事件**

修改 `listen_for_permission_commands`：

```rust
            ProviderCommand::PermissionResponse { id, approved, reason } => {
                tracing::info!(permission_id = %id, approved, "bridge received permission response");
                let mut pending_guard = pending.lock().await;
                if let Some(decision_tx) = pending_guard.remove(&id) {
                    tracing::info!(permission_id = %id, "bridge dispatched decision to pending");
                    let _ = decision_tx.send(PermissionDecision { approved, reason });
                } else {
                    tracing::warn!(permission_id = %id, "bridge: no pending entry for id");
                    // 通过 event_tx 发 ProtocolError
                    // 注意：需要 event_tx 在此作用域可用
                }
            }
```

需要确认 `listen_for_permission_commands` 是否能访问 `event_tx`。如果不能，修改函数签名接收 `event_tx: mpsc::Sender<EngineEvent>`：

```rust
async fn listen_for_permission_commands(
    mut command_rx: mpsc::Receiver<ProviderCommand>,
    pending: PendingPermissions,
    event_tx: mpsc::Sender<EngineEvent>,
) {
    // ...
    } else {
        tracing::warn!(permission_id = %id, "bridge: no pending entry for id");
        let _ = event_tx.send(EngineEvent::ProtocolError {
            code: "PERMISSION_ID_UNMATCHED".to_string(),
            message: format!("PermissionResponse id={} not found in pending", id),
            context: Some(serde_json::json!({"permission_id": id})),
        }).await;
    }
}
```

需要确认 `EngineEvent` 是否有 `ProtocolError` 变体。如果没有，在 `src/product/workspace_engine.rs` 或相关模块中添加：

```rust
    ProtocolError {
        code: String,
        message: String,
        context: Option<serde_json::Value>,
    },
```

并在 `build_session_state` 和 WebSocket handler 中处理该事件（映射到 `WsOutMessage::ProtocolError`）。

- [ ] **Step 2: 跑测试确认编译通过**

Run: `cargo check --locked`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git commit -am "feat(permission): send protocol_error on unmatched permission id"
```

---

### Task 3: PendingPermissions 超时清理

**Files:**
- 修改: `src/cross_cutting/approval_bridge.rs`

- [ ] **Step 1: 修改 PendingPermissions 存储时间戳**

```rust
type PendingPermissions = Arc<Mutex<HashMap<String, (oneshot::Sender<PermissionDecision>, Instant)>>>;
```

- [ ] **Step 2: insert 时记录时间**

```rust
    pub async fn request_permission(
        &self,
        request: PermissionRequestData,
    ) -> Result<PermissionDecision, ProviderAdapterError> {
        let id = format!("permission_{}", self.next_permission_id.fetch_add(1, Ordering::SeqCst));
        let (decision_tx, decision_rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), (decision_tx, Instant::now()));
        // ...
    }
```

- [ ] **Step 3: 新增超时清理后台任务**

```rust
    pub fn new(command_tx: mpsc::Sender<ProviderCommand>, pending: PendingPermissions, event_tx: mpsc::Sender<EngineEvent>) -> Self {
        let instance = Self {
            command_tx,
            pending: pending.clone(),
        };
        tokio::spawn(cleanup_pending_permissions(pending, event_tx));
        instance
    }
```

```rust
async fn cleanup_pending_permissions(
    pending: PendingPermissions,
    event_tx: mpsc::Sender<EngineEvent>,
) {
    const CLEANUP_INTERVAL: Duration = Duration::from_secs(60);
    const TIMEOUT: Duration = Duration::from_secs(900); // 15min

    loop {
        tokio::time::sleep(CLEANUP_INTERVAL).await;
        let now = Instant::now();
        let mut guard = pending.lock().await;
        let expired: Vec<String> = guard
            .iter()
            .filter(|(_, (_, ts))| now.duration_since(*ts) > TIMEOUT)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            if let Some((decision_tx, _)) = guard.remove(&id) {
	                let _ = event_tx.send(EngineEvent::PermissionTimeout {
	                    permission_id: id,
	                }).await;
	            }
	        }
    }
}
```

注意：timeout 不等同用户点击"拒绝"。本任务不得向 provider 发送 `PermissionDecision { approved: false, reason: "timeout" }`。正确行为是：

1. 从 pending 表移除该 permission。
2. 发送 `EngineEvent::PermissionTimeout { permission_id }`；engine 根据当前 active run / active node 绑定 node_id。
3. engine 将当前 run 标记为失败/中止，失败原因写 `permission_timeout`。
4. `NodeDetail.permission_events` 中对应事件写入 `response: {"status":"timeout"}`。
5. 用户之后若再响应同一个 id，走 `PERMISSION_ID_UNMATCHED`。

- [ ] **Step 4: 跑测试确认编译通过**

Run: `cargo check --locked`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/cross_cutting/approval_bridge.rs
git commit -m "feat(permission): add 15min timeout cleanup for PendingPermissions"
```

---

### Task 4: 前端权限 tab 展示 permission_events

**Files:**
- 修改: `web/src/components/workspace/NodeDetailPanel.tsx`
- 测试: `web/src/components/workspace/NodeDetailPanel.test.tsx`

- [ ] **Step 1: 修改 NodeDetailPanel 权限 tab**

```tsx
        {activeTab === "permission" && (
          <div className="space-y-2">
            {detail?.permission_events.length === 0 && (
              <div className="text-sm text-[var(--aria-ink-muted)]">无权限事件</div>
            )}
            {detail?.permission_events.map((pe) => {
              let statusLabel = "待应答";
              let statusColor = "bg-amber-50";
              if (pe.response) {
                if (pe.response.approved) {
                  statusLabel = "已批准";
                  statusColor = "bg-green-50";
                } else {
                  statusLabel = "已拒绝";
                  statusColor = "bg-red-50";
                }
              } else if (pe.response === null && detail.ended_at) {
                statusLabel = "超时";
                statusColor = "bg-slate-50";
              }
              return (
                <div
                  key={pe.request_id}
                  className={`rounded px-2 py-1.5 text-xs ${statusColor}`}
                >
                  <div className="flex justify-between">
                    <span className="font-medium">{pe.request_id}</span>
                    <span className="text-[var(--aria-ink-muted)]">{statusLabel}</span>
                  </div>
                  <div className="text-[var(--aria-ink-muted)]">
                    {JSON.stringify(pe.request).slice(0, 100)}
                  </div>
                </div>
              );
            })}
          </div>
        )}
```

- [ ] **Step 2: 跑测试确认通过**

Run: `pnpm --filter web test -- NodeDetailPanel`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add web/src/components/workspace/NodeDetailPanel.tsx
git commit -m "feat(ui): render permission events in NodeDetailPanel permission tab"
```

---

### Task 5: 前端 sendPermissionResponse 加 console.info

**Files:**
- 修改: `web/src/hooks/useWorkspaceWs.ts`

- [ ] **Step 1: 修改 sendPermissionResponse**

```typescript
  const sendPermissionResponse = useCallback(
    (id: string, approved: boolean, reason?: string) => {
      console.info("[permission] sending response", { id, approved });
      const ws = wsRef.current;
      if (ws?.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: "permission_response", id, approved, reason }));
      }
    },
    [],
  );
```

- [ ] **Step 2: Commit**

```bash
git add web/src/hooks/useWorkspaceWs.ts
git commit -m "feat(ui): log permission response id on send"
```

---

### Task 6: 全量回归测试

- [ ] **Step 1: 跑后端测试**

Run: `cargo test --locked -j 1`
Expected: PASS

- [ ] **Step 2: 跑前端测试**

Run: `pnpm --filter web test`
Expected: PASS

- [ ] **Step 3: Commit（如有修复）**

```bash
git commit -am "fix: permission link tests and types"
```

---

## 自审检查

**1. Spec coverage:**

| 设计 § | 实现位置 |
|---|---|
| §9.1 已确认的代码路径 | Task 1 (trace log) |
| §9.2 排查清单 | Task 1 (trace log 覆盖 1-6) |
| §9.3.1 全链路 trace log | Task 1 |
| §9.3.2 unmatched id protocol_error | Task 2 |
| §9.3.3 permission_events 持久化 | P1 (NodeDetail) + Task 4 (前端展示) |
| §9.3.4 PendingPermissions 超时 | Task 3 |
| §9.4 前端配套 | Task 4 + Task 5 |

**2. Placeholder scan:**
- 无 TBD/TODO

**3. Type consistency:**
- `PermissionEvent` 结构在 Rust (NodeDetail) 和 TS (api/types.ts) 中对齐
- `EngineEvent::ProtocolError` / `EngineEvent::PermissionTimeout` 需要与现有事件变体一致

---

## 本 plan 验收清单

- [ ] 全链路 trace log 4 个点（ws-handler / engine / bridge receive / bridge dispatch）都有 permission_id
- [ ] unmatched id 时后端发 protocol_error，前端展示
- [ ] 15min 超时后 pending 清理，发送 `PermissionTimeout`，不向 provider 伪造 deny
- [ ] timeout 后当前 run 以 `permission_timeout` 原因中止，Timeline / NodeDetail 写 timeout 审计事件
- [ ] 权限 tab 展示 pending / approved / denied / timeout 状态
- [ ] 前端发送 permission_response 时 console.info 记录 id
- [ ] `cargo test --locked -j 1` PASS
- [ ] `pnpm --filter web test` PASS
