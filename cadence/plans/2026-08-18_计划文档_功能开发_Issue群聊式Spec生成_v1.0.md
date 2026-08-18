# Issue 群聊式 Spec 生成（Group Chat Workspace）实施计划 v1.0

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在不影响现有流水线的前提下，新增可开关的「群聊式」Issue → Story Spec → Design Spec 生成交互（多角色 agent 同聊天室协作，人类显式定稿）。

**Architecture:** 独立 `group_chat_engine`（时间线事件溯源 + triage 路由 + seen-cursor HOLD + 草稿槽 claim + 窗口化上下文），与 `workspace_engine` 目录级隔离；定稿写现有 `LifecycleStore` 并创建桥接 workspace session 使老看板零改动可见；Web 侧独立 HTTP/WS 模块与新 `ChatRoomPage`。

**Tech Stack:** Rust（axum/tokio/serde）、React 18 + TS + Vite + Vitest、现有 StreamingProviderAdapter / LifecycleStore / json_store。

**设计依据:** `cadence/designs/2026-08-18_方案设计_Issue群聊式Spec生成_v1.3.1.md`（下称「设计」§N 引用该文档章节）。

## Global Constraints

- 🔴 本分支对 `src/web/handlers/lifecycle.rs` **零改动**（设计 §6.2/§11.2）；handler 收敛推迟到 add-monorepo 合并后。
- 🔴 不改 `SpecVersionRecord` / `AppendSpecVersionInput` 结构；`WorkspaceSessionRecord` / `WorkspaceSessionSummaryRecord` 仅允许 `#[serde(default)]` 可选增量字段。
- 🔴 共享文件改动一律 append 式 / 独立模块（设计 §11.2）；禁止触碰 `workspace_ws_handler/socket.rs`。
- 构建/测试遵循 `cadence/project-rules/build-test-commands.md`：标准 `cargo test` / `cargo clippy --all-targets --locked -- -D warnings`，**禁止 `-j 1`**。
- 每任务结束必须 `cargo test`（涉及模块定向）+ commit；前端任务跑 `cd web && npx vitest run <file>`。
- 全部新增代码注释/文档用中文；用户可见文案用中文。
- 分支：`.worktrees/feat-b-0818-rewrite-spec-workspace`，完成后 push。

---

## 文件结构总览

**Rust 新增：**
- `src/product/group_chat_engine/{mod.rs,types.rs,roles.rs,timeline.rs,context.rs,triage.rs,claims.rs,agent_turn.rs,coordinator.rs,finalize.rs,prompts.rs}`
- `src/product/group_chat_store.rs`（timeline.jsonl + session.json 持久化，复用 json_store）
- `src/web/group_chat_ws_types/{mod.rs,in_.rs,out_.rs}`、`src/web/group_chat_ws_handler/{mod.rs,session.rs}`
- `src/web/handlers/group_chat.rs`

**Rust 修改（均为增量）：**
- `src/cross_cutting/streaming_provider/mod.rs`（`ProviderPermissionMode::ReadOnly` 变体）
- `src/cross_cutting/claude_code_provider/mod.rs`、`codex_provider/session.rs`（ReadOnly 映射，各 +2 行 match 臂）
- `src/cross_cutting/streaming_provider/fake.rs`（脚本化 fake）
- `src/product/issue_store.rs`（`update_description` + 修订历史）
- `src/product/models/workspace.rs`（`origin: Option<SessionOrigin>`）
- `src/product/app_paths.rs`（`group_chat_root` 等路径方法，追加）
- `src/product/mod.rs`、`src/web/app.rs`（模块注册 + 路由挂载）

**前端新增：** `web/src/pages/ChatRoomPage.tsx`、`web/src/components/chat-room/*`、`web/src/hooks/useGroupChatWs.ts`、`web/src/api/groupChat.ts`

**前端修改（隔离注入）：** `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`（单点条件渲染）、设置页（开关）

---

## 组 A：基础能力（无群聊概念，可独立交付）

### Task A1: `ProviderPermissionMode::ReadOnly` 变体

**Files:**
- Modify: `src/cross_cutting/streaming_provider/mod.rs`（枚举 +1 变体）
- Modify: `src/cross_cutting/claude_code_provider/mod.rs:351`、`src/cross_cutting/codex_provider/session.rs:74,98`（+match 臂）
- Test: `src/cross_cutting/streaming_provider/tests.rs`

**Interfaces:**
- Produces: `ProviderPermissionMode::ReadOnly`（serde `read_only`）；claude 映射 `"plan"`，codex 映射 `read-only` 沙箱审批串。

- [ ] **Step 1: 写失败测试**（tests.rs 追加）

```rust
#[test]
fn read_only_permission_mode_maps_to_provider_flags() {
    assert_eq!(
        serde_json::to_string(&ProviderPermissionMode::ReadOnly).unwrap(),
        "\"read_only\""
    );
    assert_eq!(super::super::claude_code_provider::permission_flag(&ProviderPermissionMode::ReadOnly), "plan");
}
```

- [ ] **Step 2: 运行验证失败** `cargo test read_only_permission_mode` → FAIL（无此变体）
- [ ] **Step 3: 实现**：枚举加 `ReadOnly`；两处 provider 映射各加一臂（claude `"plan"`、codex 复用现有审批串生成处加 `ReadOnly => "read-only"`）；若 provider 映射函数非 pub，提取为 `pub(crate) fn permission_flag`。
- [ ] **Step 4: 运行验证通过** `cargo test -p aria read_only` + `cargo clippy --all-targets --locked -- -D warnings`
- [ ] **Step 5: Commit** `git commit -m "feat(provider): ProviderPermissionMode 新增 ReadOnly 变体（claude plan / codex read-only）"`

### Task A2: issue_store `update_description` + 修订历史

**Files:**
- Modify: `src/product/issue_store.rs`（追加方法与模型）
- Test: 同文件 `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `IssueStore::update_description(&self, project_id: &str, issue_id: &str, description: String, revised_by: &str) -> Result<IssueRecord, ProductStoreError>`；`IssueDescriptionRevision { version: u32, description: String, revised_by: String, created_at: String }`（存 `<issue>/description_revisions.json`）。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn update_description_persists_and_appends_revision() {
    let (_tmp, store) = setup_store();
    let issue = seed_issue(&store);
    let updated = store
        .update_description(PROJECT_ID, &issue.id, "细化后的描述".into(), "user")
        .unwrap();
    assert_eq!(updated.description.as_deref(), Some("细化后的描述"));
    let revs = store.list_description_revisions(PROJECT_ID, &issue.id).unwrap();
    assert_eq!(revs.len(), 1);
    assert_eq!(revs[0].description, issue.description.clone().unwrap_or_default());
}
```

- [ ] **Step 2: 运行失败** `cargo test update_description` → FAIL
- [ ] **Step 3: 实现**：复用 `read_json/write_json`（json_store）；旧 description 入 revisions（append-only，v 从 1 递增），更新 `IssueRecord.description` 与 `updated_at`。
- [ ] **Step 4: 运行通过** `cargo test issue_store`
- [ ] **Step 5: Commit** `git commit -m "feat(issue-store): update_description 与 description 修订历史"`

### Task A3: product 层 Story→Design 确认校验函数（lifecycle.rs 零改动）

**Files:**
- Create: `src/product/lifecycle_store/derivation_guard.rs`
- Modify: `src/product/lifecycle_store/mod.rs`（`pub mod derivation_guard;`）
- Test: 同文件 tests

**Interfaces:**
- Consumes: `LifecycleStore` 既有 spec 查询（`list_versions` / entity 状态）。
- Produces: `pub fn validate_design_finalize_allowed(lifecycle: &LifecycleStore, project_id: &str, issue_id: &str, story_entity_id: &str) -> Result<(), DerivationGuardError>`，语义复制 `validate_confirmed_story_specs`（`src/web/handlers/lifecycle.rs:669`，**只读参考不修改**）。

- [ ] **Step 1: 写失败测试**：seed 未确认 Story → 断言 Err(DerivationGuardError::StorySpecNotConfirmed)；确认后 → Ok。
- [ ] **Step 2: 运行失败** `cargo test derivation_guard` → FAIL
- [ ] **Step 3: 实现**（对齐 handler 语义与错误文案 `story_spec_not_confirmed`）
- [ ] **Step 4: 运行通过** `cargo test derivation_guard`
- [ ] **Step 5: Commit** `git commit -m "feat(lifecycle-store): product 层派生确认校验（供群聊引擎，handler 不动）"`

### Task A4: 群聊模式应用级设置存储

**Files:**
- Create: `src/product/group_chat_engine/settings.rs`（仿 `src/product/image_create/settings_store.rs` 先例）
- Test: 同文件 tests

**Interfaces:**
- Produces: `SpecGenerationMode { Pipeline, GroupChat }`（serde snake_case，default=Pipeline）；`load_spec_generation_mode(paths: &AriaStatePaths) -> SpecGenerationMode` / `save_spec_generation_mode(...)`。

- [ ] **Step 1: 写失败测试**（缺文件默认 Pipeline；写入 GroupChat 后读回）
- [ ] **Step 2: 运行失败** → FAIL
- [ ] **Step 3: 实现**（json 单文件 `spec_generation_mode.json`）
- [ ] **Step 4: 运行通过** `cargo test spec_generation_mode`
- [ ] **Step 5: Commit** `git commit -m "feat(group-chat): Spec 生成模式应用级设置存储（默认流水线）"`

### Task A5: 脚本化 FakeStreamingProvider（测试设施）

**Files:**
- Modify: `src/cross_cutting/streaming_provider/fake.rs`
- Test: `src/cross_cutting/streaming_provider/tests.rs`

**Interfaces:**
- Consumes: 现有 `FakeStreamingProvider` / `StreamingProviderAdapter::start`（`mod.rs:280`，带 CancellationToken）。
- Produces: `ScriptedFakeProvider::new(scripts: Vec<ScriptedReply>)`，`ScriptedReply { match_prompt_contains: String, events: Vec<ProviderEvent> }`；按 prompt 匹配依次返回脚本事件流，未匹配返回默认。

- [ ] **Step 1: 写失败测试**：两段脚本分别匹配 "author" / "reviewer"，断言各自事件流按序到达、尊重 cancel。
- [ ] **Step 2: 运行失败** → FAIL
- [ ] **Step 3: 实现**（复用现有 fake 的 mpsc 事件通道骨架，加脚本匹配层）
- [ ] **Step 4: 运行通过** `cargo test scripted_fake`
- [ ] **Step 5: Commit** `git commit -m "test(streaming-provider): 脚本化 FakeStreamingProvider（按 prompt 匹配多角色应答）"`

---

## 组 B：群聊引擎核心（纯 product 层，无 web 依赖）

### Task B1: 引擎数据模型（types.rs）

**Files:**
- Create: `src/product/group_chat_engine/types.rs` + `mod.rs`（模块骨架）
- Modify: `src/product/mod.rs`（`pub mod group_chat_engine;`）
- Test: `src/product/group_chat_engine/tests/types.rs`

**Interfaces:**
- Produces（设计 §4）：
  - `GroupChatRoleKey { Author, FrontendDesign, BackendDesign, Reviewer, Researcher }`（serde snake_case）
  - `RoleInstance { id: String, role_key, provider: ProviderName, display_name: String, permission_mode: ProviderPermissionMode, seen_cursor: u64, injection_watermark: u64 }`
  - `DraftSlotKey`（`issue_full | story_full | design_frontend | design_backend | design_summary`，字符串新类型）
  - `ArtifactDraft { version: u32, markdown: String, author_role_id: String, based_on_events: u64 }`
  - `Claim { holder_role_id: String, claimed_at: String }`
  - `DraftSlot { slot_key: DraftSlotKey, current: Option<ArtifactDraft>, claim: Option<Claim> }`
  - `ArtifactLineKind { IssueRefinement, StorySpec, DesignSpec }`；`ArtifactLine { kind, drafts: Vec<DraftSlot>, finalized_versions: Vec<String> }`
  - `RoomEvent`（serde tag="type"）：`UserMessage { text, mentions } / AgentMessage { role_instance_id, text, artifact_ref: Option<ArtifactRef> } / ClaimEvent / HeldEvent { reason } / FinalizeEvent { line, version, included_slots } / SystemNotice { text }`
  - `GroupChatSessionRecord { id, project_id, issue_id, status: Active|Finalized|Archived, roles: Vec<RoleInstance>, artifact_lines: Vec<ArtifactLine>, created_at, updated_at }`

- [ ] **Step 1: 写失败测试**（serde round-trip：全部枚举/记录序列化→反序列化相等；`RoomEvent` tag 值断言）
- [ ] **Step 2: 运行失败** `cargo test group_chat_types` → FAIL
- [ ] **Step 3: 实现全部类型**（纯数据定义 + serde derive）
- [ ] **Step 4: 运行通过**
- [ ] **Step 5: Commit** `git commit -m "feat(group-chat): 引擎数据模型（角色/草稿槽/产物线/时间线事件）"`

### Task B2: 时间线存储（timeline.rs + group_chat_store.rs）

**Files:**
- Create: `src/product/group_chat_engine/timeline.rs`、`src/product/group_chat_store.rs`
- Modify: `src/product/app_paths.rs`（追加 `group_chat_session_root(project_id, issue_id, session_id)`）
- Test: `src/product/group_chat_engine/tests/timeline.rs`

**Interfaces:**
- Consumes: `read_json/write_json/list_json_records`（json_store）、`ProductAppPaths`。
- Produces: `GroupChatStore::append_event(...) -> Result<u64 /*seq*/>`（timeline.jsonl 追加，fsync）；`load_session(...)`（先读 session.json 缓存，再重放 timeline 覆盖 seen_cursor/注入水位，设计 §4.3 写入顺序）；`save_session_snapshot(...)`。

- [ ] **Step 1: 写失败测试**：append 3 事件→重放→cursor 与事件内值一致；伪造「事件已写、快照未写」崩溃态→重放恢复一致。
- [ ] **Step 2: 运行失败** → FAIL
- [ ] **Step 3: 实现**（每事件自增 seq；append 成功后才写 session.json）
- [ ] **Step 4: 运行通过** `cargo test group_chat_store`
- [ ] **Step 5: Commit** `git commit -m "feat(group-chat): 时间线事件溯源存储（timeline.jsonl + 快照缓存 + 崩溃重放）"`

### Task B3: 角色权限映射（roles.rs）

**Files:**
- Create: `src/product/group_chat_engine/roles.rs`
- Test: 同模块 tests

**Interfaces:**
- Produces: `can_write_artifacts(role_key) -> bool`（Author/FrontendDesign/BackendDesign=true）；`writable_slots(role_key) -> Vec<DraftSlotKey>`（author→[issue_full, story_full, design_summary]，fe→[design_frontend]，be→[design_backend]，其余空）；`adapter_role_for(role_key) -> AdapterRole`（可写→Executor，只读→Reviewer）；`default_lineup() -> Vec<(GroupChatRoleKey, /*建议*/ )>`= author+reviewer+researcher；只读角色强制 `ProviderPermissionMode::ReadOnly`（A1）。

- [ ] **Step 1: 写失败测试**（映射表全覆盖断言 + 只读角色 permission 强制断言）
- [ ] **Step 2: 运行失败** → FAIL
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行通过**
- [ ] **Step 5: Commit** `git commit -m "feat(group-chat): 角色写权限/草稿槽/AdapterRole 映射与默认阵容"`

### Task B4: 窗口化上下文组装（context.rs）

**Files:**
- Create: `src/product/group_chat_engine/context.rs`
- Test: `src/product/group_chat_engine/tests/context.rs`

**Interfaces:**
- Consumes: RoomEvent 序列、RoleInstance、ArtifactLine。
- Produces: `assemble_turn_context(events, role, lines, budget_tokens) -> TurnContext { unread_events, summary, relevant_drafts }`（设计 §5.2a 四层）；`INJECTION_BUDGET_TOKENS: usize = 16_000`；截断优先级 人类消息>相关草稿>未读>摘要；**被截未读事件不推进 injection_watermark**；`maybe_update_rolling_summary`（每 20 事件压缩，摘要回调注入以便测试用小模型替身）。

- [ ] **Step 1: 写失败测试**：①超预算时按优先级截断且被截事件水位不推进；②被截事件进入下次摘要输入；③reviewer 视角含目标草稿 diff。
- [ ] **Step 2: 运行失败** → FAIL
- [ ] **Step 3: 实现**（token 估算用字符数/4 粗算，注释标明）
- [ ] **Step 4: 运行通过**
- [ ] **Step 5: Commit** `git commit -m "feat(group-chat): 窗口化上下文组装（注入水位 + 滚动摘要 + 16k 预算截断）"`

### Task B5: Triage 路由（triage.rs）

**Files:**
- Create: `src/product/group_chat_engine/triage.rs`
- Test: `src/product/group_chat_engine/tests/triage.rs`

**Interfaces:**
- Produces（设计 §5.1）：`TriageInput { triggering_seq: u64, last_speaker: Option<String>, room_state: RoomStateView, lines: Vec<ArtifactLine> }`；`TriageOutput { RespondTo(Vec<String>), NoOneNeedsToRespond }`；`TriageRouter` trait（`fn route(&self, input) -> TriageOutput`）；`RuleRouter`（职责关键词+产物线状态，**排除 last_speaker 自我路由**，上限 2 人）；`LlmRouter`（小模型调用，失败/超时 30s/不可解析→退化 RuleRouter）；`NoOneCounter`（随新触发事件重置，推进记 SystemNotice 事件）。

- [ ] **Step 1: 写失败测试**：①@必达绕过 triage（由 coordinator 测，本任务测 RuleRouter：design 话题路由到 fe/be）；②last_speaker 不被路由给自己；③LlmRouter 解析失败退化规则；④NoOne 两轮→SystemNotice 且新事件重置计数。
- [ ] **Step 2: 运行失败** → FAIL
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行通过**
- [ ] **Step 5: Commit** `git commit -m "feat(group-chat): triage 路由（规则兜底 + 小模型 + NoOne 自然终止 + 自我路由排除）"`

### Task B6: 认领机制（claims.rs）

**Files:**
- Create: `src/product/group_chat_engine/claims.rs`
- Test: 同模块 tests

**Interfaces:**
- Produces: `try_claim(line, slot_key, role_id) -> Result<(), ClaimError>`（校验 `writable_slots` 且槽未被占）；`release(...)`；`release_expired(now, timeout=10min) -> Vec<ClaimEvent>`；认领/释放产生 ClaimEvent。

- [ ] **Step 1: 写失败测试**：①author 与 fe 同槽互斥、不同槽并行；②只读角色 claim 被拒；③超时自动释放。
- [ ] **Step 2-5**: TDD 循环实现 → `cargo test claims` → Commit `feat(group-chat): 草稿槽原子认领（互斥/权限/超时释放）`

### Task B7: Agent Turn 执行（agent_turn.rs）

**Files:**
- Create: `src/product/group_chat_engine/agent_turn.rs`
- Test: `src/product/group_chat_engine/tests/agent_turn.rs`（用 A5 脚本化 fake）

**Interfaces:**
- Consumes: B3 映射、B4 TurnContext、StreamingProviderAdapter。
- Produces: `run_agent_turn(role, ctx, adapter) -> TurnOutcome`；freshness 门控：`publish_or_hold(events_len_at_start, outcome) -> Publish | Held { new_events }`（设计 §5.1 ③）；verbatim-dup 门控（§5.1 ④，不可绕过）；HOLD 重试上限 3 退避 1s/2s/4s，超限 HeldEvent(reason=retry_exhausted)；产出落盘推进 seen_cursor 与 injection_watermark。

- [ ] **Step 1: 写失败测试**：①正常产出→AgentMessage+cursor 推进；②快照后新事件→HeldEvent 重试；③与他人消息逐字相同→HOLD；④重试 3 次超限→retry_exhausted。
- [ ] **Step 2-5**: TDD 循环 → Commit `feat(group-chat): agent turn 执行（freshness/verbatim-dup 门控 + 重试退避）`

### Task B8: Coordinator 主编排（coordinator.rs）

**Files:**
- Create: `src/product/group_chat_engine/coordinator.rs`、`prompts.rs`（五条协作纪律 + 各角色 system prompt + 反附和条款，设计 §5.6）
- Test: `src/product/group_chat_engine/tests/coordinator.rs`

**Interfaces:**
- Consumes: B2-B7 全部。
- Produces: `Coordinator::on_user_message(text, mentions)`（@必达绕过 triage）；`on_agent_message(...)`（回 triage 循环）；熔断：`HARD_LOOP_CAP=12`（两次人类消息间 agent 消息总数）、发言预算（同触发事件同角色 ≤1 条）、空转检测（连续 4 条无新增参与者且无草稿槽变化）；并发：同 provider 信号量（默认 2）+ 500ms spawn 间隔 + rate-limit 60s 静默退避。

- [ ] **Step 1: 写失败测试**（脚本化 fake 全链路）：①用户消息→triage→author 发言→reviewer 被路由→循环至 NoOne；②HARD_LOOP_CAP 触发熔断 SystemNotice；③空转合取条件（v2/v3 迭代不误杀：有草稿版本变化不熔断）；④@reviewer 时仅 reviewer 发言。
- [ ] **Step 2-5**: TDD 循环 → Commit `feat(group-chat): coordinator 编排（循环熔断/发言预算/并发节奏/prompts）`

### Task B9: 定稿与看板桥接（finalize.rs）

**Files:**
- Create: `src/product/group_chat_engine/finalize.rs`
- Test: `src/product/group_chat_engine/tests/finalize.rs`

**Interfaces:**
- Consumes: A3 校验、`LifecycleStore::append_version`（`AppendSpecVersionInput`）、`LifecycleStore` session 创建/更新 API、`ArtifactVersion`（`src/web/workspace_ws_types/artifact_version.rs:9`，`ArtifactPayload::Markdown`）。
- Produces: `finalize_line(line_kind, slots, confirmed_by) -> Result<FinalizeEvent>`：
  1. 派生约束：DesignSpec 线先跑 A3 校验（Err→`story_spec_not_confirmed`）；
  2. **顺序固定**：先 entity 维度 `append_version`（溯源字段 §6.4），后建/更新桥接 session；
  3. 桥接：同线首次定稿创建 `WorkspaceSessionRecord`（status=Confirmed、reviewer_provider=author 值、messages=[]、origin=Some(GroupChat)），向其 timeline 目录 `artifact_versions.json` 追加合成 `ArtifactVersion`（generated_by=汇总 author provider、source_node_id=FinalizeEvent seq）；后续定稿复用同 session 追加；
  4. IssueRefinement 线走 A2 `update_description`（不写桥接）；
  5. 追加 FinalizeEvent（included_slots）。

- [ ] **Step 1: 写失败测试**：①Story 定稿→entity versions +1 且桥接 artifact_versions.json +1；②Design 在 Story 未确认时→Err(story_spec_not_confirmed)；③二次定稿复用同桥接 session（版本累积为 2）；④IssueRefinement→description 更新+修订历史+1。
- [ ] **Step 2-5**: TDD 循环 → Commit `feat(group-chat): 定稿流程（entity 版本 + 桥接 session artifact_versions + Issue 澄清）`

### Task B10: `origin` 来源标记字段

**Files:**
- Modify: `src/product/models/workspace.rs`（`WorkspaceSessionRecord` + SummaryRecord 各 +`#[serde(default)] pub origin: Option<SessionOrigin>`）
- Modify: DTO 暴露处（`workspace_session_summary_dto`，前端 types 同步）
- Test: models serde 兼容测试（老 json 无 origin 字段可反序列化）

- [ ] **Step 1: 写失败测试**：老格式 json（无 origin）→ 反序列化 origin=None；新写入 origin=Some(GroupChat)→DTO 含 origin。
- [ ] **Step 2-5**: TDD 循环 → Commit `feat(models): workspace session origin 来源标记（serde-default 可选）`

---

## 组 C：Web 后端

### Task C1: 群聊 HTTP handlers

**Files:**
- Create: `src/web/handlers/group_chat.rs`（仿现有 handler 模式：ProductStoreError→ApiError）
- Modify: `src/web/app.rs`（路由挂载，追加）
- Test: `tests/web_group_chat_api.rs`（仿 tests/web_* 模式）

**Interfaces:**
- Endpoints：`POST /api/group-chat/sessions`（按 issue 创建/幂等返回既有）、`GET /api/group-chat/sessions/:id`（含 timeline 分页）、`POST .../messages`（用户消息+mentions，驱动 coordinator）、`POST .../roles`（添加角色实例）、`POST .../finalize`（产物线定稿）、`GET/PUT /api/settings/spec-generation-mode`。

- [ ] **Step 1: 写失败测试**（创建→发消息→fake provider 应答→定稿→断言响应体）
- [ ] **Step 2-5**: TDD 循环 → Commit `feat(web): 群聊 HTTP API（会话/消息/角色/定稿/模式设置）`

### Task C2: 独立 WS endpoint + handler 模块

**Files:**
- Create: `src/web/group_chat_ws_types/{mod.rs,in_.rs,out_.rs}`、`src/web/group_chat_ws_handler/{mod.rs,session.rs}`
- Modify: `src/web/app.rs`（`/ws/group-chat` 挂载）
- Test: `src/web/group_chat_ws_handler/tests.rs`

**Interfaces:**
- In：`SendMessage { text, mentions } / AddRole { role_key, provider, display_name } / Finalize { line_kind, included_slots } / Ping`；Out：`RoomEvent(RoomEvent) / TurnStarted { role_instance_id } / TurnDelta { role_instance_id, delta } / TurnHeld { role_instance_id, reason } / Error { code, message }`。断线重连=重放 seq > client cursor 的事件。**禁止修改 `workspace_ws_handler/socket.rs`。**

- [ ] **Step 1: 写失败测试**（连接→SendMessage→流式 TurnDelta→RoomEvent 顺序断言；重连补发）
- [ ] **Step 2-5**: TDD 循环 → Commit `feat(web): 群聊独立 WS endpoint 与事件协议`

---

## 组 D：前端

### Task D1: API client + types + useGroupChatWs

**Files:**
- Create: `web/src/api/groupChat.ts`（+types）、`web/src/hooks/useGroupChatWs.ts`（模式对齐 useWorkspaceWs，独立协议）
- Test: `web/src/api/groupChat.test.ts`、`web/src/hooks/useGroupChatWs.test.ts`

- [ ] **Step 1-5**: TDD（mock fetch/WebSocket：消息收发、重连 cursor、错误帧）→ Commit `feat(web-ui): 群聊 API client 与 WS hook`

### Task D2: ChatRoomPage 时间线

**Files:**
- Create: `web/src/pages/ChatRoomPage.tsx`、`web/src/components/chat-room/{ChatRoomTimeline.tsx,RoomEventRow.tsx,MentionInput.tsx}`
- 复用（只读引用，不修改）：`chat-workspace/ChatEntryList.tsx`、`message-grouping.ts`、`text-display.ts`
- Test: `web/src/components/chat-room/ChatRoomTimeline.test.tsx`

**Interfaces:**
- 渲染 UserMessage/AgentMessage（角色头像+名牌）/HeldEvent/ClaimEvent/FinalizeEvent/SystemNotice（各 InlineEventRow 变体）；输入栏 @提及自动补全（角色实例列表）。

- [ ] **Step 1-5**: TDD（各事件类型渲染快照、@补全过滤、流式 TurnDelta 追加）→ Commit `feat(web-ui): ChatRoomPage 时间线与 @提及输入`

### Task D3: 产物线面板 + 定稿

**Files:**
- Create: `web/src/components/chat-room/{ArtifactLinePanel.tsx,DraftPreview.tsx,FinalizeButton.tsx}`
- Test: `web/src/components/chat-room/ArtifactLinePanel.test.tsx`

**Interfaces:**
- 三产物线状态卡（未开始/起草中 by whom/待审/可定稿/已定稿 vN）；DesignSpec 三槽展示；定稿按钮禁用态（Design 未满足前置时 tooltip「需先定稿 Story Spec」，以后端 `story_spec_not_confirmed` 错误为兜底）。

- [ ] **Step 1-5**: TDD（状态流转渲染、禁用态、草稿预览展开）→ Commit `feat(web-ui): 产物线面板与定稿交互`

### Task D4: 角色栏 + 添加角色

**Files:**
- Create: `web/src/components/chat-room/{RoleBar.tsx,AddRoleDialog.tsx}`
- 复用：ProviderConfigPanel 的 provider 选择子组件（只读引用；若不可拆则内联 provider 下拉，数据来自现有 providers API）
- Test: `web/src/components/chat-room/RoleBar.test.tsx`

- [ ] **Step 1-5**: TDD（默认阵容渲染、添加同角色多实例、provider 绑定展示）→ Commit `feat(web-ui): 角色栏与添加角色流程`

### Task D5: 看板集成 + 设置开关

**Files:**
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`（**单点条件渲染注入**：群聊模式渲染 `<GroupChatWorkbenchCard/>`，流水线模式保持原样；import + 一处分支，设计 §11.2）
- Create: `web/src/components/chat-room/GroupChatWorkbenchCard.tsx`（origin=GroupChat 的 session 入口→`/group-chat/:sessionId`）
- Modify: 设置页（Spec 生成模式开关，读 A4 API）
- Test: `web/src/components/lifecycle/IssueLifecycleWorkbench.groupchat.test.tsx`（新文件，不动现有测试）

- [ ] **Step 1-5**: TDD（两模式渲染互斥、origin 路由进 ChatRoomPage、开关读写）→ Commit `feat(web-ui): 看板群聊模式入口与应用设置开关`

---

## 组 E：端到端与收尾

### Task E1: 引擎集成场景套件

**Files:**
- Create: `tests/group_chat_engine_scenarios.rs`（脚本化 fake 驱动）

- [ ] **Step 1-5**: 场景：①澄清→Story 讨论→审稿→修订→定稿→Design 三槽→定稿 全链路；②崩溃恢复（timeline 重放一致性）；③prompt 注入样例（agent 发言含「忽略你的权限」指令→只读角色仍无法写槽）；④混合模式桥接（先流水线定稿 v1，群聊再定稿 v2，看板读取最新）→ Commit `test(group-chat): 引擎端到端场景套件`

### Task E2: 全量验证 + 文档收尾

- [ ] **Step 1**: `cargo test`（全量）+ `cargo clippy --all-targets --locked -- -D warnings` + `cd web && npx vitest run` 全绿
- [ ] **Step 2**: 手动验证清单（流水线模式回归：创建 Story/Design 走老流程无变化；群聊模式：建室→讨论→定稿→老看板可见版本历史→WorkItem 派生入口正常）
- [ ] **Step 3**: 更新 README「核心能力」一节（群聊模式简介 + 开关位置）
- [ ] **Step 4**: Commit `docs: 群聊式 Spec 生成使用说明` 并 push

---

## 依赖顺序

```text
A1 A2 A3 A4 A5（可并行，全独立）
  └─ B1 → B2 → B3 → B4 → B5 → B6 → B7 → B8 → B9（B9 依赖 A2/A3/B1-B8；B10 可在 B9 前任意点）
        └─ C1 C2（依赖 B 组 + B10）
              └─ D1 → D2 → D3 D4 → D5
                    └─ E1 → E2
```

## Self-Review 记录

- 覆盖核对：设计 §2(A1/B3) §4(B1/B2) §5.1-5.6(B4-B8) §6.1(A2) §6.2(A3) §6.4-6.5(B9/B10) §7(C1/C2/D1-D5) §8(A5/E1) §11(A3 零改动约束/D5 注入方式) —— 全部有任务承载。
- 占位符扫描：无 TBD/TODO；测试代码均给出断言主体。
- 类型一致性：`DraftSlotKey`/`RoomEvent`/`TriageOutput`/`origin` 等命名跨任务一致；`ArtifactVersion` 字段与 `src/web/workspace_ws_types/artifact_version.rs:9` 对齐。
