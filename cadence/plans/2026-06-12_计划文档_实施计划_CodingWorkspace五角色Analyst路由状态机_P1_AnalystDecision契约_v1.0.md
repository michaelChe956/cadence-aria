# CodingWorkspace AnalystDecision 契约 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 CodingWorkspace Analyst 引入结构化 `AnalystDecision` 契约和持久化，为后续状态机路由重构提供稳定数据基础。

**Architecture:** 继续复用现有 `CodingExecutionStage::Rework` 作为 Analyst 执行阶段。新增结构化 decision record 持久化到 attempt 的 `analyst-decisions/` 目录，同时保留现有 `AnalystVerdict` chat entry 兼容层，P1 不改变当前路由行为。

**Tech Stack:** Rust 1.95.0、Cargo、serde、serde_json、tokio integration tests。

---

## 范围边界

P1 做：

- 新增结构化 Analyst 决策模型。
- 新增 attempt store 读写 `analyst-decisions/`。
- 解析 Analyst 新 schema，并兼容旧 schema。
- `execute_rework_with_commands` 保存 decision record。
- 补充模型、store、engine 定向回归测试。

P1 不做：

- 不修改 Testing passed/blocked 是否进入 Analyst 的状态机策略。
- 不消费 `next_stage` 改变当前路由。
- 不改前端 UI。
- 不改 `manual_continue` 行为。

## 文件结构

- Modify: `src/product/coding_models.rs`
  - 新增 Analyst decision 相关 public model。
- Modify: `src/product/coding_attempt_store.rs`
  - 新增 `analyst_decisions_root`、保存、列表、最新记录读取方法。
- Modify: `src/product/coding_workspace_engine.rs`
  - 扩展 Analyst provider payload 解析。
  - 将解析结果保存为 `AnalystDecisionRecord`。
  - 保留旧 `AnalystVerdict` chat entry 和当前 `apply_analyst_decision` 行为。
- Modify: `tests/it_product/product_coding_models.rs`
  - 覆盖新模型 wire JSON。
- Modify: `tests/it_product/product_coding_attempt_store.rs`
  - 覆盖 decision record 保存和读取。
- Modify: `tests/it_product/product_coding_workspace_engine.rs`
  - 覆盖新 schema 解析、持久化、旧 schema 兼容。

## 数据契约

新增 provider 推荐 schema：

```json
{
  "verdict": "needs_fix",
  "next_stage": "coding",
  "reason": "required 测试步骤被跳过，需要补齐实现或测试覆盖",
  "evidence_refs": ["testing_report_0001.json"],
  "raw_provider_output_refs": ["provider-raw/testing/execute_test_plan_0001.txt"],
  "rework_instructions": {
    "summary": "补齐 B6/B7 required 测试覆盖",
    "required_changes": ["补充 required browser steps 的可执行测试"],
    "verification_expectations": ["Tester 重跑时 B6/B7 不再出现在 skipped_required_steps"]
  },
  "human_gate": null
}
```

旧 schema 继续支持：

```json
{
  "verdict": "needs_fix",
  "summary": "测试仍失败",
  "fix_hints": ["补充 climb_stairs 动态规划实现"],
  "questions": []
}
```

旧 schema 归一化规则：

| 旧 verdict | 新 verdict | 默认 next_stage |
|------------|------------|-----------------|
| `needs_fix` | `needs_fix` | `coding` |
| `needs_human_input` | `human_required` | `human_gate` |
| `no_issue` from Testing | `proceed` | `code_review` |
| `no_issue` from CodeReview | `proceed` | `review_request` |
| `no_issue` from InternalPrReview | `proceed` | `final_confirm` |

## Task 1: 模型测试先行

**Files:**
- Modify: `tests/it_product/product_coding_models.rs`
- Modify later: `src/product/coding_models.rs`

- [ ] **Step 1: 在 import 列表加入新类型**

在 `tests/it_product/product_coding_models.rs` 顶部 `use cadence_aria::product::coding_models::{ ... }` 中加入：

```rust
AnalystDecisionNextStage, AnalystDecisionRecord, AnalystDecisionVerdict,
AnalystHumanGateRecommendation, AnalystReworkInstructions,
```

- [ ] **Step 2: 写失败测试**

在 `tests/it_product/product_coding_models.rs` 中加入：

```rust
#[test]
fn analyst_decision_record_uses_stable_wire_values() {
    let record = AnalystDecisionRecord {
        id: "analyst_decision_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        source_stage: CodingExecutionStage::Testing,
        rework_round: 1,
        verdict: AnalystDecisionVerdict::NeedsFix,
        next_stage: AnalystDecisionNextStage::Coding,
        reason: "required 测试步骤被跳过".to_string(),
        evidence_refs: vec!["testing_report_0001.json".to_string()],
        raw_provider_output_refs: vec![
            "provider-raw/testing/execute_test_plan_0001.txt".to_string(),
        ],
        rework_instructions: Some(AnalystReworkInstructions {
            summary: "补齐 required 测试覆盖".to_string(),
            required_changes: vec!["补充 B6 浏览器测试".to_string()],
            verification_expectations: vec!["B6 不再出现在 skipped_required_steps".to_string()],
        }),
        human_gate: Some(AnalystHumanGateRecommendation {
            reason_code: Some("external_browser_required".to_string()),
            available_actions: vec!["provide_context".to_string(), "manual_continue".to_string()],
        }),
        created_at: "2026-06-12T00:00:00Z".to_string(),
        parse_error: None,
    };

    let value = serde_json::to_value(&record).expect("serialize decision");
    assert_eq!(
        value,
        json!({
            "id": "analyst_decision_0001",
            "attempt_id": "coding_attempt_0001",
            "source_stage": "testing",
            "rework_round": 1,
            "verdict": "needs_fix",
            "next_stage": "coding",
            "reason": "required 测试步骤被跳过",
            "evidence_refs": ["testing_report_0001.json"],
            "raw_provider_output_refs": ["provider-raw/testing/execute_test_plan_0001.txt"],
            "rework_instructions": {
                "summary": "补齐 required 测试覆盖",
                "required_changes": ["补充 B6 浏览器测试"],
                "verification_expectations": ["B6 不再出现在 skipped_required_steps"]
            },
            "human_gate": {
                "reason_code": "external_browser_required",
                "available_actions": ["provide_context", "manual_continue"]
            },
            "created_at": "2026-06-12T00:00:00Z",
            "parse_error": null
        })
    );

    let parsed: AnalystDecisionRecord =
        serde_json::from_value(value).expect("deserialize decision");
    assert_eq!(parsed, record);
}
```

- [ ] **Step 3: 运行测试，确认失败**

Run:

```bash
cargo test --locked --test it_product analyst_decision_record_uses_stable_wire_values
```

Expected:

```text
cannot find type `AnalystDecisionRecord` in this scope
```

## Task 2: 新增 AnalystDecision 模型

**Files:**
- Modify: `src/product/coding_models.rs`
- Test: `tests/it_product/product_coding_models.rs`

- [ ] **Step 1: 添加模型定义**

在 `src/product/coding_models.rs` 的 `AnalystVerdict` 后加入：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalystDecisionVerdict {
    NeedsFix,
    RerunTesting,
    Proceed,
    HumanRequired,
    Blocked,
}

impl AnalystDecisionVerdict {
    pub fn legacy_chat_verdict(&self) -> AnalystVerdict {
        match self {
            Self::NeedsFix | Self::RerunTesting => AnalystVerdict::NeedsFix,
            Self::Proceed => AnalystVerdict::NoIssue,
            Self::HumanRequired | Self::Blocked => AnalystVerdict::NeedsHumanInput,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalystDecisionNextStage {
    Coding,
    Testing,
    CodeReview,
    ReviewRequest,
    InternalPrReview,
    FinalConfirm,
    HumanGate,
}

impl AnalystDecisionNextStage {
    pub fn execution_stage(&self) -> Option<CodingExecutionStage> {
        match self {
            Self::Coding => Some(CodingExecutionStage::Coding),
            Self::Testing => Some(CodingExecutionStage::Testing),
            Self::CodeReview => Some(CodingExecutionStage::CodeReview),
            Self::ReviewRequest => Some(CodingExecutionStage::ReviewRequest),
            Self::InternalPrReview => Some(CodingExecutionStage::InternalPrReview),
            Self::FinalConfirm => Some(CodingExecutionStage::FinalConfirm),
            Self::HumanGate => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalystReworkInstructions {
    pub summary: String,
    #[serde(default)]
    pub required_changes: Vec<String>,
    #[serde(default)]
    pub verification_expectations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalystHumanGateRecommendation {
    #[serde(default)]
    pub reason_code: Option<String>,
    #[serde(default)]
    pub available_actions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalystDecisionRecord {
    pub id: String,
    pub attempt_id: String,
    pub source_stage: CodingExecutionStage,
    pub rework_round: u32,
    pub verdict: AnalystDecisionVerdict,
    pub next_stage: AnalystDecisionNextStage,
    pub reason: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub raw_provider_output_refs: Vec<String>,
    #[serde(default)]
    pub rework_instructions: Option<AnalystReworkInstructions>,
    #[serde(default)]
    pub human_gate: Option<AnalystHumanGateRecommendation>,
    pub created_at: String,
    #[serde(default)]
    pub parse_error: Option<String>,
}
```

- [ ] **Step 2: 运行模型测试**

Run:

```bash
cargo test --locked --test it_product analyst_decision_record_uses_stable_wire_values
```

Expected:

```text
test analyst_decision_record_uses_stable_wire_values ... ok
```

## Task 3: Store 测试先行

**Files:**
- Modify: `tests/it_product/product_coding_attempt_store.rs`
- Modify later: `src/product/coding_attempt_store.rs`

- [ ] **Step 1: 在 store 测试 import 加入新类型**

在 `tests/it_product/product_coding_attempt_store.rs` 顶部 `use cadence_aria::product::coding_models::{ ... }` 中加入：

```rust
AnalystDecisionNextStage, AnalystDecisionRecord, AnalystDecisionVerdict,
AnalystReworkInstructions,
```

- [ ] **Step 2: 写失败测试**

在 `saves_reads_and_consumes_latest_coding_rework_instruction` 测试之后加入：

```rust
#[test]
fn saves_reads_and_lists_latest_analyst_decision() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");
    let first = AnalystDecisionRecord {
        id: "analyst_decision_0001".to_string(),
        attempt_id: attempt.id.clone(),
        source_stage: CodingExecutionStage::Testing,
        rework_round: 1,
        verdict: AnalystDecisionVerdict::NeedsFix,
        next_stage: AnalystDecisionNextStage::Coding,
        reason: "测试失败，需要返修".to_string(),
        evidence_refs: vec!["testing_report_0001.json".to_string()],
        raw_provider_output_refs: Vec::new(),
        rework_instructions: Some(AnalystReworkInstructions {
            summary: "修复 failing test".to_string(),
            required_changes: vec!["补充边界输入处理".to_string()],
            verification_expectations: vec!["cargo test --locked --test it_product".to_string()],
        }),
        human_gate: None,
        created_at: "2026-06-12T00:00:00Z".to_string(),
        parse_error: None,
    };
    let second = AnalystDecisionRecord {
        id: "analyst_decision_0002".to_string(),
        attempt_id: attempt.id.clone(),
        source_stage: CodingExecutionStage::CodeReview,
        rework_round: 2,
        verdict: AnalystDecisionVerdict::Proceed,
        next_stage: AnalystDecisionNextStage::ReviewRequest,
        reason: "审查通过，可以创建 review request".to_string(),
        evidence_refs: vec!["code_review_0001.json".to_string()],
        raw_provider_output_refs: vec!["provider-raw/code_review/code_review_0001.txt".to_string()],
        rework_instructions: None,
        human_gate: None,
        created_at: "2026-06-12T00:01:00Z".to_string(),
        parse_error: None,
    };

    store
        .save_analyst_decision(&first)
        .expect("save first decision");
    store
        .save_analyst_decision(&second)
        .expect("save second decision");

    let decisions = store
        .list_analyst_decisions("project_0001", "issue_0001", &attempt.id)
        .expect("list decisions");
    assert_eq!(decisions, vec![first.clone(), second.clone()]);
    assert_eq!(
        store
            .latest_analyst_decision("project_0001", "issue_0001", &attempt.id)
            .expect("latest decision"),
        Some(second)
    );
}
```

- [ ] **Step 3: 运行测试，确认失败**

Run:

```bash
cargo test --locked --test it_product saves_reads_and_lists_latest_analyst_decision
```

Expected:

```text
no method named `save_analyst_decision` found for struct `CodingAttemptStore`
```

## Task 4: 实现 Store 持久化

**Files:**
- Modify: `src/product/coding_attempt_store.rs`
- Test: `tests/it_product/product_coding_attempt_store.rs`

- [ ] **Step 1: 更新 import**

在 `src/product/coding_attempt_store.rs` 顶部 `use crate::product::coding_models::{ ... }` 中加入：

```rust
AnalystDecisionRecord,
```

- [ ] **Step 2: 添加 public store 方法**

在 `save_rework_instruction` 附近加入：

```rust
    pub fn save_analyst_decision(
        &self,
        decision: &AnalystDecisionRecord,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&decision.id)?;
        let attempt = self.find_attempt_by_id(&decision.attempt_id)?;
        write_json(
            &self
                .analyst_decisions_root(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .join(format!("{}.json", decision.id)),
            decision,
        )
    }

    pub fn list_analyst_decisions(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<AnalystDecisionRecord>, ProductStoreError> {
        list_json_records(&self.analyst_decisions_root(project_id, issue_id, attempt_id))
    }

    pub fn latest_analyst_decision(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Option<AnalystDecisionRecord>, ProductStoreError> {
        Ok(self
            .list_analyst_decisions(project_id, issue_id, attempt_id)?
            .into_iter()
            .last())
    }
```

- [ ] **Step 3: 添加目录 helper**

在 `rework_instructions_root` 附近加入：

```rust
    fn analyst_decisions_root(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("analyst-decisions")
    }
```

- [ ] **Step 4: 运行 store 测试**

Run:

```bash
cargo test --locked --test it_product saves_reads_and_lists_latest_analyst_decision
```

Expected:

```text
test saves_reads_and_lists_latest_analyst_decision ... ok
```

## Task 5: Engine 新 schema 测试先行

**Files:**
- Modify: `tests/it_product/product_coding_workspace_engine.rs`
- Modify later: `src/product/coding_workspace_engine.rs`

- [ ] **Step 1: 在 engine 测试 import 加入新类型**

在 `tests/it_product/product_coding_workspace_engine.rs` 顶部 `use cadence_aria::product::coding_models::{ ... }` 中加入：

```rust
AnalystDecisionNextStage, AnalystDecisionVerdict,
```

- [ ] **Step 2: 写新 schema 持久化测试**

在 `execute_rework_creates_rework_instruction_and_consumes_context_notes` 后加入：

```rust
#[tokio::test]
async fn execute_rework_persists_structured_analyst_decision() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
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
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing stage");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{
            "verdict":"needs_fix",
            "next_stage":"coding",
            "reason":"required 测试步骤被跳过",
            "evidence_refs":["testing_report_0001.json"],
            "raw_provider_output_refs":["provider-raw/testing/execute_test_plan_0001.txt"],
            "rework_instructions":{
                "summary":"补齐 required 测试覆盖",
                "required_changes":["补充 B6 浏览器测试"],
                "verification_expectations":["B6 不再出现在 skipped_required_steps"]
            },
            "human_gate":null
        }"#.to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "testing blocked", &provider)
        .await
        .expect("execute rework");

    assert_eq!(updated.stage, CodingExecutionStage::Coding);
    let decision = store
        .latest_analyst_decision("project_0001", "issue_0001", &attempt.id)
        .expect("latest decision")
        .expect("persisted decision");
    assert_eq!(decision.id, "analyst_decision_0001");
    assert_eq!(decision.source_stage, CodingExecutionStage::Testing);
    assert_eq!(decision.rework_round, 1);
    assert_eq!(decision.verdict, AnalystDecisionVerdict::NeedsFix);
    assert_eq!(decision.next_stage, AnalystDecisionNextStage::Coding);
    assert_eq!(decision.reason, "required 测试步骤被跳过");
    assert_eq!(decision.evidence_refs, vec!["testing_report_0001.json".to_string()]);
    assert_eq!(
        decision.raw_provider_output_refs,
        vec!["provider-raw/testing/execute_test_plan_0001.txt".to_string()]
    );
    let rework = decision
        .rework_instructions
        .expect("rework instructions");
    assert_eq!(rework.summary, "补齐 required 测试覆盖");
    assert_eq!(rework.required_changes, vec!["补充 B6 浏览器测试".to_string()]);
    assert_eq!(
        rework.verification_expectations,
        vec!["B6 不再出现在 skipped_required_steps".to_string()]
    );
    assert_eq!(decision.parse_error, None);
}
```

- [ ] **Step 3: 写旧 schema 兼容持久化测试**

在同一测试文件中加入：

```rust
#[tokio::test]
async fn execute_rework_persists_legacy_analyst_verdict_as_decision() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
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
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{"verdict":"no_issue","summary":"审查通过"}"#.to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "code review approve", &provider)
        .await
        .expect("execute rework");

    assert_eq!(updated.stage, CodingExecutionStage::ReviewRequest);
    let decision = store
        .latest_analyst_decision("project_0001", "issue_0001", &attempt.id)
        .expect("latest decision")
        .expect("persisted decision");
    assert_eq!(decision.verdict, AnalystDecisionVerdict::Proceed);
    assert_eq!(decision.next_stage, AnalystDecisionNextStage::ReviewRequest);
    assert_eq!(decision.reason, "审查通过");
    assert_eq!(decision.rework_instructions, None);
    assert_eq!(decision.human_gate, None);
}
```

- [ ] **Step 4: 运行测试，确认失败**

Run:

```bash
cargo test --locked --test it_product execute_rework_persists_structured_analyst_decision
cargo test --locked --test it_product execute_rework_persists_legacy_analyst_verdict_as_decision
```

Expected:

```text
no method named `latest_analyst_decision` found for struct `CodingAttemptStore`
```

若 Task 4 已完成，Expected 改为：

```text
called `Option::unwrap()` on a `None` value
```

## Task 6: Engine 解析并保存 AnalystDecisionRecord

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`
- Test: `tests/it_product/product_coding_workspace_engine.rs`

- [ ] **Step 1: 更新 import**

在 `src/product/coding_workspace_engine.rs` 顶部 `use crate::product::coding_models::{ ... }` 中加入：

```rust
AnalystDecisionNextStage, AnalystDecisionRecord, AnalystDecisionVerdict,
AnalystHumanGateRecommendation, AnalystReworkInstructions,
```

- [ ] **Step 2: 扩展内部 AnalystDecision**

替换当前私有 `AnalystDecision` 和 `AnalystProviderPayload`：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
struct AnalystDecision {
    verdict: AnalystVerdict,
    structured_verdict: AnalystDecisionVerdict,
    next_stage: Option<AnalystDecisionNextStage>,
    summary: String,
    reason: String,
    evidence_refs: Vec<String>,
    raw_provider_output_refs: Vec<String>,
    rework_instructions: Option<AnalystReworkInstructions>,
    human_gate: Option<AnalystHumanGateRecommendation>,
    fix_hints: Vec<String>,
    questions: Vec<String>,
    parse_error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnalystProviderPayload {
    verdict: AnalystProviderVerdict,
    #[serde(default)]
    next_stage: Option<AnalystDecisionNextStage>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    evidence_refs: Vec<String>,
    #[serde(default)]
    raw_provider_output_refs: Vec<String>,
    #[serde(default)]
    rework_instructions: Option<AnalystReworkInstructions>,
    #[serde(default)]
    human_gate: Option<AnalystHumanGateRecommendation>,
    #[serde(default)]
    fix_hints: Vec<String>,
    #[serde(default)]
    questions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AnalystProviderVerdict {
    NeedsFix,
    NeedsHumanInput,
    NoIssue,
    RerunTesting,
    Proceed,
    HumanRequired,
    Blocked,
}

impl AnalystProviderVerdict {
    fn structured(&self) -> AnalystDecisionVerdict {
        match self {
            Self::NeedsFix => AnalystDecisionVerdict::NeedsFix,
            Self::NeedsHumanInput => AnalystDecisionVerdict::HumanRequired,
            Self::NoIssue => AnalystDecisionVerdict::Proceed,
            Self::RerunTesting => AnalystDecisionVerdict::RerunTesting,
            Self::Proceed => AnalystDecisionVerdict::Proceed,
            Self::HumanRequired => AnalystDecisionVerdict::HumanRequired,
            Self::Blocked => AnalystDecisionVerdict::Blocked,
        }
    }
}
```

- [ ] **Step 3: 添加归一化 helper**

在 `parse_analyst_verdict` 附近加入：

```rust
fn default_next_stage_for_legacy_verdict(
    verdict: &AnalystDecisionVerdict,
    source_stage: &CodingExecutionStage,
) -> AnalystDecisionNextStage {
    match verdict {
        AnalystDecisionVerdict::NeedsFix => AnalystDecisionNextStage::Coding,
        AnalystDecisionVerdict::RerunTesting => AnalystDecisionNextStage::Testing,
        AnalystDecisionVerdict::HumanRequired | AnalystDecisionVerdict::Blocked => {
            AnalystDecisionNextStage::HumanGate
        }
        AnalystDecisionVerdict::Proceed => match source_stage {
            CodingExecutionStage::Testing => AnalystDecisionNextStage::CodeReview,
            CodingExecutionStage::CodeReview => AnalystDecisionNextStage::ReviewRequest,
            CodingExecutionStage::InternalPrReview => AnalystDecisionNextStage::FinalConfirm,
            _ => AnalystDecisionNextStage::CodeReview,
        },
    }
}

fn decision_reason(summary: &str, reason: Option<&str>) -> String {
    reason
        .and_then(non_empty_trimmed)
        .unwrap_or_else(|| summary.to_string())
}
```

- [ ] **Step 4: 调整 parser 签名和实现**

把 `parse_analyst_verdict(&full_output)` 改成：

```rust
fn parse_analyst_verdict(
    full_output: &str,
    source_stage: &CodingExecutionStage,
) -> AnalystDecision {
    let Some(json_text) = extract_json_object(full_output) else {
        let summary = "Analyst 输出不是有效 JSON，已转人工确认。".to_string();
        return AnalystDecision {
            verdict: AnalystVerdict::NeedsHumanInput,
            structured_verdict: AnalystDecisionVerdict::HumanRequired,
            next_stage: Some(AnalystDecisionNextStage::HumanGate),
            summary: summary.clone(),
            reason: summary,
            evidence_refs: Vec::new(),
            raw_provider_output_refs: Vec::new(),
            rework_instructions: None,
            human_gate: None,
            fix_hints: Vec::new(),
            questions: vec!["请人工确认 Analyst 输出并补充下一步处理意见。".to_string()],
            parse_error: Some("missing_json_object".to_string()),
        };
    };

    match serde_json::from_str::<AnalystProviderPayload>(json_text) {
        Ok(payload) => {
            let structured_verdict = payload.verdict.structured();
            let summary = payload
                .summary
                .as_deref()
                .and_then(non_empty_trimmed)
                .or_else(|| {
                    payload
                        .rework_instructions
                        .as_ref()
                        .and_then(|instruction| non_empty_trimmed(&instruction.summary))
                })
                .unwrap_or_else(|| default_analyst_decision_summary(&structured_verdict));
            let next_stage = payload.next_stage.unwrap_or_else(|| {
                default_next_stage_for_legacy_verdict(&structured_verdict, source_stage)
            });
            let reason = decision_reason(&summary, payload.reason.as_deref());
            AnalystDecision {
                verdict: structured_verdict.legacy_chat_verdict(),
                structured_verdict,
                next_stage: Some(next_stage),
                summary,
                reason,
                evidence_refs: payload.evidence_refs,
                raw_provider_output_refs: payload.raw_provider_output_refs,
                rework_instructions: payload.rework_instructions,
                human_gate: payload.human_gate,
                fix_hints: payload.fix_hints,
                questions: payload.questions,
                parse_error: None,
            }
        }
        Err(error) => {
            let summary = "Analyst 输出不是有效 JSON，已转人工确认。".to_string();
            AnalystDecision {
                verdict: AnalystVerdict::NeedsHumanInput,
                structured_verdict: AnalystDecisionVerdict::HumanRequired,
                next_stage: Some(AnalystDecisionNextStage::HumanGate),
                summary: summary.clone(),
                reason: summary,
                evidence_refs: Vec::new(),
                raw_provider_output_refs: Vec::new(),
                rework_instructions: None,
                human_gate: None,
                fix_hints: Vec::new(),
                questions: vec!["请人工确认 Analyst 输出并补充下一步处理意见。".to_string()],
                parse_error: Some(error.to_string()),
            }
        }
    }
}

fn default_analyst_decision_summary(verdict: &AnalystDecisionVerdict) -> String {
    match verdict {
        AnalystDecisionVerdict::NeedsFix => "Analyst 判定需要自动修复".to_string(),
        AnalystDecisionVerdict::RerunTesting => "Analyst 判定需要重跑测试".to_string(),
        AnalystDecisionVerdict::Proceed => "Analyst 未发现阻塞问题".to_string(),
        AnalystDecisionVerdict::HumanRequired => "Analyst 判定需要人工补充信息".to_string(),
        AnalystDecisionVerdict::Blocked => "Analyst 判定当前流程被阻塞".to_string(),
    }
}
```

- [ ] **Step 5: 保存 decision record**

在 `execute_rework_with_commands` 中把：

```rust
let decision = parse_analyst_verdict(&full_output);
```

替换为：

```rust
let decision = parse_analyst_verdict(&full_output, &source_stage);
let existing_decisions = self.store.list_analyst_decisions(
    &attempt.project_id,
    &attempt.issue_id,
    &attempt.id,
)?;
let decision_record = AnalystDecisionRecord {
    id: next_sequential_id("analyst_decision", existing_decisions.len()),
    attempt_id: attempt.id.clone(),
    source_stage: source_stage.clone(),
    rework_round,
    verdict: decision.structured_verdict.clone(),
    next_stage: decision.next_stage.clone().unwrap_or_else(|| {
        default_next_stage_for_legacy_verdict(&decision.structured_verdict, &source_stage)
    }),
    reason: decision.reason.clone(),
    evidence_refs: decision.evidence_refs.clone(),
    raw_provider_output_refs: decision.raw_provider_output_refs.clone(),
    rework_instructions: decision.rework_instructions.clone(),
    human_gate: decision.human_gate.clone(),
    created_at: Utc::now().to_rfc3339(),
    parse_error: decision.parse_error.clone(),
};
self.store.save_analyst_decision(&decision_record)?;
```

- [ ] **Step 6: 让 rework instruction 优先使用结构化 rework_instructions**

在 `apply_analyst_decision` 创建 `CodingReworkInstruction` 时，将 `summary` 和 `fix_hints` 计算为：

```rust
let instruction_summary = decision
    .rework_instructions
    .as_ref()
    .map(|instruction| instruction.summary.clone())
    .unwrap_or_else(|| decision.summary.clone());
let instruction_fix_hints = decision
    .rework_instructions
    .as_ref()
    .map(|instruction| {
        instruction
            .required_changes
            .iter()
            .chain(instruction.verification_expectations.iter())
            .cloned()
            .collect::<Vec<_>>()
    })
    .filter(|items| !items.is_empty())
    .unwrap_or_else(|| decision.fix_hints.clone());
```

然后 `CodingReworkInstruction` 使用：

```rust
summary: instruction_summary,
fix_hints: instruction_fix_hints,
```

- [ ] **Step 7: 保留旧测试兼容**

运行现有 Analyst 相关测试：

```bash
cargo test --locked --test it_product execute_rework_creates_rework_instruction_and_consumes_context_notes
cargo test --locked --test it_product execute_rework_no_issue_routes_by_previous_stage
cargo test --locked --test it_product execute_rework_invalid_json_falls_back_to_human_input
```

Expected:

```text
test execute_rework_creates_rework_instruction_and_consumes_context_notes ... ok
test execute_rework_no_issue_routes_by_previous_stage ... ok
test execute_rework_invalid_json_falls_back_to_human_input ... ok
```

## Task 7: 定向验证与格式化

**Files:**
- Verify only.

- [ ] **Step 1: 运行新增定向测试**

Run:

```bash
cargo test --locked --test it_product analyst_decision
cargo test --locked --test it_product execute_rework_persists_structured_analyst_decision
cargo test --locked --test it_product execute_rework_persists_legacy_analyst_verdict_as_decision
```

Expected:

```text
test result: ok
```

- [ ] **Step 2: 运行相关既有回归**

Run:

```bash
cargo test --locked --test it_product execute_rework
cargo test --locked --test it_product saves_reads_and_consumes_latest_coding_rework_instruction
```

Expected:

```text
test result: ok
```

- [ ] **Step 3: 运行格式检查**

Run:

```bash
cargo fmt --check
```

Expected:

```text
command exits with status 0
```

## 自检清单

- [ ] `AnalystDecisionRecord` JSON 字段为 snake_case。
- [ ] 旧 provider schema 不需要改前端即可继续显示 AnalystVerdict chat entry。
- [ ] P1 没有改变 Testing、CodeReview、InternalPrReview 的现有路由。
- [ ] `analyst-decisions/` 与现有 attempt store 目录模式一致。
- [ ] 没有使用 `cargo test -j 1`。
- [ ] 当前 worktree 原有未提交 bugfix 没有被回滚。

## 交付边界

P1 完成后可以进入 P2。P2 才开始消费 `AnalystDecisionRecord.next_stage` 改造 Testing 后路由。
