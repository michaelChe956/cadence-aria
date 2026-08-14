# SpecDesign 对话式修订与可选 Review 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Change:** `openspec/changes/spec-design-dialog-revision/`（proposal/design/specs/tasks 四份契约已获批，两轮独立审查问题全部闭环）

**Goal:** Story/Design workspace 的 spec 生成从"Accept 强制进 Review + Reject 推倒重来"改造为"AuthorConfirm 对话式反馈修订循环 + 确认双出口（送审/定稿）+ review 结果回对话流"。

**Architecture:** 全部改动落在交互链（workspace_engine + WS 协议 + web 前端）；状态机变为 `AuthorConfirm ⇄ Revision 循环 + 两个出口`；HumanConfirm/ReviewDecision 从 Story/Design 流程退役。daemon 链（`run_planning_full_chain`）不动。reviewer 降级为可选只读建议来源，provisional reviewer 快照机制支撑"配置=默认值、确认时可偏离"。

**Tech Stack:** Rust（tokio/serde）/ TypeScript + React（zustand store + WebSocket）/ vitest + Playwright。

## Global Constraints

- 标准验证四条（cadence/project-rules/build-test-commands.md）：`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo check --locked`、`cargo test --locked`；🔴 禁止 `cargo test -j 1`。
- 定向 lib 单测：`cargo test --locked --lib <过滤名>`；approval_bridge 用 `cargo test-approval-bridge`。
- 与 `feat-b-0808-add-monorepo` 分支冲突规避：新增逻辑一律走新函数/新文件；`decisions.rs` 只追加 match 臂不重写结构；`run/provider_run.rs` 只新增 match 臂与独立函数，不触碰既有 `match run_kind` 臂与 ReviewOnly 分支；不修改 `build_revision_full_prompt`/`build_revision_delta_prompt`；新测试放新文件，不塞 `part_*.rs`。
- 引用契约条款时以 `openspec/changes/spec-design-dialog-revision/specs/spec-design-dialog-revision/spec.md` 的 Requirement/Scenario 为验收依据。
- 前端测试命令：`cd web && pnpm tsc -b && pnpm test`。

## 任务与契约工作包映射

| Plan Task | 契约 tasks.md | 覆盖 Requirement |
|---|---|---|
| Task 1-3 | 1.1/1.2/2.1/2.2/2.2b | 修订循环、确认双出口（Scenario: 反馈触发/空拒绝/推倒移除/快照送审/无快照报错/Accept 兼容） |
| Task 4-5 | 2.3/2.4/2.5 | 多轮循环、review 结果回对话流（Scenario: 报告回 AuthorConfirm/pass 不自动定稿/定稿直接完成） |
| Task 6 | 3.1/3.2 | 存量会话恢复兼容 |
| Task 7 | 3.3 | 修订中断线恢复 |
| Task 8 | 4.1/4.1b/4.2/4.3 | 前端三动作（支撑全部场景的 UI 面） |
| Task 9 | 5.1/5.2/5.3 | 全量回归收尾 |

---

### Task 1: 协议层 AuthorDecision 新变体

**Files:**
- Modify: `src/web/workspace_ws_types/in_.rs:133-136`（AuthorDecision 枚举）
- Test: `src/web/workspace_ws_types/tests.rs`（追加 serde roundtrip 用例）

**Interfaces:**
- Produces: `AuthorDecision::Revise { feedback: String }`、`AuthorDecision::AcceptWithReview`、`AuthorDecision::AcceptFinalize`（serde snake_case：`"revise": {"feedback": "..."}`、`"accept_with_review"`、`"accept_finalize"`）；`Accept`/`Reject` 保留。Task 3 的 engine 分支与 Task 8 前端发送均使用这些变体。

- [ ] **Step 1: 写失败的 serde 测试**

在 `src/web/workspace_ws_types/tests.rs` 追加（跟随该文件现有 roundtrip 测试风格）：

```rust
#[test]
fn author_decision_new_variants_roundtrip() {
    let revise = serde_json::json!({"revise": {"feedback": "补充异常场景"}});
    let parsed: AuthorDecision = serde_json::from_value(revise.clone()).unwrap();
    assert_eq!(parsed, AuthorDecision::Revise { feedback: "补充异常场景".to_string() });
    assert_eq!(serde_json::to_value(&parsed).unwrap(), revise);

    for (raw, expected) in [
        (serde_json::json!("accept_with_review"), AuthorDecision::AcceptWithReview),
        (serde_json::json!("accept_finalize"), AuthorDecision::AcceptFinalize),
        (serde_json::json!("accept"), AuthorDecision::Accept),
        (serde_json::json!("reject"), AuthorDecision::Reject),
    ] {
        let parsed: AuthorDecision = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(parsed, expected);
        assert_eq!(serde_json::to_value(&parsed).unwrap(), raw);
    }
}
```

- [ ] **Step 2: 运行验证失败**

Run: `cargo test --locked --lib author_decision_new_variants`
Expected: FAIL（`Revise`/`AcceptWithReview`/`AcceptFinalize` 未定义，编译错误）

- [ ] **Step 3: 最小实现**

`src/web/workspace_ws_types/in_.rs:133-136` 改为：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum AuthorDecision {
    Accept,
    Reject,
    Revise { feedback: String },
    AcceptWithReview,
    AcceptFinalize,
}
```

- [ ] **Step 4: 运行验证通过**

Run: `cargo test --locked --lib author_decision_new_variants`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add src/web/workspace_ws_types/in_.rs src/web/workspace_ws_types/tests.rs
git commit -m "feat(ws): AuthorDecision 新增 Revise/AcceptWithReview/AcceptFinalize 变体（spec-design-dialog-revision T1）"
```

---

### Task 2: provisional 持久化闭环（四处落盘）

**Files:**
- Modify: `src/product/models/workspace.rs:48-70`（WorkspaceSessionRecord 加两字段）
- Modify: `src/product/workspace_engine/types.rs:74-91`（WorkspaceSession 加两字段）、`:94-122`（from_record 恢复）
- Modify: `src/product/workspace_engine/lifecycle.rs:639-661`（start_generation 写入 + 落盘调用）
- Test: `src/product/workspace_engine/tests/author_revision_loop.rs`（新建文件，serde 兼容 + from_record 用例）

**Interfaces:**
- Produces: `WorkspaceSessionRecord.provisional_reviewer_provider: Option<ProviderName>`、`WorkspaceSessionRecord.reviewer_enabled_at_start: Option<bool>`（均 `#[serde(default)]`）；`WorkspaceSession` 同名字段；`from_record` 恢复语义：旧 record 两字段为 None。Task 3 的判定逻辑消费这两个字段。

- [ ] **Step 1: 新建测试文件，写失败的持久化测试**

```rust
// src/product/workspace_engine/tests/author_revision_loop.rs
use crate::product::models::{ProviderName, WorkspaceSessionRecord};

#[test]
fn legacy_session_record_without_provisional_fields_deserializes() {
    let legacy = serde_json::json!({
        "id": "s1", "project_id": "p1", "issue_id": "i1", "entity_id": "e1",
        "workspace_type": "story", "status": "open",
        "author_provider": "claude_code", "reviewer_provider": "codex", "review_rounds": 1,
        "superpowers_enabled": true, "openspec_enabled": true,
        "messages": [], "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
    });
    let record: WorkspaceSessionRecord = serde_json::from_value(legacy).unwrap();
    assert_eq!(record.provisional_reviewer_provider, None);
    assert_eq!(record.reviewer_enabled_at_start, None);
}

#[test]
fn provisional_fields_roundtrip() {
    let record = WorkspaceSessionRecord {
        provisional_reviewer_provider: Some(ProviderName::Codex),
        reviewer_enabled_at_start: Some(false),
        ..serde_json::from_value(serde_json::json!({
            "id": "s2", "project_id": "p1", "issue_id": "i1", "entity_id": "e1",
            "workspace_type": "story", "status": "open",
            "author_provider": "claude_code", "reviewer_provider": "claude_code", "review_rounds": 1,
            "superpowers_enabled": true, "openspec_enabled": true,
            "messages": [], "created_at": "t", "updated_at": "t"
        })).unwrap()
    };
    let json = serde_json::to_value(&record).unwrap();
    assert_eq!(json["provisional_reviewer_provider"], "codex");
    assert_eq!(json["reviewer_enabled_at_start"], false);
    let back: WorkspaceSessionRecord = serde_json::from_value(json).unwrap();
    assert_eq!(back.provisional_reviewer_provider, Some(ProviderName::Codex));
    assert_eq!(back.reviewer_enabled_at_start, Some(false));
}
```

在 `src/product/workspace_engine/tests.rs`（或其 mod 声明处，跟随现有 part_* 注册方式）追加 `mod author_revision_loop;`。注意：WorkspaceSessionRecord 的构造字段名以 `src/product/models/workspace.rs:48-70` 实际为准，`..` 展开基座用 `from_value` 构造，避免手写全部字段。

- [ ] **Step 2: 运行验证失败**

Run: `cargo test --locked --lib author_revision_loop`
Expected: FAIL（字段不存在，编译错误）

- [ ] **Step 3: 实现四处落盘**

`models/workspace.rs` WorkspaceSessionRecord 追加（在 `permission_modes` 之后）：

```rust
    #[serde(default)]
    pub provisional_reviewer_provider: Option<ProviderName>,
    #[serde(default)]
    pub reviewer_enabled_at_start: Option<bool>,
```

`types.rs` WorkspaceSession 追加同两字段（普通字段，非 serde）；`from_record` 追加：

```rust
            provisional_reviewer_provider: record.provisional_reviewer_provider,
            reviewer_enabled_at_start: record.reviewer_enabled_at_start,
```

`lifecycle.rs` start_generation（:639-661 区域，`reviewer_enabled` 清空逻辑之后）追加：

```rust
        self.session.provisional_reviewer_provider = locked_snapshot.reviewer.clone();
        self.session.reviewer_enabled_at_start = Some(reviewer_enabled);
        if let Some(store) = &self.lifecycle_store {
            store.update_workspace_session_provisional_reviewer(
                &self.session.session_id,
                self.session.provisional_reviewer_provider.clone(),
                self.session.reviewer_enabled_at_start,
            )?;
        }
```

并在 lifecycle_store（`src/product/lifecycle_store` 对应 trait/impl）新增 `update_workspace_session_provisional_reviewer(&self, session_id, provisional, enabled_at_start) -> Result<...>`，SQL/JSON 写入方式跟随同文件 `update_workspace_session_providers` 的既有实现。

- [ ] **Step 4: 运行验证通过**

Run: `cargo test --locked --lib author_revision_loop`
Expected: PASS（两用例）

- [ ] **Step 5: 提交**

```bash
git add src/product/models/workspace.rs src/product/workspace_engine/types.rs src/product/workspace_engine/lifecycle.rs src/product/lifecycle_store src/product/workspace_engine/tests/author_revision_loop.rs src/product/workspace_engine/tests.rs
git commit -m "feat(engine): provisional reviewer 快照四处落盘闭环（spec-design-dialog-revision T2/任务2.2b）"
```

---

### Task 3: 引擎决策分支与协议守卫

**Files:**
- Modify: `src/product/workspace_engine/types.rs:307-312`（AuthorDecisionOutcome）
- Modify: `src/product/workspace_engine/decisions.rs:52-131`（handle_author_decision 追加分支）
- Modify: `src/web/workspace_ws_handler/protocol.rs:52-63`（AuthorConfirm 守卫已在 author_decision 消息内，变体无需守卫改动；确认无额外拦截）
- Test: `src/product/workspace_engine/tests/author_revision_loop.rs`（追加决策用例）

**Interfaces:**
- Consumes: Task 1 的 `AuthorDecision` 新变体；Task 2 的 `provisional_reviewer_provider`/`reviewer_enabled_at_start`。
- Produces: `AuthorDecisionOutcome::StartRevision { feedback: String }`、`AuthorDecisionOutcome::Finalized`（Task 4 的 handler 接线消费 StartRevision；Finalized 仅状态推送）。行为契约：spec「AuthorConfirm 对话式修订循环」4 场景 + 「确认双出口」4 场景。

- [ ] **Step 1: 写失败的决策测试**

`tests/author_revision_loop.rs` 追加（engine 构造参考 `src/product/workspace_engine/prompts/revision.rs` monorepo 测试中的 `engine_for_revision` 模式：`WorkspaceEngine::new(store, event_tx, session)`；session 的 `stage: WorkspaceStage::AuthorConfirm`、`workspace_type: WorkspaceType::Story`、`artifact: Some(ArtifactPayload::Markdown {...})`、`reviewer_enabled_at_start: Some(false)`、`provisional_reviewer_provider: Some(ProviderName::Codex)`）：

```rust
#[tokio::test]
async fn revise_with_feedback_transitions_to_revision() {
    let mut engine = author_confirm_engine().await;
    let outcome = engine.handle_author_decision(
        crate::web::workspace_ws_types::AuthorDecision::Revise { feedback: "补充异常场景".into() }
    ).await.unwrap();
    assert_eq!(outcome, AuthorDecisionOutcome::StartRevision { feedback: "补充异常场景".into() });
    assert_eq!(engine.session().stage, WorkspaceStage::Revision);
    assert_eq!(engine.pending_revision_context.as_deref(), Some("补充异常场景"));
    assert!(engine.session().artifact.is_some(), "反馈修订不得清空产物");
}

#[tokio::test]
async fn revise_with_blank_feedback_rejected() {
    let mut engine = author_confirm_engine().await;
    let err = engine.handle_author_decision(
        crate::web::workspace_ws_types::AuthorDecision::Revise { feedback: "  ".into() }
    ).await.unwrap_err();
    assert!(err.contains("feedback"));
    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
}

#[tokio::test]
async fn reject_returns_guidance_error_without_reset() {
    let mut engine = author_confirm_engine().await;
    let err = engine.handle_author_decision(
        crate::web::workspace_ws_types::AuthorDecision::Reject
    ).await.unwrap_err();
    assert!(err.contains("反馈"), "引导改用反馈修订: {err}");
    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
    assert!(engine.session().artifact.is_some());
}

#[tokio::test]
async fn accept_finalize_completes_workspace() {
    let mut engine = author_confirm_engine().await;
    let outcome = engine.handle_author_decision(
        crate::web::workspace_ws_types::AuthorDecision::AcceptFinalize
    ).await.unwrap();
    assert_eq!(outcome, AuthorDecisionOutcome::Finalized);
    assert_eq!(engine.session().stage, WorkspaceStage::Completed);
}

#[tokio::test]
async fn accept_with_review_restores_provisional_when_disabled() {
    let mut engine = author_confirm_engine().await; // reviewer_provider=None, rounds=0, provisional=Some(Codex)
    let outcome = engine.handle_author_decision(
        crate::web::workspace_ws_types::AuthorDecision::AcceptWithReview
    ).await.unwrap();
    assert!(matches!(outcome, AuthorDecisionOutcome::StartReview));
    assert_eq!(engine.session().reviewer_provider, Some(ProviderName::Codex));
    assert_eq!(engine.session().review_rounds, 1);
    assert_eq!(engine.session().stage, WorkspaceStage::CrossReview);
}

#[tokio::test]
async fn accept_with_review_errors_without_provisional() {
    let mut engine = author_confirm_engine_no_provisional().await;
    let err = engine.handle_author_decision(
        crate::web::workspace_ws_types::AuthorDecision::AcceptWithReview
    ).await.unwrap_err();
    assert!(err.contains("reviewer"), "{err}");
    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
}

#[tokio::test]
async fn legacy_accept_routes_by_enabled_at_start() {
    // reviewer_enabled_at_start=Some(false) + provisional 已恢复(rounds=1) → Accept 仍定稿（按创建默认值）
    let mut engine = author_confirm_engine_provisional_restored().await;
    let outcome = engine.handle_author_decision(
        crate::web::workspace_ws_types::AuthorDecision::Accept
    ).await.unwrap();
    assert_eq!(outcome, AuthorDecisionOutcome::Finalized);
    // 旧记录（None）按有效态：rounds>0 && reviewer.is_some() → StartReview
    let mut legacy = author_confirm_engine_legacy_record().await;
    let outcome2 = legacy.handle_author_decision(
        crate::web::workspace_ws_types::AuthorDecision::Accept
    ).await.unwrap();
    assert!(matches!(outcome2, AuthorDecisionOutcome::StartReview));
}
```

（辅助构造函数 `author_confirm_engine*` 写在同文件，字段组合差异：有无 provisional、有无 enabled_at_start、rounds 值。）

- [ ] **Step 2: 运行验证失败**

Run: `cargo test --locked --lib author_revision_loop`
Expected: FAIL（outcome 变体与新分支未定义）

- [ ] **Step 3: 实现**

`types.rs` AuthorDecisionOutcome 追加：

```rust
    StartRevision { feedback: String },
    Finalized,
```

`decisions.rs` handle_author_decision 在既有 `match decision` 的 Reject 臂**之后追加**（不改动 Accept/Reject 既有臂的代码结构；Accept 臂按下方语义重写内部路由但保留 `start_review_or_skip` 调用形态）：

```rust
            AuthorDecision::Revise { feedback } => {
                let trimmed = feedback.trim().to_string();
                if trimmed.is_empty() {
                    return Err("revise feedback must not be empty".to_string());
                }
                self.complete_active_node(Some("用户提交反馈，进入修订".to_string()))
                    .await;
                self.pending_revision_context = Some(trimmed.clone());
                self.transition_stage(WorkspaceStage::Revision).await;
                let _ = self
                    .create_timeline_node(TimelineNodeDraft {
                        node_type: TimelineNodeType::Revision,
                        agent: Some(self.session.author_provider.clone()),
                        stage: WorkspaceStage::Revision,
                        round: None,
                        title: "反馈修订".to_string(),
                        summary: Some(trimed_summary(&trimmed)),
                        status: TimelineNodeStatus::Active,
                    })
                    .await;
                Ok(AuthorDecisionOutcome::StartRevision { feedback: trimmed })
            }
            AuthorDecision::AcceptWithReview => {
                self.ensure_reviewer_available_for_review_request()?;
                self.complete_active_node(Some("已确认，进入 Review".to_string()))
                    .await;
                self.start_review().await;
                Ok(AuthorDecisionOutcome::StartReview)
            }
            AuthorDecision::AcceptFinalize => {
                self.finalize_current_artifact("人工确认定稿").await?;
                Ok(AuthorDecisionOutcome::Finalized)
            }
```

Reject 臺改为（Story/Design 拒绝重置，仅返回引导错误；WorkItemPlan 路径分流逻辑保持既有 handle_work_item_plan_outline_decision 不变）：

```rust
            AuthorDecision::Reject => {
                if self.session.workspace_type == WorkspaceType::WorkItemPlan {
                    // 既有 outline 分流保持（decisions.rs:60-76 的 active_node_type 检查在前）
                }
                return Err(
                    "已移除推倒重来：请通过反馈修订表达重写意图（spec-design-dialog-revision）"
                        .to_string(),
                );
            }
```

新增私有方法（同文件）：

```rust
    /// AcceptWithReview 的 reviewer 就绪检查：判定依据是落盘的 reviewer_enabled_at_start，
    /// 不可用 reviewer_provider.is_none()（from_record 恒 Some + fallback author，重连后失真）。
    fn ensure_reviewer_available_for_review_request(&mut self) -> Result<(), String> {
        let review_disabled_at_start =
            self.session.reviewer_enabled_at_start == Some(false);
        let review_active =
            self.session.review_rounds > 0 && self.session.reviewer_provider.is_some();
        if review_active {
            return Ok(());
        }
        if review_disabled_at_start {
            if let Some(provisional) = self.session.provisional_reviewer_provider.clone() {
                self.session.reviewer_provider = Some(provisional);
                self.session.review_rounds = 1;
                return Ok(());
            }
            return Err("创建时未启用 review 且未保留 reviewer 选择：请确认定稿，或重新开始并启用 review".to_string());
        }
        Err("当前会话无可用 reviewer：请确认定稿，或重新开始并启用 review".to_string())
    }
```

Accept 兼容路由：既有 Accept 臂改为先走 `ensure_reviewer_available_for_review_request` 的镜像判定——`reviewer_enabled_at_start == Some(false)` → `AcceptFinalize` 语义；`Some(true)` 或激活态 → `AcceptWithReview` 语义；`None`（旧记录）→ 保持现有 `review_rounds > 0 && reviewer_provider.is_some()` 有效态判定。`start_review()` 为从 `start_review_or_skip`（decisions.rs:654）提取的"只进 CrossReview 不再跳 HumanConfirm"的新私有方法：跳过路径删除（`AcceptFinalize` 已由显式决策覆盖），Fake provider 快速路径保留。`finalize_current_artifact` 复用 HumanConfirm::Confirm 的定稿实现（`mark_latest_artifact_confirmed` + `transition_stage(Completed)`，见 decisions.rs:440-468 的 Confirm 分支逻辑，提取为方法）。

- [ ] **Step 4: 运行验证通过**

Run: `cargo test --locked --lib author_revision_loop`
Expected: PASS（Task 2 两例 + 本任务 7 例）

- [ ] **Step 5: 提交**

```bash
git add src/product/workspace_engine/types.rs src/product/workspace_engine/decisions.rs src/product/workspace_engine/tests/author_revision_loop.rs
git commit -m "feat(engine): AuthorConfirm 三决策分支与 provisional 恢复判定（spec-design-dialog-revision T3/任务2.2）"
```

---

### Task 4: author 反馈修订 prompt 与 run 接线

**Files:**
- Create: `src/product/workspace_engine/prompts/author_revision.rs`
- Modify: `src/product/workspace_engine/prompts.rs`（mod 注册）
- Modify: `src/web/workspace_ws_handler/decisions.rs:186-211`（handle_author_decision_from_handler 追加 StartRevision/Finalized 臂）
- Test: `src/product/workspace_engine/tests/author_revision_loop.rs`（prompt 构造用例）

**Interfaces:**
- Consumes: Task 3 的 `AuthorDecisionOutcome::StartRevision`；既有 `spawn_provider_run_from_handler(run_context, ProviderRunKind::Revision, outbound_tx)`（src/web/workspace_ws_handler/decisions.rs:151-165 的 ReviewDecision StartRevision 同款接线）。
- Produces: `WorkspaceEngine::build_author_revision_prompt(&self, feedback: &str) -> String`（Task 5/7 不直接消费，但 provider_drive 的 Revision run 经 `pending_revision_context` 走到此函数——在 provider_drive 的 revision prompt 选择处新增分流：`pending_revision_context` 存在且来源于 author 反馈时用本函数；实现上以 `latest_author_feedback` 标志或直接判断 `pending_revision_context.is_some() && latest_review_verdict.is_none()`）。

- [ ] **Step 1: 写失败的 prompt 测试**

```rust
#[test]
fn author_revision_prompt_includes_feedback_and_changelog_section() {
    let engine = prompt_engine_with_artifact("# Story Spec\n\n旧内容");
    let prompt = engine.build_author_revision_prompt("补充异常场景与回滚策略");
    assert!(prompt.contains("补充异常场景与回滚策略"));
    assert!(prompt.contains("# Story Spec"));
    assert!(prompt.contains("改动摘要"), "必须要求输出改动摘要小节: {prompt}");
    assert!(prompt.contains("增量修订"), "约束不得整体重写无关章节");
}
```

（`prompt_engine_with_artifact` 辅助构造：session.artifact = Markdown。）

- [ ] **Step 2: 运行验证失败**

Run: `cargo test --locked --lib author_revision_prompt`
Expected: FAIL（函数未定义）

- [ ] **Step 3: 实现新 prompt 文件**

```rust
// src/product/workspace_engine/prompts/author_revision.rs
use super::*;

impl WorkspaceEngine {
    /// AuthorConfirm 反馈修订专用 prompt：当前产物 + 用户自由文本反馈 → 增量修订 + 改动摘要。
    /// 独立于 build_revision_full_prompt/build_revision_delta_prompt（后者面向 reviewer 返修，
    /// 且 add-monorepo 分支已参数化，避免触碰）。
    pub(crate) fn build_author_revision_prompt(&self, feedback: &str) -> String {
        let artifact = self
            .session
            .artifact
            .as_ref()
            .map(|a| a.markdown_or_empty())
            .unwrap_or_default();
        let mut prompt = String::new();
        prompt.push_str("请作为 author 基于用户反馈对当前 Workspace 产物做增量修订。\n\n");
        prompt.push_str("## 修订规则\n");
        prompt.push_str("- 只修改与反馈相关的部分，保持其余章节原样（增量修订，不是重写）。\n");
        prompt.push_str("- 若反馈要求全部重写（如方向调整），保留仍然有效的事实性内容并整体重组。\n");
        prompt.push_str("- 输出修订后的完整产物正文（markdown），并在文末追加「## 改动摘要」小节：逐条列出本次改动的位置与原因。\n\n");
        prompt.push_str("## 当前产物\n\n```\n");
        prompt.push_str(&artifact);
        prompt.push_str("\n```\n\n## 用户反馈\n\n");
        prompt.push_str(feedback.trim());
        prompt.push('\n');
        prompt
    }
}
```

（`ArtifactPayload::markdown_or_empty` 若不存在则在 ArtifactPayload 上加一个小工具方法。）在 provider_drive.rs 的 revision prompt 选择处新增分流：`self.pending_revision_context.is_some() && self.latest_review_verdict.is_none()` → `build_author_revision_prompt`，否则维持既有路径。

- [ ] **Step 4: handler 接线**

`src/web/workspace_ws_handler/decisions.rs` 的 handle_author_decision_from_handler outcome match（:186-211）追加：

```rust
        Ok(AuthorDecisionOutcome::StartRevision { feedback: _ }) => {
            // engine 已置 pending_revision_context 并进入 Revision 阶段；复用 ReviewDecision 的 Revision run 接线
            if let Err(message) = spawn_provider_run_from_handler(
                run_context, ProviderRunKind::Revision, outbound_tx.clone(),
            ).await {
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(&outbound_tx, &err).await;
            }
        }
        Ok(AuthorDecisionOutcome::Finalized) => {}
```

（`StartRevision { feedback }` 的 feedback 已存入 engine 的 pending_revision_context，handler 无需再传，`feedback: _` 忽略。）

- [ ] **Step 5: 运行验证通过 + 提交**

Run: `cargo test --locked --lib author_revision`
Expected: PASS

```bash
git add src/product/workspace_engine/prompts/author_revision.rs src/product/workspace_engine/prompts.rs src/product/workspace_engine/provider_drive.rs src/web/workspace_ws_handler/decisions.rs src/product/workspace_engine/tests/author_revision_loop.rs
git commit -m "feat(engine): author 反馈修订 prompt 与 Revision run 接线（spec-design-dialog-revision T4/任务2.4+2.5）"
```

---

### Task 5: review 完成路由回 AuthorConfirm

**Files:**
- Modify: `src/product/workspace_engine/review/routing.rs:65-118`（Story/Design 路由终点）
- Test: `src/product/workspace_engine/tests/author_revision_loop.rs`（追加路由用例）

**Interfaces:**
- Consumes: 既有 `format_review_feedback`（review/feedback.rs:14）、`record_review_message`（session_state/timeline.rs:450）。
- Produces: Story/Design 的 review 完成统一回 AuthorConfirm（spec「review 结果回对话流」2 场景）；WorkItemPlan 路由不变。

- [ ] **Step 1: 写失败的路由测试**

```rust
#[tokio::test]
async fn review_completion_routes_back_to_author_confirm_for_story() {
    let mut engine = cross_review_engine_with_verdict(ReviewVerdictType::Revise).await;
    engine.apply_review_completion_for_story_design().await; // 或直接调用被测完成路径
    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
}

#[tokio::test]
async fn review_pass_does_not_auto_complete() {
    let mut engine = cross_review_engine_with_verdict(ReviewVerdictType::Pass).await;
    engine.apply_review_completion_for_story_design().await;
    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm,
        "reviewer pass 不得自动定稿");
}
```

- [ ] **Step 2: 运行验证失败**

Run: `cargo test --locked --lib review_completion_routes`
Expected: FAIL（路由方法未定义）

- [ ] **Step 3: 实现路由**

`review/routing.rs` 非 WorkItemPlan 的 `_` 分支（:65-80）改造：`UserConfirmAllowed` 与 `RequiresRevision` 统一走新私有方法：

```rust
    /// Story/Design：review 报告进对话流，回到 AuthorConfirm（spec：review 结果回对话流）。
    async fn route_review_report_to_author_confirm(&mut self, verdict: &ReviewVerdict) {
        let report = format_review_feedback(verdict);
        self.record_review_message(&report).await;
        self.complete_active_node(Some("Review 完成，报告已进入对话流".to_string()))
            .await;
        self.enter_author_confirm(Some("请基于 Review 报告继续修订或确认定稿".to_string()))
            .await;
    }
```

（`enter_author_confirm` 已存在 decisions.rs:693；删除该路径上进入 `enter_review_decision`/`enter_human_confirm` 的调用，仅限 Story/Design 类型分支，WorkItemPlan 分支保持原样。）

- [ ] **Step 4: 运行验证通过 + 提交**

Run: `cargo test --locked --lib author_revision_loop`
Expected: PASS

```bash
git add src/product/workspace_engine/review/routing.rs src/product/workspace_engine/tests/author_revision_loop.rs
git commit -m "feat(engine): review 完成统一回 AuthorConfirm，报告进对话流（spec-design-dialog-revision T5/任务2.3）"
```

---

### Task 6: 存量会话迁移（HumanConfirm/ReviewDecision → AuthorConfirm）

**Files:**
- Modify: `src/product/workspace_engine/lifecycle.rs:282-300`（new_persistent fallback 链追加一个 fallback）
- Test: `src/product/workspace_engine/tests/author_revision_loop.rs`（追加两例）

**Interfaces:**
- Produces: `recover_story_design_retired_stage_fallback`——检测 Story/Design 且 stage ∈ {HumanConfirm, ReviewDecision} 时迁移回 AuthorConfirm，保留 artifact/messages，review verdict（若有）注入对话流。spec「存量会话恢复兼容」2 场景。

- [ ] **Step 1: 写失败的迁移测试**

注意：`new_persistent` 签名为 `pub fn new_persistent(checkpoint_store: Arc<CheckpointStore>, lifecycle_store: LifecycleStore, event_tx, session: WorkspaceSession) -> Self`（lifecycle.rs:222-226，非 async、接收 WorkspaceSession）。测试直接对新 fallback 函数做单元测试 + 一条 from_record 路径：

```rust
#[test]
fn retired_stage_fallback_migrates_story_design_sessions() {
    for stage in [WorkspaceStage::HumanConfirm, WorkspaceStage::ReviewDecision] {
        let session = story_session_with_stage(WorkspaceType::Story, stage, Some(ArtifactPayload::Markdown { markdown: "# Story".into(), diff: None }));
        let migrated = recover_story_design_retired_stage_fallback(session);
        assert_eq!(migrated.stage, WorkspaceStage::AuthorConfirm);
        assert!(migrated.artifact.is_some(), "产物保留");
    }
    // WorkItemPlan 不受影响
    let plan_session = story_session_with_stage(WorkspaceType::WorkItemPlan, WorkspaceStage::HumanConfirm, None);
    assert_eq!(recover_story_design_retired_stage_fallback(plan_session).stage, WorkspaceStage::HumanConfirm);
    // 其他阶段不迁移
    let running = story_session_with_stage(WorkspaceType::Story, WorkspaceStage::Running, None);
    assert_eq!(recover_story_design_retired_stage_fallback(running).stage, WorkspaceStage::Running);
}

#[test]
fn legacy_record_with_review_decision_restores_to_author_confirm_via_persistent_path() {
    // 经 lifecycle_store 持久化 record → from_record → new_persistent 验证集成路径（engine 构造参考现有 part_06 的持久化测试基建）
    // 断言最终 session().stage == AuthorConfirm 且 messages 中含评审报告
}
```

（`story_session_with_stage` 辅助构造写在同文件；第二个集成用例复用现有持久化测试基建，若 part_06 中有现成 fixture 模式则跟随。）

- [ ] **Step 2: 运行验证失败**

Run: `cargo test --locked --lib legacy_human_confirm_session`
Expected: FAIL（fallback 不存在，stage 保持原样）

- [ ] **Step 3: 实现 fallback**

在 `new_persistent` 的 fallback 链（lifecycle.rs:282-300，与 `recover_complete_artifact_misclassified_as_text_fallback` 并列）追加：

```rust
        session = recover_story_design_retired_stage_fallback(session);
```

新函数（同文件）：

```rust
/// spec-design-dialog-revision：HumanConfirm/ReviewDecision 从 Story/Design 流程退役，
/// 存量会话恢复时迁移回 AuthorConfirm（保留产物与消息，verdict 留在消息流）。
fn recover_story_design_retired_stage_fallback(mut session: WorkspaceSession) -> WorkspaceSession {
    let retired = matches!(session.stage, WorkspaceStage::HumanConfirm | WorkspaceStage::ReviewDecision);
    if retired && matches!(session.workspace_type, WorkspaceType::Story | WorkspaceType::Design) {
        session.stage = WorkspaceStage::AuthorConfirm;
    }
    session
}
```

- [ ] **Step 4: 运行验证通过 + 提交**

Run: `cargo test --locked --lib author_revision_loop`
Expected: PASS

```bash
git add src/product/workspace_engine/lifecycle.rs src/product/workspace_engine/tests/author_revision_loop.rs
git commit -m "feat(engine): 存量 HumanConfirm/ReviewDecision 会话迁移回 AuthorConfirm（spec-design-dialog-revision T6/任务3.1）"
```

---

### Task 7: 修订断线恢复扩展（Revision 恢复臂）

**Files:**
- Modify: `src/product/workspace_engine/interrupted_run_recovery.rs:4-7`（枚举）、`:82-121`（retry 臂）、`:175-180`（节点检测）
- Modify: `src/web/workspace_ws_handler/decisions/inbound.rs:14-23`（run kind 映射）
- Test: `src/product/workspace_engine/tests/author_revision_loop.rs`（追加两例）

**Interfaces:**
- Consumes: 既有 `ProviderRunKind::Revision`（run/provider_run.rs:26）；`recover_stale_active_run_after_disconnect`（lifecycle.rs:733-747，Revision 断线归位 PrepareContext 的既有前置）。
- Produces: `InterruptedRunRecoveryOutcome::Revision` + 检测失败修订 AuthorRun 节点 + retry 臂 + 映射。spec「修订中断线恢复」2 场景。

- [ ] **Step 1: 写失败的恢复测试**

```rust
#[tokio::test]
async fn interrupted_revision_run_is_recoverable_and_retryable() {
    let mut engine = prepare_context_engine_with_failed_author_run_node().await;
    let recoverable = engine.recoverable_interrupted_run().expect("修订 run 应可恢复");
    assert_eq!(recoverable.operation, InterruptedRunRecoveryOutcome::Revision);
    engine.retry_interrupted_run(&recoverable.failed_node_id).await.unwrap();
    assert_eq!(engine.session().stage, WorkspaceStage::Running);
}
```

（`prepare_context_engine_with_failed_author_run_node`：stage=PrepareContext、timeline 含 status=Failed 的 AuthorRun 节点、无 active_run。）

- [ ] **Step 2: 运行验证失败**

Run: `cargo test --locked --lib interrupted_revision_run`
Expected: FAIL（Revision 变体不存在）

- [ ] **Step 3: 实现四处扩展**

```rust
// interrupted_run_recovery.rs:4-7
pub enum InterruptedRunRecoveryOutcome {
    Review,
    WorkItemDraftGeneration,
    Revision,
}
```

`recoverable_shared_review`（:175-180）扩展匹配失败的 AuthorRun 节点 → `Revision`；`retry_interrupted_run`（:82-121）新增 `Revision` 臂：置 stage=Running、复用 AuthorRun timeline 模式启动修订 run（跟随 Review 臂的既有写法）；`inbound.rs:14-23` 的 `provider_run_kind_for_interrupted_recovery` 新增 `Revision => ProviderRunKind::Revision`。

- [ ] **Step 4: 运行验证通过 + 提交**

Run: `cargo test --locked --lib author_revision_loop`
Expected: PASS

```bash
git add src/product/workspace_engine/interrupted_run_recovery.rs src/web/workspace_ws_handler/decisions/inbound.rs src/product/workspace_engine/tests/author_revision_loop.rs
git commit -m "feat(engine): interrupted run 恢复新增 Revision 臂（spec-design-dialog-revision T7/任务3.3）"
```

---

### Task 8: 前端三动作与 provisional 快照

**Files:**
- Modify: `web/src/components/chat-workspace/ChatInputBar.tsx:279-308`（author_confirm 按钮区 + :55 isAuthorConfirm + placeholderForStage）
- Modify: `web/src/pages/ChatWorkspacePageParts.tsx:336-356`（providerConfigFor）
- Modify: `web/src/pages/ChatWorkspacePage.tsx:590,643`（sendAuthorDecision 接新变体）
- Create: `web/src/components/chat-workspace/entries/RevisionSummaryEntry.tsx`（改动摘要 entry，随 Task 4 的 changelog 消息渲染）
- Test: `web/src/components/chat-workspace/ChatInputBar.test.tsx`、`web/src/pages/ChatWorkspacePage.actions.test.tsx`（追加用例）

**Interfaces:**
- Consumes: Task 1 的 WS 变体（`{revise: {feedback}}`/`accept_with_review`/`accept_finalize`）；store 中 reviewer 配置（默认高亮）。
- Produces: `onAuthorDecision("revise", feedback)` / `onAuthorDecision("accept_with_review")` / `onAuthorDecision("accept_finalize")`；`providerConfigFor` 未启用 review 时仍携带用户已选 reviewer、`review_rounds` 传 0。

- [ ] **Step 1: 写失败的组件测试**

ChatInputBar.test.tsx 追加：

```tsx
it("author_confirm 阶段展示三动作，反馈输入可用", async () => {
  render(<ChatInputBar stage="author_confirm" reviewerEnabled ... />);
  expect(screen.getByPlaceholderText(/输入修改意见/)).toBeEnabled();
  fireEvent.change(screen.getByPlaceholderText(/输入修改意见/), { target: { value: "补充回滚策略" } });
  fireEvent.click(screen.getByRole("button", { name: /发送反馈/ }));
  expect(onAuthorDecision).toHaveBeenCalledWith("revise", "补充回滚策略");
  expect(screen.queryByRole("button", { name: /重新编写/ })).not.toBeInTheDocument();
});

it("reviewer 未启用时默认高亮确认定稿，送审仍可点", () => {
  render(<ChatInputBar stage="author_confirm" reviewerEnabled={false} ... />);
  expect(screen.getByRole("button", { name: /确认定稿/ }).className).toContain("aria-primary");
  expect(screen.getByRole("button", { name: /确认并送审/ })).toBeEnabled();
});
```

ChatWorkspacePage.actions.test.tsx 追加 providerConfigFor 用例：

```tsx
it("providerConfigFor 未启用 review 时仍携带已选 reviewer 且 rounds 为 0", () => {
  const snapshot = providerConfigFor({ author: "claude_code", reviewer: "codex" }, false, 3);
  expect(snapshot.reviewer).toBe("codex");
  expect(snapshot.review_rounds).toBe(0);
});
```

- [ ] **Step 2: 运行验证失败**

Run: `cd web && pnpm vitest run src/components/chat-workspace/ChatInputBar.test.tsx src/pages/ChatWorkspacePage.actions.test.tsx`
Expected: FAIL（按钮不存在 / reviewer 为 null）

- [ ] **Step 3: 实现**

ChatInputBar.tsx：author_confirm 按钮区（:279-308）替换为：输入框（受控 `feedbackDraft`）+ 三按钮——「发送反馈」（`feedbackDraft.trim()` 非空时 disabled=false，点击 `onAuthorDecision("revise", feedbackDraft.trim())` 后清空）、「确认并送审」（`onAuthorDecision("accept_with_review")`，`reviewerEnabled` 时主样式）、「确认定稿」（`onAuthorDecision("accept_finalize")`，`!reviewerEnabled` 时主样式）；删除「重新编写」按钮；`placeholderForStage` 的 author_confirm 分支改 `"输入修改意见，或直接确认"`。ChatWorkspacePage 的 sendAuthorDecision 扩展第二参数 feedback（构造 `{revise: {feedback}}` payload）。ChatWorkspacePageParts.tsx:346-352 改：

```tsx
  const reviewer = providerNameFor(providers?.reviewer, "codex"); // 不再随 reviewerEnabled 置 null
  const reviewRounds = reviewerEnabled ? clampReviewRounds(reviewRounds_) : 0; // 解耦
```

（注意保持函数签名不变，内部用解耦后的 rounds 变量。）

- [ ] **Step 4: 运行验证通过 + 提交**

Run: `cd web && pnpm tsc -b && pnpm vitest run src/components/chat-workspace src/pages/ChatWorkspacePage.actions.test.tsx`
Expected: PASS

```bash
git add web/src/components/chat-workspace/ChatInputBar.tsx web/src/pages/ChatWorkspacePageParts.tsx web/src/pages/ChatWorkspacePage.tsx web/src/components/chat-workspace/entries/RevisionSummaryEntry.tsx web/src/components/chat-workspace/ChatInputBar.test.tsx web/src/pages/ChatWorkspacePage.actions.test.tsx
git commit -m "feat(web): AuthorConfirm 三动作与 provisional 快照携带（spec-design-dialog-revision T8/任务4.x）"
```

---

### Task 9: 既有测试迁移与全量验证

**Files:**
- Modify: `src/product/workspace_engine/tests/part_06.rs:514-650` 等固化 "Accept→CrossReview→ReviewDecision→HumanConfirm" 的用例（按 rg 定位：`rg -l "enter_human_confirm|HumanConfirm" src/product/workspace_engine/tests/ src/web/workspace_ws_handler/tests/ | sort`）
- Modify: `web/whats-new`（如项目惯例要求）

**Interfaces:**
- Consumes: 全部前序 Task 的行为。

- [ ] **Step 1: 迁移旧路径用例**

逐文件将 Story/Design 的旧四段路径断言改为新状态机（AcceptWithReview→CrossReview→AuthorConfirm；AcceptFinalize→Completed；Reject 期望 Err），保留 WorkItemPlan 用例不动。每改一组运行：`cargo test --locked --lib part_06`（按实际文件名）确认 PASS。

- [ ] **Step 2: 全量标准验证**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
cd web && pnpm tsc -b && pnpm test
```

Expected: 全部 PASS，零 warning。🔴 不携带 `-j 1`。

- [ ] **Step 3: whats-new 与收尾提交**

```bash
git add -A
git commit -m "test: 迁移旧 Accept→Review→HumanConfirm 路径用例并全量验证（spec-design-dialog-revision T9/任务5.x）"
```

---

## Self-Review 记录

1. **Spec 覆盖**：14 个 Scenario 逐一核对——修订循环 4 场景（T3/T4）、双出口 4 场景（T3：快照送审/无快照报错/Accept 兼容/定稿完成）、review 回对话流 2 场景（T5）、存量恢复 2 场景（T6）、断线恢复 2 场景（T7）；前端三动作（T8）支撑全部 UI 面。无遗漏。
2. **占位符扫描**：无 TBD/TODO；每步含真实代码或精确命令与预期。
3. **类型一致性**：`AuthorDecision::Revise{feedback}`（T1 定义 ↔ T3/T8 使用）；`StartRevision{feedback}`（T3 产出 ↔ T4 消费，handler 侧 `feedback: _` 因已入 pending_revision_context）；`provisional_reviewer_provider`/`reviewer_enabled_at_start`（T2 定义 ↔ T3 判定）；`InterruptedRunRecoveryOutcome::Revision`（T7 内闭环）。`update_workspace_session_provisional_reviewer` 为 T2 新增 store API，签名已给出。
4. **冲突规避核验**：T4 新文件 author_revision.rs；T3 decisions.rs 追加臂；T7 不改 run/provider_run.rs（仅 inbound.rs 映射 + engine 侧 retry 臂）——符合与 add-monorepo 的规避约定（ProviderRunKind::Revision 为既有定义、复用不修改）。
5. **API 事实核验（v1.0 自审后修正）**：`ReviewVerdictType` 实际变体为 `Pass/Revise/NeedsHuman`（非 Approve/NeedsChanges）；`new_persistent` 为同步方法、接收 `WorkspaceSession`（lifecycle.rs:222-226）；`pending_revision_context` 为 `pub(crate)` 字段直接访问。Task 5/6/3 测试代码已按实际 API 修正。
