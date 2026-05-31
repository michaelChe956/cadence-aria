# Coding Workspace P2：后端 Engine 与 WebSocket

## 文档信息

- 文档类型：计划文档
- 分支：`product-workbench-issue-lifecycle`
- 制定日期：2026-05-23
- 版本：v1.0
- 前置：P1（数据模型与 Store）
- 产出：CodingWorkspaceEngine、WebSocket handler、REST API、测试命令执行

---

## 一、目标

实现 Coding Workspace 的后端执行引擎：

1. `CodingWorkspaceEngine`：编排 worktree → coding → testing → code_review → rework → review_request → internal_pr_review → final_confirm
2. Coding WebSocket handler：独立 endpoint `/ws/coding-attempts/:attempt_id`
3. REST API：创建/启动/中止/确认 attempt
4. 测试命令执行器：在 worktree 中真实执行测试
5. Provider 集成：复用现有 provider contract 驱动 coding/review

---

## 二、任务清单

### 2.1 CodingWorkspaceEngine（新建）

**文件**：`src/product/coding_workspace_engine.rs`（新建）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 1.1 | 定义 `CodingWorkspaceEngine` 结构体 | — | 持有 store、git_service、tx（WS sender） |
| 1.2 | 实现 `start_attempt` | 单元测试 | 从 prepare_context 进入 worktree_prepare |
| 1.3 | 实现 `execute_worktree_prepare` | 集成测试 | 创建 worktree + branch，更新 attempt |
| 1.4 | 实现 `execute_coding` | 集成测试 | 调用 provider，产出代码变更 |
| 1.5 | 实现 `execute_testing` | 集成测试 | 真实执行测试命令，生成 TestingReport |
| 1.6 | 实现 `execute_code_review` | 集成测试 | 调用 reviewer provider，产出 findings |
| 1.7 | 实现 `execute_rework` | 单元测试 | 带失败证据调用 provider，回到 testing |
| 1.8 | 实现 `execute_review_request` | 集成测试 | git add + commit + push + 生成 ReviewRequest |
| 1.9 | 实现 `execute_internal_pr_review` | 集成测试 | 基于 pushed branch 做最终审查 |
| 1.10 | 实现 `handle_final_confirm` | 单元测试 | 用户确认后更新 status=completed |
| 1.11 | 实现 `handle_abort` | 单元测试 | 中止 attempt |
| 1.12 | 实现 rework 循环控制 | 单元测试 | rework_count < max_auto_rework 时自动返工 |
| 1.13 | 实现 blocked 处理 | 单元测试 | 超限/错误时进入 blocked + 动态 gate |

**Engine 核心流程**：

```
start_attempt()
  → execute_worktree_prepare()
  → execute_coding()
  → execute_testing()
    → if failed && rework_count < max: execute_rework() → loop back to testing
    → if failed && rework_count >= max: blocked
  → execute_code_review()
    → if request_changes && rework_count < max: execute_rework() → loop back to testing
    → if request_changes && rework_count >= max: blocked
  → execute_review_request()
    → if push failed: blocked
  → execute_internal_pr_review()
    → if request_changes && rework_count < max: execute_rework() → loop back to testing
    → if approve: final_confirm (wait for human)
  → handle_final_confirm()
    → completed
```

**Timeline 节点创建规则**：
- 每个阶段开始时创建一个 node（status=running）
- 阶段完成时更新 node（status=completed/failed）
- rework 每轮创建新 node（title="返工 #N"）

### 2.2 测试命令执行器

**文件**：`src/product/test_executor.rs`（新建）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 2.1 | 实现 `discover_test_commands(worktree_path) -> Vec<TestCommandSpec>` | 单元测试 | 按优先级发现测试命令 |
| 2.2 | 实现 `execute_test_command(spec, worktree_path) -> TestCommand` | 集成测试 | 真实执行，记录 stdout/stderr/exit_code |
| 2.3 | 实现 `run_all_tests(worktree_path, specs) -> TestingReport` | 集成测试 | 执行所有命令，汇总结果 |

**测试命令发现优先级**（P0 简化版）：
1. Work Item plan 中明确的测试命令（从 attempt context 传入）
2. 项目类型推断：
   - 存在 `Cargo.toml` → Rust 命令集
   - 存在 `pyproject.toml` 或 `setup.py` → Python 命令集
   - 存在 `package.json` → Node 命令集
3. 无法推断 → 返回空，engine 进入 blocked

**执行约束**：
- 使用 `tokio::process::Command`，argv 数组
- cwd 设为 worktree_path
- 单命令超时：5 分钟
- stdout/stderr 写入 artifacts 目录，返回文件引用

### 2.3 WebSocket Handler（新建）

**文件**：`src/web/coding_ws_handler.rs`（新建）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 3.1 | 定义 `CodingWsOutMessage` 枚举 | 序列化测试 | 所有 coding_* 消息类型 |
| 3.2 | 定义 `CodingWsInMessage` 枚举 | 反序列化测试 | 所有客户端消息类型 |
| 3.3 | 实现 WebSocket 握手和 session 恢复 | 集成测试 | 连接后发送 snapshot |
| 3.4 | 实现消息路由（入站） | 单元测试 | 按 stage 校验消息合法性 |
| 3.5 | 实现 snapshot 构建 | 单元测试 | 包含 attempt 全部状态 |
| 3.6 | 实现断连处理 | 单元测试 | 不中止 attempt，保持状态 |

**WS 消息定义**：

````rust
// Server -> Client
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CodingWsOutMessage {
    CodingSessionState {
        attempt_id: String,
        status: CodingAttemptStatus,
        stage: CodingExecutionStage,
        branch_name: String,
        base_branch: String,
        worktree_path: Option<PathBuf>,
        rework_count: u32,
        max_auto_rework: u32,
        head_commit: Option<String>,
        pushed_remote: Option<String>,
        provider_config_snapshot: ProviderConfigSnapshot,
        timeline_nodes: Vec<CodingTimelineNode>,
        active_node_id: Option<String>,
        testing_report: Option<TestingReport>,
        code_review_reports: Vec<CodeReviewReport>,
        review_request: Option<ReviewRequest>,
        internal_pr_review: Option<InternalPrReview>,
        pending_gates: Vec<CodingGateRequired>,
    },
    CodingStageChange {
        stage: CodingExecutionStage,
    },
    CodingTimelineNodeCreated {
        node: CodingTimelineNode,
    },
    CodingTimelineNodeUpdated {
        node_id: String,
        status: CodingTimelineNodeStatus,
        summary: Option<String>,
        completed_at: Option<String>,
    },
    CodingExecutionEvent {
        event: WsExecutionEvent,  // 复用现有类型
    },
    CodingStreamChunk {
        content: String,
        node_id: Option<String>,
    },
    CodingMessageComplete {
        node_id: Option<String>,
    },
    TestingReportUpdate {
        report: TestingReport,
    },
    CodeReviewComplete {
        report: CodeReviewReport,
    },
    ReviewRequestUpdate {
        review_request: ReviewRequest,
    },
    InternalPrReviewComplete {
        review: InternalPrReview,
    },
    CodingGateRequired {
        gate: CodingGateRequired,
    },
    CodingProtocolError {
        code: String,
        message: String,
    },
    CodingPong,
}

// Client -> Server
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CodingWsInMessage {
    CodingHello {
        attempt_id: String,
        last_seen_node_id: Option<String>,
    },
    StartCoding,
    ContextNote {
        content: String,
    },
    PermissionResponse {
        id: String,
        approved: bool,
        reason: Option<String>,
    },
    GateResponse {
        gate_id: String,
        action_id: String,
        extra_context: Option<String>,
    },
    FinalConfirm,
    AbortAttempt,
    RequestManualPause,
    CodingPing,
}
````

**阶段-消息有效性**：

| Stage | 允许的入站消息 |
|-------|--------------|
| `prepare_context` | ContextNote, StartCoding, AbortAttempt |
| `worktree_prepare` | AbortAttempt |
| `coding` | ContextNote, PermissionResponse, AbortAttempt |
| `testing` | AbortAttempt |
| `code_review` | AbortAttempt |
| `rework` | ContextNote, PermissionResponse, AbortAttempt |
| `review_request` | AbortAttempt |
| `internal_pr_review` | AbortAttempt |
| `final_confirm` | FinalConfirm, GateResponse, AbortAttempt |

blocked 状态（由 status 表达）：GateResponse, AbortAttempt

传输类消息 `CodingHello` 和 `CodingPing` 不受阶段限制；`GateResponse` 除了 `final_confirm` 外，也用于 `status=blocked` 的动态 gate。

### 2.4 REST API（扩展）

**文件**：`src/web/handlers.rs`（修改）+ `src/web/app.rs`（修改）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 4.1 | `POST /api/projects/:pid/issues/:iid/work-items/:wid/coding-attempts` | 集成测试 | 创建 attempt，校验前置条件 |
| 4.2 | `GET /api/coding-attempts/:attempt_id` | 集成测试 | 返回 attempt snapshot |
| 4.3 | `POST /api/coding-attempts/:attempt_id/abort` | 集成测试 | 中止 attempt |
| 4.4 | `GET /api/coding-attempts/:attempt_id/artifacts/:artifact_id` | 集成测试 | 返回证据文件 |
| 4.5 | 注册 WebSocket endpoint `/ws/coding-attempts/:attempt_id` | 集成测试 | WS 握手成功 |

**创建 attempt 前置校验**：
1. Work Item 存在
2. `plan_status = confirmed`
3. Repository 配置存在且 path 指向 git repo
4. 当前 Work Item 没有 active attempt
   - active attempt 定义来自 P1：`created`、`running`、`waiting_for_human`、`blocked` 都会阻止新建 attempt

**响应 DTO**：

````rust
#[derive(Serialize)]
pub struct CodingAttemptDto {
    pub attempt_id: String,
    pub work_item_id: String,
    pub attempt_no: u32,
    pub status: String,
    pub stage: String,
    pub branch_name: String,
    pub base_branch: String,
    pub worktree_path: Option<String>,
    pub rework_count: u32,
    pub head_commit: Option<String>,
    pub push_status: Option<String>,
    pub review_request_url: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
````

### 2.5 Provider 集成

**文件**：`src/product/coding_workspace_engine.rs`（内部）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 5.1 | 构建 coding provider 输入（plan + specs + repo context） | 单元测试 | 输入包含所有必要上下文 |
| 5.2 | 构建 rework provider 输入（失败证据 + diff） | 单元测试 | 输入包含失败命令和 findings |
| 5.3 | 构建 code_review provider 输入（diff + test report） | 单元测试 | 输入包含 diff 和测试结果 |
| 5.4 | 保存 `CodeReviewReport` 并推送 `code_review_complete` | 单元测试 | 刷新 snapshot 后仍可恢复基础 code review |
| 5.5 | 复用 `drive_provider_session` 模式 | 集成测试 | provider 流式输出通过 WS 推送 |

**Provider 输入模板**（coding 阶段）：

```
你是一个代码编写助手。请根据以下 Work Item 计划修改代码。

## Work Item 计划
{work_item_plan}

## 关联 Story Spec
{story_spec_content}

## 关联 Design Spec
{design_spec_content}

## 工作区信息
- 仓库路径：{worktree_path}
- 分支：{branch_name}
- 基础分支：{base_branch}

## 项目规则
{project_rules_summary}

## 约束
- 只修改与计划相关的文件
- 不要自动 merge 到其他分支
- 不要删除 worktree
- 修改完成后说明你做了什么
```

### 2.6 Lifecycle API 扩展

**文件**：`src/web/handlers.rs`（修改）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 6.1 | `IssueLifecycleResponse` 新增 `coding_attempts` 字段 | 单元测试 | 返回 work_item 关联的 attempts |
| 6.2 | `LifecycleWorkItemDto` 新增 `latest_attempt` 字段 | 单元测试 | 包含最新 attempt 摘要 |

### 2.7 模块注册与路由

**文件**：`src/product/mod.rs`、`src/web/mod.rs`、`src/web/app.rs`

| # | 任务 | 验收 |
|---|------|------|
| 7.1 | 注册 `coding_workspace_engine` 模块 | 编译通过 |
| 7.2 | 注册 `test_executor` 模块 | 编译通过 |
| 7.3 | 注册 `coding_ws_handler` 模块 | 编译通过 |
| 7.4 | 在 app.rs 注册新路由 | 编译通过 |

---

## 三、完成标准

- [ ] `cargo fmt --check` 通过
- [ ] `cargo check --locked` 通过
- [ ] `cargo test --locked -j 1` 全部通过
- [ ] `cargo clippy --all-targets --all-features --locked -- -D warnings` 无警告
- [ ] Engine happy path 集成测试通过（使用临时 git repo + fake provider）
- [ ] WebSocket 握手和 snapshot 恢复测试通过
- [ ] snapshot 包含 CodeReviewReport、ReviewRequest、InternalPrReview
- [ ] REST API 创建 attempt 前置校验测试通过
- [ ] 测试命令执行器在临时目录中真实执行并记录结果

---

## 四、不在本阶段范围

- 前端组件（P3）
- GitLab push option MR 创建（P1 后续增强）
- 外部平台 API token 集成
- Worktree 清理策略
