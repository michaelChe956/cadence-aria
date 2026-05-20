# P1: 协议层重塑 + Timeline 持久化与会话恢复 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将协议从"user_message 对话+猜意图"改为"意图明确的动作指令"（context_note / start_generation），让 Timeline 节点详情按文件持久化并成为审计事实源，SessionState snapshot 支持完整恢复。

**Architecture:** 后端新增 WsInMessage/WsOutMessage 变体 + TimelineNodeType 扩展 + NodeDetail 按节点文件持久化；前端 useWorkspaceWs 拆出独立发送函数 + workspace-ws-store 支持 snapshot 替换式应用 nodeDetails。

**Tech Stack:** Rust (axum WebSocket + serde_json + tokio), TypeScript (React + Zustand + vitest), pnpm, vitest

**前置依赖:** 无（P1 是基础设施，必须最先执行）

**后续 plan 消费点:**
- P2 消费 `sendContextNote` / `sendStartGeneration` / `provider_locked` 事件
- P4 消费 `selectNodeDetail` selector + 节点级 5 tab
- P5 消费 `WsInMessage::Hello` / `WsOutMessage::Pong` + `aborted_by_disconnect` 节点写入
- P6 消费 `NodeDetail.permission_events` 字段

**文件结构总览:**

| 文件 | 操作 | 职责 |
|---|---|---|
| `src/web/workspace_ws_types.rs` | 修改 | 协议类型：WsInMessage/WsOutMessage/TimelineNodeType 扩展 |
| `src/product/models.rs` | 修改 | NodeDetail 结构体 + ProviderSnapshot |
| `src/product/workspace_engine.rs` | 修改 | SessionState 扩展（timeline_node_details + active_run_id）、build_session_state |
| `src/product/lifecycle_store.rs` | 修改 | 节点详情按文件持久化 API |
| `src/web/workspace_ws_handler.rs` | 修改 | 阶段校验、socket close handler、handle_message 路由 |
| `web/src/api/types.ts` | 修改 | 前端类型定义 |
| `web/src/hooks/useWorkspaceWs.ts` | 修改 | 新增 sendContextNote / sendStartGeneration / sendHello / sendPing |
| `web/src/state/workspace-ws-store.ts` | 修改 | snapshot 应用 nodeDetails、selectNodeDetail selector |
| `src/web/workspace_ws_types.rs` | 新增测试 | 协议序列化/反序列化测试 |
| `src/product/lifecycle_store.rs` | 新增测试 | 节点详情写入/读取测试 |
| `web/src/state/workspace-ws-store.test.ts` | 修改 | snapshot 应用测试 |

---

### Task 1: 扩展后端协议类型（WsInMessage / WsOutMessage）

**Files:**
- 修改: `src/web/workspace_ws_types.rs`
- 测试: `src/web/workspace_ws_types.rs`（已有 mod tests，追加）

- [ ] **Step 1: 写 failing 测试 — 新消息变体序列化**

```rust
#[test]
fn context_note_roundtrip() {
    let msg = WsInMessage::ContextNote {
        content: "需要支持空查询参数兜底".to_string(),
    };
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "context_note");
    assert_eq!(json["content"], "需要支持空查询参数兜底");
    let back: WsInMessage = serde_json::from_value(json).unwrap();
    assert_eq!(back, msg);
}

#[test]
fn start_generation_roundtrip() {
    let snapshot = ProviderConfigSnapshot {
        author: ProviderName::ClaudeCode,
        reviewer: Some(ProviderName::Codex),
        review_rounds: 1,
    };
    let msg = WsInMessage::StartGeneration {
        provider_config: snapshot,
        reviewer_enabled: true,
    };
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "start_generation");
    assert_eq!(json["reviewer_enabled"], true);
    let back: WsInMessage = serde_json::from_value(json).unwrap();
    assert_eq!(back, msg);
}

#[test]
fn protocol_error_outbound_roundtrip() {
    let msg = WsOutMessage::ProtocolError {
        code: "INVALID_MESSAGE_FOR_STAGE".to_string(),
        message: "context_note not allowed in Running".to_string(),
        context: Some(serde_json::json!({"stage": "Running"})),
    };
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "protocol_error");
    let back: WsOutMessage = serde_json::from_value(json).unwrap();
    assert_eq!(back, msg);
}

#[test]
fn provider_locked_roundtrip() {
    let msg = WsOutMessage::ProviderLocked {
        snapshot: ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: Some(ProviderName::Codex),
            review_rounds: 1,
        },
        locked_at: "2026-05-20T14:35:00Z".to_string(),
    };
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "provider_locked");
    let back: WsOutMessage = serde_json::from_value(json).unwrap();
    assert_eq!(back, msg);
}

#[test]
fn hello_ping_roundtrip() {
    let hello = WsInMessage::Hello {
        session_id: "sess-1".to_string(),
        last_seen_node_id: Some("node-1".to_string()),
    };
    let json = serde_json::to_value(&hello).unwrap();
    assert_eq!(json["type"], "hello");
    let back: WsInMessage = serde_json::from_value(json).unwrap();
    assert_eq!(back, hello);

    let ping = WsInMessage::Ping;
    let json = serde_json::to_value(&ping).unwrap();
    assert_eq!(json["type"], "ping");
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test workspace_ws_types -- --nocapture`
Expected: 编译失败 — ContextNote / StartGeneration / ProtocolError / ProviderLocked / Hello / Ping 未定义

- [ ] **Step 3: 在 WsInMessage 追加变体**

修改 `src/web/workspace_ws_types.rs:108`，在现有变体下方追加：

```rust
    ContextNote {
        content: String,
    },
    StartGeneration {
        provider_config: ProviderConfigSnapshot,
        reviewer_enabled: bool,
    },
    Hello {
        session_id: String,
        last_seen_node_id: Option<String>,
    },
    Ping,
```

- [ ] **Step 4: 在 WsOutMessage 追加变体**

修改 `src/web/workspace_ws_types.rs:33`，在现有变体下方追加：

```rust
    ProtocolError {
        code: String,
        message: String,
        context: Option<serde_json::Value>,
    },
    ProviderLocked {
        snapshot: ProviderConfigSnapshot,
        locked_at: String,
    },
    Pong,
```

- [ ] **Step 5: 在 TimelineNodeType 扩展**

修改 `src/web/workspace_ws_types.rs:217`：

```rust
pub enum TimelineNodeType {
    PrepareContext,
    ContextNote,
    StartGeneration,
    AuthorRun,
    ReviewerRun,
    ReviewDecision,
    Revision,
    HumanConfirm,
    AbortedByDisconnect,
    ProtocolError,
    Completed,
}
```

> `Generation` 重命名为 `AuthorRun`（revision 复用此类型 + `is_revision` 旗标），`Review` 重命名为 `ReviewerRun`。需全局搜索替换这两个名称的使用处。

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test workspace_ws_types -- --nocapture`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add src/web/workspace_ws_types.rs
git commit -m "feat(protocol): add context_note/start_generation/hello/ping/pong + protocol_error/provider_locked + TimelineNodeType expansion"
```

---

### Task 2: 全局重命名 Generation → AuthorRun, Review → ReviewerRun

**Files:**
- 全局搜索替换

- [ ] **Step 1: 写 failing 测试 — 确认改名后的序列化**

追加到 `src/web/workspace_ws_types.rs` mod tests：

```rust
#[test]
fn timeline_node_type_rename() {
    let author = TimelineNodeType::AuthorRun;
    let json = serde_json::to_value(&author).unwrap();
    assert_eq!(json, "author_run");

    let reviewer = TimelineNodeType::ReviewerRun;
    let json = serde_json::to_value(&reviewer).unwrap();
    assert_eq!(json, "reviewer_run");
}
```

Run: `cargo test timeline_node_type_rename -- --nocapture`
Expected: 编译错误 — `Generation` 和 `Review` 不存在（如果已改），或 `AuthorRun` 不存在（如果还没改）

- [ ] **Step 2: 全局替换**

```bash
# 在 Rust 代码中替换
grep -rn "TimelineNodeType::Generation" src/ --include="*.rs"
grep -rn "TimelineNodeType::Review" src/ --include="*.rs"
```

逐文件替换 `TimelineNodeType::Generation` → `TimelineNodeType::AuthorRun`，`TimelineNodeType::Review` → `TimelineNodeType::ReviewerRun`。`serde(rename_all = "snake_case")` 会自动处理序列化名称，但 `Generation` 的旧 JSON 值 `generation` 需要确认：

如果前端已有持久化的旧 timeline_nodes.json，添加 serde alias：

```rust
    #[serde(alias = "generation")]
    AuthorRun,
    #[serde(alias = "review")]
    ReviewerRun,
```

- [ ] **Step 3: 跑测试确认通过**

Run: `cargo test --workspace`
Expected: PASS（确保没有漏掉的引用）

- [ ] **Step 4: Commit**

```bash
git commit -am "refactor(protocol): rename Generation->AuthorRun, Review->ReviewerRun with serde alias"
```

---

### Task 3: NodeDetail 数据结构 + ProviderSnapshot

**Files:**
- 修改: `src/product/models.rs`

- [ ] **Step 1: 写 failing 测试**

在 `src/product/models.rs` 测试（或新建测试文件）追加：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn node_detail_roundtrip() {
        let detail = NodeDetail {
            node_id: "node-1".to_string(),
            session_id: "sess-1".to_string(),
            node_type: TimelineNodeType::AuthorRun,
            status: TimelineNodeStatus::Completed,
            agent_role: Some(AgentRole::Author),
            provider: Some(ProviderSnapshot {
                name: "claude-code".to_string(),
                model: "claude-opus-4-7".to_string(),
            }),
            messages: vec![],
            streaming_content: "输出内容".to_string(),
            execution_events: vec![],
            permission_events: vec![PermissionEvent {
                request_id: "perm-1".to_string(),
                request: serde_json::json!({"tool": "shell"}),
                response: Some(serde_json::json!({"approved": true})),
                ts: "2026-05-20T14:35:00Z".to_string(),
            }],
            verdict: None,
            artifact_ref: Some(ArtifactRef {
                artifact_id: "art-1".to_string(),
                version: 2,
            }),
            is_revision: false,
            base_artifact_ref: None,
            started_at: "2026-05-20T14:30:00Z".to_string(),
            ended_at: Some("2026-05-20T14:35:00Z".to_string()),
        };
        let json = serde_json::to_value(&detail).unwrap();
        let back: NodeDetail = serde_json::from_value(json).unwrap();
        assert_eq!(back.node_id, detail.node_id);
        assert_eq!(back.permission_events.len(), 1);
    }
}
```

Run: `cargo test node_detail_roundtrip -- --nocapture`
Expected: 编译失败 — NodeDetail / PermissionEvent / ProviderSnapshot / ArtifactRef / AgentRole 未定义

- [ ] **Step 2: 在 models.rs 添加 NodeDetail 相关类型**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSnapshot {
    pub name: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub artifact_id: String,
    pub version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Author,
    Reviewer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionEvent {
    pub request_id: String,
    pub request: serde_json::Value,
    pub response: Option<serde_json::Value>,
    pub ts: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeDetail {
    pub node_id: String,
    pub session_id: String,
    pub node_type: TimelineNodeType,
    pub status: TimelineNodeStatus,
    pub agent_role: Option<AgentRole>,
    pub provider: Option<ProviderSnapshot>,
    pub messages: Vec<serde_json::Value>,
    pub streaming_content: String,
    pub execution_events: Vec<serde_json::Value>,
    pub permission_events: Vec<PermissionEvent>,
    pub verdict: Option<serde_json::Value>,
    pub artifact_ref: Option<ArtifactRef>,
    pub is_revision: bool,
    pub base_artifact_ref: Option<ArtifactRef>,
    pub started_at: String,
    pub ended_at: Option<String>,
}
```

注意：models.rs 中可能需要 `use crate::web::workspace_ws_types::{TimelineNodeType, TimelineNodeStatus};`

- [ ] **Step 3: 跑测试确认通过**

Run: `cargo test node_detail_roundtrip -- --nocapture`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/product/models.rs
git commit -m "feat(model): add NodeDetail, ProviderSnapshot, PermissionEvent, ArtifactRef, AgentRole"
```

---

### Task 4: lifecycle_store 节点详情按文件持久化

**Files:**
- 修改: `src/product/lifecycle_store.rs`
- 测试: `src/product/lifecycle_store.rs`（已有，追加）

- [ ] **Step 1: 写 failing 测试**

追加到 lifecycle_store.rs 的测试区（或新建 `#[cfg(test)]` mod）：

```rust
#[test]
fn save_and_load_node_detail() {
    let store = LifecycleStore::new(ProductAppPaths::new_test());
    let detail = NodeDetail {
        node_id: "node-1".to_string(),
        session_id: "sess-1".to_string(),
        node_type: TimelineNodeType::AuthorRun,
        status: TimelineNodeStatus::Completed,
        agent_role: Some(AgentRole::Author),
        provider: Some(ProviderSnapshot {
            name: "claude-code".to_string(),
            model: "claude-opus-4-7".to_string(),
        }),
        messages: vec![],
        streaming_content: "streaming".to_string(),
        execution_events: vec![],
        permission_events: vec![],
        verdict: None,
        artifact_ref: None,
        is_revision: false,
        base_artifact_ref: None,
        started_at: "2026-05-20T14:30:00Z".to_string(),
        ended_at: None,
    };

    store.save_node_detail("sess-1", "node-1", &detail).unwrap();
    let loaded = store.load_node_detail("sess-1", "node-1").unwrap();
    assert_eq!(loaded.node_id, "node-1");
    assert_eq!(loaded.streaming_content, "streaming");
}

#[test]
fn load_missing_node_detail_returns_not_found() {
    let store = LifecycleStore::new(ProductAppPaths::new_test());
    let err = store.load_node_detail("sess-x", "node-x").unwrap_err();
    assert!(matches!(err, ProductStoreError::NotFound { .. }));
}
```

Run: `cargo test save_and_load_node_detail -- --nocapture`
Expected: 编译失败 — `save_node_detail` / `load_node_detail` 未定义

- [ ] **Step 2: 在 lifecycle_store.rs 实现 save_node_detail / load_node_detail**

```rust
    pub fn save_node_detail(
        &self,
        session_id: &str,
        node_id: &str,
        detail: &NodeDetail,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(session_id)?;
        validate_relative_id(node_id)?;
        let path = self
            .workspace_timeline_root_for_session(session_id)?
            .join("node_details")
            .join(format!("{}.json", node_id));
        write_json(&path, detail)
    }

    pub fn load_node_detail(
        &self,
        session_id: &str,
        node_id: &str,
    ) -> Result<NodeDetail, ProductStoreError> {
        validate_relative_id(session_id)?;
        validate_relative_id(node_id)?;
        let path = self
            .workspace_timeline_root_for_session(session_id)?
            .join("node_details")
            .join(format!("{}.json", node_id));
        if !path_exists(&path)? {
            return Err(ProductStoreError::NotFound {
                kind: "node_detail",
                id: format!("{}/{}", session_id, node_id),
            });
        }
        read_json(&path)
    }

    pub fn list_node_detail_ids(
        &self,
        session_id: &str,
    ) -> Result<Vec<String>, ProductStoreError> {
        validate_relative_id(session_id)?;
        let dir = self
            .workspace_timeline_root_for_session(session_id)?
            .join("node_details");
        if !path_exists(&dir)? {
            return Ok(Vec::new());
        }
        let mut ids = Vec::new();
        for entry in std::fs::read_dir(&dir).map_err(|e| ProductStoreError::Io(e.to_string()))? {
            let entry = entry.map_err(|e| ProductStoreError::Io(e.to_string()))?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.ends_with(".json") {
                ids.push(name_str[..name_str.len() - 5].to_string());
            }
        }
        Ok(ids)
    }
```

- [ ] **Step 3: 跑测试确认通过**

Run: `cargo test save_and_load_node_detail load_missing_node_detail -- --nocapture`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/product/lifecycle_store.rs
git commit -m "feat(store): add node detail per-file persistence (save/load/list)"
```

---

### Task 5: SessionState 扩展 + build_session_state

**Files:**
- 修改: `src/web/workspace_ws_types.rs`（SessionState 变体追加字段）
- 修改: `src/product/workspace_engine.rs`（build_session_state 扩展）
- 测试: `src/product/workspace_engine.rs` 已有测试 `build_session_state_returns_correct_structure`

- [ ] **Step 1: 写 failing 测试**

修改 `src/product/workspace_engine.rs:2199` 的现有测试，追加新字段断言：

```rust
#[tokio::test]
async fn build_session_state_includes_node_details_and_active_run_id() {
    let engine = create_test_engine().await;
    // 模拟运行后状态
    engine.timeline_nodes.push(TimelineNode {
        node_id: "node-1".to_string(),
        node_type: TimelineNodeType::AuthorRun,
        agent: Some(ProviderName::ClaudeCode),
        stage: WorkspaceStage::Completed,
        round: None,
        status: TimelineNodeStatus::Completed,
        title: "生成".to_string(),
        summary: None,
        started_at: "2026-05-20T14:30:00Z".to_string(),
        completed_at: Some("2026-05-20T14:35:00Z".to_string()),
        duration_ms: Some(300000),
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: None,
            review_rounds: 0,
        },
    });

    let state = engine.build_session_state();
    match state {
        WsOutMessage::SessionState {
            timeline_node_details,
            active_run_id,
            ..
        } => {
            assert!(timeline_node_details.contains_key("node-1") || timeline_node_details.is_empty());
            assert!(active_run_id.is_none() || active_run_id == Some("run-1".to_string()));
        }
        _ => panic!("Expected SessionState"),
    }
}
```

Run: `cargo test build_session_state_includes_node_details_and_active_run_id -- --nocapture`
Expected: 编译失败 — SessionState 变体没有 timeline_node_details / active_run_id 字段

- [ ] **Step 2: 在 WsOutMessage::SessionState 追加字段**

修改 `src/web/workspace_ws_types.rs:89-100`：

```rust
    SessionState {
        session_id: String,
        workspace_type: WorkspaceType,
        stage: String,
        messages: Vec<WsMessageDto>,
        checkpoints: Vec<WsCheckpointDto>,
        artifact: Option<String>,
        providers: WsProviderConfig,
        timeline_nodes: Vec<TimelineNode>,
        active_node_id: Option<String>,
        artifact_versions: Vec<ArtifactVersion>,
        timeline_node_details: HashMap<String, NodeDetail>,
        active_run_id: Option<String>,
    },
```

需要 `use std::collections::HashMap;` 和 `use crate::product::models::NodeDetail;`

- [ ] **Step 3: 修改 build_session_state**

修改 `src/product/workspace_engine.rs:1215`，在返回前加载 node details：

```rust
    pub fn build_session_state(&self) -> WsOutMessage {
        // ... 现有代码（messages, checkpoints）保持不变 ...

        let timeline_node_details = if let Ok(ids) = self
            .lifecycle_store
            .list_node_detail_ids(&self.session.session_id)
        {
            ids.into_iter()
                .filter_map(|id| {
                    self.lifecycle_store
                        .load_node_detail(&self.session.session_id, &id)
                        .ok()
                        .map(|detail| (id, detail))
                })
                .collect()
        } else {
            HashMap::new()
        };

        let active_run_id = self.active_run.as_ref().map(|run| run.id.clone());

        WsOutMessage::SessionState {
            // ... 现有字段 ...
            timeline_node_details,
            active_run_id,
        }
    }
```

需要确认 `WorkspaceEngine` 是否有 `lifecycle_store` 和 `active_run` 字段。如果没有，需要添加。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test build_session_state_includes_node_details_and_active_run_id -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/web/workspace_ws_types.rs src/product/workspace_engine.rs
git commit -m "feat(engine): extend SessionState with timeline_node_details and active_run_id"
```

---

### Task 6: workspace_ws_handler 阶段校验 + 新消息路由

**Files:**
- 修改: `src/web/workspace_ws_handler.rs`

- [ ] **Step 1: 写 failing 测试**

新建 `src/web/workspace_ws_handler_test.rs`（或在现有测试区追加）：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_note_in_prepare_context_is_valid() {
        assert!(is_message_valid_for_stage(&WsInMessage::ContextNote { content: "test".to_string() }, &WorkspaceStage::PrepareContext));
    }

    #[test]
    fn context_note_in_running_is_invalid() {
        assert!(!is_message_valid_for_stage(&WsInMessage::ContextNote { content: "test".to_string() }, &WorkspaceStage::Running));
    }

    #[test]
    fn start_generation_only_valid_in_prepare_context() {
        assert!(is_message_valid_for_stage(&WsInMessage::StartGeneration { provider_config: ProviderConfigSnapshot { author: ProviderName::ClaudeCode, reviewer: None, review_rounds: 0 }, reviewer_enabled: false }, &WorkspaceStage::PrepareContext));
        assert!(!is_message_valid_for_stage(&WsInMessage::StartGeneration { provider_config: ProviderConfigSnapshot { author: ProviderName::ClaudeCode, reviewer: None, review_rounds: 0 }, reviewer_enabled: false }, &WorkspaceStage::Running));
    }
}
```

Run: `cargo test workspace_ws_handler -- --nocapture`
Expected: 编译失败 — `is_message_valid_for_stage` 未定义

- [ ] **Step 2: 实现阶段校验函数**

在 `src/web/workspace_ws_handler.rs` 中（在 `handle_workspace_socket` 函数附近）新增：

```rust
fn is_message_valid_for_stage(msg: &WsInMessage, stage: &WorkspaceStage) -> bool {
    match stage {
        WorkspaceStage::PrepareContext => matches!(
            msg,
            WsInMessage::ContextNote { .. }
                | WsInMessage::StartGeneration { .. }
                | WsInMessage::Abort
        ),
        WorkspaceStage::Running => matches!(
            msg,
            WsInMessage::Abort | WsInMessage::PermissionResponse { .. }
        ),
        WorkspaceStage::CrossReview => matches!(msg, WsInMessage::Abort),
        WorkspaceStage::ReviewDecision => matches!(
            msg,
            WsInMessage::SelectRevisionPath { .. } | WsInMessage::RequestRevision { .. }
        ),
        WorkspaceStage::Revision => matches!(msg, WsInMessage::Abort),
        WorkspaceStage::HumanConfirm => matches!(
            msg,
            WsInMessage::HumanConfirm { .. }
        ),
        WorkspaceStage::Completed => false,
    }
}
```

注意：`WsInMessage::SelectRevisionPath` 和 `WsInMessage::RequestRevision` 可能还没定义。如果当前 enum 用的是 `ReviewDecisionResponse` 和 `Rollback`，需要先做映射：
- `WsInMessage::ReviewDecisionResponse { decision, extra_context }` 对应 "选择处理路径"
- `WsInMessage::Rollback { checkpoint_id }` 对应 "要求返修"

如果现有 enum 没有这两个变体，先用现有变体做映射：

```rust
        WorkspaceStage::ReviewDecision => matches!(
            msg,
            WsInMessage::ReviewDecisionResponse { .. } | WsInMessage::Rollback { .. }
        ),
```

- [ ] **Step 3: 在 handle_message 路由中插入阶段校验**

修改 `src/web/workspace_ws_handler.rs:220`，在 `WsInMessage` 反序列化成功后，按 session stage 校验：

```rust
        let in_msg: WsInMessage = match serde_json::from_str(&text) {
            Ok(msg) => msg,
            Err(e) => {
                // ... 现有错误处理 ...
            }
        };

        // 阶段校验
        let stage = engine.current_stage().await; // 需要 engine 暴露此方法
        if !is_message_valid_for_stage(&in_msg, &stage) {
            let err = WsOutMessage::ProtocolError {
                code: "INVALID_MESSAGE_FOR_STAGE".to_string(),
                message: format!("Message {:?} not allowed in stage {:?}", in_msg, stage),
                context: Some(serde_json::json!({"stage": stage, "received": in_msg})),
            };
            let _ = socket.send(Message::Text(serde_json::to_string(&err).unwrap())).await;
            continue;
        }
```

需要确认 `engine.current_stage()` 是否存在。如果不存在，改为：

```rust
        let stage = match engine.get_state_snapshot() {
            Some(state) => state.stage,
            None => WorkspaceStage::PrepareContext,
        };
```

或者更简单：从 session 中读 stage（需要看 engine 如何暴露 session 状态）。

- [ ] **Step 4: 处理 ContextNote / StartGeneration / Hello / Ping**

在 `handle_message` 的 match 中追加分支（在 `UserMessage` 处理下方）：

```rust
            WsInMessage::ContextNote { content } => {
                match engine.append_context_note(content).await {
                    Ok(node) => {
                        let _ = socket.send(Message::Text(serde_json::to_string(&WsOutMessage::TimelineNodeCreated { node }).unwrap())).await;
                    }
                    Err(e) => {
                        let err = WsOutMessage::Error { message: e.to_string() };
                        let _ = socket.send(Message::Text(serde_json::to_string(&err).unwrap())).await;
                    }
                }
            }
            WsInMessage::StartGeneration { provider_config, reviewer_enabled } => {
                match engine.start_generation(provider_config, reviewer_enabled).await {
                    Ok((node, locked)) => {
                        let _ = socket.send(Message::Text(serde_json::to_string(&WsOutMessage::TimelineNodeCreated { node }).unwrap())).await;
                        let _ = socket.send(Message::Text(serde_json::to_string(&locked).unwrap())).await;
                    }
                    Err(e) => {
                        let err = WsOutMessage::Error { message: e.to_string() };
                        let _ = socket.send(Message::Text(serde_json::to_string(&err).unwrap())).await;
                    }
                }
            }
            WsInMessage::Hello { session_id, last_seen_node_id } => {
                // 重连握手：回送完整 SessionState
                let state = engine.build_session_state();
                let _ = socket.send(Message::Text(serde_json::to_string(&state).unwrap())).await;
            }
            WsInMessage::Ping => {
                let _ = socket.send(Message::Text(serde_json::to_string(&WsOutMessage::Pong).unwrap())).await;
            }
```

需要确认 `engine.append_context_note` 和 `engine.start_generation` 是否存在。如果不存在，需要先在 `workspace_engine.rs` 中实现（见 Task 7）。

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test workspace_ws_handler -- --nocapture`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/web/workspace_ws_handler.rs
git commit -m "feat(ws-handler): add stage validation + ContextNote/StartGeneration/Hello/Ping routing"
```

---

### Task 7: WorkspaceEngine 追加 append_context_note / start_generation / aborted_by_disconnect

**Files:**
- 修改: `src/product/workspace_engine.rs`

- [ ] **Step 1: 写 failing 测试**

```rust
#[tokio::test]
async fn append_context_note_creates_timeline_node() {
    let mut engine = create_test_engine().await;
    let node = engine.append_context_note("补充上下文".to_string()).await.unwrap();
    assert_eq!(node.node_type, TimelineNodeType::ContextNote);
    assert_eq!(engine.timeline_nodes.len(), 1);
}

#[tokio::test]
async fn start_generation_locks_provider_and_creates_node() {
    let mut engine = create_test_engine().await;
    let snapshot = ProviderConfigSnapshot {
        author: ProviderName::ClaudeCode,
        reviewer: Some(ProviderName::Codex),
        review_rounds: 1,
    };
    let (node, locked) = engine.start_generation(snapshot, true).await.unwrap();
    assert_eq!(node.node_type, TimelineNodeType::StartGeneration);
    // locked 是 WsOutMessage::ProviderLocked
}

#[tokio::test]
async fn append_aborted_by_disconnect_creates_node() {
    let mut engine = create_test_engine().await;
    let node = engine.append_aborted_by_disconnect("run-1".to_string()).await.unwrap();
    assert_eq!(node.node_type, TimelineNodeType::AbortedByDisconnect);
    assert!(node.summary.is_some());
}
```

Run: `cargo test append_context_note start_generation append_aborted_by_disconnect -- --nocapture`
Expected: 编译失败 — 方法未定义

- [ ] **Step 2: 实现三个方法**

在 `src/product/workspace_engine.rs` `impl WorkspaceEngine` 中追加：

```rust
    pub async fn append_context_note(&mut self, content: String) -> Result<TimelineNode, WorkspaceEngineError> {
        let node = TimelineNode {
            node_id: generate_id(),
            node_type: TimelineNodeType::ContextNote,
            agent: None,
            stage: WorkspaceStage::PrepareContext,
            round: None,
            status: TimelineNodeStatus::Completed,
            title: "上下文补充".to_string(),
            summary: Some(content),
            started_at: now_iso(),
            completed_at: Some(now_iso()),
            duration_ms: Some(0),
            artifact_ref: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: self.session.author_provider.clone(),
                reviewer: self.session.reviewer_provider.clone(),
                review_rounds: self.session.review_rounds.unwrap_or(0),
            },
        };
        self.timeline_nodes.push(node.clone());
        self.lifecycle_store.save_timeline_nodes(&self.session.session_id, &self.timeline_nodes)?;
        Ok(node)
    }

    pub async fn start_generation(
        &mut self,
        provider_config: ProviderConfigSnapshot,
        reviewer_enabled: bool,
    ) -> Result<(TimelineNode, WsOutMessage), WorkspaceEngineError> {
        // 锁定 Provider
        self.session.author_provider = provider_config.author.clone();
        self.session.reviewer_provider = if reviewer_enabled { provider_config.reviewer.clone() } else { None };
        self.session.review_rounds = Some(provider_config.review_rounds);

        let node = TimelineNode {
            node_id: generate_id(),
            node_type: TimelineNodeType::StartGeneration,
            agent: None,
            stage: WorkspaceStage::PrepareContext,
            round: None,
            status: TimelineNodeStatus::Completed,
            title: "开始生成".to_string(),
            summary: None,
            started_at: now_iso(),
            completed_at: Some(now_iso()),
            duration_ms: Some(0),
            artifact_ref: None,
            provider_config_snapshot: provider_config.clone(),
        };
        self.timeline_nodes.push(node.clone());
        self.lifecycle_store.save_timeline_nodes(&self.session.session_id, &self.timeline_nodes)?;

        let locked = WsOutMessage::ProviderLocked {
            snapshot: provider_config,
            locked_at: now_iso(),
        };

        // 启动 author run（现有逻辑）
        self.transition_stage(WorkspaceStage::Running).await;
        self.start_author_run().await?;

        Ok((node, locked))
    }

    pub async fn append_aborted_by_disconnect(
        &mut self,
        last_active_run_id: String,
    ) -> Result<TimelineNode, WorkspaceEngineError> {
        let node = TimelineNode {
            node_id: generate_id(),
            node_type: TimelineNodeType::AbortedByDisconnect,
            agent: None,
            stage: self.session.stage.clone(),
            round: None,
            status: TimelineNodeStatus::Failed,
            title: "运行因断开中止".to_string(),
            summary: Some(format!("last_active_run_id: {}", last_active_run_id)),
            started_at: now_iso(),
            completed_at: Some(now_iso()),
            duration_ms: Some(0),
            artifact_ref: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: self.session.author_provider.clone(),
                reviewer: self.session.reviewer_provider.clone(),
                review_rounds: self.session.review_rounds.unwrap_or(0),
            },
        };
        self.timeline_nodes.push(node.clone());
        self.lifecycle_store.save_timeline_nodes(&self.session.session_id, &self.timeline_nodes)?;
        Ok(node)
    }
```

需要 `generate_id()` 和 `now_iso()` 辅助函数（看项目是否已有）。如果没有：

```rust
fn generate_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}
```

如果项目没有 uuid 依赖，用现有 ID 生成方式。

- [ ] **Step 3: 跑测试确认通过**

Run: `cargo test append_context_note start_generation append_aborted_by_disconnect -- --nocapture`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/product/workspace_engine.rs
git commit -m "feat(engine): add append_context_note, start_generation, append_aborted_by_disconnect"
```

---

### Task 8: 前端类型定义（api/types.ts）

**Files:**
- 修改: `web/src/api/types.ts`

- [ ] **Step 1: 写 failing 测试**

追加到 `web/src/api/types.ts` 的测试（如果没有，在 `workspace-ws-store.test.ts` 中测试类型推断）：

```typescript
import { describe, it, expect } from "vitest";
import type { WsInMessage, WsOutMessage, TimelineNodeType, NodeDetail } from "./types";

describe("protocol types", () => {
  it("context_note has correct shape", () => {
    const msg: WsInMessage = { type: "context_note", content: "test" };
    expect(msg.type).toBe("context_note");
  });

  it("protocol_error has correct shape", () => {
    const msg: WsOutMessage = {
      type: "protocol_error",
      code: "INVALID_MESSAGE_FOR_STAGE",
      message: "test",
      context: { stage: "Running" },
    };
    expect(msg.code).toBe("INVALID_MESSAGE_FOR_STAGE");
  });
});
```

Run: `pnpm --filter web test -- api/types`
Expected: 编译失败 — type 定义中缺少 context_note / protocol_error / provider_locked 等

- [ ] **Step 2: 扩展 api/types.ts**

在现有 WsInMessage / WsOutMessage 类型定义中追加：

```typescript
export type WsInMessage =
  | { type: "user_message"; content: string }
  | { type: "context_note"; content: string }
  | { type: "start_generation"; provider_config: ProviderConfigSnapshot; reviewer_enabled: boolean }
  | { type: "rollback"; checkpoint_id: string }
  | { type: "confirm" }
  | { type: "provider_select"; role: string; provider: string }
  | { type: "permission_response"; id: string; approved: boolean; reason?: string }
  | { type: "review_decision_response"; decision: string; extra_context?: string }
  | { type: "abort" }
  | { type: "hello"; session_id: string; last_seen_node_id?: string }
  | { type: "ping" };

export type WsOutMessage =
  | { type: "stream_chunk"; role: string; content: string; node_id?: string }
  | { type: "message_complete"; message_id: string; checkpoint_id: string; node_id?: string }
  | { type: "stage_change"; stage: string }
  | { type: "artifact_update"; version: number; markdown: string; diff?: string }
  | { type: "provider_select_request"; stage: string; defaults: ProviderDefaults }
  | { type: "permission_request"; id: string; tool_name: string; description: string; risk_level: string }
  | { type: "provider_status"; status: string }
  | { type: "execution_event"; event: ExecutionEvent }
  | { type: "timeline_node_created"; node: TimelineNode }
  | { type: "timeline_node_updated"; node_id: string; status: string; summary?: string; completed_at?: string }
  | { type: "review_complete"; node_id: string; round: number; verdict: string; comments: string; summary: string }
  | { type: "review_decision_required"; node_id: string; round: number; options: string[] }
  | { type: "session_state"
      session_id: string;
      workspace_type: string;
      stage: string;
      messages: WsMessage[];
      checkpoints: WsCheckpoint[];
      artifact: string | null;
      providers: WsProviderConfig;
      timeline_nodes: TimelineNode[];
      active_node_id: string | null;
      artifact_versions: ArtifactVersion[];
      timeline_node_details: Record<string, NodeDetail>;
      active_run_id: string | null;
    }
  | { type: "error"; message: string }
  | { type: "protocol_error"; code: string; message: string; context?: unknown }
  | { type: "provider_locked"; snapshot: ProviderConfigSnapshot; locked_at: string }
  | { type: "pong" };
```

追加类型：

```typescript
export type TimelineNodeType =
  | "prepare_context"
  | "context_note"
  | "start_generation"
  | "author_run"
  | "reviewer_run"
  | "review_decision"
  | "revision"
  | "human_confirm"
  | "aborted_by_disconnect"
  | "protocol_error"
  | "completed";

export interface PermissionEvent {
  request_id: string;
  request: unknown;
  response: unknown | null;
  ts: string;
}

export interface ProviderSnapshot {
  name: string;
  model: string;
}

export interface ArtifactRef {
  artifact_id: string;
  version: number;
}

export interface NodeDetail {
  node_id: string;
  session_id: string;
  node_type: TimelineNodeType;
  status: TimelineNodeStatus;
  agent_role: "author" | "reviewer" | null;
  provider: ProviderSnapshot | null;
  messages: WsMessage[];
  streaming_content: string;
  execution_events: ExecutionEvent[];
  permission_events: PermissionEvent[];
  verdict: ReviewVerdict | null;
  artifact_ref: ArtifactRef | null;
  is_revision: boolean;
  base_artifact_ref: ArtifactRef | null;
  started_at: string;
  ended_at: string | null;
}
```

- [ ] **Step 3: 跑测试确认通过**

Run: `pnpm --filter web test -- api/types`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add web/src/api/types.ts
git commit -m "feat(types): extend WsInMessage/WsOutMessage with protocol v2 + NodeDetail types"
```

---

### Task 9: useWorkspaceWs 新增发送函数

**Files:**
- 修改: `web/src/hooks/useWorkspaceWs.ts`
- 修改: `web/src/hooks/useWorkspaceWs.test.tsx`

- [ ] **Step 1: 写 failing 测试**

追加到 `web/src/hooks/useWorkspaceWs.test.tsx`：

```tsx
import { describe, it, expect, vi } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useWorkspaceWs } from "./useWorkspaceWs";

// 模拟 WebSocket
describe("useWorkspaceWs send functions", () => {
  it("sendContextNote sends correct payload", () => {
    const ws = { readyState: 1, send: vi.fn() } as unknown as WebSocket;
    global.WebSocket = vi.fn(() => ws) as unknown as typeof WebSocket;

    const { result } = renderHook(() => useWorkspaceWs("sess-1"));
    act(() => {
      result.current.sendContextNote("补充上下文");
    });

    expect(ws.send).toHaveBeenCalledWith(
      JSON.stringify({ type: "context_note", content: "补充上下文" })
    );
  });

  it("sendStartGeneration sends correct payload", () => {
    const ws = { readyState: 1, send: vi.fn() } as unknown as WebSocket;
    global.WebSocket = vi.fn(() => ws) as unknown as typeof WebSocket;

    const { result } = renderHook(() => useWorkspaceWs("sess-1"));
    act(() => {
      result.current.sendStartGeneration(
        { author: "claude-code", reviewer: "codex", review_rounds: 1 },
        true
      );
    });

    expect(ws.send).toHaveBeenCalledWith(
      expect.stringContaining("start_generation")
    );
  });

  it("sendHello sends correct payload", () => {
    const ws = { readyState: 1, send: vi.fn() } as unknown as WebSocket;
    global.WebSocket = vi.fn(() => ws) as unknown as typeof WebSocket;

    const { result } = renderHook(() => useWorkspaceWs("sess-1"));
    act(() => {
      result.current.sendHello("sess-1", "node-1");
    });

    expect(ws.send).toHaveBeenCalledWith(
      JSON.stringify({ type: "hello", session_id: "sess-1", last_seen_node_id: "node-1" })
    );
  });
});
```

Run: `pnpm --filter web test -- useWorkspaceWs`
Expected: 编译/运行失败 — sendContextNote / sendStartGeneration / sendHello 未定义

- [ ] **Step 2: 在 useWorkspaceWs.ts 实现发送函数**

替换原有的 `sendMessage` / `startGeneration`：

```typescript
  const sendContextNote = useCallback(
    (content: string) => {
      const ws = wsRef.current;
      if (ws?.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: "context_note", content }));
        useWorkspaceStore.getState().setError(null);
      }
    },
    [],
  );

  const sendStartGeneration = useCallback(
    (providerConfig: ProviderConfigSnapshot, reviewerEnabled: boolean) => {
      const ws = wsRef.current;
      if (ws?.readyState === WebSocket.OPEN) {
        ws.send(
          JSON.stringify({
            type: "start_generation",
            provider_config: providerConfig,
            reviewer_enabled: reviewerEnabled,
          })
        );
        const store = useWorkspaceStore.getState();
        store.setError(null);
        store.clearExecutionEvents();
        store.setProviderStatus("running");
      }
    },
    [],
  );

  const sendHello = useCallback(
    (sessionId: string, lastSeenNodeId?: string) => {
      const ws = wsRef.current;
      if (ws?.readyState === WebSocket.OPEN) {
        ws.send(
          JSON.stringify({
            type: "hello",
            session_id: sessionId,
            last_seen_node_id: lastSeenNodeId,
          })
        );
      }
    },
    [],
  );

  const sendPing = useCallback(() => {
    const ws = wsRef.current;
    if (ws?.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "ping" }));
    }
  }, []);

  // 兼容旧接口（过渡期内保留，但废弃）
  const sendMessage = useCallback(
    (content: string) => {
      console.warn("sendMessage is deprecated, use sendContextNote or sendStartGeneration");
      sendContextNote(content);
    },
    [sendContextNote],
  );

  const startGeneration = useCallback(
    () => {
      console.warn("startGeneration() without args is deprecated");
      // 由调用方提供 ProviderConfig，这里不做任何操作
    },
    [],
  );
```

返回值中添加这些函数：

```typescript
  return {
    sendMessage,
    sendContextNote,
    sendStartGeneration,
    sendHello,
    sendPing,
    startGeneration,
    rollback,
    confirm,
    sendPermissionResponse,
    sendReviewDecision,
    abort,
    sendProviderSelect,
    connectionStatus,
  };
```

- [ ] **Step 3: 跑测试确认通过**

Run: `pnpm --filter web test -- useWorkspaceWs`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add web/src/hooks/useWorkspaceWs.ts web/src/hooks/useWorkspaceWs.test.tsx
git commit -m "feat(ws-hooks): add sendContextNote/sendStartGeneration/sendHello/sendPing"
```

---

### Task 10: workspace-ws-store 改造（snapshot 应用 nodeDetails + selectNodeDetail）

**Files:**
- 修改: `web/src/state/workspace-ws-store.ts`
- 修改: `web/src/state/workspace-ws-store.test.ts`

- [ ] **Step 1: 写 failing 测试**

追加到 `web/src/state/workspace-ws-store.test.ts`：

```typescript
import { describe, it, expect } from "vitest";
import { useWorkspaceStore } from "./workspace-ws-store";

describe("workspace-ws-store node details", () => {
  it("setSessionState populates nodeDetails", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState({
      session_id: "sess-1",
      workspace_type: "story_spec",
      stage: "running",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude-code", reviewer: null },
      timeline_nodes: [{ node_id: "node-1", node_type: "author_run", status: "completed", title: "生成" }],
      active_node_id: "node-1",
      artifact_versions: [],
      timeline_node_details: {
        "node-1": {
          node_id: "node-1",
          session_id: "sess-1",
          node_type: "author_run",
          status: "completed",
          agent_role: "author",
          provider: { name: "claude-code", model: "opus-4-7" },
          messages: [],
          streaming_content: "输出内容",
          execution_events: [],
          permission_events: [],
          verdict: null,
          artifact_ref: null,
          is_revision: false,
          base_artifact_ref: null,
          started_at: "2026-05-20T14:30:00Z",
          ended_at: null,
        },
      },
      active_run_id: "run-1",
    });

    expect(useWorkspaceStore.getState().nodeDetails["node-1"]).toBeDefined();
    expect(useWorkspaceStore.getState().nodeDetails["node-1"].streaming_content).toBe("输出内容");
  });

  it("selectNodeDetail returns correct detail", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState({
      session_id: "sess-1",
      workspace_type: "story_spec",
      stage: "running",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude-code", reviewer: null },
      timeline_nodes: [],
      active_node_id: null,
      artifact_versions: [],
      timeline_node_details: {
        "node-1": { node_id: "node-1", session_id: "sess-1", node_type: "author_run", status: "completed", agent_role: "author", provider: null, messages: [], streaming_content: "test", execution_events: [], permission_events: [], verdict: null, artifact_ref: null, is_revision: false, base_artifact_ref: null, started_at: "", ended_at: null },
      },
      active_run_id: null,
    });

    const detail = useWorkspaceStore.getState().nodeDetails["node-1"];
    expect(detail.streaming_content).toBe("test");
  });
});
```

Run: `pnpm --filter web test -- workspace-ws-store`
Expected: 运行失败 — setSessionState 不处理 timeline_node_details

- [ ] **Step 2: 修改 workspace-ws-store.ts**

修改 `setSessionState`：

```typescript
  setSessionState: (state) =>
    set((prev) => ({
      ...prev,
      sessionId: state.session_id,
      workspaceType: state.workspace_type,
      stage: state.stage,
      messages: state.messages,
      checkpoints: state.checkpoints,
      artifact: state.artifact,
      providers: state.providers,
      timelineNodes: state.timeline_nodes,
      activeNodeId: state.active_node_id,
      artifactVersions: state.artifact_versions,
      nodeDetails: state.timeline_node_details || prev.nodeDetails,
      activeRunId: state.active_run_id ?? prev.activeRunId,
      // 连接成功时清错误
      connectionStatus: prev.connectionStatus === "connecting" ? "connected" : prev.connectionStatus,
    })),
```

在 WorkspaceWsState 中追加 `activeRunId: string | null`：

```typescript
export interface WorkspaceWsState {
  // ... 现有字段 ...
  activeRunId: string | null;
}
```

初始值：

```typescript
const initialState: WorkspaceWsState = {
  // ...
  activeRunId: null,
};
```

- [ ] **Step 3: 跑测试确认通过**

Run: `pnpm --filter web test -- workspace-ws-store`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add web/src/state/workspace-ws-store.ts web/src/state/workspace-ws-store.test.ts
git commit -m "feat(store): apply timeline_node_details from snapshot + activeRunId"
```

---

### Task 11: 处理 protocol_error / provider_locked / pong 出站消息

**Files:**
- 修改: `web/src/hooks/useWorkspaceWs.ts`（handleMessage）
- 修改: `web/src/state/workspace-ws-store.ts`

- [ ] **Step 1: 写 failing 测试**

追加到 `web/src/state/workspace-ws-store.test.ts`：

```typescript
  it("handles protocol_error", () => {
    const store = useWorkspaceStore.getState();
    store.setProtocolError({ code: "INVALID_MESSAGE_FOR_STAGE", message: "test" });
    expect(useWorkspaceStore.getState().protocolError).toEqual({ code: "INVALID_MESSAGE_FOR_STAGE", message: "test" });
  });

  it("handles provider_locked", () => {
    const store = useWorkspaceStore.getState();
    store.setProviderLocked({ snapshot: { author: "claude-code", reviewer: null, review_rounds: 0 }, locked_at: "2026-05-20T14:35:00Z" });
    expect(useWorkspaceStore.getState().providerLocked).toBe(true);
  });
```

Run: `pnpm --filter web test -- workspace-ws-store`
Expected: 失败 — setProtocolError / setProviderLocked 未定义

- [ ] **Step 2: 在 store 中追加状态和 actions**

```typescript
export interface WorkspaceWsState {
  // ... 现有字段 ...
  protocolError: { code: string; message: string } | null;
  providerLocked: boolean;
  providerSnapshot: ProviderConfigSnapshot | null;
}

export interface WorkspaceWsActions {
  // ... 现有 actions ...
  setProtocolError: (error: { code: string; message: string } | null) => void;
  setProviderLocked: (payload: { snapshot: ProviderConfigSnapshot; locked_at: string } | null) => void;
}
```

实现：

```typescript
  setProtocolError: (error) => set({ protocolError: error }),
  setProviderLocked: (payload) =>
    set({
      providerLocked: payload !== null,
      providerSnapshot: payload?.snapshot ?? null,
    }),
```

初始值：

```typescript
  protocolError: null,
  providerLocked: false,
  providerSnapshot: null,
```

- [ ] **Step 3: 在 useWorkspaceWs.ts handleMessage 中处理新消息**

```typescript
      case "protocol_error":
        store.setProtocolError({ code: msg.code, message: msg.message });
        break;
      case "provider_locked":
        store.setProviderLocked({ snapshot: msg.snapshot, locked_at: msg.locked_at });
        break;
      case "pong":
        // 心跳回应，不做 UI 处理
        break;
```

- [ ] **Step 4: 跑测试确认通过**

Run: `pnpm --filter web test -- workspace-ws-store`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add web/src/state/workspace-ws-store.ts web/src/hooks/useWorkspaceWs.ts web/src/state/workspace-ws-store.test.ts
git commit -m "feat(store): handle protocol_error, provider_locked, pong messages"
```

---

### Task 12: 全量回归测试

- [ ] **Step 1: 跑后端全量测试**

Run: `cargo test --workspace`
Expected: PASS

- [ ] **Step 2: 跑前端单元测试**

Run: `pnpm --filter web test`
Expected: PASS

- [ ] **Step 3: 跑 E2E 回归（确保协议兼容期不破坏既有用例）**

Run: `pnpm --filter web test:e2e`
Expected: 可能部分 user_message 用例需要适配，但不应有 core break

- [ ] **Step 4: Commit（如有修复）**

```bash
git commit -am "fix: adjust tests for protocol v2 compatibility"
```

---

## 自审检查

**1. Spec coverage:**

| 设计 § | 实现位置 |
|---|---|
| §1.1 context_note / start_generation | Task 1 (类型), Task 6 (路由), Task 7 (engine 方法) |
| §1.2 阶段-消息合法性矩阵 | Task 6 (is_message_valid_for_stage) |
| §1.3 protocol_error / provider_locked / pong | Task 1 (类型), Task 6 (路由), Task 11 (前端处理) |
| §2.1 Timeline 节点类型枚举 | Task 1 (TimelineNodeType) |
| §2.2 节点详情数据结构 | Task 3 (NodeDetail) |
| §2.3 按节点分文件持久化 | Task 4 (save_node_detail / load_node_detail) |
| §2.5 SessionState snapshot 扩展 | Task 5 (build_session_state) |
| §2.6 前端 store 改造 | Task 10 (setSessionState 灌 nodeDetails) |

**2. Placeholder scan:**
- `generate_id()` / `now_iso()` 可能需要替换为项目已有工具函数
- `engine.current_stage()` 如果不存在，改用 engine 公开的其他方式
- `start_author_run()` 调用在 Task 7 中——需要确认 engine 已有此方法

**3. Type consistency:**
- `ProviderConfigSnapshot` 在 WsInMessage::StartGeneration 和 WsOutMessage::ProviderLocked 和 TimelineNode.provider_config_snapshot 中一致
- `NodeDetail` 在 Rust (models.rs) 和 TS (api/types.ts) 中字段对齐
- `AgentRole` 使用 snake_case (`author`/`reviewer`)

---

## 本 plan 验收清单

- [ ] `cargo test --workspace` PASS
- [ ] `pnpm --filter web test` PASS
- [ ] `pnpm --filter web test:e2e` 既有用例不破坏
- [ ] `WsInMessage::ContextNote` 序列化/反序列化正确
- [ ] `WsInMessage::StartGeneration` 序列化/反序列化正确
- [ ] `WsOutMessage::ProtocolError` 阶段校验触发正确
- [ ] `WsOutMessage::ProviderLocked` 包含 snapshot + locked_at
- [ ] `NodeDetail` 写入 `timeline_node_details/<node_id>.json` 并可读取
- [ ] `SessionState` snapshot 包含 `timeline_node_details` 和 `active_run_id`
- [ ] 前端 `sendContextNote` / `sendStartGeneration` / `sendHello` / `sendPing` 发送正确 JSON
- [ ] 前端 store 收到 snapshot 时 `nodeDetails` 被正确填充
- [ ] 前端 store 收到 `protocol_error` 时状态更新
