# Coding Workspace P1：后端数据模型与 Store

## 文档信息

- 文档类型：计划文档
- 分支：`product-workbench-issue-lifecycle`
- 制定日期：2026-05-23
- 版本：v1.0
- 前置：无
- 产出：数据模型、Store CRUD、GitWorkspaceService

---

## 一、目标

建立 Coding Workspace 的数据基础：

1. 定义 `CodingExecutionAttempt`、`CodingWorkspaceStage`（新版）、`TestingReport`、`CodeReviewReport`、`ReviewRequest`、`InternalPrReview`、`CodingTimelineNode` 模型
2. 实现 `CodingAttemptStore` 持久化读写
3. 实现 `GitWorkspaceService` 封装 git worktree/branch/commit/push
4. 扩展 `LifecycleStore` 支持 `update_work_item_execution_status`

---

## 二、任务清单

### 2.1 数据模型定义（src/product/models.rs 扩展 + 新文件）

**文件**：`src/product/coding_models.rs`（新建）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 1.1 | 定义 `CodingExecutionStage` 枚举 | 序列化测试 | 9 个阶段值正确序列化为 snake_case |
| 1.2 | 定义 `CodingAttemptStatus` 枚举 | 序列化测试 | 7 个状态值正确序列化 |
| 1.3 | 定义 `CodingExecutionAttempt` 结构体 | 序列化/反序列化测试 | 所有字段正确映射 |
| 1.4 | 定义 `TestingReport` 和 `TestCommand` | 序列化测试 | 命令使用 argv 数组 |
| 1.5 | 定义 `ReviewFinding`、`ReviewVerdict`、`FindingSeverity` | 序列化测试 | verdict、findings、severity |
| 1.6 | 定义 `CodeReviewReport` | 序列化测试 | 基础 code review 可独立恢复 |
| 1.7 | 定义 `ReviewRequest` 和相关枚举 | 序列化测试 | `ReviewRequestKind`、`RemoteKind`、`PushStatus` |
| 1.8 | 定义 `InternalPrReview` | 序列化测试 | internal PR review 关联 ReviewRequest |
| 1.9 | 定义 `CodingTimelineNode` | 序列化测试 | 与技术方案 5.3 节一致 |
| 1.10 | 定义 `CodingGateAction` 和 `CodingGateRequired` | 序列化测试 | 动态按钮列表 |

**模型详细定义**：

````rust
// CodingExecutionStage - 只表示执行阶段，不含终态
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingExecutionStage {
    PrepareContext,
    WorktreePrepare,
    Coding,
    Testing,
    CodeReview,
    Rework,
    ReviewRequest,
    InternalPrReview,
    FinalConfirm,
}

// CodingAttemptStatus - 包含终态
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingAttemptStatus {
    Created,
    Running,
    WaitingForHuman,
    Blocked,
    Completed,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingExecutionAttempt {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub work_item_id: String,
    pub attempt_no: u32,
    pub status: CodingAttemptStatus,
    pub stage: CodingExecutionStage,
    pub base_branch: String,
    pub branch_name: String,
    pub worktree_path: Option<PathBuf>,
    pub provider_config_snapshot: ProviderConfigSnapshot,
    pub rework_count: u32,
    pub max_auto_rework: u32,
    pub head_commit: Option<String>,
    pub pushed_remote: Option<String>,
    pub review_request_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestCommandStatus {
    Passed,
    Failed,
    TimedOut,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestCommand {
    pub command: Vec<String>,  // argv 数组，不用 shell 拼接
    pub cwd: PathBuf,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub stdout_ref: String,
    pub stderr_ref: String,
    pub status: TestCommandStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestingOverallStatus {
    Passed,
    Failed,
    SkippedByUserDecision,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestingReport {
    pub id: String,
    pub attempt_id: String,
    pub commands: Vec<TestCommand>,
    pub overall_status: TestingOverallStatus,
    pub provider_claim: Option<serde_json::Value>,
    pub backend_verified: bool,
    pub started_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewRequestKind {
    GitBranchOnly,
    GitlabMergeRequest,
    GithubPullRequest,
    ManualExternalRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteKind {
    Github,
    Gitlab,
    GenericGit,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushStatus {
    NotPushed,
    Pushed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewRequest {
    pub id: String,
    pub attempt_id: String,
    pub kind: ReviewRequestKind,
    pub remote_kind: RemoteKind,
    pub remote: String,
    pub base_branch: String,
    pub branch_name: String,
    pub commit_sha: String,
    pub push_status: PushStatus,
    pub external_url: Option<String>,
    pub manual_instructions: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdict {
    Approve,
    RequestChanges,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewFinding {
    pub severity: FindingSeverity,
    pub file_path: Option<String>,
    pub line: Option<u32>,
    pub message: String,
    pub required_action: Option<String>,
    pub source_stage: CodingExecutionStage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeReviewReport {
    pub id: String,
    pub attempt_id: String,
    pub round: u32,
    pub verdict: ReviewVerdict,
    pub findings: Vec<ReviewFinding>,
    pub tested_evidence_refs: Vec<String>,
    pub diff_refs: Vec<String>,
    pub summary: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InternalPrReview {
    pub id: String,
    pub attempt_id: String,
    pub review_request_id: String,
    pub verdict: ReviewVerdict,
    pub findings: Vec<ReviewFinding>,
    pub tested_evidence_refs: Vec<String>,
    pub diff_refs: Vec<String>,
    pub summary: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingTimelineNodeStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingAgentRole {
    Author,
    Tester,
    Reviewer,
    Git,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingTimelineNode {
    pub id: String,
    pub attempt_id: String,
    pub stage: CodingExecutionStage,
    pub title: String,
    pub status: CodingTimelineNodeStatus,
    pub agent_role: Option<CodingAgentRole>,
    pub summary: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub artifact_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingGateAction {
    pub action_id: String,
    pub label: String,
    pub action_type: CodingGateActionType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingGateActionType {
    ContinueRework,
    AcceptRisk,
    Abort,
    RetryPush,
    ManualFix,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingGateKind {
    Permission,
    Blocked,
    FinalConfirm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingGateRequired {
    pub gate_id: String,
    pub kind: CodingGateKind,
    pub title: String,
    pub description: String,
    pub available_actions: Vec<CodingGateAction>,
}
````

### 2.2 CodingAttemptStore（新建）

**文件**：`src/product/coding_attempt_store.rs`（新建）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 2.1 | 实现 `create_attempt` | 单元测试 | 创建后可读取，attempt_no 递增 |
| 2.2 | 实现 `get_attempt` / `list_attempts_for_work_item` | 单元测试 | 按 ID 和 work_item_id 查询 |
| 2.3 | 实现 `update_attempt_status` | 单元测试 | 状态转换合法性校验 |
| 2.4 | 实现 `update_attempt_stage` | 单元测试 | stage 只能前进或进入 rework |
| 2.5 | 实现 `get_active_attempt` | 单元测试 | 同一 work_item 只有一个 active |
| 2.6 | 实现 `save_testing_report` / `get_testing_report` | 单元测试 | 持久化和读取 |
| 2.7 | 实现 `save_code_review_report` / `list_code_review_reports` | 单元测试 | 基础 code review 可按 attempt 恢复 |
| 2.8 | 实现 `save_review_request` / `get_review_request` | 单元测试 | 持久化和读取 |
| 2.9 | 实现 `save_internal_pr_review` / `get_internal_pr_review` | 单元测试 | 持久化和读取 |
| 2.10 | 实现 `save_timeline_node` / `get_timeline_nodes` | 单元测试 | 按 attempt_id 查询，有序 |
| 2.11 | 实现 `update_timeline_node_status` | 单元测试 | 状态更新 |

**存储路径设计**（沿用 lifecycle_store 的 JSON 文件模式）：

```
<issue_lifecycle_root>/coding-attempts/<attempt_id>.json
<issue_lifecycle_root>/coding-attempts/<attempt_id>/testing-reports/<report_id>.json
<issue_lifecycle_root>/coding-attempts/<attempt_id>/code-reviews/<report_id>.json
<issue_lifecycle_root>/coding-attempts/<attempt_id>/review-requests/<request_id>.json
<issue_lifecycle_root>/coding-attempts/<attempt_id>/internal-reviews/<review_id>.json
<issue_lifecycle_root>/coding-attempts/<attempt_id>/timeline-nodes.json
<issue_lifecycle_root>/coding-attempts/<attempt_id>/artifacts/<artifact_id>
```

**状态转换规则**：

```
created → running（start_coding）
running → waiting_for_human（permission gate / final_confirm）
running → blocked（错误/超限）
running → completed（final_confirm 通过）
running → failed（不可恢复错误）
waiting_for_human → running（用户响应后继续）
blocked → running（用户选择继续）
blocked → aborted（用户放弃）
* → aborted（用户中止）
```

Active attempt 定义：`created`、`running`、`waiting_for_human`、`blocked` 都属于 active；只有 `completed`、`failed`、`aborted` 允许同一 Work Item 创建新的 attempt。

### 2.3 GitWorkspaceService（新建）

**文件**：`src/product/git_workspace_service.rs`（新建）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 3.1 | 实现 `create_worktree(repo_path, branch_name, worktree_path)` | 集成测试（临时 git repo） | 创建成功，路径存在 |
| 3.2 | 实现 `create_branch(repo_path, branch_name, base_branch)` | 集成测试 | 分支创建成功 |
| 3.3 | 实现 `git_status(worktree_path) -> Vec<FileStatus>` | 集成测试 | 正确解析 porcelain 输出 |
| 3.4 | 实现 `git_add_all(worktree_path)` | 集成测试 | 暂存所有变更 |
| 3.5 | 实现 `git_commit(worktree_path, message) -> CommitResult` | 集成测试 | 返回 commit sha |
| 3.6 | 实现 `git_push(worktree_path, remote, branch) -> PushResult` | 集成测试 | 返回成功/失败 |
| 3.7 | 实现 `detect_remote_kind(repo_path) -> RemoteKind` | 单元测试 | 识别 github/gitlab/generic |
| 3.8 | 实现 `git_diff_stat(worktree_path, base_branch) -> DiffStat` | 集成测试 | 文件列表和增删行数 |

**安全约束**：
- 所有 git 命令使用 `tokio::process::Command` + argv 数组
- 不使用 shell 拼接
- 路径必须在 repo_root 或 `.worktrees/aria-work-items/` 下
- 超时：单个 git 命令 30 秒

**分支命名**：`aria/work-items/<work_item_id>/attempt-<attempt_no>`

**Worktree 路径**：`<repo_root>/.worktrees/aria-work-items/<work_item_id>/attempt-<attempt_no>`

### 2.4 LifecycleStore 扩展

**文件**：`src/product/lifecycle_store.rs`（修改）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 4.1 | 新增 `update_work_item_execution_status(project_id, issue_id, work_item_id, status)` | 单元测试 | 状态更新持久化 |
| 4.2 | 新增 `update_work_item_worktree_path(project_id, issue_id, work_item_id, path)` | 单元测试 | 路径更新持久化 |

### 2.5 模块注册

**文件**：`src/product/mod.rs`（修改）

| # | 任务 | 验收 |
|---|------|------|
| 5.1 | 添加 `pub mod coding_models;` | 编译通过 |
| 5.2 | 添加 `pub mod coding_attempt_store;` | 编译通过 |
| 5.3 | 添加 `pub mod git_workspace_service;` | 编译通过 |

---

## 三、完成标准

- [ ] `cargo fmt --check` 通过
- [ ] `cargo check --locked` 通过
- [ ] `cargo test --locked -j 1` 全部通过
- [ ] `cargo clippy --all-targets --all-features --locked -- -D warnings` 无警告
- [ ] 所有新模型的序列化/反序列化测试覆盖
- [ ] CodingAttemptStore 的 CRUD 测试覆盖
- [ ] CodeReviewReport 与 InternalPrReview 都可从 snapshot 所需查询恢复
- [ ] GitWorkspaceService 的集成测试使用临时 git repo
- [ ] LifecycleStore 新方法有单元测试

---

## 四、不在本阶段范围

- CodingWorkspaceEngine 编排逻辑（P2）
- WebSocket handler（P2）
- REST API（P2）
- 前端组件（P3）
- Provider 集成（P2）
- 测试命令发现逻辑（P2）
