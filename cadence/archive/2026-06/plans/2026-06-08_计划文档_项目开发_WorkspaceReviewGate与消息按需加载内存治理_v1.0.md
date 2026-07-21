# Workspace Review Gate 与消息按需加载内存治理 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Story Spec、Design Spec、Work Item 的 review 只在强返修问题上阻塞用户确认，并让刷新恢复后的消息气泡能按 API 补全完整内容，同时限制前端大文本缓存内存。

**Architecture:** 后端把 reviewer 输出解析成结构化 `ReviewVerdict + findings + review_gate`，由 `ReviewGate` 决定进入 `review_decision` 还是 `human_confirm`。前端把 review findings 分组展示，并把完整 prompt/output/artifact 从 Zustand 长期状态中移到按需加载、带 byte budget 的 LRU 缓存。

**Tech Stack:** Rust 1.95、Axum WebSocket、serde、React 19、Zustand、Vitest、pnpm、Cargo。

---

## 执行前约束

- 工作目录必须是 `/Users/michaelche/Documents/git-folder/github-folder/cadence-aria/.worktrees/fix_author_confirm_followup`。
- 不要回退当前工作区已有改动，执行每个任务前用 `git status --short` 确认只暂存本任务相关文件。
- Rust 本地验证直接使用宿主机 Cargo，禁止 Docker，禁止给 `cargo test` 加 `-j 1`。
- 前端使用 `pnpm`，不要使用 npm 或 yarn。
- Story、Design、Work Item 三类 workspace 的共享链路必须一起覆盖。

## 文件结构

- Modify: `src/web/workspace_ws_types.rs`
  - 扩展 review contract：finding severity、finding、review gate、ReviewVerdict 字段。
- Modify: `src/product/workspace_engine.rs`
  - 更新 review prompt、review JSON 解析、review gate 分类、`complete_review()` 状态分流和相关单元测试。
- Modify: `web/src/api/types.ts`
  - 扩展前端 ReviewVerdict 类型，增加 ReviewFinding 与 ReviewGate。
- Modify: `web/src/state/workspace-ws-store.ts`
  - 保存完整 review verdict、提供 node detail hydration action、接入 bounded content cache。
- Create: `web/src/state/workspace-content-cache.ts`
  - 提供 byte-budget LRU 缓存纯函数。
- Test: `web/src/state/workspace-content-cache.test.ts`
  - 覆盖缓存命中、访问刷新、超预算淘汰、session 清理。
- Modify: `web/src/api/workspace-content.ts`
  - 把 `fetchWorkspaceNodeDetail` 返回类型改为 `NodeDetail`。
- Modify: `web/src/components/chat-workspace/entries/ReviewVerdictEntry.tsx`
  - 按“需要解决 / 可选建议”分组展示 findings。
- Modify: `web/src/components/chat-workspace/entries/GatePromptEntry.tsx`
  - 在可确认 review gate 下显示“确认使用当前版本”。
- Modify: `web/src/components/chat-workspace/InlineEventRow.tsx`
  - 使用 bounded content cache 的只读 values 视图，保留展开加载行为。
- Modify: `web/src/components/chat-workspace/ArtifactPane.tsx`
  - Artifact 内容读取接入 bounded cache values，避免组件 state 与 store 双重长期缓存。
- Modify: `web/src/pages/ChatWorkspacePage.tsx`
  - 对选中 node、active node、当前恢复态自动 hydration，连接 bounded cache action。
- Test: `web/src/components/chat-workspace/entries/p1-entries.test.tsx`
  - 覆盖 findings 分组和确认文案。
- Test: `web/src/pages/ChatWorkspacePage.test.tsx`
  - 覆盖 human_confirm 可确认当前版本、恢复态 node detail hydration、三类 workspace review decision 仍显示返修按钮。
- Test: `web/src/state/workspace-ws-store.test.ts`
  - 覆盖 session_state 保存 review_gate、node detail hydration 合并、cache session 清理。

---

### Task 1: 后端 ReviewVerdict 合约与解析

**Files:**
- Modify: `src/web/workspace_ws_types.rs`
- Modify: `src/product/workspace_engine.rs`
- Test: `src/product/workspace_engine.rs`

- [ ] **Step 1: 写 failing tests，覆盖 ReviewGate 分类**

在 `src/product/workspace_engine.rs` 的 `#[cfg(test)] mod tests` 中追加以下测试：

```rust
#[test]
fn parse_review_verdict_classifies_optional_findings_as_user_confirm_allowed() {
    let output = r#"整体可用，建议补充措辞。

```json
{
  "verdict": "revise",
  "summary": "有非阻塞建议",
  "findings": [
    {
      "severity": "suggestion",
      "message": "建议补充边界说明",
      "evidence": "验收标准已经覆盖主路径",
      "impact": "不影响下一阶段执行",
      "required_action": "可在后续优化中补充"
    }
  ]
}
```"#;

    let verdict = WorkspaceEngine::parse_review_verdict(output);

    assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
    assert_eq!(verdict.review_gate, ReviewGate::UserConfirmAllowed);
    assert_eq!(verdict.findings.len(), 1);
    assert_eq!(verdict.findings[0].severity, ReviewFindingSeverity::Suggestion);
}

#[test]
fn parse_review_verdict_classifies_strong_findings_as_requires_revision() {
    let output = r#"缺少 Work Item 可执行验证命令。

```json
{
  "verdict": "revise",
  "summary": "必须补充验证命令",
  "findings": [
    {
      "severity": "must_fix",
      "message": "Work Item 没有验证命令",
      "evidence": "Artifact 未出现验证命令段落",
      "impact": "Coding Workspace 无法执行验收",
      "required_action": "补充明确验证命令"
    }
  ]
}
```"#;

    let verdict = WorkspaceEngine::parse_review_verdict(output);

    assert_eq!(verdict.verdict, ReviewVerdictType::Revise);
    assert_eq!(verdict.review_gate, ReviewGate::RequiresRevision);
    assert_eq!(verdict.findings[0].severity, ReviewFindingSeverity::MustFix);
}

#[test]
fn parse_review_verdict_revise_without_findings_falls_back_to_human_confirm_allowed() {
    let output = r#"建议修改一些描述。

```json
{"verdict":"revise","summary":"建议修改描述"}
```"#;

    let verdict = WorkspaceEngine::parse_review_verdict(output);

    assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
    assert_eq!(verdict.review_gate, ReviewGate::UserConfirmAllowed);
    assert!(verdict.findings.is_empty());
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib parse_review_verdict
```

Expected: 编译失败，提示 `ReviewGate`、`ReviewFindingSeverity` 或 `findings` 字段不存在。

- [ ] **Step 3: 扩展 Rust review contract**

在 `src/web/workspace_ws_types.rs` 的 `ReviewVerdictType` 后增加：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewFindingSeverity {
    Blocking,
    MustFix,
    StrongRecommendFix,
    Suggestion,
    Minor,
    Optional,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewFinding {
    pub severity: ReviewFindingSeverity,
    pub message: String,
    pub evidence: String,
    pub impact: String,
    pub required_action: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewGate {
    RequiresRevision,
    UserConfirmAllowed,
}
```

把 `ReviewVerdict` 改为：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewVerdict {
    pub verdict: ReviewVerdictType,
    pub comments: String,
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<ReviewFinding>,
    #[serde(default = "default_review_gate")]
    pub review_gate: ReviewGate,
}

fn default_review_gate() -> ReviewGate {
    ReviewGate::UserConfirmAllowed
}
```

在 `src/product/workspace_engine.rs` 顶部对应 import 中加入：

```rust
ReviewFinding, ReviewFindingSeverity, ReviewGate,
```

- [ ] **Step 4: 实现 JSON 解析与分类**

把 `parse_review_json` 改为返回完整 `ReviewVerdict`：

```rust
fn parse_review_json(json: &str, comments: &str) -> Option<ReviewVerdict> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let raw_verdict = value.get("verdict")?.as_str()?;
    let parsed_verdict = match raw_verdict {
        "pass" => ReviewVerdictType::Pass,
        "revise" => ReviewVerdictType::Revise,
        "needs_human" => ReviewVerdictType::NeedsHuman,
        _ => return None,
    };
    let summary = value
        .get("summary")
        .and_then(|value| value.as_str())
        .unwrap_or(match parsed_verdict {
            ReviewVerdictType::Pass => "审核通过",
            ReviewVerdictType::Revise => "需要返修",
            ReviewVerdictType::NeedsHuman => "需要人工确认",
        })
        .to_string();
    let findings = parse_review_findings(value.get("findings"));
    let review_gate = review_gate_for(&parsed_verdict, &findings);
    let verdict = match review_gate {
        ReviewGate::RequiresRevision => ReviewVerdictType::Revise,
        ReviewGate::UserConfirmAllowed => match parsed_verdict {
            ReviewVerdictType::Pass => ReviewVerdictType::Pass,
            ReviewVerdictType::Revise | ReviewVerdictType::NeedsHuman => ReviewVerdictType::NeedsHuman,
        },
    };
    Some(ReviewVerdict {
        verdict,
        comments: comments.trim().to_string(),
        summary,
        findings,
        review_gate,
    })
}
```

新增 helper：

```rust
fn parse_review_findings(value: Option<&serde_json::Value>) -> Vec<ReviewFinding> {
    let Some(items) = value.and_then(|value| value.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            Some(ReviewFinding {
                severity: parse_review_finding_severity(item.get("severity")?.as_str()?)?,
                message: item.get("message")?.as_str()?.to_string(),
                evidence: item.get("evidence").and_then(|value| value.as_str()).unwrap_or("").to_string(),
                impact: item.get("impact").and_then(|value| value.as_str()).unwrap_or("").to_string(),
                required_action: item
                    .get("required_action")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect()
}

fn parse_review_finding_severity(value: &str) -> Option<ReviewFindingSeverity> {
    match value {
        "blocking" => Some(ReviewFindingSeverity::Blocking),
        "must_fix" => Some(ReviewFindingSeverity::MustFix),
        "strong_recommend_fix" => Some(ReviewFindingSeverity::StrongRecommendFix),
        "suggestion" => Some(ReviewFindingSeverity::Suggestion),
        "minor" => Some(ReviewFindingSeverity::Minor),
        "optional" => Some(ReviewFindingSeverity::Optional),
        _ => None,
    }
}

fn review_gate_for(verdict: &ReviewVerdictType, findings: &[ReviewFinding]) -> ReviewGate {
    if findings.iter().any(|finding| {
        matches!(
            finding.severity,
            ReviewFindingSeverity::Blocking
                | ReviewFindingSeverity::MustFix
                | ReviewFindingSeverity::StrongRecommendFix
        )
    }) {
        return ReviewGate::RequiresRevision;
    }
    match verdict {
        ReviewVerdictType::Pass | ReviewVerdictType::NeedsHuman | ReviewVerdictType::Revise => {
            ReviewGate::UserConfirmAllowed
        }
    }
}
```

更新 `parse_review_verdict` 中调用：

```rust
let parsed = extract_tail_json(trimmed)
    .and_then(|(comments, json)| parse_review_json(&json, &comments));

parsed.unwrap_or_else(|| ReviewVerdict {
    verdict: ReviewVerdictType::NeedsHuman,
    comments: output.to_string(),
    summary: "需要人工确认".to_string(),
    findings: Vec::new(),
    review_gate: ReviewGate::UserConfirmAllowed,
})
```

删除 `review_comments_indicate_actionable_revision()` 及其调用，避免自然语言关键词把普通建议升级为强返修。

- [ ] **Step 5: 跑测试确认通过**

Run:

```bash
cargo test --locked --lib parse_review_verdict
```

Expected: `parse_review_verdict` 相关测试通过。

- [ ] **Step 6: 提交**

```bash
git add src/web/workspace_ws_types.rs src/product/workspace_engine.rs
git commit -m "feat: classify workspace review gates"
```

---

### Task 2: 后端状态机分流与 Prompt 收敛

**Files:**
- Modify: `src/product/workspace_engine.rs`
- Test: `src/product/workspace_engine.rs`

- [ ] **Step 1: 写 failing tests，覆盖三类 workspace 状态分流**

在 `src/product/workspace_engine.rs` 的 tests 中追加：

```rust
#[tokio::test]
async fn optional_review_findings_enter_human_confirm_for_all_workspace_types() {
    for workspace_type in [WorkspaceType::Story, WorkspaceType::Design, WorkspaceType::WorkItem] {
        let mut session = make_session(&format!("sess_optional_review_{workspace_type:?}"));
        session.workspace_type = workspace_type;
        session.review_rounds = 2;
        let mut engine = make_engine_with_provider(
            session,
            Arc::new(ReviewVerdictStreamingProvider {
                output: r#"建议补充说明。

```json
{
  "verdict": "revise",
  "summary": "仅有可选建议",
  "findings": [
    {
      "severity": "optional",
      "message": "可补充说明",
      "evidence": "当前主路径完整",
      "impact": "不影响下一阶段执行",
      "required_action": "可后续优化"
    }
  ]
}
```"#.to_string(),
            }),
        );
        engine.session.artifact = Some("# Artifact\n\n可用版本".to_string());
        engine.start_review_or_skip().await;
        engine.drive_current_provider_to_completion().await;

        assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
        assert!(engine
            .timeline_nodes
            .iter()
            .any(|node| node.node_type == TimelineNodeType::HumanConfirm));
        assert!(!engine
            .timeline_nodes
            .iter()
            .any(|node| node.node_type == TimelineNodeType::ReviewDecision));
    }
}

#[tokio::test]
async fn strong_review_findings_enter_review_decision_for_all_workspace_types() {
    for workspace_type in [WorkspaceType::Story, WorkspaceType::Design, WorkspaceType::WorkItem] {
        let mut session = make_session(&format!("sess_strong_review_{workspace_type:?}"));
        session.workspace_type = workspace_type;
        session.review_rounds = 2;
        let mut engine = make_engine_with_provider(
            session,
            Arc::new(ReviewVerdictStreamingProvider {
                output: r#"必须补充验收标准。

```json
{
  "verdict": "revise",
  "summary": "必须补充验收标准",
  "findings": [
    {
      "severity": "strong_recommend_fix",
      "message": "验收标准不足",
      "evidence": "Artifact 未列出可测试验收值",
      "impact": "下一阶段无法判断实现是否完成",
      "required_action": "补充明确验收标准"
    }
  ]
}
```"#.to_string(),
            }),
        );
        engine.session.artifact = Some("# Artifact\n\n缺少验收标准".to_string());
        engine.start_review_or_skip().await;
        engine.drive_current_provider_to_completion().await;

        assert_eq!(engine.session().stage, WorkspaceStage::ReviewDecision);
        assert!(engine
            .timeline_nodes
            .iter()
            .any(|node| node.node_type == TimelineNodeType::ReviewDecision));
    }
}
```

- [ ] **Step 2: 写 failing test，确认 Prompt 不再扩大返修**

追加测试：

```rust
#[test]
fn review_prompt_limits_revise_to_strong_findings() {
    let mut session = make_session("sess_review_prompt_gate");
    session.artifact = Some("# Story Spec\n\n可用版本".to_string());
    let engine = make_engine(session);

    let input = engine.build_review_input().expect("review input");

    assert!(input.prompt.contains("blocking|must_fix|strong_recommend_fix"));
    assert!(input.prompt.contains("suggestion|minor|optional"));
    assert!(input.prompt.contains("没有强返修 finding 时，必须允许用户确认当前版本"));
    assert!(!input.prompt.contains("High/Medium 问题、建议改动或可执行返修项，必须使用 `revise`"));
}
```

- [ ] **Step 3: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib review_prompt_limits_revise_to_strong_findings
cargo test --locked --lib optional_review_findings_enter_human_confirm_for_all_workspace_types
cargo test --locked --lib strong_review_findings_enter_review_decision_for_all_workspace_types
```

Expected: Prompt 测试因旧文案失败；状态机测试因 `complete_review()` 仍只看 `ReviewVerdictType::Revise` 失败。

- [ ] **Step 4: 修改 `complete_review()` 使用 review_gate**

在 `complete_review()` 中把 `mark_latest_artifact_reviewed` 的 verdict 参数改为：

```rust
let artifact_verdict = match verdict.review_gate {
    ReviewGate::RequiresRevision => ReviewVerdictType::Revise,
    ReviewGate::UserConfirmAllowed => match verdict.verdict {
        ReviewVerdictType::Pass => ReviewVerdictType::Pass,
        ReviewVerdictType::Revise | ReviewVerdictType::NeedsHuman => ReviewVerdictType::NeedsHuman,
    },
};
self.mark_latest_artifact_reviewed(reviewer, Some(artifact_verdict));
```

把 `match verdict.verdict` 改为：

```rust
match verdict.review_gate {
    ReviewGate::UserConfirmAllowed => {
        self.enter_human_confirm(Some(verdict.summary)).await;
    }
    ReviewGate::RequiresRevision => {
        self.transition_stage(WorkspaceStage::ReviewDecision).await;
        let decision_node_id = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::ReviewDecision,
                agent: None,
                stage: WorkspaceStage::ReviewDecision,
                round: Some(round),
                title: format!("Review Decision Round {round}"),
                summary: Some(verdict.summary),
                status: TimelineNodeStatus::Paused,
            })
            .await;
        let _ = self
            .event_tx
            .send(EngineEvent::ReviewDecisionRequired {
                node_id: decision_node_id,
                round,
                options: vec![
                    "continue".to_string(),
                    "continue_with_context".to_string(),
                    "human_intervene".to_string(),
                ],
            })
            .await;
    }
}
```

在 `persist_review_verdict()` 调用中保存完整 JSON：

```rust
serde_json::json!({
    "verdict": verdict.verdict.clone(),
    "comments": verdict.comments.clone(),
    "summary": verdict.summary.clone(),
    "findings": verdict.findings.clone(),
    "review_gate": verdict.review_gate.clone(),
})
```

- [ ] **Step 5: 修改 reviewer prompt**

替换 `build_review_input()` 末尾 JSON 合约文案为：

```rust
prompt.push_str(
    "\n\n请输出审核意见，并在末尾附加 JSON 代码块：\n\
     - 只有影响下一阶段可用性的 finding 才能标记为 `blocking`、`must_fix` 或 `strong_recommend_fix`。\n\
     - 风格、措辞、文档美化、未来扩展、非必要补充只能标记为 `suggestion`、`minor` 或 `optional`。\n\
     - 没有强返修 finding 时，必须允许用户确认当前版本，不要为了普通建议使用强返修。\n\
     - 第二轮及后续 review 只复核上一轮强返修项是否关闭；除非 revision 新引入真正阻塞问题，不得重新发散普通建议。\n\
     - `pass`：产物可进入最终人工确认。\n\
     - `revise`：仅当存在 blocking/must_fix/strong_recommend_fix finding。\n\
     - `needs_human`：没有明确可自动返修内容，需要用户做产品/范围判断。\n\
     ```json\n\
     {\"verdict\":\"pass|revise|needs_human\",\"summary\":\"一句话摘要\",\"findings\":[{\"severity\":\"blocking|must_fix|strong_recommend_fix|suggestion|minor|optional\",\"message\":\"问题描述\",\"evidence\":\"当前产物中的具体证据\",\"impact\":\"为什么影响或不影响下一阶段\",\"required_action\":\"需要作者执行的最小动作\"}]}\n\
     ```\n",
);
```

- [ ] **Step 6: 跑后端相关测试**

Run:

```bash
cargo test --locked --lib review_prompt_limits_revise_to_strong_findings
cargo test --locked --lib optional_review_findings_enter_human_confirm_for_all_workspace_types
cargo test --locked --lib strong_review_findings_enter_review_decision_for_all_workspace_types
cargo test --locked --lib parse_review_verdict
```

Expected: 全部通过。

- [ ] **Step 7: 提交**

```bash
git add src/product/workspace_engine.rs
git commit -m "feat: route workspace reviews by gate severity"
```

---

### Task 3: 前端类型与 review findings 分组展示

**Files:**
- Modify: `web/src/api/types.ts`
- Modify: `web/src/state/workspace-ws-store.ts`
- Modify: `web/src/components/chat-workspace/entries/ReviewVerdictEntry.tsx`
- Test: `web/src/components/chat-workspace/entries/p1-entries.test.tsx`

- [ ] **Step 1: 写 failing tests**

在 `web/src/components/chat-workspace/entries/p1-entries.test.tsx` 追加：

```tsx
it("groups review findings by required and optional severity", () => {
  const entry = makeEntry({
    type: "review_verdict",
    role: "reviewer",
    content: "存在需要解决和可选建议",
    metadata: {
      verdict: "revise",
      summary: "存在分级 findings",
      review_gate: "requires_revision",
      findings: [
        {
          severity: "must_fix",
          message: "缺少验证命令",
          evidence: "未出现验证命令段落",
          impact: "Coding Workspace 无法执行验收",
          required_action: "补充验证命令",
        },
        {
          severity: "optional",
          message: "可以补充复杂度说明",
          evidence: "主体方案完整",
          impact: "不影响下一阶段",
          required_action: "后续优化时补充",
        },
      ],
    },
  });

  render(<ReviewVerdictEntry entry={entry} />);

  expect(screen.getByText("需要解决")).toBeInTheDocument();
  expect(screen.getByText("缺少验证命令")).toBeInTheDocument();
  expect(screen.getByText("补充验证命令")).toBeInTheDocument();
  expect(screen.getByText("可选建议")).toBeInTheDocument();
  expect(screen.getByText("可以补充复杂度说明")).toBeInTheDocument();
});

it("labels optional-only review verdicts as confirmable", () => {
  const entry = makeEntry({
    type: "review_verdict",
    role: "reviewer",
    content: "仅有可选建议",
    metadata: {
      verdict: "needs_human",
      summary: "可确认当前版本",
      review_gate: "user_confirm_allowed",
      findings: [
        {
          severity: "suggestion",
          message: "建议优化措辞",
          evidence: "内容已覆盖主路径",
          impact: "不影响下一阶段",
          required_action: "可后续优化",
        },
      ],
    },
  });

  render(<ReviewVerdictEntry entry={entry} />);

  expect(screen.getByText("可确认当前版本")).toBeInTheDocument();
  expect(screen.getByText("建议优化措辞")).toBeInTheDocument();
  expect(screen.queryByText("需要解决")).not.toBeInTheDocument();
});
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
pnpm -C web exec vitest --run src/components/chat-workspace/entries/p1-entries.test.tsx
```

Expected: 新测试失败，页面没有 findings 分组。

- [ ] **Step 3: 扩展前端类型**

在 `web/src/api/types.ts` 中扩展 review 类型：

```ts
export type ReviewFindingSeverity =
  | "blocking"
  | "must_fix"
  | "strong_recommend_fix"
  | "suggestion"
  | "minor"
  | "optional";

export type ReviewGate = "requires_revision" | "user_confirm_allowed";

export type ReviewFinding = {
  severity: ReviewFindingSeverity;
  message: string;
  evidence: string;
  impact: string;
  required_action: string;
};

export type ReviewVerdict = {
  verdict: ReviewVerdictType;
  comments: string;
  summary: string;
  findings?: ReviewFinding[];
  review_gate?: ReviewGate;
};
```

在 `web/src/state/workspace-ws-store.ts` 的本地 `ReviewVerdict` interface 中同步字段：

```ts
export type ReviewFindingSeverity =
  | "blocking"
  | "must_fix"
  | "strong_recommend_fix"
  | "suggestion"
  | "minor"
  | "optional";

export type ReviewGate = "requires_revision" | "user_confirm_allowed";

export interface ReviewFinding {
  severity: ReviewFindingSeverity;
  message: string;
  evidence: string;
  impact: string;
  required_action: string;
}

export interface ReviewVerdict {
  verdict: ReviewVerdictType;
  comments: string;
  summary: string;
  findings?: ReviewFinding[];
  review_gate?: ReviewGate;
}
```

更新 `setNodeVerdict()` 和 `buildChatEntries()` metadata，确保保留：

```ts
findings: verdict.findings ?? [],
review_gate: verdict.review_gate ?? "user_confirm_allowed",
```

- [ ] **Step 4: 实现 ReviewVerdictEntry 分组**

在 `ReviewVerdictEntry.tsx` 中解析 metadata：

```tsx
type ReviewFinding = {
  severity: string;
  message: string;
  evidence?: string;
  impact?: string;
  required_action?: string;
};

function findingsFromEntry(entry: ChatEntry): ReviewFinding[] {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  const findings = Array.isArray(metadata?.findings) ? metadata.findings : [];
  return findings.filter(isReviewFinding);
}

function isReviewFinding(value: unknown): value is ReviewFinding {
  if (!value || typeof value !== "object") return false;
  const item = value as Record<string, unknown>;
  return typeof item.severity === "string" && typeof item.message === "string";
}

function isRequiredFinding(finding: ReviewFinding) {
  return (
    finding.severity === "blocking" ||
    finding.severity === "must_fix" ||
    finding.severity === "strong_recommend_fix"
  );
}
```

在组件内渲染：

```tsx
const findings = findingsFromEntry(entry);
const requiredFindings = findings.filter(isRequiredFinding);
const optionalFindings = findings.filter((finding) => !isRequiredFinding(finding));
```

增加 JSX：

```tsx
{requiredFindings.length > 0 ? (
  <FindingGroup title="需要解决" findings={requiredFindings} tone="required" />
) : null}
{optionalFindings.length > 0 ? (
  <FindingGroup title="可选建议" findings={optionalFindings} tone="optional" />
) : null}
```

新增子组件：

```tsx
function FindingGroup({
  title,
  findings,
  tone,
}: {
  title: string;
  findings: ReviewFinding[];
  tone: "required" | "optional";
}) {
  const titleClass =
    tone === "required" ? "text-red-800" : "text-[var(--aria-ink)]";
  return (
    <div className="space-y-2 rounded-md border border-amber-200 bg-white p-2">
      <div className={`text-xs font-semibold ${titleClass}`}>{title}</div>
      <div className="space-y-2">
        {findings.map((finding, index) => (
          <div key={`${finding.severity}-${index}`} className="space-y-1 text-xs">
            <div className="font-semibold text-[var(--aria-ink)]">{finding.message}</div>
            {finding.evidence ? <div className="text-[var(--aria-ink-muted)]">证据：{finding.evidence}</div> : null}
            {finding.impact ? <div className="text-[var(--aria-ink-muted)]">影响：{finding.impact}</div> : null}
            {finding.required_action ? (
              <div className="text-[var(--aria-ink-muted)]">动作：{finding.required_action}</div>
            ) : null}
          </div>
        ))}
      </div>
    </div>
  );
}
```

更新 `verdictLabel()`：

```tsx
function verdictLabel(verdict: string | null, reviewGate?: string | null) {
  if (reviewGate === "requires_revision") return "需要解决后再继续";
  if (reviewGate === "user_confirm_allowed") return "可确认当前版本";
  if (verdict === "pass") return "通过";
  if (verdict === "revise") return "建议返修";
  if (verdict === "needs_human") return "需要人工确认";
  return "审核结论";
}
```

- [ ] **Step 5: 跑前端 entry 测试**

Run:

```bash
pnpm -C web exec vitest --run src/components/chat-workspace/entries/p1-entries.test.tsx
```

Expected: 通过。

- [ ] **Step 6: 提交**

```bash
git add web/src/api/types.ts web/src/state/workspace-ws-store.ts web/src/components/chat-workspace/entries/ReviewVerdictEntry.tsx web/src/components/chat-workspace/entries/p1-entries.test.tsx
git commit -m "feat: show workspace review findings by gate"
```

---

### Task 4: 前端 human_confirm 可确认当前版本交互

**Files:**
- Modify: `web/src/components/chat-workspace/entries/GatePromptEntry.tsx`
- Modify: `web/src/pages/ChatWorkspacePage.test.tsx`
- Test: `web/src/components/chat-workspace/entries/p1-entries.test.tsx`
- Test: `web/src/pages/ChatWorkspacePage.test.tsx`

- [ ] **Step 1: 写 failing test，确认 human_confirm 显示确认当前版本**

在 `p1-entries.test.tsx` 追加：

```tsx
it("renders user confirm wording when review gate allows current version", () => {
  const onDecision = vi.fn();
  const entry = makeEntry({
    type: "gate_prompt",
    role: "system",
    content: "等待人工确认",
    metadata: {
      verdict: "needs_human",
      review_gate: "user_confirm_allowed",
      summary: "仅有可选建议",
    },
  });

  render(<GatePromptEntry entry={entry} onDecision={onDecision} />);
  fireEvent.click(screen.getByRole("button", { name: "确认使用当前版本" }));

  expect(onDecision).toHaveBeenCalledWith("confirm");
});
```

在 `ChatWorkspacePage.test.tsx` 追加：

```tsx
it("allows confirming the current version from human confirm after optional review findings", async () => {
  const api = mockWorkspaceWs();
  useWorkspaceStore.setState({
    sessionId: "workspace_session_0001",
    workspaceType: "design",
    stage: "human_confirm",
    providers: { author: "claude_code", reviewer: "codex" },
    timelineNodes: [
      timelineNode({
        node_id: "timeline_node_human",
        node_type: "human_confirm",
        stage: "human_confirm",
        status: "paused",
        title: "人工确认",
        summary: "仅有可选建议",
      }),
    ],
    chatEntries: [
      chatEntry({
        type: "review_verdict",
        role: "reviewer",
        content: "仅有可选建议",
        metadata: {
          verdict: "needs_human",
          summary: "仅有可选建议",
          review_gate: "user_confirm_allowed",
          findings: [
            {
              severity: "suggestion",
              message: "建议补充说明",
              evidence: "当前版本可用",
              impact: "不影响下一阶段",
              required_action: "可后续优化",
            },
          ],
        },
      }),
      chatEntry({
        id: "timeline_node_human:gate-prompt",
        type: "gate_prompt",
        role: "system",
        content: "等待人工确认",
        node_id: "timeline_node_human",
        metadata: {
          verdict: "needs_human",
          summary: "仅有可选建议",
          review_gate: "user_confirm_allowed",
        },
      }),
    ],
  });

  render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

  await userEvent.click(screen.getByRole("button", { name: "确认使用当前版本" }));

  expect(api.sendHumanConfirm).toHaveBeenCalledWith("confirm");
});
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
pnpm -C web exec vitest --run src/components/chat-workspace/entries/p1-entries.test.tsx src/pages/ChatWorkspacePage.test.tsx
```

Expected: 找不到 `确认使用当前版本` 按钮。

- [ ] **Step 3: 修改 GatePromptEntry 文案**

在 `GatePromptEntry.tsx` 增加 helper：

```tsx
function reviewGateFromEntry(entry: ChatEntry) {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  return typeof metadata?.review_gate === "string" ? metadata.review_gate : null;
}
```

组件内增加：

```tsx
const reviewGate = reviewGateFromEntry(entry);
const confirmLabel =
  reviewGate === "user_confirm_allowed"
    ? "确认使用当前版本"
    : needsHuman
      ? "提交人工确认"
      : "确认产物";
```

替换确认按钮文案：

```tsx
{confirmLabel}
```

- [ ] **Step 4: 跑前端测试**

Run:

```bash
pnpm -C web exec vitest --run src/components/chat-workspace/entries/p1-entries.test.tsx src/pages/ChatWorkspacePage.test.tsx
```

Expected: 通过。

- [ ] **Step 5: 提交**

```bash
git add web/src/components/chat-workspace/entries/GatePromptEntry.tsx web/src/components/chat-workspace/entries/p1-entries.test.tsx web/src/pages/ChatWorkspacePage.test.tsx
git commit -m "feat: allow confirming non-blocking workspace reviews"
```

---

### Task 5: Bounded content cache 纯函数

**Files:**
- Create: `web/src/state/workspace-content-cache.ts`
- Test: `web/src/state/workspace-content-cache.test.ts`

- [ ] **Step 1: 写 failing tests**

创建 `web/src/state/workspace-content-cache.test.ts`：

```ts
import { describe, expect, it } from "vitest";
import {
  emptyWorkspaceContentCache,
  getWorkspaceContentCacheValue,
  setWorkspaceContentCacheEntry,
  workspaceContentCacheValues,
} from "./workspace-content-cache";

describe("workspace content cache", () => {
  it("stores and reads cached values", () => {
    const cache = setWorkspaceContentCacheEntry(
      emptyWorkspaceContentCache(100),
      "prompt:1",
      "abc",
      10,
    );

    expect(getWorkspaceContentCacheValue(cache, "prompt:1", 20)?.value).toBe("abc");
    expect(workspaceContentCacheValues(cache)).toEqual({ "prompt:1": "abc" });
  });

  it("evicts least recently used entries when byte budget is exceeded", () => {
    let cache = emptyWorkspaceContentCache(6);
    cache = setWorkspaceContentCacheEntry(cache, "a", "aaa", 1);
    cache = setWorkspaceContentCacheEntry(cache, "b", "bbb", 2);
    cache = getWorkspaceContentCacheValue(cache, "a", 3)?.cache ?? cache;
    cache = setWorkspaceContentCacheEntry(cache, "c", "ccc", 4);

    expect(workspaceContentCacheValues(cache)).toEqual({ a: "aaa", c: "ccc" });
  });

  it("keeps oversized latest entries by evicting older entries", () => {
    let cache = emptyWorkspaceContentCache(4);
    cache = setWorkspaceContentCacheEntry(cache, "a", "aa", 1);
    cache = setWorkspaceContentCacheEntry(cache, "large", "123456", 2);

    expect(workspaceContentCacheValues(cache)).toEqual({ large: "123456" });
    expect(cache.totalBytes).toBe(6);
  });
});
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
pnpm -C web exec vitest --run src/state/workspace-content-cache.test.ts
```

Expected: 模块不存在。

- [ ] **Step 3: 创建 bounded cache 实现**

创建 `web/src/state/workspace-content-cache.ts`：

```ts
export type WorkspaceContentCacheEntry = {
  value: string;
  bytes: number;
  lastAccessed: number;
};

export type WorkspaceContentCache = {
  maxBytes: number;
  totalBytes: number;
  entries: Record<string, WorkspaceContentCacheEntry>;
};

const DEFAULT_MAX_BYTES = 30 * 1024 * 1024;

export function emptyWorkspaceContentCache(maxBytes = DEFAULT_MAX_BYTES): WorkspaceContentCache {
  return { maxBytes, totalBytes: 0, entries: {} };
}

export function setWorkspaceContentCacheEntry(
  cache: WorkspaceContentCache,
  key: string,
  value: string,
  now = Date.now(),
): WorkspaceContentCache {
  const bytes = byteLength(value);
  const previous = cache.entries[key];
  const entries = {
    ...cache.entries,
    [key]: { value, bytes, lastAccessed: now },
  };
  const totalBytes = cache.totalBytes - (previous?.bytes ?? 0) + bytes;
  return trimCache({ ...cache, entries, totalBytes }, key);
}

export function getWorkspaceContentCacheValue(
  cache: WorkspaceContentCache,
  key: string,
  now = Date.now(),
): { value: string; cache: WorkspaceContentCache } | null {
  const entry = cache.entries[key];
  if (!entry) {
    return null;
  }
  return {
    value: entry.value,
    cache: {
      ...cache,
      entries: {
        ...cache.entries,
        [key]: { ...entry, lastAccessed: now },
      },
    },
  };
}

export function workspaceContentCacheValues(cache: WorkspaceContentCache): Record<string, string> {
  return Object.fromEntries(
    Object.entries(cache.entries).map(([key, entry]) => [key, entry.value]),
  );
}

function trimCache(cache: WorkspaceContentCache, protectedKey: string): WorkspaceContentCache {
  if (cache.totalBytes <= cache.maxBytes) {
    return cache;
  }
  const entries = { ...cache.entries };
  let totalBytes = cache.totalBytes;
  const evictionCandidates = Object.entries(entries)
    .filter(([key]) => key !== protectedKey)
    .sort((left, right) => left[1].lastAccessed - right[1].lastAccessed);

  for (const [key, entry] of evictionCandidates) {
    if (totalBytes <= cache.maxBytes) {
      break;
    }
    delete entries[key];
    totalBytes -= entry.bytes;
  }

  return { ...cache, entries, totalBytes };
}

function byteLength(value: string) {
  return new TextEncoder().encode(value).length;
}
```

- [ ] **Step 4: 跑 cache 测试**

Run:

```bash
pnpm -C web exec vitest --run src/state/workspace-content-cache.test.ts
```

Expected: 通过。

- [ ] **Step 5: 提交**

```bash
git add web/src/state/workspace-content-cache.ts web/src/state/workspace-content-cache.test.ts
git commit -m "feat: add bounded workspace content cache"
```

---

### Task 6: Store 接入 bounded cache 与 node detail hydration

**Files:**
- Modify: `web/src/state/workspace-ws-store.ts`
- Modify: `web/src/api/workspace-content.ts`
- Modify: `web/src/api/types.ts`
- Test: `web/src/state/workspace-ws-store.test.ts`
- Test: `web/src/pages/ChatWorkspacePage.test.tsx`

- [ ] **Step 1: 写 failing store tests**

在 `web/src/state/workspace-ws-store.test.ts` 追加：

```ts
it("evicts content cache entries by byte budget", () => {
  const store = useWorkspaceStore.getState();
  useWorkspaceStore.setState({
    contentCache: emptyWorkspaceContentCache(6),
  });

  store.setContentCacheEntry("a", "aaa", 1);
  store.setContentCacheEntry("b", "bbb", 2);
  store.touchContentCacheEntry("a", 3);
  store.setContentCacheEntry("c", "ccc", 4);

  expect(workspaceContentCacheValues(useWorkspaceStore.getState().contentCache)).toEqual({
    a: "aaa",
    c: "ccc",
  });
});

it("merges hydrated node detail and rebuilds chat entries", () => {
  const store = useWorkspaceStore.getState();
  useWorkspaceStore.setState({
    sessionId: "workspace_session_0001",
    timelineNodes: [timelineNode({ node_id: "node-1", node_type: "reviewer_run" })],
    nodeDetails: {
      "node-1": makeNodeDetail({
        node_id: "node-1",
        streaming_content: "summary only",
      }),
    },
  });

  store.setNodeDetail(
    makeNodeDetail({
      node_id: "node-1",
      streaming_content: "complete review output",
      verdict: {
        verdict: "needs_human",
        comments: "完整 comments",
        summary: "仅有可选建议",
        findings: [],
        review_gate: "user_confirm_allowed",
      },
    }),
  );

  expect(useWorkspaceStore.getState().nodeDetails["node-1"].streaming_content).toBe(
    "complete review output",
  );
  expect(useWorkspaceStore.getState().chatEntries.some((entry) => entry.content.includes("complete review output"))).toBe(true);
});
```

需要在测试文件 import：

```ts
import {
  emptyWorkspaceContentCache,
  workspaceContentCacheValues,
} from "./workspace-content-cache";
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
pnpm -C web exec vitest --run src/state/workspace-ws-store.test.ts
```

Expected: `setNodeDetail`、`touchContentCacheEntry` 或 cache 类型不存在。

- [ ] **Step 3: 修改 API 类型**

在 `web/src/api/types.ts` 中把：

```ts
export type WorkspaceNodeDetailResponse = unknown;
```

改为：

```ts
export type WorkspaceNodeDetailResponse = NodeDetail;
```

`web/src/api/workspace-content.ts` 无需改函数体，返回类型会随 type alias 生效。

- [ ] **Step 4: 修改 workspace store cache 类型和 actions**

在 `workspace-ws-store.ts` import：

```ts
import {
  emptyWorkspaceContentCache,
  getWorkspaceContentCacheValue,
  setWorkspaceContentCacheEntry,
  type WorkspaceContentCache,
} from "./workspace-content-cache";
```

把 state 字段改为：

```ts
contentCache: WorkspaceContentCache;
artifactContentCache: WorkspaceContentCache;
```

把 actions 改为：

```ts
setNodeDetail: (detail: TimelineNodeDetail) => void;
setContentCacheEntry: (key: string, value: string, now?: number) => void;
touchContentCacheEntry: (key: string, now?: number) => void;
setArtifactContentCacheEntry: (version: number, value: string, now?: number) => void;
touchArtifactContentCacheEntry: (version: number, now?: number) => void;
```

初始 state：

```ts
contentCache: emptyWorkspaceContentCache(),
artifactContentCache: emptyWorkspaceContentCache(),
```

session 切换时：

```ts
contentCache:
  prev.sessionId === state.session_id ? prev.contentCache : emptyWorkspaceContentCache(),
artifactContentCache:
  prev.sessionId === state.session_id ? prev.artifactContentCache : emptyWorkspaceContentCache(),
```

新增 action 实现：

```ts
setNodeDetail: (detail) =>
  set((prev) => {
    const nodeDetails = { ...prev.nodeDetails, [detail.node_id]: detail };
    const nextState = { ...prev, nodeDetails };
    return {
      nodeDetails,
      chatEntries: buildChatEntries(nextState),
    };
  }),

setContentCacheEntry: (key, value, now) =>
  set((prev) => ({
    contentCache: setWorkspaceContentCacheEntry(prev.contentCache, key, value, now),
  })),

touchContentCacheEntry: (key, now) =>
  set((prev) => {
    const touched = getWorkspaceContentCacheValue(prev.contentCache, key, now);
    return touched ? { contentCache: touched.cache } : {};
  }),

setArtifactContentCacheEntry: (version, value, now) =>
  set((prev) => ({
    artifactContentCache: setWorkspaceContentCacheEntry(
      prev.artifactContentCache,
      String(version),
      value,
      now,
    ),
  })),

touchArtifactContentCacheEntry: (version, now) =>
  set((prev) => {
    const touched = getWorkspaceContentCacheValue(prev.artifactContentCache, String(version), now);
    return touched ? { artifactContentCache: touched.cache } : {};
  }),
```

- [ ] **Step 5: 跑 store 测试**

Run:

```bash
pnpm -C web exec vitest --run src/state/workspace-ws-store.test.ts
```

Expected: 通过。

- [ ] **Step 6: 更新现有前端测试 fixture 的 cache 初始化**

把已经直接写 `contentCache: {}` 的测试改为：

```ts
contentCache: emptyWorkspaceContentCache(),
```

把已经直接写 `artifactContentCache: {}` 的测试改为：

```ts
artifactContentCache: emptyWorkspaceContentCache(),
```

把已经断言 `contentCache` values 的测试改为：

```ts
expect(workspaceContentCacheValues(useWorkspaceStore.getState().contentCache)).toEqual({});
```

把已经断言 `artifactContentCache[1]` 的测试改为：

```ts
expect(workspaceContentCacheValues(useWorkspaceStore.getState().artifactContentCache)["1"]).toBe(
  "# Loaded Artifact\n\n内容",
);
```

Run:

```bash
pnpm -C web exec vitest --run src/pages/ChatWorkspacePage.test.tsx src/components/chat-workspace/ArtifactPane.test.tsx src/components/chat-workspace/InlineEventRow.test.tsx src/state/workspace-ws-store.test.ts
```

Expected: 通过。

- [ ] **Step 7: 提交**

```bash
git add web/src/state/workspace-ws-store.ts web/src/state/workspace-ws-store.test.ts web/src/api/types.ts web/src/api/workspace-content.ts
git commit -m "feat: hydrate workspace node details into bounded cache"
```

---

### Task 7: ChatWorkspacePage 自动 Hydration 与组件 cache values 适配

**Files:**
- Modify: `web/src/pages/ChatWorkspacePage.tsx`
- Modify: `web/src/components/chat-workspace/InlineEventRow.tsx`
- Modify: `web/src/components/chat-workspace/ArtifactPane.tsx`
- Modify: `web/src/components/chat-workspace/ChatEntryList.tsx`
- Modify: `web/src/components/chat-workspace/MessageGroupView.tsx`
- Test: `web/src/pages/ChatWorkspacePage.test.tsx`
- Test: `web/src/components/chat-workspace/InlineEventRow.test.tsx`
- Test: `web/src/components/chat-workspace/ArtifactPane.test.tsx`

- [ ] **Step 1: 写 failing page hydration test**

在 `ChatWorkspacePage.test.tsx` 扩展 imports：

```ts
import {
  fetchWorkspaceArtifactVersion,
  fetchWorkspaceEventOutput,
  fetchWorkspaceNodeDetail,
} from "../api/workspace-content";
```

mock 增加：

```ts
fetchWorkspaceNodeDetail: vi.fn(),
```

新增测试：

```tsx
it("hydrates selected node detail after restored lightweight session state", async () => {
  mockWorkspaceWs();
  vi.mocked(fetchWorkspaceNodeDetail).mockResolvedValue(
    makeNodeDetail({
      node_id: "timeline_node_017",
      streaming_content: "完整 review 输出",
      verdict: {
        verdict: "needs_human",
        comments: "完整 comments",
        summary: "仅有可选建议",
        findings: [],
        review_gate: "user_confirm_allowed",
      },
    }),
  );
  useWorkspaceStore.setState({
    sessionId: "workspace_session_0001",
    workspaceType: "design",
    stage: "human_confirm",
    selectedNodeId: "timeline_node_017",
    activeNodeId: "timeline_node_017",
    timelineNodes: [
      timelineNode({
        node_id: "timeline_node_017",
        node_type: "reviewer_run",
        title: "Review Round 1",
        status: "completed",
      }),
    ],
    nodeDetails: {
      timeline_node_017: makeNodeDetail({
        node_id: "timeline_node_017",
        streaming_content: "摘要",
      }),
    },
  });

  render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

  await waitFor(() => {
    expect(fetchWorkspaceNodeDetail).toHaveBeenCalledWith(
      "workspace_session_0001",
      "timeline_node_017",
    );
  });
  expect(await screen.findByText("完整 review 输出")).toBeInTheDocument();
});
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
pnpm -C web exec vitest --run src/pages/ChatWorkspacePage.test.tsx -t "hydrates selected node detail"
```

Expected: `fetchWorkspaceNodeDetail` 未被调用。

- [ ] **Step 3: 在 ChatWorkspacePage 加 node detail hydration effect**

在 import 中加入：

```ts
fetchWorkspaceNodeDetail,
```

在组件内增加：

```tsx
const hydratedNodeIdsRef = useRef<Set<string>>(new Set());

useEffect(() => {
  hydratedNodeIdsRef.current.clear();
}, [sessionId]);

useEffect(() => {
  if (!sessionReady) {
    return;
  }
  const nodeIds = [selectedNodeId, activeNodeId]
    .filter((nodeId): nodeId is string => typeof nodeId === "string" && nodeId.length > 0);
  for (const nodeId of nodeIds) {
    if (hydratedNodeIdsRef.current.has(nodeId)) {
      continue;
    }
    hydratedNodeIdsRef.current.add(nodeId);
    fetchWorkspaceNodeDetail(sessionId, nodeId)
      .then((detail) => {
        const state = useWorkspaceStore.getState();
        if (state.sessionId !== sessionId) {
          return;
        }
        state.setNodeDetail(detail);
      })
      .catch(() => {
        hydratedNodeIdsRef.current.delete(nodeId);
      });
  }
}, [activeNodeId, selectedNodeId, sessionId, sessionReady]);
```

- [ ] **Step 4: 适配 bounded cache values**

在 `ChatWorkspacePage.tsx` import：

```ts
import { workspaceContentCacheValues } from "../state/workspace-content-cache";
```

将传入 `ChatEntryList` 的 `contentCache` 改为：

```tsx
contentCache={workspaceContentCacheValues(contentCache)}
```

将传入 `ArtifactPane` 的 `artifactContentCache` 改为：

```tsx
artifactContentCache={artifactContentCacheValues(artifactContentCache)}
```

新增 helper：

```tsx
function artifactContentCacheValues(cache: ReturnType<typeof useWorkspaceStore.getState>["artifactContentCache"]) {
  const values = workspaceContentCacheValues(cache);
  return Object.fromEntries(
    Object.entries(values).map(([version, markdown]) => [Number(version), markdown]),
  );
}
```

在 `handleCacheContent` 中保留原 action 调用：

```tsx
state.setContentCacheEntry(key, value);
```

在 `handleCacheArtifactContent` 中保留：

```tsx
state.setArtifactContentCacheEntry(version, value);
```

- [ ] **Step 5: 跑相关前端测试**

Run:

```bash
pnpm -C web exec vitest --run src/pages/ChatWorkspacePage.test.tsx src/components/chat-workspace/InlineEventRow.test.tsx src/components/chat-workspace/ArtifactPane.test.tsx
```

Expected: 通过。

- [ ] **Step 6: 提交**

```bash
git add web/src/pages/ChatWorkspacePage.tsx web/src/pages/ChatWorkspacePage.test.tsx web/src/components/chat-workspace/InlineEventRow.tsx web/src/components/chat-workspace/ArtifactPane.tsx web/src/components/chat-workspace/ChatEntryList.tsx web/src/components/chat-workspace/MessageGroupView.tsx
git commit -m "feat: hydrate restored workspace chat content"
```

---

### Task 8: 全量验证与真实 E2E 检查清单

**Files:**
- Modify: `cadence/reports/2026-06-08_进度报告_WorkspaceReviewGate与内存治理验证_v1.0.md`

- [ ] **Step 1: 跑后端定向测试**

Run:

```bash
cargo test --locked --lib parse_review_verdict
cargo test --locked --lib optional_review_findings_enter_human_confirm_for_all_workspace_types
cargo test --locked --lib strong_review_findings_enter_review_decision_for_all_workspace_types
cargo test --locked --lib review_prompt_limits_revise_to_strong_findings
```

Expected: 全部通过。

- [ ] **Step 2: 跑前端定向测试**

Run:

```bash
pnpm -C web exec vitest --run src/state/workspace-content-cache.test.ts src/state/workspace-ws-store.test.ts src/components/chat-workspace/entries/p1-entries.test.tsx src/pages/ChatWorkspacePage.test.tsx
```

Expected: 全部通过。

- [ ] **Step 3: 跑前端 build**

Run:

```bash
pnpm -C web build
```

Expected: build 成功；允许既有 Vite chunk size warning。

- [ ] **Step 4: 跑 Rust check**

Run:

```bash
cargo check --locked
```

Expected: check 成功。

- [ ] **Step 5: 写验证报告**

创建 `cadence/reports/2026-06-08_进度报告_WorkspaceReviewGate与内存治理验证_v1.0.md`：

```markdown
# Workspace Review Gate 与内存治理验证报告

## 变更范围

- Review Gate 分级：完成
- 非阻塞 review 允许确认当前版本：完成
- 刷新恢复消息 hydration：完成
- Bounded content cache：完成

## 自动化验证

| 命令 | 结果 |
| --- | --- |
| `cargo test --locked --lib parse_review_verdict` | 通过 |
| `cargo test --locked --lib optional_review_findings_enter_human_confirm_for_all_workspace_types` | 通过 |
| `cargo test --locked --lib strong_review_findings_enter_review_decision_for_all_workspace_types` | 通过 |
| `cargo test --locked --lib review_prompt_limits_revise_to_strong_findings` | 通过 |
| `pnpm -C web exec vitest --run src/state/workspace-content-cache.test.ts src/state/workspace-ws-store.test.ts src/components/chat-workspace/entries/p1-entries.test.tsx src/pages/ChatWorkspacePage.test.tsx` | 通过 |
| `pnpm -C web build` | 通过 |
| `cargo check --locked` | 通过 |

## 真实 E2E 检查项

- Story：reviewer 只有可选建议时，页面进入 `human_confirm`，可点击 `确认使用当前版本`。
- Design：reviewer 有 `strong_recommend_fix` 时，页面进入 `review_decision`，显示返修按钮。
- Work Item：刷新已落盘 workspace 后，选中 reviewer timeline node 能加载完整 review 输出。
- 多轮 review 后，前端 content cache 不无限增长，超过预算会淘汰旧内容。

## 残余风险

- 旧落盘 review 没有 findings 时只能进入人工确认兼容路径。
- Provider 仍可能输出不合规 JSON，后端会降级为 `needs_human`。
```

- [ ] **Step 6: 提交验证报告**

```bash
git add cadence/reports/2026-06-08_进度报告_WorkspaceReviewGate与内存治理验证_v1.0.md
git commit -m "docs: record workspace review gate verification"
```

---

## Self-Review

- Spec coverage:
  - Review 分级、Prompt 收敛、后端状态分流：Task 1、Task 2 覆盖。
  - 非阻塞建议允许确认当前版本：Task 3、Task 4 覆盖。
  - 刷新恢复消息通过 API 补全：Task 6、Task 7 覆盖。
  - 前端内存预算与大文本缓存：Task 5、Task 6、Task 7 覆盖。
  - Story / Design / Work Item 三类一致性：Task 2 后端表驱动测试、Task 8 E2E 检查覆盖。
- Placeholder scan:
  - 本计划不包含占位任务、延后实现项或缺少具体内容的步骤。
  - 每个实现任务都包含测试、实现片段、验证命令和提交命令。
- Type consistency:
  - Rust 使用 `ReviewFindingSeverity`、`ReviewFinding`、`ReviewGate`、`ReviewVerdict`。
  - TypeScript 使用 `ReviewFindingSeverity`、`ReviewFinding`、`ReviewGate`、`ReviewVerdict`。
  - 前端 store 使用 `setNodeDetail`、`setContentCacheEntry`、`touchContentCacheEntry`、`setArtifactContentCacheEntry`、`touchArtifactContentCacheEntry`。
