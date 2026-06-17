# WorkItem 对话式 Workspace 生成 WP2a：后端 artifact payload union 挂载 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 WP1 定义的 `ArtifactPayload` union 真正挂载到 artifact 链路——`WsOutMessage::ArtifactUpdate`、`SessionState.artifact`、`EngineEvent::ArtifactUpdate`、`WorkspaceSession.artifact`、`ArtifactVersion`、`ArtifactVersionSummary` 从 markdown `String` 切换到 `ArtifactPayload`；Story/Design/WorkItem 行为等价（`Markdown` 变体），`WorkItemPlanCandidate` 变体类型就位（WP2b 才产生数据）。本 WP 是全 workspace type 基础设施改造，不涉及 WorkItemPlan 业务逻辑。

**Architecture:** WP1 用选项 B 纯新增了 `ArtifactPayload` enum（`untagged`，JSON 扁平形态）与 `WorkItemPlanCandidateDto`，但未挂载到任何现有类型。本 WP 完成 union 挂载：消息层（`WsOutMessage::ArtifactUpdate` / `SessionState.artifact`）、引擎内存层（`WorkspaceSession.artifact` / `EngineEvent::ArtifactUpdate` / `ArtifactVersion`）、摘要层（`ArtifactVersionSummary`）三层统一切 union。serde 用 `#[serde(flatten)]` 把 `ArtifactPayload` 扁平进 `ArtifactUpdate`/`SessionState`，产出设计方案 :339-348 要求的 `{ type, version, markdown?/diff?/candidate? }` 形态。`ArtifactVersionSummary` 的 `markdown_size`/`markdown_preview` 字段名保留（向后兼容前端旧契约，WP6/WP7 无需为本 WP 适配），但对 `WorkItemPlanCandidate` 变体填派生值。Story/Design/WorkItem 现有流程经 union 化后行为完全等价。

**Tech Stack:** Rust 1.95.0（edition 2024）、Cargo、serde（`flatten` + `untagged`）、tokio、axum。本 WP 不涉及前端。

**版本：** v1.0
**创建日期：** 2026-06-17
**目标分支：** `feat-b-0616`
**工作区：** `.worktrees/feat-b-0616`
**依赖总览：** `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_拆分总览_v1.0.md`（v1.1，WP2a 章节）
**设计方案：** `cadence/designs/2026-06-17_技术方案_WorkItem对话式Workspace生成_v1.0.md`（第 204-213 行 artifact payload union）
**前置 WP：** WP1（`cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_WP1_后端枚举与context与prepare_v1.0.md`）

---

## 全局约束（Global Constraints）

- **运行命令固定**：Rust 工具链锁 1.95.0；所有 cargo 命令带 `--locked`；🔴 **禁止 `-j 1`**。
- **强制检查链**：`cargo fmt --check` + `cargo clippy --all-targets --all-features --locked -- -D warnings` + `cargo check --locked` + 定向测试，四者缺一不可。
- **TDD**：每个 Task 先写失败测试，再改类型 + 消费点 + 夹具让测试通过，再提交。
- **serde 约定**：新增/修改 enum/struct 保持 `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]` + `#[serde(rename_all = "snake_case")]`。
- **写入范围严格**：只改「File Structure」声明的文件；本 WP 是类型签名变更，会让所有构造/消费 `session.artifact` / `ArtifactUpdate` / `ArtifactVersion` 的位置编译失败——这些适配属本 WP 范围（用 `grep` 定位全部命中点，机械适配）。若发现需改声明范围外的文件才能编译，停下来确认。
- **行号是参考**：基于 2026-06-17 的 `feat-b-0616` HEAD（`8a2eee4`）；实现时以 `grep -n` 实际定位为准。

---

## 前置交付摘要（来自 WP1）

WP1 已在 `src/web/workspace_ws_types.rs` **纯新增**（未挂载、未修改任何现有类型）：

- `ArtifactPayload` enum（`#[serde(rename_all = "snake_case", untagged)]`）：
  ```rust
  pub enum ArtifactPayload {
      Markdown { markdown: String, #[serde(default, skip_serializing_if = "Option::is_none")] diff: Option<String> },
      WorkItemPlanCandidate { candidate: WorkItemPlanCandidateDto },
  }
  ```
  JSON 扁平形态：`{"markdown":"...","diff":"..."}` 或 `{"candidate":{...}}`（无 `kind` tag）。
- `WorkItemPlanCandidateDto` + `WorkItemPlanDto` + `WorkItemCandidateDto` + `WorkItemCandidateMetaDto` + `WorkItemSplitOptionsDto` + `WorkItemDependencyEdgeDto` + `ValidatorFindingDto`（+ 可能的 `VerificationPlanDto` / `RepositoryProfileDto`）。
- `WsInMessage::RevertWorkItem { work_item_id, feedback: Option<String>, clear: bool }`（WP4 处理）。
- WP1 **未改** `WsOutMessage::ArtifactUpdate`（仍是 `{ version, markdown, diff }`）、`SessionState.artifact`（仍是 `Option<String>`）、`ArtifactVersion.markdown`（仍是 `String`）——这些都留给本 WP 切换。

**WP1 同时落地**：`WorkspaceType::WorkItemPlan` 变体（serde `"work_item_plan"`）、`workspace_context.rs` 全分支、`prepare_work_item_plan` handler + 路由。本 WP 不再涉及这些。

---

## 关键既有事实（避免重新探查）

所有行号基于 `feat-b-0616` HEAD `8a2eee4`，实现时用 `grep -n` 确认。

### `src/web/workspace_ws_types.rs`
- `WsOutMessage::ArtifactUpdate { version: u32, markdown: String, diff: Option<String> }`（:50-54）。`WsOutMessage` 整体 `#[serde(tag = "type", rename_all = "snake_case")]`。
- `SessionState` 变体字段 `artifact: Option<String>`（:110，在 `WsOutMessage::SessionState` :102-119 内）。
- `ArtifactVersion { version, markdown: String, generated_by, reviewed_by, review_verdict, confirmed_by, is_current, created_at, source_node_id }`（:423-435）。
- `ArtifactVersionSummary { version, generated_by, reviewed_by, review_verdict, confirmed_by, is_current, created_at, source_node_id, markdown_size: usize, markdown_preview: String }`（:455-467）。
- `ArtifactPayload` + `WorkItemPlanCandidateDto` 等（WP1 新增，位于 `ArtifactVersion` 附近）。

### `src/product/workspace_engine.rs`（8362 行）
- `WorkspaceSession.artifact: Option<String>`（:171，struct 定义 :163-179）。
- `EngineEvent::ArtifactUpdate { version: u32, markdown: String }`（:257-260，enum 定义 :248-392 区间）。
- `build_artifact_version_summary(version: &ArtifactVersion) -> ArtifactVersionSummary`（:98-111）：读 `version.markdown.len()`（:108）与 `preview(&version.markdown)`（:109）。
- `update_artifact(&mut self, markdown: String)`（:2772-2815）：
  - :2773 `self.session.artifact = Some(markdown.clone());`
  - :2782-2792 构造 `ArtifactVersion { markdown, ... }` push 到 `self.artifact_versions`
  - :2808-2814 发 `EngineEvent::ArtifactUpdate { version, markdown: self.session.artifact.clone().unwrap_or_default() }`
- `build_session_state`（:2996-3067）：:3054 `artifact: self.session.artifact.clone()`。
- `new_persistent`（:470-524）：:483-487 从 `list_artifact_versions` 取 `is_current` 版本的 `version.markdown.clone()` 填 `session.artifact`。
- `complete_assistant_message`（:2300-2438）：:2366 `let artifact_markdown = extract_artifact_content(&full_content);`；:2396 `self.update_artifact(artifact_markdown).await;`；:2399 `let artifact_snapshot = self.session.artifact.clone().unwrap_or_default();`（用于 checkpoint）。
- `handle_rollback`（:2652-2681 附近）：:2668-2670 `self.session.artifact = Some(target.artifact_snapshot)` / `None`（`artifact_snapshot` 来自 checkpoint_store，String）。
- `build_review_input`（:2470 附近）：:2477 `let artifact = self.session.artifact.clone().unwrap_or_default();`（喂 review prompt）。
- `build_revision_input`（:2550 附近）：:2550 同上（喂 revision prompt）。
- `handle_author_decision` Reject 路径：:2110 `self.session.artifact = None;`。
- **测试夹具**（`session.artifact = Some("# ...")` 或读 `session.artifact` / `version.markdown` / `EngineEvent::ArtifactUpdate { markdown, .. }`）：:4785, :4810, :4875, :4908, :5466, :5527, :5581, :5641, :5681, :6199, :6240, :6254, :6337, :6421, :6476, :6905, :7413, :7555, :7607, :7665。
- 文件顶部 `use`（:1-33）：从 `crate::web::workspace_ws_types` 导入 `ArtifactVersion, ArtifactVersionSummary, ...`；本 WP 需补 `ArtifactPayload` 导入。

### `src/web/workspace_ws_handler.rs`（1553 行）
- event forwarder（:248-392 闭包）：:270-274
  ```rust
  EngineEvent::ArtifactUpdate { version, markdown } => WsOutMessage::ArtifactUpdate {
      version,
      markdown,
      diff: None,
  },
  ```
- 文件顶部 `use`（:1-34）：从 `crate::product::workspace_engine` 导入 `EngineEvent`；从 `crate::web::workspace_ws_types` 导入 `WsOutMessage` 等。本 WP 需补 `ArtifactPayload` 导入（若 forwarder 构造 payload）。

### checkpoint 层
- `checkpoint_store.create_checkpoint(&session_id, message_index, &artifact_snapshot, ...)`（engine.rs:2400-2403）：`artifact_snapshot: String`。`handle_rollback` 读 `target.artifact_snapshot: String`。
- ⚠️ **checkpoint 的 `artifact_snapshot` 是 `String`**。本 WP 把 `session.artifact` 切成 `Option<ArtifactPayload>` 后，checkpoint 的 snapshot 也要对齐——见 Task 3 决策。

---

## File Structure

| 文件 | 操作 | 职责 / 本 WP 改动 |
|---|---|---|
| `src/web/workspace_ws_types.rs` | M | `WsOutMessage::ArtifactUpdate` 切 `{ version, #[serde(flatten)] payload: ArtifactPayload }`；`SessionState.artifact: Option<ArtifactPayload>`；`ArtifactVersion.markdown` → `payload: ArtifactPayload`；`ArtifactVersionSummary` 的 `markdown_size`/`markdown_preview` 按 payload 变体派生（字段名保留）；补 serde 测试 |
| `src/product/workspace_engine.rs` | M | `WorkspaceSession.artifact: Option<ArtifactPayload>`；`EngineEvent::ArtifactUpdate { version, payload }`；`update_artifact(payload: ArtifactPayload)`；`build_artifact_version_summary` 派生；`build_session_state`；`new_persistent`；`complete_assistant_message`（包 `Markdown`）；`handle_rollback`；`build_review_input`/`build_revision_input`；`handle_author_decision`；所有测试夹具迁移 |
| `src/web/workspace_ws_handler.rs` | M | event forwarder :270-274 适配 `EngineEvent::ArtifactUpdate { version, payload }` → `WsOutMessage::ArtifactUpdate { version, payload }` |
| `src/product/checkpoint_store.rs` | M | `create_checkpoint` 的 `artifact_snapshot` 参数 + `CheckpointRecord.artifact_snapshot` 字段切 `ArtifactPayload`（见 Task 3 决策） |
| `tests/it_web.rs` 及子模块 | M | 受影响集成测试夹具（若有直接构造 `ArtifactUpdate`/`session.artifact` 的地方） |

**不改：**
- ❌ `src/web/workspace_context.rs` / `handlers.rs` / `app.rs`（WP1 已完成，本 WP 不碰）
- ❌ `src/product/lifecycle_store.rs`（WP2b 才改）
- ❌ `src/product/work_item_split_engine.rs` / `work_item_split_validator.rs`（WP2b/WP4）
- ❌ 前端（WP6/WP7）
- ❌ 不产生 `ArtifactPayload::WorkItemPlanCandidate` 数据（WP2b）

> ⚠️ **checkpoint_store 是否在写入范围？** 探查显示 `checkpoint_store.create_checkpoint` 的 `artifact_snapshot` 是 `String`，`handle_rollback` 读它恢复 `session.artifact`。若 `session.artifact` 切 `Option<ArtifactPayload>`，checkpoint snapshot 必须同步切，否则 `handle_rollback` 编译失败。因此 `checkpoint_store.rs` 在本 WP 写入范围内。Task 3 处理。**实现前先 `grep -rn "artifact_snapshot" src/` 确认全部命中点。**

---

## Task 1：消息层 + 引擎内存层切 union（`ArtifactUpdate` / `SessionState` / `EngineEvent` / `WorkspaceSession.artifact`）

**目标**：把出站消息、入站事件、引擎 session 的 artifact 字段从 `String` 切到 `ArtifactPayload`；`update_artifact` 签名改 `ArtifactPayload`；`complete_assistant_message` 把 markdown 包成 `Markdown` 变体；所有 `session.artifact` 读/写点适配；event forwarder 适配。**本 Task 暂不动 `ArtifactVersion`（仍是 `markdown: String`，Task 2 切）**——`update_artifact` 内部从 payload 取 markdown 存 `ArtifactVersion`，避免 Task 1 范围过大。

**Files:**
- Modify: `src/web/workspace_ws_types.rs`（`WsOutMessage::ArtifactUpdate`、`SessionState.artifact`）
- Modify: `src/product/workspace_engine.rs`（`WorkspaceSession.artifact`、`EngineEvent::ArtifactUpdate`、`update_artifact`、`build_session_state`、`new_persistent`、`complete_assistant_message`、`handle_rollback`、`build_review_input`、`build_revision_input`、`handle_author_decision` + 测试夹具）
- Modify: `src/web/workspace_ws_handler.rs`（event forwarder :270-274）
- Modify: `src/product/checkpoint_store.rs`（`artifact_snapshot` 切 `ArtifactPayload`——`create_checkpoint` 签名 + `CheckpointRecord` 字段 + `restore`/读点）

**Interfaces:**
- Consumes: WP1 的 `ArtifactPayload` enum。
- Produces: `WsOutMessage::ArtifactUpdate { version, payload: ArtifactPayload }`；`SessionState.artifact: Option<ArtifactPayload>`；`EngineEvent::ArtifactUpdate { version, payload }`；`WorkspaceSession.artifact: Option<ArtifactPayload>`；`update_artifact(&mut self, payload: ArtifactPayload)`。这些被 WP2b 用 `ArtifactPayload::WorkItemPlanCandidate` 推送 candidate。

- [ ] **Step 1.1：写失败测试 —— ArtifactUpdate 携带 ArtifactPayload 扁平序列化**

在 `src/web/workspace_ws_types.rs` 的 `#[cfg(test)] mod tests` 末尾追加。验证 `WsOutMessage::ArtifactUpdate` 携带 `Markdown` 变体时 JSON 为扁平形态（与方案 :339-348 一致），且 `SessionState.artifact` 接受 `Option<ArtifactPayload>`。

```rust
    #[test]
    fn artifact_update_with_markdown_payload_serializes_flat() {
        let msg = WsOutMessage::ArtifactUpdate {
            version: 2,
            payload: ArtifactPayload::Markdown {
                markdown: "# 标题".to_string(),
                diff: None,
            },
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        // 扁平：version / markdown 与 type 并列，无 "payload" 包裹层
        assert!(json.contains("\"type\":\"artifact_update\""));
        assert!(json.contains("\"version\":2"));
        assert!(json.contains("\"markdown\":\"# 标题\""));
        assert!(!json.contains("\"payload\""));
        assert!(!json.contains("\"kind\""));
        let back: WsOutMessage = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(msg, back);
    }

    #[test]
    fn session_state_artifact_accepts_markdown_payload() {
        // 构造一个最小 SessionState，artifact 字段为 Markdown payload
        let state = WsOutMessage::SessionState {
            session_id: "session_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            stage: "author_confirm".to_string(),
            superpowers_enabled: false,
            openspec_enabled: false,
            messages: Vec::new(),
            checkpoints: Vec::new(),
            artifact: Some(ArtifactPayload::Markdown {
                markdown: "# Story".to_string(),
                diff: None,
            }),
            providers: WsProviderConfig {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::Codex),
            },
            timeline_nodes: Vec::new(),
            active_node_id: None,
            artifact_versions: Vec::new(),
            artifact_version_summaries: Vec::new(),
            timeline_node_details: std::collections::HashMap::new(),
            timeline_node_summaries: std::collections::HashMap::new(),
            active_run_id: None,
        };
        let json = serde_json::to_string(&state).expect("serialize");
        assert!(json.contains("\"artifact\":{\"markdown\":\"# Story\"}"));
        let back: WsOutMessage = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(state, back);
    }
```

> 实现者注意：`WsProviderConfig` 字段（`author` / `reviewer`）以实际定义为准——先 `grep -n "struct WsProviderConfig" src/web/workspace_ws_types.rs` 确认字段名与类型（可能是 `author: ProviderName, reviewer: Option<ProviderName>`）。`SessionState` 的字段清单以 :102-119 实际为准；上面是参考，字段名/类型不符时按实际调整。test mod `use` 补 `ArtifactPayload`。

- [ ] **Step 1.2：运行测试，确认失败**

Run: `cargo test --locked --lib workspace_ws_types`
Expected: 编译失败——`WsOutMessage::ArtifactUpdate` 无 `payload` 字段、`SessionState.artifact` 是 `Option<String>` 不接受 `ArtifactPayload`。

- [ ] **Step 1.3：`WsOutMessage::ArtifactUpdate` 切 union（flatten）**

`src/web/workspace_ws_types.rs:50-54`：

```rust
    ArtifactUpdate {
        version: u32,
        #[serde(flatten)]
        payload: ArtifactPayload,
    },
```

> `#[serde(flatten)]` 把 `ArtifactPayload`（`untagged`）的字段扁平展开到 `ArtifactUpdate` 层级，与 `version`、`type` 并列。JSON：`{"type":"artifact_update","version":2,"markdown":"..."}` 或 `{"type":"artifact_update","version":2,"candidate":{...}}`。`diff` 因 `skip_serializing_if = "Option::is_none"` 在 None 时省略。
>
> ⚠️ `flatten` + `untagged` 组合：serde 在反序列化时把 `version`/`type` 之外的剩余字段交给 `ArtifactPayload` 的 untagged 反序列化。两变体必填字段（`markdown` vs `candidate`）不重叠，无歧义。**若 serde 版本对 flatten+untagged 有已知 bug，fallback：不用 flatten，改为 `ArtifactUpdate { version, markdown: Option<String>, diff: Option<String>, candidate: Option<WorkItemPlanCandidateDto> }` + 自定义序列化保证互斥。先试 flatten，若 Step 1.6 测试不过再 fallback。**

- [ ] **Step 1.4：`SessionState.artifact` 切 union**

`src/web/workspace_ws_types.rs:110`：

```rust
        artifact: Option<ArtifactPayload>,
```

> `SessionState` 整体是 `WsOutMessage` 的变体，`WsOutMessage` 用 `#[serde(tag = "type", rename_all = "snake_case")]`。`artifact: Option<ArtifactPayload>` 会序列化为 `"artifact":{"markdown":"..."}` 或 `"artifact":{"candidate":{...}}` 或 `"artifact":null`。无需 flatten（`artifact` 是命名字段，不是 union 展开）。

- [ ] **Step 1.5：引擎层类型 + 消费点适配**

`src/product/workspace_engine.rs`：

1. **`WorkspaceSession.artifact`**（:171）：`pub artifact: Option<ArtifactPayload>,`
2. **`EngineEvent::ArtifactUpdate`**（:257-260）：
   ```rust
       ArtifactUpdate {
           version: u32,
           payload: ArtifactPayload,
       },
   ```
3. **`update_artifact`**（:2772-2815）签名 + 函数体改：
   ```rust
       pub async fn update_artifact(&mut self, payload: ArtifactPayload) {
           self.session.artifact = Some(payload.clone());
           for version in &mut self.artifact_versions {
               version.is_current = false;
           }
           let version = self.artifact_versions.len() as u32 + 1;
           let source_node_id = self
               .active_node_id
               .clone()
               .unwrap_or_else(|| "timeline_node_unknown".to_string());
           // Task 1 阶段：ArtifactVersion 仍是 markdown: String，从 payload 取 markdown。
           // Task 2 会把 ArtifactVersion 切 payload: ArtifactPayload。
           let markdown = match &payload {
               ArtifactPayload::Markdown { markdown, .. } => markdown.clone(),
               ArtifactPayload::WorkItemPlanCandidate { .. } => String::new(),
           };
           self.artifact_versions.push(ArtifactVersion {
               version,
               markdown,
               generated_by: self.session.author_provider.clone(),
               reviewed_by: None,
               review_verdict: None,
               confirmed_by: None,
               is_current: true,
               created_at: chrono::Utc::now().to_rfc3339(),
               source_node_id,
           });
           self.persist_artifact_versions();
           let source_node_id = self
               .artifact_versions
               .last()
               .map(|version| version.source_node_id.clone())
               .unwrap_or_else(|| "timeline_node_unknown".to_string());
           let _ = self
               .persist_artifact_ref(
                   &source_node_id,
                   ArtifactRef {
                       artifact_id: format!("artifact_version_{version:03}"),
                       version,
                   },
               )
               .await;
           let _ = self
               .event_tx
               .send(EngineEvent::ArtifactUpdate {
                   version,
                   payload,
               })
               .await;
       }
   ```
   > `WorkItemPlanCandidate` 变体在 Task 1 阶段 `ArtifactVersion` 存空 markdown（Task 2 切 payload 后此分支消失）。WP2b 才会产生 candidate payload，本 WP 不走该分支。
4. **`complete_assistant_message`**（:2366, :2396, :2399）：
   - :2396 把 `self.update_artifact(artifact_markdown).await;` 改为：
     ```rust
     self.update_artifact(ArtifactPayload::Markdown {
         markdown: artifact_markdown.clone(),
         diff: None,
     }).await;
     ```
   - :2399 `let artifact_snapshot = self.session.artifact.clone().unwrap_or_default();` → 这里的 `artifact_snapshot` 传给 `checkpoint_store.create_checkpoint`。若 checkpoint 的 snapshot 切 `ArtifactPayload`（Task 3），改为：
     ```rust
     let artifact_snapshot = self.session.artifact.clone().unwrap_or(
         ArtifactPayload::Markdown { markdown: String::new(), diff: None }
     );
     ```
     > 若 Task 3 决定 checkpoint snapshot 仍用 `String`（从 payload 取 markdown），则此处 `let artifact_snapshot = match &self.session.artifact { Some(ArtifactPayload::Markdown { markdown, .. }) => markdown.clone(), _ => String::new() };`。**以 Task 3 决策为准——先做 Task 3 的 checkpoint 决策再定此处。** 为避免 Task 1 卡在 Task 3，建议把 checkpoint 适配放 Task 3，本 Task 1 先把 `artifact_snapshot` 的构造按「从 payload 取 markdown」做（临时），Task 3 再切。
5. **`handle_rollback`**（:2668-2670）：`self.session.artifact = Some(target.artifact_snapshot)` → 适配为 `Option<ArtifactPayload>`。**以 Task 3 checkpoint 决策为准**（同上，临时按 String→payload 取）。
6. **`build_session_state`**（:3054）：`artifact: self.session.artifact.clone()` ——类型已是 `Option<ArtifactPayload>`，clone 即可，无需改代码（但类型变了，编译会过）。
7. **`new_persistent`**（:483-487）：当前 `session.artifact = version.markdown.clone()`。切 union 后：
   ```rust
   if !persisted_artifact_versions.is_empty() {
       session.artifact = persisted_artifact_versions
           .iter().rev()
           .find(|version| version.is_current)
           .map(|version| ArtifactPayload::Markdown {
               markdown: version.markdown.clone(),
               diff: None,
           });
   }
   ```
   > Task 1 阶段 `ArtifactVersion` 仍是 `markdown: String`，所以从 `version.markdown` 包成 `Markdown` payload。Task 2 切 `ArtifactVersion.payload` 后改为 `.map(|version| version.payload.clone())`。
8. **`build_review_input`**（:2477）/ **`build_revision_input`**（:2550）：`let artifact = self.session.artifact.clone().unwrap_or_default();` 当前 `artifact: String`。切 union 后读 payload 的 markdown：
   ```rust
   let artifact = match &self.session.artifact {
       Some(ArtifactPayload::Markdown { markdown, .. }) => markdown.clone(),
       Some(ArtifactPayload::WorkItemPlanCandidate { .. }) => String::new(), // WP3 会改 build_work_item_plan_review_input
       None => String::new(),
   };
   ```
   > WP3 会新增 `build_work_item_plan_review_input` 替代此分支对 WorkItemPlan 的处理；本 WP 对 candidate 变体返回空字符串（本 WP 不产生 candidate 数据，此分支不触发）。
9. **`handle_author_decision` Reject**（:2110）：`self.session.artifact = None;` —— `Option<ArtifactPayload>` 的 `None`，无需改代码。
10. **顶部 `use`**（:1-33）：补 `ArtifactPayload` 导入：在 `use crate::web::workspace_ws_types::{...}` 块加 `ArtifactPayload`。

- [ ] **Step 1.6：event forwarder 适配**

`src/web/workspace_ws_handler.rs:270-274`：

```rust
                EngineEvent::ArtifactUpdate { version, payload } => WsOutMessage::ArtifactUpdate {
                    version,
                    payload,
                },
```

> 顶部 `use`（:31-34）补 `ArtifactPayload`（若 forwarder 不直接构造 payload 只透传，则无需导入 `ArtifactPayload`，但 `WsOutMessage::ArtifactUpdate { version, payload }` 的 `payload` 类型来自 `EngineEvent`，透传即可）。先 `cargo check` 看是否需要补 import。

- [ ] **Step 1.7：checkpoint_store 适配（Task 3 前置决策）**

**决策**：`checkpoint_store.create_checkpoint` 的 `artifact_snapshot` 与 `CheckpointRecord.artifact_snapshot` 切 `ArtifactPayload`，与 `session.artifact` 类型对齐。这样 `handle_rollback` 直接 `session.artifact = Some(target.artifact_snapshot)` 无需转换。

`src/product/checkpoint_store.rs`：
- `grep -n "artifact_snapshot" src/product/checkpoint_store.rs` 定位全部命中。
- `create_checkpoint` 签名：`artifact_snapshot: &str` → `artifact_snapshot: &ArtifactPayload`（或 `Option<&ArtifactPayload>`，以现有签名为准——若当前是 `&str` 非空，改为 `&ArtifactPayload`）。
- `CheckpointRecord.artifact_snapshot: String` → `ArtifactPayload`。
- 读点（`restore` / `handle_rollback` 读取处）适配。
- 顶部 `use` 补 `crate::web::workspace_ws_types::ArtifactPayload`（或从 `crate::product::models`，以 `ArtifactPayload` 实际定义模块为准——WP1 定义在 `workspace_ws_types.rs`）。

> ⚠️ `checkpoint_store` 是否在 `src/product/` 下、是否依赖 `web::workspace_ws_types`？若 `product` 模块不能依赖 `web` 模块（架构分层），需把 `ArtifactPayload` 下沉到 `src/product/models.rs`。**实现前先 `grep -rn "use crate::web" src/product/` 确认 product 模块是否已依赖 web 模块。** 若 product 不依赖 web，把 `ArtifactPayload` 从 `workspace_ws_types.rs` 移到 `src/product/models.rs`（WP1 定义位置调整），或在 `product` 模块定义等价类型。**这是潜在架构决策点——若遇到，停下来与维护者确认。**

- [ ] **Step 1.8：测试夹具迁移**

`src/product/workspace_engine.rs` 测试夹具：所有 `session.artifact = Some("# ...".to_string())` → `session.artifact = Some(ArtifactPayload::Markdown { markdown: "# ...".to_string(), diff: None })`；所有 `EngineEvent::ArtifactUpdate { markdown, .. }` → `EngineEvent::ArtifactUpdate { payload, .. }`（解构后 `payload` 是 `ArtifactPayload`）；所有读 `session.artifact.clone().unwrap_or_default()`（String）→ `match &session.artifact { Some(ArtifactPayload::Markdown { markdown, .. }) => markdown.clone(), _ => String::new() }`；所有 `version.markdown.contains(...)` 保留（Task 1 阶段 ArtifactVersion 仍是 markdown）。

定位命令：
```bash
grep -n "session.artifact = Some\|session\.artifact\.clone()\|ArtifactUpdate {.*markdown\|markdown:.*session.artifact\|\.unwrap_or_default()" src/product/workspace_engine.rs
```
对每个命中行（:4785, :4810, :4875, :4908, :5466, :5527, :5581, :5641, :5681, :6199, :6240, :6254, :6337, :6421, :6476, :6905, :7413, :7555, :7607, :7665）机械适配。

> 建议用 `ArtifactPayload::Markdown { markdown: <原值>, diff: None }` 包裹模式。若夹具量大，可加一个 test-only helper：`fn md(s: &str) -> Option<ArtifactPayload> { Some(ArtifactPayload::Markdown { markdown: s.to_string(), diff: None }) }`，但**不要**为此重构测试结构——机械替换最稳。

- [ ] **Step 1.9：运行 Task 1 测试 + cargo check**

Run:
```
cargo test --locked --lib workspace_ws_types
cargo test --locked --lib workspace_engine
cargo check --locked
```
Expected:
- Step 1.1 的两个新测试 PASS
- 现有 `workspace_engine` 测试全绿（夹具已迁移，行为等价）
- `cargo check --locked` 全绿（所有消费点已适配）

> 若 `cargo check` 报其他文件还有 `ArtifactUpdate { version, markdown, diff }` 构造点或 `session.artifact = Some(String)` 点，`grep -rn` 定位并适配——这些是类型变更的必要适配，属本 Task 范围。

- [ ] **Step 1.10：提交**

```bash
git add src/web/workspace_ws_types.rs src/product/workspace_engine.rs src/web/workspace_ws_handler.rs src/product/checkpoint_store.rs
git commit -m "refactor(WP2a): artifact 链路消息层+内存层切 ArtifactPayload union"
```

---

## Task 2：版本持久化层切 union（`ArtifactVersion` / `ArtifactVersionSummary`）

**目标**：把 `ArtifactVersion.markdown: String` 切到 `payload: ArtifactPayload`；`ArtifactVersionSummary` 的 `markdown_size`/`markdown_preview` 按 payload 变体派生（字段名保留，向后兼容前端）；`build_artifact_version_summary` 改造；`update_artifact` 内部 `ArtifactVersion` 构造改用 `payload`；`new_persistent` 恢复改用 `version.payload`。

**Files:**
- Modify: `src/web/workspace_ws_types.rs`（`ArtifactVersion`、`ArtifactVersionSummary`）
- Modify: `src/product/workspace_engine.rs`（`build_artifact_version_summary`、`update_artifact` 的 ArtifactVersion 构造、`new_persistent`、`persist_artifact_versions` 序列化、测试夹具 `ArtifactVersion { markdown }` 构造点）

**Interfaces:**
- Consumes: Task 1 的 `ArtifactPayload`。
- Produces: `ArtifactVersion { version, payload: ArtifactPayload, ... }`；`ArtifactVersionSummary` 的 `markdown_size`/`markdown_preview` 对 candidate 变体填派生值。

- [ ] **Step 2.1：写失败测试 —— ArtifactVersion 携带 payload + summary 对两变体派生**

在 `src/web/workspace_ws_types.rs` 的 `#[cfg(test)] mod tests` 末尾追加：

```rust
    #[test]
    fn artifact_version_roundtrips_with_markdown_payload() {
        let v = ArtifactVersion {
            version: 1,
            payload: ArtifactPayload::Markdown {
                markdown: "# Story".to_string(),
                diff: None,
            },
            generated_by: ProviderName::ClaudeCode,
            reviewed_by: None,
            review_verdict: None,
            confirmed_by: None,
            is_current: true,
            created_at: "2026-06-17T00:00:00Z".to_string(),
            source_node_id: "N05".to_string(),
        };
        let json = serde_json::to_string(&v).expect("serialize");
        assert!(json.contains("\"markdown\":\"# Story\""));
        assert!(!json.contains("\"payload\""));
        let back: ArtifactVersion = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(v, back);
    }
```

在 `src/product/workspace_engine.rs` 的 `#[cfg(test)] mod tests` 末尾追加 summary 派生测试：

```rust
    #[test]
    fn build_artifact_version_summary_derives_size_for_markdown_and_candidate() {
        let md_version = ArtifactVersion {
            version: 1,
            payload: ArtifactPayload::Markdown {
                markdown: "# 标题".to_string(),
                diff: None,
            },
            generated_by: ProviderName::ClaudeCode,
            reviewed_by: None,
            review_verdict: None,
            confirmed_by: None,
            is_current: true,
            created_at: "2026-06-17T00:00:00Z".to_string(),
            source_node_id: "N05".to_string(),
        };
        let summary = build_artifact_version_summary(&md_version);
        assert_eq!(summary.markdown_size, "# 标题".len());
        assert!(summary.markdown_preview.contains("标题"));

        let candidate = WorkItemPlanCandidateDto {
            plan: WorkItemPlanDto {
                id: "plan_1".to_string(),
                status: "draft".to_string(),
                options: WorkItemSplitOptionsDto {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                dependency_graph: Vec::new(),
            },
            work_items: Vec::new(),
            verification_plans: Vec::new(),
            repository_profile: None,
            validator_findings: Vec::new(),
        };
        let candidate_json = serde_json::to_string(&candidate).unwrap();
        let candidate_version = ArtifactVersion {
            version: 2,
            payload: ArtifactPayload::WorkItemPlanCandidate {
                candidate,
            },
            generated_by: ProviderName::ClaudeCode,
            reviewed_by: None,
            review_verdict: None,
            confirmed_by: None,
            is_current: true,
            created_at: "2026-06-17T00:00:00Z".to_string(),
            source_node_id: "WORK_ITEM_PLAN".to_string(),
        };
        let summary = build_artifact_version_summary(&candidate_version);
        assert_eq!(summary.markdown_size, candidate_json.len());
        assert!(!summary.markdown_preview.is_empty());
    }
```

> 实现者注意：`WorkItemPlanCandidateDto` 等类型需在 engine.rs test mod `use` 中从 `crate::web::workspace_ws_types` 导入。candidate 构造较繁琐，可简化测试——只断言 `markdown_size > 0` 且 `markdown_preview` 非空，不强求精确值。若构造太繁，简化为：`let payload = ArtifactPayload::WorkItemPlanCandidate { candidate: <minimal> };` 断言 summary 派生非空。

- [ ] **Step 2.2：运行测试，确认失败**

Run: `cargo test --locked --lib workspace_ws_types && cargo test --locked --lib workspace_engine`
Expected: 编译失败——`ArtifactVersion` 无 `payload` 字段（仍是 `markdown`）。

- [ ] **Step 2.3：`ArtifactVersion` 切 payload**

`src/web/workspace_ws_types.rs:423-435`：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactVersion {
    pub version: u32,
    #[serde(flatten)]
    pub payload: ArtifactPayload,
    pub generated_by: ProviderName,
    pub reviewed_by: Option<ProviderName>,
    pub review_verdict: Option<ReviewVerdictType>,
    pub confirmed_by: Option<String>,
    #[serde(default = "default_true")]
    pub is_current: bool,
    pub created_at: String,
    pub source_node_id: String,
}
```

> `#[serde(flatten)] payload` 让 `ArtifactVersion` 的 JSON 扁平（`markdown`/`candidate` 与 `version`/`generated_by` 并列，无 `payload` 包裹）。与 `ArtifactUpdate` 的 flatten 一致。`ArtifactVersion` 用于持久化（`persist_artifact_versions` 写 JSON 文件）——flatten 后持久化形态变化（`markdown` 字段替代 `payload`），但 Task 1 阶段 `ArtifactVersion` 还是 `markdown: String`，持久化形态是 `{"version":1,"markdown":"...","generated_by":...}`；切 payload + flatten 后是 `{"version":1,"markdown":"...","generated_by":...}`——**形态相同**（Markdown 变体），向后兼容旧持久化文件。candidate 变体持久化为 `{"version":1,"candidate":{...},...}`。

- [ ] **Step 2.4：`ArtifactVersionSummary` 派生（字段名保留）**

`src/web/workspace_ws_types.rs:455-467` 字段名 `markdown_size`/`markdown_preview` 保留（向后兼容前端）。构造逻辑在 engine.rs `build_artifact_version_summary`（:98-111）按 payload 变体派生：

`src/product/workspace_engine.rs:98-111`：
```rust
fn build_artifact_version_summary(version: &ArtifactVersion) -> ArtifactVersionSummary {
    let (size, preview) = match &version.payload {
        ArtifactPayload::Markdown { markdown, .. } => {
            (markdown.len(), preview(markdown))
        }
        ArtifactPayload::WorkItemPlanCandidate { candidate } => {
            // candidate 变体：size = JSON 序列化长度，preview = plan.id 或首个 work_item title
            let json = serde_json::to_string(candidate).unwrap_or_default();
            let preview = candidate.work_items.first()
                .map(|wi| wi.title.clone())
                .unwrap_or_else(|| candidate.plan.id.clone());
            (json.len(), preview)
        }
    };
    ArtifactVersionSummary {
        version: version.version,
        generated_by: version.generated_by.clone(),
        reviewed_by: version.reviewed_by.clone(),
        review_verdict: version.review_verdict.clone(),
        confirmed_by: version.confirmed_by.clone(),
        is_current: version.is_current,
        created_at: version.created_at.clone(),
        source_node_id: version.source_node_id.clone(),
        markdown_size: size,
        markdown_preview: preview,
    }
}
```

> `preview` 函数（现有，对 markdown 截断）以实际定义为准——`grep -n "fn preview" src/product/workspace_engine.rs`。candidate 的 preview 用首个 work_item title 或 plan.id（review 展示用，WP7 前端会细化）。

- [ ] **Step 2.5：`update_artifact` 的 ArtifactVersion 构造改用 payload**

Task 1 Step 1.5.3 里 `update_artifact` 临时从 payload 取 markdown 存 `ArtifactVersion { markdown }`。Task 2 把 `ArtifactVersion` 切 `payload` 后，改为：

```rust
       pub async fn update_artifact(&mut self, payload: ArtifactPayload) {
           self.session.artifact = Some(payload.clone());
           for version in &mut self.artifact_versions {
               version.is_current = false;
           }
           let version = self.artifact_versions.len() as u32 + 1;
           let source_node_id = self
               .active_node_id
               .clone()
               .unwrap_or_else(|| "timeline_node_unknown".to_string());
           self.artifact_versions.push(ArtifactVersion {
               version,
               payload: payload.clone(),
               generated_by: self.session.author_provider.clone(),
               reviewed_by: None,
               review_verdict: None,
               confirmed_by: None,
               is_current: true,
               created_at: chrono::Utc::now().to_rfc3339(),
               source_node_id,
           });
           self.persist_artifact_versions();
           // ... persist_artifact_ref + event_tx.send 不变 ...
           let _ = self
               .event_tx
               .send(EngineEvent::ArtifactUpdate {
                   version,
                   payload,
               })
               .await;
       }
```

> 删除 Task 1 临时加的 `let markdown = match &payload { ... }` 分支。

- [ ] **Step 2.6：`new_persistent` 恢复改用 version.payload**

Task 1 Step 1.5.7 里 `new_persistent` 临时 `session.artifact = ...map(|version| ArtifactPayload::Markdown { markdown: version.markdown.clone(), ... })`。Task 2 改为：

```rust
           session.artifact = persisted_artifact_versions
               .iter().rev()
               .find(|version| version.is_current)
               .map(|version| version.payload.clone());
```

- [ ] **Step 2.7：`persist_artifact_versions` 适配**

`grep -n "fn persist_artifact_versions" src/product/workspace_engine.rs` 定位。该函数把 `artifact_versions` 序列化持久化。`ArtifactVersion` 字段变了（`markdown` → `payload`），序列化形态对 Markdown 变体等价（flatten），函数体可能无需改（只是写 `Vec<ArtifactVersion>`）。`cargo check` 确认。

- [ ] **Step 2.8：测试夹具迁移**

`src/product/workspace_engine.rs` 测试中所有 `ArtifactVersion { markdown: "...", ... }` 构造 → `ArtifactVersion { payload: ArtifactPayload::Markdown { markdown: "...", diff: None }, ... }`；所有 `version.markdown.contains(...)` / `version.markdown.len()` → `match &version.payload { ArtifactPayload::Markdown { markdown, .. } => markdown..., _ => ... }`。

定位：`grep -n "ArtifactVersion {" src/product/workspace_engine.rs`（:6905 等命中点）。

- [ ] **Step 2.9：运行 Task 2 测试 + 收口**

Run:
```
cargo test --locked --lib workspace_ws_types
cargo test --locked --lib workspace_engine
cargo check --locked
```
Expected: Step 2.1 测试 PASS；现有测试全绿；`cargo check` 全绿。

- [ ] **Step 2.10：提交**

```bash
git add src/web/workspace_ws_types.rs src/product/workspace_engine.rs
git commit -m "refactor(WP2a): ArtifactVersion/Summary 切 ArtifactPayload union（派生 size/preview）"
```

---

## Task 3：WP2a 收口验证（全量回归）

**目标**：跑完整验证链，确保 union 化未破坏 Story/Design/WorkItem 既有流程；确认 serde 往返与 flatten JSON 形态符合方案。

**Files:** 无新增改动；仅运行验证命令。若验证暴露真实缺陷，回到对应 Task 修复。

- [ ] **Step 3.1：全量验证链**

Run（依次，任一失败即停并修复）:
```
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked --lib workspace_ws_types
cargo test --locked --lib workspace_engine
cargo test --locked --test it_web
```
Expected: 全绿。

> `cargo test --locked --test it_web` 覆盖 Story/Design/WorkItem generate/confirm/review 的 WS 与 HTTP 流程，是 union 化最大的回归保障。若 it_web 有夹具直接构造 `ArtifactUpdate`/`session.artifact`，适配之（grep 定位）。

- [ ] **Step 3.2：确认 WorkItemPlan prepare 仍工作（WP1 成果未破坏）**

Run:
```
cargo test --locked --test it_web prepare_work_item_plan_creates_draft_plan_and_session_without_generating
```
Expected: PASS（WP1 的 prepare 不触及 artifact 链路，但确认无误）。

- [ ] **Step 3.3：交付摘要（供 WP2b 前置交付摘要使用）**

commit 后，把以下内容写入 WP2b plan 的「前置交付摘要」章节：

- `ArtifactPayload` 已挂载到：`WsOutMessage::ArtifactUpdate { version, #[serde(flatten)] payload }`、`SessionState.artifact: Option<ArtifactPayload>`、`EngineEvent::ArtifactUpdate { version, payload }`、`WorkspaceSession.artifact: Option<ArtifactPayload>`、`ArtifactVersion { #[serde(flatten)] payload }`、`CheckpointRecord.artifact_snapshot: ArtifactPayload`。
- `update_artifact(&mut self, payload: ArtifactPayload)` 接收 union——WP2b 传 `ArtifactPayload::WorkItemPlanCandidate { candidate }` 推送 candidate。
- `build_artifact_version_summary` 对 `WorkItemPlanCandidate` 变体派生 `markdown_size`（JSON 长度）+ `markdown_preview`（首个 work_item title 或 plan.id）。
- `build_review_input` / `build_revision_input` 当前对 `WorkItemPlanCandidate` 变体返回空字符串——WP3 会新增 `build_work_item_plan_review_input` 替代。
- JSON 形态（设计方案 :339-348）：`ArtifactUpdate` → `{"type":"artifact_update","version":N,"markdown":"..."|"candidate":{...}}`（扁平，无 payload 包裹）。
- **WP2b 待办**：产生 `ArtifactPayload::WorkItemPlanCandidate` 数据（author run 调 `WorkItemSplitEngine::generate` → 组装 candidate DTO → `update_artifact(ArtifactPayload::WorkItemPlanCandidate { candidate })`）；实现 `replace_issue_work_item_plan_candidate`；新增 `ProviderRunKind::WorkItemPlanAuthor`。

---

## Self-Review（写完后的自查）

**1. Spec 覆盖**（对照总览 v1.1 WP2a 目标/写入范围/验证 + 设计方案 :204-213）：
- ✅ `WsOutMessage::ArtifactUpdate` 切 union → Task 1 Step 1.3
- ✅ `SessionState.artifact` 切 union → Task 1 Step 1.4
- ✅ `EngineEvent::ArtifactUpdate` 切 union → Task 1 Step 1.5.2
- ✅ `WorkspaceSession.artifact` 切 union → Task 1 Step 1.5.1
- ✅ `ArtifactVersion` 切 union → Task 2 Step 2.3
- ✅ `ArtifactVersionSummary` 派生 → Task 2 Step 2.4（字段名保留，兼容前端）
- ✅ Story/Design/WorkItem 行为等价 → Task 1/2 测试夹具迁移 + Task 3 回归
- ✅ WorkItemPlanCandidate 变体类型就位（不产生数据）→ Task 1/2 的 candidate 分支填占位值，WP2b 产生数据
- ✅ JSON 扁平形态（方案 :339-348）→ Task 1 Step 1.3 `#[serde(flatten)]` + Step 1.1 测试断言
- ✅ 验证命令链 → Task 3
- ✅ 不做项：未实现 author run（WP2b）、未实现 replace candidate（WP2b）、未改前端、未产生 candidate 数据——均在「不做」清单标注。

**2. Placeholder 扫描**：
- 无「TBD/TODO」；每个 step 给真实代码或精确 grep 定位。
- checkpoint_store 适配（Step 1.7）给出「先 grep 确认 artifact_snapshot 全部命中点 + 决策切 ArtifactPayload」的明确路径，并标注架构分层潜在决策点（product 是否能依赖 web 模块）——这不是占位符，是真实的架构约束，需实现时确认。
- `build_artifact_version_summary` 的 candidate preview 逻辑给出具体实现（首个 work_item title 或 plan.id）。
- `WsProviderConfig` 字段以实际为准——给出 grep 定位指引，非占位符。

**3. 类型一致性**：
- `ArtifactPayload` 两变体（`Markdown { markdown, diff }` / `WorkItemPlanCandidate { candidate }`）在 WP1 定义，本 WP 全程引用一致。
- `update_artifact(payload: ArtifactPayload)` 签名在 Task 1 定义，Task 2 沿用。
- `ArtifactVersion.payload` 在 Task 2 定义，`new_persistent` / `build_artifact_version_summary` / 测试夹具一致引用。
- `EngineEvent::ArtifactUpdate { version, payload }` 在 Task 1 定义，event forwarder 透传 `payload` 一致。

**4. 边界风险**：
- **checkpoint_store 架构分层**（Step 1.7）：`product` 模块依赖 `web::workspace_ws_types::ArtifactPayload` 可能违反分层。已要求实现前 `grep -rn "use crate::web" src/product/` 确认；若违反，需把 `ArtifactPayload` 下沉到 `src/product/models.rs`（牵连 WP1 已写定义）。**这是本 WP 最大的架构风险点**——若遇到，停下来与维护者确认，可能需要把 WP1 的 `ArtifactPayload` 定义位置调整（从 `workspace_ws_types.rs` 移到 `models.rs`），这会回传影响 WP1 plan。已在 Step 1.7 显式标注。
- **flatten + untagged serde 兼容性**（Step 1.3）：serde 对 `flatten` + `untagged` 组合有已知 quirks（反序列化时字段归属判定）。给出 fallback（改为 `markdown?/diff?/candidate?` 三可选字段 + 自定义序列化保证互斥）。Step 1.6 测试若不过用 fallback。
- **持久化向后兼容**（Step 2.3）：`ArtifactVersion` flatten 后，旧持久化文件（`markdown: String` 形态）能否被新类型反序列化？Markdown 变体 flatten 形态与旧形态一致（`{"version":1,"markdown":"...","generated_by":...}`），应兼容。但 `diff` 字段旧文件没有（`Option` + `default`），兼容。实现时跑一次现有 `new_persistent` 测试确认旧 fixture 能恢复。
- **测试夹具量大**（Step 1.8/2.8）：engine.rs 十几个夹具机械迁移，易漏。给出 grep 命令定位全部命中行号，逐行适配；`cargo check` 会捕获遗漏。

---

## Execution Handoff

本 WP2a plan 已保存至 `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_WP2a_后端artifact_union挂载_v1.0.md`。

**执行方式（待用户选择）：**

1. **Subagent-Driven（推荐）** — 每个 Task 派一个 fresh subagent 实现，Task 间做 two-stage review。
2. **Inline Execution** — 在当前 session 按 superpowers:executing-plans 批量执行，带 checkpoint 审查。

**完成后**：交付 WP2a 后，按同样标准继续 WP2b（后端 author run + replace candidate，依赖 WP2a 的 union 挂载）。WP2b 的「前置交付摘要」直接引用本 plan Task 3 Step 3.3 的产出。

**⚠️ 实现前注意**：Step 1.7 的 checkpoint_store 架构分层确认是本 WP 的前置决策点——若 `product` 模块不能依赖 `web` 模块，需先把 `ArtifactPayload` 定义下沉到 `src/product/models.rs`（回传调整 WP1 plan 的定义位置），再开始 Task 1。建议执行者第一步就跑 `grep -rn "use crate::web" src/product/` 确认。
