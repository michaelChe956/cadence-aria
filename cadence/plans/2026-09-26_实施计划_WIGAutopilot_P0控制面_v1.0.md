# WIG Autopilot P0 控制面 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 交付 P0 1.1–1.4：持久授权与服务端归属、无 driver 的 workspace/coding choice 与人工门控制面；不启动任何自动 prepare、advance 或 coding。

**Architecture:** issue 级独立 enrollment 以文件锁 + 原子 JSON 写实现 CAS；HTTP summary、活跃引擎和 observer 快照都按同一精确绑定从 durable 事实计算 owner。choice 经 manager/registry 的 run-incarnation claim、provider 等待者回执及共用 WS/REST 门面处理；驾驶舱使用观察态完整选项，发有身份校验的 REST 命令，不提升 observer WS 权限。人工门/recovery 提取既有引擎动作供 REST 调用而非创建第二个 provider drive。

**Tech Stack:** Rust、Axum、Tokio、serde、文件持久层；React、TypeScript、Zustand、Vitest、Testing Library。

**Spec:** `openspec/changes/work-item-group-autopilot/{proposal.md,design.md,tasks.md,specs/}`；评审补强 `.superpowers/sdd/2026-09-26_评审报告_maxtask_迭代计划_v1.0.md`。本计划只覆盖 tasks.md §1 P0；本文接口为**拟新增契约**，不是声称当前代码已有。

## Global Constraints

- REQ-WIGA-01/02：enrollment 缺失或旧 issue/旧 plan 恒 off，禁止历史扫描接管；精确 story/design id+version、provider/options、plan/session/target 绑定；CAS 修订线性化，关闭后未认领许可无效，已运行 run 不 Abort。
- REQ-WIGA-01、REQ-MTG-03：自动授权仅恰一 logical repository/单 attempt；零/多 target fail-closed；既有多 target **逐 target 人工**启动不变。
- REQ-WIGA-02、REQ-CG-04：WorkItemPlan session `RunPolicy::Interactive`，不得以 `AutoIfValid` 代表授权；approve/门关闭不等于 compile/publication 成功的 durable Confirmed；人工门与 coding Final Confirm 均由人决定。
- REQ-ADV-05、REQ-MTG-03：advance 只到 Ready，P0 不创建编排器、不自动 prepare/start_generation/advance/StartCoding，也不修改人工 StartCoding 准入；未 Ready 的原 `SC_CODING_REQUIRES_ADVANCE` 不变。
- REQ-WIGA-05/08：observer WS 只读；REST 入口核验 session/attempt/当前 run 与具体 choice/gate；WS 和 REST 共用 claim，回执与 durable/内存状态不能假装跨进程存活，原手动非 enrolled 行为不变。
- 验证从 worktree 根运行：Rust 定向 `cargo test --locked --lib <filter> -- --nocapture`，前端 `pnpm -C web exec vitest run <file>`；逐任务红→绿，禁止 `cargo test -j 1`；最后由集成负责人统一跑全量。仅计划文件在本次派工落盘，**本文未实施任何代码**。

## Review Focus

1. **两次并发启用/关闭与异 source/options**：revision CAS 只有一次胜出；旧请求不能撤销新修订；同键同内容幂等（任务 2/3 的 store 与 HTTP 用例）。
2. **旧 run 的同名 choice / 新进程丢失 oneshot / HTTP 回执超时**：不能把新 run 当旧 run，accepted 只能 202，旧请求 410，可按原 command_id 查询真实结果（任务 6–9 的 run/bridge/路由用例）。
3. **两个问题分别回答或漏答其一、coding 旧 gate 不带 questions**：完整 `answers` 原样到 provider，漏答保持表单；旧 gate 按单默认问题兼容（任务 9/11 的 provider 与 UI 用例）。
4. **provider 长持 engine 锁 + HTTP/WS 同时提交**：先在短临界区认领，不等待 engine mutex；败者 409、唯一 command 投递（任务 7/8 的竞争用例）。
5. **无 driver 人工 gate/recovery + observer 误写 + owner 首帧未知**：REST 可解除人工阻塞而 WS observer 仍拒，D6 只在明确 client owner 后保留原行为（任务 5/10/12 的门与页面用例）。

---

## 文件责任与统一接口

现有结构：`src/product/app_paths.rs:33-43` 定位 issue 根；`src/product/coding_attempt_store/locking.rs:16-22` 提供跨进程独占锁；`src/product/json_store.rs:27-66` 提供原子 JSON 写；`src/product/models/lifecycle.rs:102-143` 保存 source revision；`src/web/handlers/lifecycle.rs:612-785` 当前手动 prepare 每次新建 plan/session，**P0 不调用它作为自动准备**。`src/web/workspace_session/manager/{mod,choices,runs}.rs` 是唯一 workspace run 所有者；`src/product/coding_workspace_runner.rs:11-31` 是 coding 命令族；`web/src/components/chat-workspace/entries/ChoiceRequestEntry.tsx:76-107` 已有逐题答案构造，可重用。

全任务沿用如下**唯一**新契约，不另造同义类型；带「新增」的符号需由相应任务落地：

```rust
// 新增于 product/models/automation.rs；revision 为 u64，0 表示缺失。
pub struct SourceRevisionRef { pub id: String, pub version: u32 }
pub struct EnrollmentSource {
    pub stories: Vec<SourceRevisionRef>, pub designs: Vec<SourceRevisionRef>,
}
pub struct EnrollmentOptions {
    pub author_provider: ProviderName, pub reviewer_provider: ProviderName,
    pub review_rounds: u32, pub superpowers_enabled: bool, pub openspec_enabled: bool,
    pub plan_options: IssueWorkItemPlanOptions,
}
pub struct IssueAutomationEnrollment {
    pub enrollment_id: String, pub selection_key: String,
    pub project_id: String, pub issue_id: String, pub enabled: bool,
    pub policy_revision: u64, pub source: EnrollmentSource, pub options: EnrollmentOptions,
    pub logical_repository_id: LogicalRepositoryId,
    pub prepare_intent_id: String, pub plan_id: Option<String>, pub session_id: Option<String>,
    pub created_at: String, pub updated_at: String,
}
pub enum AutomationOwner { Client, Server }
pub struct AutomationOwnership {
    pub owner: AutomationOwner, pub enrollment_id: Option<String>,
    pub policy_revision: Option<u64>, pub enabled: bool,
}

// ChoiceReplyState 新增于 cross_cutting/choice_delivery.rs；其余 DTO 于 web/choice_reply.rs。
pub struct ChoiceResponseRequest {
    pub command_id: String, pub expected_run_id: String,
    pub answers: Vec<ChoiceAnswerData>,
}
pub enum ChoiceReplyState { Submitting, Resolving, Delivered, Rejected, Expired }
pub struct ChoiceReplyStatus {
    pub command_id: String, pub expected_run_id: String, pub choice_id: String,
    pub state: ChoiceReplyState,
}
// 两层回执：mpsc 发送仅 Resolving；provider 等待者解出 ChoiceDecision
// 或 pi 对应的命令等待者真正接收才 Delivered。
```

```rust
// EnrollmentWriteCommand 与 EnrollmentError（Task 2 产出、Task 3 消费，统一在此定义）：
pub enum EnrollmentWriteCommand {
    Enable { selection_key: String, source: EnrollmentSource,
             options: EnrollmentOptions, logical_repository_id: LogicalRepositoryId },
    Disable,
}
pub enum EnrollmentError {
    Conflict { current_revision: Option<u64> },   // HTTP 409，details 带 current_revision
    InvalidScope(String),                          // HTTP 422
    NotFound,                                      // HTTP 404（仅 binding/状态类）
    Store(ProductStoreError),                      // fail-closed 其余映射
}
```
REST GET 语义统一：`GET .../automation-enrollment` 是读投影——不存在时返回 `200 + null`（前端需要区分「未启用」与「启用」而非报错）；`404` 仅用于 choice response 状态查询的未知 command_id（Task 8/9）与 binding 目标对象不存在（Task 3）。新建测试函数名一律以本任务红灯过滤词开头（如 `issue_automation_store_...`、`choice_delivery_...`、`automation_enrollment_...`），保证 `cargo test -- --nocapture` 过滤词可靠命中。

REST 路由统一：`PUT/GET /api/projects/{project_id}/issues/{issue_id}/automation-enrollment`，`POST /api/projects/{project_id}/issues/{issue_id}/automation-enrollment/binding`；`POST /api/workspace-sessions/{session_id}/choices/{choice_id}/response`，`GET /api/workspace-sessions/{session_id}/choices/{choice_id}/responses/{command_id}`；`POST /api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/choices/{choice_id}/response`，`GET /api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/choices/{choice_id}/responses/{command_id}`；`POST /api/workspace-sessions/{session_id}/human-actions`。HTTP `202` 体同 `ChoiceReplyStatus`，`200` 仅 Delivered；状态查询 `200` 返回状态体而不意味着 Delivered。沿用 `src/web/error.rs:48-198` 的 `ApiError` 码表而不是让新码落默认 500。所有写 API 在当前本机 WebAppState、现有 issue/session/attempt 精确作用域校验之内；若未来引入通用用户身份鉴权，在共用路由层继承，不从 observer WS 升权，也不认为 body 中的 `issue_id` 是授权凭据。

## Task 1：1.1 advance 注释与例外矩阵

**Files:** Modify `src/product/workspace_engine/advance_split.rs:24-38`；Test `src/product/workspace_engine/tests/advance_split_targets.rs:653-686`（既有多 target Ready/无自动启动断言）。Spec 只引用，不编辑。

**Interfaces:** Consumes: 既有 `WorkspaceEngine::initialize_advance_split(...) -> Result<AdvanceOutcome,String>`、`RunPolicy::Interactive`。Produces: 准确注释，**不改 Rust 行为或签名**；映射 REQ-ADV-05/CG-04/MTG-03 五红线。

- [ ] **Step 1: 写失败的契约检查。** 先在 `split_advance_leaves_attempts_unorchestrated_at_created_prepare_context` 增加/确认 `AdvanceOutcome::Completed { record, .. }` 的 `record.status == AdvanceStatus::Ready`，以及每 target 仍为 `CodingAttemptStatus::Created + CodingExecutionStage::PrepareContext`；不把注释措辞写成永久测试。逐条对照 REQ-CG-04：门关闭并非 Confirmed；REQ-MTG-03：多 target 不逐仓自动发 StartCoding。
- [ ] **Step 2: 红灯。** `cargo test --locked --lib split_advance_leaves_attempts_unorchestrated_at_created_prepare_context -- --nocapture`；若既有行为已满足，应记录断言为绿，注释属于纯文档修改无需制造假失败；任何真实失败先定位，禁止修改实现掩盖。
- [ ] **Step 3: 只改注释。** 将第 37 行改为：
  ```rust
  /// 本函数只建组至 Ready，不发 coding provider；多 target 仍须人工逐 target
  /// 发显式 StartCoding。仅精确绑定单 target 的 durable opt-in 可由独立服务端
  /// StartCoding 命令单发首启；不是 advance 的副作用（REQ-ADV-05/MTG-03/CG-04）。
  ```
- [ ] **Step 4: 绿灯。** 同一过滤命令；核对三份 MODIFIED delta 的门/Ready/单 target 措辞，`git diff --check -- src/product/workspace_engine/advance_split.rs`。
- [ ] **Step 5: 提交。** `git add src/product/workspace_engine/advance_split.rs src/product/workspace_engine/tests/advance_split_targets.rs && git commit -m "fix: clarify advance ready-only start contract"`（未改测试则只暂存注释）。

## Task 2：1.2 独立 enrollment 持久记录与 CAS

**Files:** Create `src/product/models/automation.rs`、`src/product/issue_automation_store.rs`；Modify `src/product/models/mod.rs:1-32`、`src/product/mod.rs:1-45`、`src/product/app_paths.rs:33-43`；Test `src/product/issue_automation_store.rs` 内部 `#[cfg(test)]` 模块（放于新文件末尾）。

**Interfaces:** Consumes: `ProductAppPaths::issue_root`（`app_paths.rs:33-43`）、`coding_attempt_store::locking::with_exclusive_lock(&Path, FnOnce()->Result<T,ProductStoreError>)`（`locking.rs:16-22`）、`json_store::{read_json,write_json,validate_relative_id}`、`SourceRevisionRef`、`LogicalRepositoryId`、`IssueWorkItemPlanOptions`。Produces: `IssueAutomationStore::new(ProductAppPaths)`、`get(&self,project_id:&str,issue_id:&str)->Result<Option<IssueAutomationEnrollment>,ProductStoreError>`、`compare_and_set(&self,project_id:&str,issue_id:&str,expected_revision:Option<u64>,command:EnrollmentWriteCommand)->Result<IssueAutomationEnrollment,EnrollmentError>`、`bind_plan(&self,project_id:&str,issue_id:&str,expected_revision:u64,plan_id:&str,session_id:&str)->Result<IssueAutomationEnrollment,EnrollmentError>`。`EnrollmentError::{Conflict,Missing,Store(ProductStoreError)}`。

- [ ] **Step 1: 测试先行。** `tempfile::tempdir()` + `IssueAutomationStore::new(ProductAppPaths::new(tmp.path()))`；断言 `get(...).unwrap()==None`；两个 `std::thread::spawn` 同一 `expected_revision=None`、相同 Enable，最终同 `enrollment_id`、revision=1、仅一条 JSON；一个线程异 options 则 Conflict；禁用 revision=2 后以旧 revision 启用则 Conflict；重开 revision=3 保留原绑定。另测 `bind_plan` 重绑不同 plan 失败且绑定仍唯一，revision 不因同键重试变化。
  ```rust
  assert_eq!(store.get("project_1", "issue_1").unwrap(), None);
  let first = store.compare_and_set("project_1", "issue_1", None, enable.clone()).unwrap();
  assert_eq!(first.policy_revision, 1);
  let same = store.compare_and_set("project_1", "issue_1", None, enable).unwrap();
  assert_eq!(same.enrollment_id, first.enrollment_id);
  assert_eq!(same.policy_revision, 1);
  ```
- [ ] **Step 2: 红灯。** `cargo test --locked --lib issue_automation_store -- --nocapture`；预期未定义 store/模型。
- [ ] **Step 3: 最小实现。** enrollment 文件放 `issue_root(...).join("automation-enrollment.json")`，以该路径上的 `with_exclusive_lock` 包住**读、比 revision、写**；缺失判定用 `path.metadata()` 预检（`read_json` 的 Io 错误已是格式化字符串、不含可判 ErrorKind，**不得**对其做 NotFound 匹配；预检与读之间存在窗口没关系——锁内无并发写），预检不存在=`None`，存在再 `read_json`，损坏/权限错 fail-closed；同 selection_key 同 source/options/target 重试返回原值，异内容 Conflict；Disable 再 Enable revision+1 且不抹绑定/prepare_intent；首次 `Uuid::new_v4()` 生成 enrollment_id，`prepare_intent_id` 同此 ID，P0 不创建 plan/session。绑定操作相同锁内要求唯一 plan/session 或幂等原值，不允许按最新 plan 推断。示意核心：
  ```rust
  with_exclusive_lock(&path, || {
      let existing = read_optional_enrollment(&path)?;
      match (&existing, &command) {
          (Some(saved), EnrollmentWriteCommand::Enable { selection_key, source, options, logical_repository_id })
              if saved.enabled && saved.selection_key == *selection_key
                  && saved.source == *source && saved.options == *options
                  && saved.logical_repository_id == *logical_repository_id
                  && (expected_revision.is_none() || expected_revision == Some(saved.policy_revision)) => Ok(saved.clone()),
          (Some(saved), _) if expected_revision != Some(saved.policy_revision) =>
              Err(ProductStoreError::Conflict { kind: "automation_enrollment", id: issue_id.into() }),
          _ => apply_revision_and_write(&path, existing, command),
      }
  })
  ```
  同键从初次 `None` 重发时明确允许读取 `saved.selection_key/source/options/target` 后返回原版本；异 payload 必须先 Conflict；此分支不要因 `expected_revision=None` 被 revision 守卫误拒。store 不启动 provider，也不更改 `IssueRecord`。
- [ ] **Step 4: 绿灯。** 同一过滤命令；测试并发 20 次以不同 tmp 目录运行，不共享全局仓数据。
- [ ] **Step 5: 提交。** `git add src/product/models/automation.rs src/product/issue_automation_store.rs src/product/models/mod.rs src/product/mod.rs src/product/app_paths.rs && git commit -m "feat: persist issue automation enrollment with cas"`。

## Task 3：1.2 enrollment REST 创建、绑定及授权校验

**Files:** Create `src/web/handlers/automation_enrollment.rs`；Modify `src/web/handlers/mod.rs:72-93,125-158`、`src/web/app.rs:227-275`、`src/web/types.rs:859-880`、`src/web/error.rs:48-110`；Test `src/web/handlers/lifecycle_tests.inc.rs:304-457,518-550`（复用 `seed_prepare_work_item_plan_fixture`，或新模块的 HTTP 测试）。

**Interfaces:** Consumes: Task 2 `IssueAutomationStore::{get,compare_and_set,bind_plan}`、`EnrollmentWriteCommand`；既有 `validate_confirmed_story_specs`/`validate_confirmed_design_specs` (`lifecycle.rs:1028-1097`)、`RepositoryRouting::load_for_issue(&ProductAppPaths,&str,&str,...)`、`preflight_single_repository_candidate(&[String])` (`lifecycle/preflight.rs:28-60`)、`LifecycleStore::get_workspace_session`。Produces: `PUT/GET .../automation-enrollment` 和 `POST .../binding`；请求 `EnrollmentPutRequest{expected_revision:Option<u64>,command:EnrollmentWriteCommand}`、`EnrollmentBindRequest{expected_revision:u64,plan_id:String,session_id:String}`（serde snake_case）；分别返回 `Json<IssueAutomationEnrollment>`、`Json<Option<IssueAutomationEnrollment>>`，失败稳定 404/409/422。

- [ ] **Step 1: HTTP 失败测试。** 使用 `build_test_router`（现有 `lifecycle_tests.inc.rs:259-264`）与 fixture：未确认 design 拒绝；单 logical member 确认后 PUT 同键两次 `200` 且同 ID/revision；异 options/旧 revision `409`；0/2 logical member `422` 且 enrollment 文件不存在；绑定测试新建明确 plan/session 后校验 source ids、源 current_version、`session.workspace_type==WorkspaceType::WorkItemPlan`、`session.run_policy==RunPolicy::Interactive`、plan/session 同 issue 且 `entity_id==plan_id`，重绑他者 `409`。实际 story/design id 与 current_version 从 fixture 返回记录填入 JSON；版本不存在拒绝，不猜 latest；旧 plan 不能隐式认领，绑定只能由 POST 明确指定。
  ```json
  {"expected_revision":null,"command":{"type":"enable","selection_key":"human-choice-1","source":{"stories":[{"id":"story_spec_0001","version":1}],"designs":[{"id":"design_spec_0001","version":1}]},"options":{"author_provider":"fake","reviewer_provider":"fake","review_rounds":1,"superpowers_enabled":false,"openspec_enabled":false,"plan_options":{"include_integration_tests":true,"include_e2e_tests":false,"force_frontend_backend_split":false,"require_execution_plan_confirm":false}},"logical_repository_id":"00000000-0000-0000-0000-000000000001"}}
  ```
  实际 story/design id 与 current_version 从 fixture 返回记录填入 JSON；版本不存在时拒绝，不替其猜 latest。旧 plan (`plan.created_at < enrollment.created_at`) 不能被批量/隐式认领，允许的绑定只由 POST 明确指定后来创建的 plan/session。
- [ ] **Step 2: 红灯。** `cargo test --locked --lib automation_enrollment -- --nocapture`；预期 route 404。
- [ ] **Step 3: 实现。** Enable 先校验 issue、确认的源 id/version、design 引用 story、target 与有效 manifest/selection 恰一个且一致，再调用 CAS；Disable 只需存在且 revision 匹配，不重新验证可能已失效的 source；bind 在 store CAS 临界区复查授权仍 enabled 和显式对象身份、原绑定不可改。`RunPolicy::AutoIfValid` 直接 409；不隐式调用 `prepare_work_item_plan`。HTTP 映射在 `error.rs` 明确添加 `automation_enrollment_conflict`→409、`automation_enrollment_not_found`→404、`automation_enrollment_invalid_scope`→422，错误 details 带当前 revision，不回 500。
  ```rust
  pub async fn put_automation_enrollment(
      State(state): State<WebAppState>, Path((project_id, issue_id)): Path<(String, String)>,
      Json(request): Json<EnrollmentPutRequest>,
  ) -> ApiResult<Json<IssueAutomationEnrollment>> {
      validate_enrollment_scope(&state, &project_id, &issue_id, &request)?;
      IssueAutomationStore::new(product_app_paths(&state))
          .compare_and_set(&project_id, &issue_id, request.expected_revision, request.command)
          .map(Json).map_err(enrollment_api_error)
  }
  ```
- [ ] **Step 4: 绿灯。** 同一过滤命令；额外用 `curl` 对运行中的本机 WebAppState 只做 `GET`（无服务时跳过、不得误报已验收）。
- [ ] **Step 5: 提交。** `git add src/web/handlers/automation_enrollment.rs src/web/handlers/mod.rs src/web/app.rs src/web/types.rs src/web/error.rs src/web/handlers/lifecycle_tests.inc.rs && git commit -m "feat: expose scoped enrollment cas endpoints"`。

## Task 4：1.2 同源服务端归属投影与 observer hydration

**Files:** Modify `src/product/issue_automation_store.rs`（Task 2）、`src/product/workspace_engine/session_state.rs:386-523`、`src/web/workspace_ws_types/out.rs:174-241`、`src/web/workspace_session/manager/{attachment.rs:85-99,durable_projection.rs:48-75}`、`src/web/workspace_session/router.rs:67-73,149-158,368-385`、`src/web/handlers/dto.rs:643-699`、`src/web/handlers/lifecycle.rs:403-406`、`src/web/handlers/workspace_session.rs:15-274`、`src/web/types.rs:951-980`、`web/src/api/types/workspace.ts:116-136`、`web/src/state/workspace-ws-store-types.ts:437-558`、`web/src/state/workspace-ws-store.ts:94-165,227-349`、`web/src/state/workspace-observer-store.ts:380-439`；Test `src/web/workspace_session/tests/part_02.rs:1-108`、`web/src/state/workspace-observer-store.test.ts`。

**Interfaces:** Consumes: Task 2 `IssueAutomationStore::get`；`WorkspaceSessionRecord` 的 project_id/issue_id/entity_id/id/workspace_type/run_policy。Produces: `IssueAutomationStore::ownership_for_session(&self,record:&WorkspaceSessionRecord)->Result<AutomationOwnership,ProductStoreError>`；`project_session_automation(frame:&mut WsOutMessage,record:&WorkspaceSessionRecord,store:&IssueAutomationStore)->Result<(),ProductStoreError>` 在所有 manager 对外 SessionState 出口注入 `automation:{owner,enrollment_id,policy_revision,enabled}`。无 enrollment/未精确绑定=`{client,null,null,false}`，读取失败不伪 client：HTTP 明确错误，前端保留未知 null；对 `WorkspaceEngine::build_session_state(&self)->WsOutMessage` 的现有同步函数不改签名。

- [ ] **Step 1: 红测。** 创建同 issue 两个 `WorkspaceSessionRecord`，仅 enrollment 绑定其中之一；两个 summary、活跃 engine 初帧、`durable_projection_with`/observer 都断言只绑定者 `server/enabled=true/revision`，disable 后同 manager 新快照为 `client/enabled=false/revision+1`；删除 enrollment 文件的老 session 按 client，损坏文件返回显式 projection error 而不伪作 client。前端首次状态 `automation:null`，收到明确 client/server 后投影与 REST 同形：
  ```ts
  expect(observerStateFromSessionState({ ...snapshot, automation: { owner: "server", enrollment_id: "en-1", policy_revision: 2, enabled: true } }).automation?.owner).toBe("server");
  ```
- [ ] **Step 2: 红灯。** `cargo test --locked --lib automation_ownership -- --nocapture`；`pnpm -C web exec vitest run src/state/workspace-observer-store.test.ts`；预期新字段断言失败。
- [ ] **Step 3: 实现。** session summary 不复制归属进旧持久 JSON，扩展 `workspace_session_summary_dto`/`workspace_session_dto` 的调用签名并迁移 `handlers/lifecycle.rs:405` 与 `handlers/workspace_session.rs:16,50,72,193,206,219,274`；manager 在 `attached_session_state`、`durable_projection_with` 和 `router` 对外广播 SessionState 之前统一调用 `project_session_automation`，内存 engine 和 durable fallback 都取同一 `ownership_for_session`，不从 `run_policy` 猜 owner。`WsOutMessage::SessionState` 增 `automation`，`WorkspaceEngine::build_session_state` 保持同步签名和旧独立 fixture 默认 client/off；对外 owner 始终由 manager 二次覆盖并传播读取错误，HTTP/WS 错误不能降级伪 client。web store 读取 `state.automation ?? null` 并在 reset 回 null；observer 同步快照，D6 不从 stopPoints 猜测。
- [ ] **Step 4: 绿灯。** 上述 Rust+Vitest 命令；再跑 `cargo test --locked --lib workspace_session -- --nocapture` 确认旧摘要消费者兼容。
- [ ] **Step 5: 提交。** 精确暂存本任务 Files 所列文件，`git commit -m "feat: project durable automation ownership to sessions"`。

## Task 5：1.2 D6 前端退位与非 enrolled 回归

**Files:** Modify `web/src/hooks/useCockpitAutopilot.ts:7-128`、`web/src/hooks/useCockpitAutopilot.test.tsx:19-231`；Test 同文件。

**Interfaces:** Consumes: Task 4 `WorkspaceWsState.automation:AutomationOwnership|null`；Produces: `useCockpitAutopilot(input:CockpitAutopilotInput):void` 原签名不变；owner 未知/server 时不分配 commandId、不调用 `sendAdvance`；`owner/revision/enrollment_id/enabled` 改变清除 anchors + stopped session；client 继续旧 stopPoints 分支。

- [ ] **Step 1: 红测。** 既有 fixture 改为明确 client；增加三个场景：归属未知不 send；同一 Confirmed 门 client→server→client/revision+1 恰有独立新 commandId 而 server 阶段无发令；未 enrolled client 重渲染依然稳定单发。用 `vi.spyOn(crypto,"randomUUID")` 证明未知/server 不生成 id。
  ```ts
  const view = renderAutopilot(state({ ...confirmedGate("turn-7"), automation: null }));
  expect(view.sendAdvance).not.toHaveBeenCalled();
  view.rerender(<AutopilotHarness state={state({ ...confirmedGate("turn-7"), automation: { owner: "client", enrollment_id: null, policy_revision: null, enabled: false } })} sendAdvance={view.sendAdvance} stopPoints={[]} />);
  expect(view.sendAdvance).toHaveBeenCalledTimes(1);
  ```
- [ ] **Step 2: 红灯。** `pnpm -C web exec vitest run src/hooks/useCockpitAutopilot.test.tsx`；预期未知仍抢发。
- [ ] **Step 3: 最小实现。** `AutopilotState` Pick 增 `automation`；独立 effect 以 `(sessionId, owner, enrollment_id, policy_revision, enabled)` tuple 为 key，变化立即 `anchorsRef.current.clear(); stoppedSessionRef.current=null`；发送 effect 在读取 `advanceCommands`/分配 id 前 `if (stateSessionId!==sessionId || automation?.owner!=="client") return;`；owner 变化 effect 定义在发送 effect 前。**不要**把 enabled=false 解释成未知：显式 client 就保留旧行为。
- [ ] **Step 4: 绿灯。** 同一 Vitest 文件；跨 tab 水合重连 fixture 必须明确 client 才允许既有发令断言。
- [ ] **Step 5: 提交。** `git add web/src/hooks/useCockpitAutopilot.ts web/src/hooks/useCockpitAutopilot.test.tsx && git commit -m "fix: retire cockpit advance effect for server owner"`。

## Task 6：1.3 choice 共用真实回执与多问题透传

**Files:** Create `src/web/choice_reply.rs`（共享请求/状态 DTO）与 `src/cross_cutting/choice_delivery.rs`（供 provider、runner、bridge 使用的 signal）；Modify `src/web/mod.rs`、`src/cross_cutting/mod.rs:1-42`、`src/cross_cutting/streaming_provider/mod.rs:515-547,762-777`、`src/cross_cutting/approval_bridge/{commands.rs:35-74,mod.rs:35-51,170-226}`、`src/product/workspace_engine/{provider_drive.rs:483-533,provider_drive/work_item_plan.rs:109-140,review/drive.rs:808-850}`、`src/product/coding_workspace_engine/{provider_stream.rs:478-539,tool_format.rs:7-53}`、`src/product/coding_workspace_runner.rs:9-31`、`src/cross_cutting/pi_provider/session.rs:385-394`；Test `src/cross_cutting/approval_bridge/tests.rs:235-301`、`src/product/workspace_engine/tests/part_01.rs:739-845`、`src/product/coding_workspace_engine/tests/provider_start_persistence.rs:378-488`。

**Interfaces:** Consumes: `ChoiceAnswerData` (`src/cross_cutting/streaming_provider/mod.rs:531-536`)、现有 `ProviderCommand::ChoiceResponse`、`ApprovalBridge::request_choice`。Produces: cross_cutting 层 `ChoiceReplyState::{Submitting,Resolving,Delivered,Rejected,Expired}`（serde snake_case）与 `ChoiceDeliverySignal(Arc<watch::Sender<ChoiceReplyState>>)`，`new()->(Self,watch::Receiver<ChoiceReplyState>)` 初始 Submitting、`mark_resolving()/deliver()/reject()/expire()`；web 层复用同一 `ChoiceReplyState`，定义 `ChoiceResponseRequest{command_id:String,expected_run_id:String,answers:Vec<ChoiceAnswerData>}`、`ChoiceReplyStatus{command_id:String,expected_run_id:String,choice_id:String,state:ChoiceReplyState}`（serde snake_case）。现有 `ProviderCommand::ChoiceResponse` 和 `CodingRunnerCommand::ChoiceResponse` 都增 `answers:Vec<ChoiceAnswerData>,receipt:Option<ChoiceDeliverySignal>`；为 `ChoiceOptionData/ChoiceQuestionData/ChoiceAnswerData` 增 serde derives 支持持久 gate/REST，`CodingRunnerCommand.receipt` 用 `#[serde(skip)]` 保持命令 JSON 兼容，`ChoiceDeliverySignal` 按 `Arc::ptr_eq` 手写 `PartialEq/Eq` 保持命令值比较。`ChoiceDecision` 增 `receipt:Option<ChoiceDeliverySignal>`；旧调用方明确传 `None`，不新增平行命令 variant。payload 比较包含同一 choice/run 身份与完整 answers。

- [ ] **Step 1: 写失败测试。** bridge 两题 fixture 提交带 receipt 的 `ChoiceResponse`，用受控 waiter 暂停解析并断言 `Resolving`，释放 waiter、收到 `ChoiceDecision.answers` 后才断言 `Delivered`；取消 waiter 或关闭 channel 返回 `Expired/Rejected`，mpsc sender 成功但 waiter 未解析不得是 `Delivered`。workspace 主 provider、work-item-plan、review 三条 forwarding path 各断言 `answers` 不变；coding 双题 gate 在回执前仍 Open。不能假定 `send().await` 推进消费者，也不能假定 watch 必逐一观察瞬态，测试栅栏固定状态边界。
  ```rust
  let (receipt, mut status) = ChoiceDeliverySignal::new();
  command_tx.send(ProviderCommand::ChoiceResponse {
      id: "choice-1".into(), selected_option_ids: vec!["a".into()],
      free_text: None, answers: two_answers.clone(), receipt: Some(receipt),
  }).await.unwrap();
  // provider 命令消费者已领取命令，request_choice waiter 被测试栅栏暂停。
  assert_eq!(*status.borrow(), ChoiceReplyState::Resolving);
  waiter_barrier.notify_one();
  let decision = waiting.await.unwrap().unwrap();
  assert_eq!(decision.answers, two_answers);
  assert_eq!(*status.borrow(), ChoiceReplyState::Delivered);
  ```
- [ ] **Step 2: 红灯。** `cargo test --locked --lib choice_delivery -- --nocapture`；预期 shared status/receipt 字段与完整 answers 断言失败。
- [ ] **Step 3: 最小实现。** `commands.rs` 的 pending map 保存 `{decision_tx,receipt}`：收到 `ChoiceResponse` 时先从 map 原子摘出，再向 oneshot 发送包含完整 answers/receipt 的 `ChoiceDecision`；send 失败则 `receipt.reject()`，waiter 成功解析后才 `receipt.deliver()`。mpsc 入队只调用 `mark_resolving()`。workspace 三条 drive 与 coding `provider_stream.rs`、`tool_format.rs` 逐字段透传 answers/receipt；coding 只在 signal 已 Delivered 后调用 `resolve_choice_gate`。Pi 不经过 bridge：实施时先核对 `pi_provider/session.rs` 的 `ChoiceResponse` 等待者结构——若其命令结构承载 `answers`（或可扩展承载）则逐字段透传并 `receipt.deliver()`；若 pi 会话类型确实无多问题槽位，则该路径仅透传单问题等价字段并在 receipt Delivered 前提下允许（在代码注释标注 pi 多问题为已知限制、REST 层不做 provider 分支拒绝）；旧无 receipt provider 行为保持不变。
  ```rust
  let decision = decision_rx.await.map_err(|_| permission_bridge_error("choice response channel closed"))?;
  if let Some(receipt) = decision.receipt.as_ref() { receipt.deliver(); }
  Ok(decision)
  ```
- [ ] **Step 4: 绿灯。** `cargo test --locked --lib choice_delivery -- --nocapture`；再跑 `cargo test --locked --lib approval_bridge_ -- --nocapture` 与 provider 的 `ask_user_question`/coding choice 过滤用例，证明旧单问题和 TextFallback 零回归；不得以 mpsc accepted 断言 Delivered。
- [ ] **Step 5: 提交。** `git add src/web/choice_reply.rs src/web/mod.rs src/cross_cutting/choice_delivery.rs src/cross_cutting/mod.rs src/cross_cutting/streaming_provider/mod.rs src/cross_cutting/approval_bridge/commands.rs src/cross_cutting/approval_bridge/mod.rs src/cross_cutting/approval_bridge/tests.rs src/product/coding_workspace_runner.rs src/product/coding_workspace_engine/provider_stream.rs src/product/coding_workspace_engine/tool_format.rs src/product/coding_workspace_engine/tests/provider_start_persistence.rs src/product/workspace_engine/provider_drive.rs src/product/workspace_engine/provider_drive/work_item_plan.rs src/product/workspace_engine/review/drive.rs src/product/workspace_engine/tests/part_01.rs src/cross_cutting/pi_provider/session.rs && git commit -m "feat: acknowledge choice only at live waiter"`。

## Task 7：1.3 workspace manager 认领与 WS 共用接入

**Files:** Modify `src/web/workspace_session/manager/{mod.rs:42-68,choices.rs:8-79,runs.rs:94-146}`、`src/web/workspace_session/router.rs:85-129`、`src/web/workspace_ws_types/{in_.rs:54-60,out.rs:41-56,187-241}`、`src/web/workspace_ws_handler/decisions/inbound.rs:252-352`、`src/product/workspace_engine/session_state.rs:532-590`、`web/src/state/workspace-ws-store-helpers.ts:536-574`；Create `src/web/workspace_session/tests/part_07.rs` and include it in `src/web/workspace_session/tests.rs:22-27`；retain `part_02.rs:1-108` attach/degraded regression tests.

**Interfaces:** Consumes: Task 6 `ChoiceResponseRequest`, `ChoiceReplyStatus`, `ChoiceReplyState`, `ChoiceDeliverySignal`。Produces: `WorkspaceSessionManager::claim_choice(&self,choice_id:&str,request:&ChoiceResponseRequest)->Result<(ChoiceReplyStatus,bool),ChoiceReplyError>`、`submit_claimed_choice(&self,choice_id:&str,request:&ChoiceResponseRequest)->Result<ChoiceReplyStatus,ChoiceReplyError>`、`choice_status(&self,choice_id:&str,command_id:&str)->Result<ChoiceReplyStatus,ChoiceReplyError>`、`wait_choice_receipt(&self,choice_id:&str,command_id:&str,deadline:Duration)->Result<ChoiceReplyStatus,ChoiceReplyError>`；`ChoiceReplyError::{Unknown,Conflict,Expired}`。`ActiveRun.run_incarnation:String` 为每次启动生成 UUID，绝不复用可重启归零的 `id:u64/token:u64`；pending projection 逐项带 `expected_run_id` 和处理态。TextFallback 无 active run 继续 `take_pending_author_choice_prompt` follow-up，稳定身份为 `fallback:{session_id}:{choice_id}`，不冒充 provider Delivered。

- [ ] **Step 1: 写失败测试。** 同一 manager 登记两题 choice，HTTP/WS 两路并发 claim 只有一个 `won=true`；同 command+同 answers 返回原状态且不二发，同 command 异 payload 或另一 command 返回 Conflict；run 结束/重启后旧 `expected_run_id` 返回 Expired；先 claim 后持有 engine mutex，claim 仍立即完成；发送失败或 receipt 未知时 pending frame 不删除，Delivered 后才广播收敛的 session_state。
  ```rust
  let first = manager.claim_choice("choice-1", &request).unwrap();
  assert!(first.1);
  assert!(!manager.claim_choice("choice-1", &request).unwrap().1);
  assert_eq!(manager.claim_choice("choice-1", &different).unwrap_err(), ChoiceReplyError::Conflict);
  ```
- [ ] **Step 2: 红灯。** `cargo test --locked --lib workspace_choice_claim -- --nocapture`；预期 claim/status API 尚不存在。
- [ ] **Step 3: 最小实现。** manager 状态锁短临界区登记 `(run_incarnation,choice_id)->(command_id,payload_fingerprint,status,watch)`，不在锁内 `.await`；`submit_claimed_choice` 锁外向当前 run `command_tx` 发带 answers/receipt 的 command，成功只更新 Resolving，waiter Delivered 后才摘 pending frame。`wait_choice_receipt` 在 deadline 到期时返回 `Ok(self.choice_status(choice_id,command_id)?)`；run 过期/冲突仍返回错误，绝不吞成 202。run finish/abort/supersede 将旧 claim 置 Expired 并保留有界 status 供同 command 查询；WS `ChoiceResponse` 缺省 command_id/expected_run_id 时只绑定当前唯一 run，再调用相同 claim 门面；Conflict/Expired 映射既有 `ProtocolError`；保留 TextFallback 原 follow-up 分支。
- [ ] **Step 4: 绿灯。** `cargo test --locked --lib workspace_choice_claim -- --nocapture` 与 `cargo test --locked --lib workspace_session::tests -- --nocapture`；后者覆盖 attach、cursor replay、degraded choice 重投。
- [ ] **Step 5: 提交。** 精确暂存本任务 Files，`git commit -m "feat: arbitrate workspace choice by run and command"`。

## Task 8：1.3 workspace choice REST 与 200/202/404/409/410

**Files:** Create `src/web/handlers/workspace_choice.rs`；Modify `src/web/app.rs:324-350`、`src/web/handlers/mod.rs:72-93,153-158`、`src/web/error.rs:48-110`、`src/web/types.rs:951-980`；Test new handler `#[cfg(test)]` module and append observer write-rejection case to `src/web/workspace_ws_handler/tests.rs:153-205`.

**Interfaces:** Consumes: Task 7 manager APIs、`WorkspaceSessionRegistry::get_or_create` (`registry.rs:23-46`)、`LifecycleStore::get_workspace_session`。Produces `post_workspace_choice_response(State<WebAppState>,Path<(String,String)>,Json<ChoiceResponseRequest>)->ApiResult<(StatusCode,Json<ChoiceReplyStatus>)>` and `get_workspace_choice_response_status(State<WebAppState>,Path<(String,String,String)>)->ApiResult<Json<ChoiceReplyStatus>>` for `POST/GET /api/workspace-sessions/{session_id}/choices/{choice_id}/response(s)/{command_id}`。

- [ ] **Step 1: 写失败测试。** fixture waiter 尚未解析时 POST 仅 202/Resolving，真实 waiter 解析后同 command retry 200/Delivered；unknown choice 404、same command different payload/second claimant 409、old run/expired choice 410；GET Resolving 返回 200 状态体但不承诺 Delivered；持有 engine mutex 时仍在 350ms HTTP deadline 内返回 202。
  ```rust
  assert_eq!(response.status(), StatusCode::ACCEPTED);
  assert_eq!(body["state"], "resolving");
  assert_eq!(body["command_id"], "cmd-one");
  ```
- [ ] **Step 2: 红灯。** `cargo test --locked --lib workspace_choice_http -- --nocapture`；预期 route 404。
- [ ] **Step 3: 最小实现。** 先从 durable session 查 project/issue，再用 registry `get_or_create` 得到唯一 manager（不挂 attachment、不抢 driver lease）；先 `claim_choice` 再 `submit_claimed_choice`，只在锁外等待 `wait_choice_receipt(...,Duration::from_millis(350))`。状态映射固定为 Delivered→200，Submitting/Resolving→202，Unknown→404，Conflict→409，Expired/Rejected→410；仅 deadline 到期返回最新内存状态并允许同 command 查询，其他错误不吞掉。`error.rs` 显式映射，禁止 timeout 启动新 command。
  ```rust
  let (_, won) = manager.claim_choice(&choice_id, &request).map_err(choice_api_error)?;
  if won {
      manager.submit_claimed_choice(&choice_id, &request).await.map_err(choice_api_error)?;
  }
  let current = manager.wait_choice_receipt(&choice_id, &request.command_id, Duration::from_millis(350))
      .await.map_err(choice_api_error)?;
  let code = if current.state == ChoiceReplyState::Delivered { StatusCode::OK } else { StatusCode::ACCEPTED };
  Ok((code, Json(current)))
  ```
- [ ] **Step 4: 绿灯。** 上述 HTTP 过滤用例及 observer WS 的 ChoiceResponse/人工门/recovery 写拒绝用例，证明 REST 不反向提升 observer 权限。
- [ ] **Step 5: 提交。** 精确暂存本任务 Files，`git commit -m "feat: answer workspace choice without driver websocket"`。

## Task 9：1.3 coding choice REST、WS 同 claim 与 durable 多问题 gate

**Files:** Modify `src/product/coding_models/gate.rs:47-75`、`src/product/coding_attempt_store/{inputs.rs:64-77,gate.rs:375-478}`、`src/product/coding_workspace_engine/{gates.rs:225-259,provider_stream.rs:478-539}`、`src/product/coding_workspace_runner.rs:24-28`、`src/web/state/coding_run_registry.rs:41-70,251-270`、`src/web/coding_ws_handler/{protocol.rs:197-206,socket.rs:919-947,state.rs:74-82}`；Create `src/web/handlers/coding_choice.rs`；Modify `src/web/app.rs:276-285`、`src/web/handlers/mod.rs:72-93,140-158`、`web/src/api/types/coding.ts:424-469`；Test `src/web/coding_ws_handler/tests.rs:1-160`、gate tests adjacent to `src/product/coding_attempt_store/gate.rs`、new handler tests.

**Interfaces:** Consumes: Task 6 `ChoiceResponseRequest/ChoiceReplyStatus/ChoiceDeliverySignal`、`CodingRunRegistry::command_sender`、`CodingAttemptStore::list_open_choice_gates`。Produces: `CodingRunRegistry::claim_choice(&self,attempt_key:&CodingAttemptRunKey,choice_id:&str,request:&ChoiceResponseRequest)->Result<(ChoiceReplyStatus,bool),ChoiceReplyError>`、`choice_status(&self,attempt_key:&CodingAttemptRunKey,choice_id:&str,command_id:&str)->Result<ChoiceReplyStatus,ChoiceReplyError>`、`wait_choice_receipt(&self,attempt_key:&CodingAttemptRunKey,choice_id:&str,command_id:&str,deadline:Duration)->Result<ChoiceReplyStatus,ChoiceReplyError>`；coding REST POST/GET 沿用 Task 8 的 200/202/404/409/410 映射。新增 `CodingChoiceQuestion{id,prompt,options:Vec<CodingChoiceOption>,allow_multiple,allow_free_text}`（`src/product/coding_models/gate.rs`，`Serialize+Deserialize`）；`CodingChoiceGate.questions:Vec<CodingChoiceQuestion>`、`CodingChoiceGateResponse.answers:Vec<ChoiceAnswerData>` 均 `#[serde(default)]`，`CreateChoiceGateInput.questions:Vec<CodingChoiceQuestion>`。provider `ChoiceRequestData.effective_questions()` 转成 coding 持久 DTO；旧 gate 的 questions=[] 投影成单一 `id="default"` 问题；`CodingRunnerCommand::ChoiceResponse` 的 receipt 不序列化。coding run 注册生成 UUID run_incarnation 并与精确 attempt key 绑定。

- [ ] **Step 1: 写失败测试。** 以真实 `ChoiceRequestData` 两题创建 gate，REST/WS 同 attempt+run-incarnation 并发只产生一条 runner command；waiter 未解析时 gate 仍 Open/HTTP 202，receipt Delivered 后 gate 才 Resolved/200 且 response.answers 两题与输入一致；旧 gate 缺 questions 时按单一 `default` 题兼容；无 runner、attempt 不同、旧 run、不匹配 choice 分别覆盖 410/404；同 command 同 payload 不二发，异 payload 409。coding session snapshot 和 GET attempt snapshot 均含 questions，不以 coding socket 为前提。
  ```rust
  assert_eq!(gate.questions.len(), 2);
  assert_eq!(resolved.response.unwrap().answers[1].question_id, "q-2");
  assert_eq!(registry.claim_choice(&attempt_key, "ch-1", &old_run_request).unwrap_err(), ChoiceReplyError::Expired);
  ```
- [ ] **Step 2: 红灯。** `cargo test --locked --lib coding_choice_reply -- --nocapture`；预期 questions/answers 字段与 route 断言失败。
- [ ] **Step 3: 最小实现。** `CodingRunEntry` 保存每次启动生成且永不复用的 `run_incarnation`；registry 短锁按 attempt+run+gate claim，runner command 携完整 answers/receipt，WS 与 REST 都走该 claim；不可只调用现有 `command_sender` 丢失 incarnation。`provider_stream.rs` 收到 command 后送 provider，只有 receipt Delivered 才调用 `resolve_choice_gate`（同时落盘 answers）；`tool_format.rs` 同样透传，失效/取消使用现有 Stale/Cancelled 而非假 Resolved。HTTP 先 `get_attempt(project_id,issue_id,attempt_id)` 验作用域，再查 gate；P0 不触发 coding 首启。
- [ ] **Step 4: 绿灯。** `cargo test --locked --lib coding_choice_reply -- --nocapture`、`cargo test --locked --lib coding_ws_handler::tests -- --nocapture`，并复跑现有 coding WS 单题测试。
- [ ] **Step 5: 提交。** 精确暂存本任务 Files，`git commit -m "feat: answer coding choice through shared runner claim"`。

## Task 10：1.3 无 driver 人工门与 compile recovery

**Files:** Create `src/web/handlers/workspace_human_action.rs`；Modify `src/web/workspace_ws_handler/decisions.rs:90-232`、`src/web/workspace_ws_handler/decisions/inbound.rs:107-163,192-205,357-375`、`src/web/app.rs:324-350`、`src/web/handlers/mod.rs:72-93,153-158`、`src/web/types.rs:932-941`、`src/web/error.rs:48-110`、`web/src/state/cockpit-action-routing.ts:51-166`、`web/src/pages/useCockpitGateConfirm.ts:28-175`、`web/src/pages/ChatCockpitPage.tsx:342-375`、`web/src/api/client.ts:61-76`；Test `src/web/workspace_ws_handler/tests/campaign_stage3_interactive/cases.rs:1-90`、`web/src/pages/ChatCockpitPage.inbox.test.tsx:93-170`。

**Interfaces:** Consumes: `WorkspaceSessionRegistry::get_or_create`、现有 `HumanGateFeedbackInput`、`HumanGateCloseDecision`、`WorkspaceEngine::handle_work_item_plan_compile_recovery_action` (`src/product/workspace_engine/compile.rs:910-914`) 和既有 batch/story `workspace_session_confirm` HTTP。Produces: `POST /api/workspace-sessions/{session_id}/human-actions`；`HumanActionRequest::{Approve{command_id,expected_gate_id},Abandon{command_id,expected_gate_id},Feedback{command_id,expected_gate_id,feedback},CompileRecovery{command_id,expected_gate_id,action,reason}}`（`#[serde(tag="type",rename_all="snake_case")]`），`HumanActionStatus{command_id:String,state:HumanActionState,gate_id:String}`；`HumanActionState::{Accepted,Busy,Rejected}`（snake_case wire）；`postWorkspaceHumanAction(sessionId:string,action:HumanActionRequest):Promise<HumanActionStatus>`。WS 与 REST 调同一 engine 门动作；observer WS 仍只读，HTTP 的 409 busy/error 和 WS 帧分别映射。

- [ ] **Step 1: 红测。** 先让 SC 进入 HumanConfirm，关闭 driver：REST feedback 的同 command 重试只创建一个 turn；approve 仍经过 deterministic compile，失败不宣称 Confirmed；abandon 进终态；compile recovery 只在 `WorkItemPlanCompileRecovery` 节点允许；gate id 不匹配、run 持锁均不误答/不堵死；observer WS 人工命令仍 `OBSERVER_WRITE_REJECTED`。前端观察态 gate 卡点 feedback/abandon/recovery 只调用 REST facade，不调 `sendHumanGateFeedback/sendAbandonGate/sendWorkItemPlanCompileRecoveryAction`；batch/story 既有 `confirmWorkspaceSession` 保留。

  ```ts
  await userEvent.click(screen.getByRole("button", { name: "发送反馈" }));
  expect(postWorkspaceHumanAction).toHaveBeenCalledWith(sessionId, expect.objectContaining({ type: "feedback" }));
  expect(currentMockWorkspaceWs().sendHumanGateFeedback).not.toHaveBeenCalled();
  ```
- [ ] **Step 2: 红灯。** `cargo test --locked --lib workspace_human_action_http -- --nocapture`；`pnpm -C web exec vitest run src/pages/ChatCockpitPage.inbox.test.tsx`；预期 REST 路径未接线。
- [ ] **Step 3: 最小实现。** 从 `decisions.rs:108-232` 提取引擎调用与 `HumanGateCommandOutcome` 判定为可返回值的共用函数，WS 层只负责既有 OutboundControl 序列化；REST manager 的 `provider_run_context` 使用 `connection_id=None`，继续经唯一 manager run 与 event router，不新建第二 engine；`expected_gate_id` 比对当前 active gate/timeline node，忙态返回 409，compile recovery 调既有 `handle_work_item_plan_compile_recovery_action`；UI `CockpitActionFacade` 增 REST 异步回调但不让 observer WS 写过只读仲裁，client/非 enrolled 当前会话仍保留既有手动 WS 语义。
- [ ] **Step 4: 绿灯。** 上述两组定向测试；退回旧有预算耗尽/AlreadyClosed 用例验证不新建门或跳过 compile。
- [ ] **Step 5: 提交。** 精确暂存本任务文件，`git commit -m "feat: operate human plan gates without driver"`。

## Task 11：1.3 驾驶舱 workspace/coding choice 卡片就地作答

> **实施偏差记录（2026-09-26，Main 裁决 A）**：冷驾驶舱答 coding choice 依赖两个
> 原计划未覆盖的上游事实，经 Main 裁决按"既有拥有者任务实现的补全"扩入本任务
> 文件域：① `src/product/coding_models/gate.rs`、`src/product/coding_attempt_store/gate.rs`、
> `src/web/handlers/coding.rs`（+`coding_choice.rs` 测试）——`CodingChoiceGate` 增
> `expected_run_id` 投影字段（handler 层 stamp active run incarnation，serde
> optional 向后兼容；claim 校验语义不变），否则 REST claim 因
> `expected_run_id !== active_run_incarnation` 恒 410；②
> `web/src/hooks/useWorkspaceSessionObservers.ts`、`web/src/components/cockpit/CockpitShell.tsx`
> ——lifecycle 目录轮询副产物 `coding_attempts` + 会话→issue 映射构成 attempt
> 发现通道（`useCockpitCodingAttemptForSession`），页面按需拉
> `getCodingAttemptSnapshot`（挂载/抽屉打开各一次，不常驻轮询）。

**Files:** Modify `web/src/state/workspace-ws-store-types.ts:429-435`、`web/src/state/workspace-ws-store-helpers.ts:536-574`、`web/src/state/workspace-cockpit-projection.ts:459-474,524-642`、`web/src/state/workspace-observer-store.ts:99-112,380-439`、`web/src/components/chat-workspace/cockpit/CockpitInbox.tsx:50-93,139-270,272-372`、`web/src/components/chat-workspace/entries/ChoiceRequestEntry.tsx:10-14,94-107`、`web/src/pages/ChatCockpitPage.tsx:128-141,253-266,983-999`、`web/src/api/client.ts:61-76,377-395`、`web/src/api/types/{workspace.ts:116-136,coding.ts:424-469}`；Test `web/src/components/chat-workspace/cockpit/CockpitInbox.test.tsx`、`web/src/pages/ChatCockpitPage.choice-notice.test.tsx`、`web/src/pages/ChatCockpitPage.inbox.test.tsx`。

**Interfaces:** Consumes: Task 7 `pending_choice_requests[{id,prompt,options,questions,expected_run_id,status,source,role}]`；Task 9 coding snapshot `pending_choices`；Task 8/9 HTTP response `{command_id,expected_run_id,choice_id,state}`；Task 6 `answers` 数组。Produces: `CockpitInboxKind` 新增 `choice`，`CockpitInboxItem.choice:ChoiceInboxProjection|null`（非 choice=null），`ChoiceInboxProjection={sessionId:string,choiceId:string,expectedRunId:string,questions:ChoiceQuestion[],source:"workspace"|"coding",attemptAddress?:CodingAttemptAddress,status:"open"|"submitting"|"resolving"|"delivered"|"expired"}`；前端 `postWorkspaceChoiceResponse(sessionId,choiceId,request):Promise<ChoiceReplyStatus>`、`postCodingChoiceResponse(address,choiceId,request):Promise<ChoiceReplyStatus>`、对应 GET 查询。

- [ ] **Step 1: UI 红测。** 从 observer session_state 填两题：各题选择不同 id，点击选择卡提交，API 只收到一个 command_id、当前 expected_run_id 和**两个各自独立** answers；第一次 `202` 后卡片仍展示“处理中”并禁重复分配 id，同 command GET 查询后返回 Delivered 才消失；409 显示冲突/可刷新，410 显示已失效且不路由到新 run；无 run id 的旧投影只显示“刷新后作答”不猜 run；coding `getCodingAttemptSnapshot` 中 open choice 同样可在该 issue 的驾驶舱投影呈现，不需打开 Coding Workspace 或保持 socket。现有 `ChoiceRequestEntry` 禁用的 `submitting` 在 202/错误时由外部状态控制，错误可重试**同命令同 payload**，不是换新 id。
  ```ts
  expect(postWorkspaceChoiceResponse).toHaveBeenCalledWith("session-1", "choice-1", {
    command_id: expect.any(String), expected_run_id: "run-1",
    answers: [
      { question_id: "q-1", selected_option_ids: ["yes"], free_text: null },
      { question_id: "q-2", selected_option_ids: ["no"], free_text: null },
    ],
  });
  expect(screen.getByText("处理中")).toBeVisible();
  ```
- [ ] **Step 2: 红灯。** `pnpm -C web exec vitest run src/components/chat-workspace/cockpit/CockpitInbox.test.tsx src/pages/ChatCockpitPage.choice-notice.test.tsx src/pages/ChatCockpitPage.inbox.test.tsx`；预期 choice 不在 inbox/回答走 WS。
- [ ] **Step 3: 实现。** `pendingChoiceRequestsFromSession` 保留完整 `options/questions/source/expected_run_id/status`，严格检查 question id/数组，数据缺失禁止 REST 提交，不抹老等待提示；`selectCockpitInbox` 每个 pending choice 生一张不可批量选择的卡，`CockpitInbox` 复用 `ChoiceRequestEntry` 的表单渲染/`ChoiceResponsePayload.answers`（若未携 answers，则从默认单问题投影转换，不能把多题扁平化）；新增 `submitting`/服务器状态受控 prop，发送后 `202` 保卡并按同 command GET 复查，Unmount/刷新后按服务器状态+稳定 run/choice 恢复；response 200 仅表示 Delivered，不宣称 provider 成功。`ChatCockpitPage` 从 `useCockpitShellInbox` 取观察态会话 id，交给 REST 回调；coding 卡的 attempt 地址由 `IssueLifecycleResponse.coding_attempts` 确认 + `getCodingAttemptSnapshot` 的 open gate 提供，定期刷新复用现有 observer 轮询，勿把 P3 的近期完成信息混入待处理数。主对话已有 WS `handleChoiceResponse` 对非 enrolled 不做行为改写。
- [ ] **Step 4: 绿灯。** 相同 Vitest 命令；页面 smoke：本机服务可运行时启动浏览器，在 cockpit 打开抽屉实际勾两题、关 driver 后观察 202→已答/刷新收敛；无服务时用 targeted UI 渲染脚本并在验收记录中声明浏览器实测未发生，不伪称视觉证据。
- [ ] **Step 5: 提交。** 精确暂存本任务文件，`git commit -m "feat: resolve workspace and coding choice in cockpit"`。

## Task 12：1.4 P0 双清单关闸与真实链证据

**Files:** Modify `src/web/workspace_session/tests/part_07.rs`（任务 7 新增，且 `src/web/workspace_session/tests.rs:22-27` 须加入 include）、`src/web/handlers/{workspace_choice.rs,coding_choice.rs,workspace_human_action.rs}` 测试子模块、`web/src/pages/ChatCockpitPage.inbox.test.tsx`；Modify `openspec/changes/work-item-group-autopilot/tasks.md:5-10` **仅在两份证据全部真实通过后勾选 1.1–1.4**；维护实现说明时更新仓库已有用户说明，不新建无关文档。证据填入本计划末尾的两张清单（记录环境/命令/结论，不把 mock 当真实 provider）。

**Interfaces:** Consumes: Tasks 2–11 的接口/回执；Produces: 可复现替身测试及真实 provider 手工运行记录；P0 仍无 `AutopilotOrchestrator`、无定时 reconcile、无 typed 自动 StartCoding。

- [ ] **Step 1: 补失败前的关闸回归。** 对 review focus 逐项补贴近使用者的集成用例：HTTP/WS 两个实际路由同时竞态、同 command 重发/异 payload、等待超时后 GET 从 Resolving 到 Delivered、Abort/重启旧 run 410、observer 同 ChoiceResponse/HumanGateFeedback/CompileRecoveryAction 都拒写；默认无 enrollment/旧 session 投影 client、owner 未知暂停、非 enrolled 旧 WS choice/门/advance 行为不变；不会自动生成 plan、不会自动 advance、不会调用 StartCoding。
  ```rust
  assert_eq!(winner_count, 1);
  assert_eq!(different_payload.status(), StatusCode::CONFLICT);
  assert_eq!(expired_run.status(), StatusCode::GONE);
  assert_eq!(mock_provider_start_count.load(Ordering::SeqCst), 0);
  ```
- [ ] **Step 2: 红灯。** 各新增用例的 `cargo test --locked --lib workspace_choice_http -- --nocapture`、`cargo test --locked --lib coding_choice_reply -- --nocapture`、`pnpm -C web exec vitest run src/pages/ChatCockpitPage.inbox.test.tsx`，记录真实失败行为；已有绿断言不强制回退制造失败。
- [ ] **Step 3: 最小修复。** 若测试失败，修对应**既有拥有者**任务的实现（manager claim、bridge receipt、coding gate、REST handler 或 UI），不另起旁路、不放宽断言；通过后在本计划末尾如实填写替身矩阵，保留可复现的定向命令及结果。
- [ ] **Step 4: 绿灯 + 真实链。** 定向测试后集成负责人统一运行 `cargo test --locked --lib`、`pnpm -C web exec vitest run`；**前置**：启动本 worktree 的 aria server 与真实可交互 provider（当前未运行），人工在已有页面手动启动 plan → 关闭 driver WS/执行页 → 驾驶舱 observer 仅观察，从抽屉答 workspace choice → 处理审批/反馈/abandon 分支与 compile recovery → 运行续接；在 Ready 的编码 attempt 手动启动 coding 后关闭 coding WS，从驾驶舱答 coding choice；刷新核实同一 plan/session/attempt 与未决状态；检查无任何自动 prepare/advance/coding 首启。真实链不能用 fake provider 单测替代；服务未能启动则本任务保持未完成，并精准报告前置阻碍。
- [ ] **Step 5: 仅验收齐备后提交。** `git add src/web/workspace_session/tests/part_07.rs src/web/handlers/workspace_choice.rs src/web/handlers/coding_choice.rs src/web/handlers/workspace_human_action.rs web/src/pages/ChatCockpitPage.inbox.test.tsx openspec/changes/work-item-group-autopilot/tasks.md cadence/plans/2026-09-26_实施计划_WIGAutopilot_P0控制面_v1.0.md && git commit -m "test: close wig autopilot p0 dual evidence gates"`；未取得真实链证据时**不勾任务/不宣称 P0 done**，此前可独立提交 1–11 的测试和实现。

## 关闸证据记录格式（实施时在本节填实测结果，非占位断言）

### 替身清单实测（2026-09-26，第六棒，环境：worktree feat-b-0808-add-monorepo @ e3b54631）

命令与结果（全部真实执行，非 mock provider 断言冒充）：

| 替身/自动化检查 | 实测命令 | 结果 |
|---|---|---|
| CAS 同键/异源/启停并发、旧数据默认 off、单 target 约束（任务 2–4） | `cargo test --locked --lib automation_enrollment` / `issue_automation` / `workspace_session`（含 part_02/03） | 6 passed / 7 passed / 61 passed，0 failed |
| D6 unknown/server/client 与 revision 切换（任务 4–5） | `cargo test --locked --lib workspace_session` + `pnpm -C web exec vitest run src/hooks/useCockpitAutopilot.test.tsx`（随全量） | 全绿；owner=server 退位不抢发在 part_02/03 断言 |
| workspace 与 coding HTTP/WS 竞争及双题原样（任务 6–9、11） | `cargo test --locked --lib choice` / `workspace_choice_http` / `coding_choice_reply` | 65 passed / 3 passed / 7 passed；并发唯一赢家、双题 answers 原样、消费前不删卡 |
| accepted 与 Delivered、HTTP 202+GET/同键重试、旧 run 410（任务 6–9） | 同上 + `pnpm -C web exec vitest run src/pages/ChatCockpitPage.inbox.test.tsx`（202 保卡→同 command GET Delivered 消卡、409 同命令重试、410 已失效） | 全绿；未知回执不冒充 200、过期不可送新 run |
| observer 写拒、无 driver 的门/recovery、无自动启动（任务 10–12） | `cargo test --locked --lib observer_write` / `workspace_human_action_http` / `attempt_snapshot_stamps` + `pnpm -C web exec vitest run src/state/cockpit-action-routing.test.ts` | 1 / 4 / 1 / 37 passed；人工命令经 REST、自动 provider starts=0 在引擎测试族断言 |
| 全量闸门 | `cargo test --locked --lib` = **3710 passed / 0 failed / 3 ignored**；`pnpm -C web test` = **1890 passed / 0 failed**；`pnpm -C web exec tsc --noEmit` 零错误 | 双侧全绿 |

### 真实链实测（2026-09-26，aria-dev-v48m @ e3b54631，可交互 provider=pi，载体 issue_0003「计数器小工具」）

| 行 | 结果 | 证据 |
|---|---|---|
| 手动 plan 生成后关闭 driver，驾驶舱答 choice | ✔ | 全程 driver WS 仅"点火即关"（hello→start→close <1.5s）。story 会话 0016：5 个 pending choice 全部经 `POST /workspace-sessions/{id}/choices/{id}/response` 作答（200 delivered，含 expected_run_id 绑定），run 持续推进；plan 会话 0019：author 危险命令选择卡经 REST 作答后续跑。跨刷新（REST/observer 重连）pending 状态一致 |
| 人工 plan 门三分支 | ✔ | 反馈 ✔×2（0018 两轮 REST feedback→修订→复评→门重开）；确认 ✔（0019 REST approve→deterministic compile→plan confirmed 落盘+3 WI 发布）；门关闭≠成功 ✔（0018 首次 approve 机械校验失败→500 `human_action_engine_error`、门保持开放）；放弃 ✔（F2 修复后 0018 现场复活→REST abandon→200 accepted→durable terminated + terminal 节点，v48o 实证）；compile recovery 中断 ✘（自然链未触发该形态，由 REQ-PCG-02 测试族覆盖） |
| 手动启动 coding 后关闭 coding socket | 部分 | attempt f28d182d 经 coding WS flash（hello→start_coding→close）启动后，全程仅 REST snapshot 观察：status=running、节点 0002→0004 推进（零 socket 续跑 ✔、无自动首启 ✔——attempt 由人显式创建+启动）；自然链未出现 pending coding choice（6 分钟轮询无卡），冷作答由 Rust `attempt_snapshot_stamps_pending_choice_expected_run_id` + `coding_choice_reply` HTTP 族 + 前端 coding e2e 用例覆盖 |
| 默认 off 对照 | ✔ | issue_0003 全程无 enrollment，lifecycle 中全部会话 automation.owner=client；每个 run 均由人显式触发（kick/feedback/approve/create），服务端日志无任何自发动作；无自动 prepare/advance/StartCoding |
| relax-legacy 4.1 冒烟（搭车） | ✔ | pi 在 naruto（无 .aria 路由规则文件）完成 issue_0003 outline 生成，机械校验 0 error，产物正常进入校验流程；tasks.md 4.1 已勾、change 已归档（主规范 REQ-PROMPT-03 已同步） |

### 真实链发现（P0 关闸阻塞项）

- **F1（已修复 e3b54631）**：provider run 持 engine 锁期间，attach/广播全部走 durable 降级帧，run 化身注入缺席——无 driver 驾驶舱拿不到 expected_run_id（REST 作答恒 410）。修复：`stamp_pending_choice_run_ids` 于 `durable_projection`/attach 降级路径同样注入；红测 `durable_fallback_pending_choice_carries_active_run_incarnation`；v48m 实测会话 0016 五题全部 REST 200 delivered。
- **F2（已修复 677f88f1 + 7662f8c0）**：真实根因比初判更精确——`spawn_provider_run_from_event` 的「同节点去重」纯按 `node_id`：REST feedback 链的 `HumanGateScManualRevision` run 注册在仍开启的 human_confirm 门节点（0018=node_009）上，委托返修接力 `ProviderRunRequested{WorkItemPlanSingleCandidateAuthor}`（emit 时活动节点仍是同一门节点）命中纯 node_id 判据被静默 drain（tracing::debug 零可见），followups 同时按 phase=Generate 让位→双方都退出、无人驱动重跑→durable 卡 (running, generate)：无门、无 run、零事件。次生缺陷（复活第一轮实测）：F-23 僵尸恢复 stage 集合 {Running, CrossReview, Revision} 漏掉「stage=HumanConfirm ∧ durable=running」不自洽形态（manager 重建 stage 取自最后 Active 门节点）→重启也不复活。修复：①`ActiveRun` 增 kind，去重判据改**同 kind ∧ 同 node**；②relay spawn 失败/被拒回执落 durable——引擎守卫 `recover_delegated_author_rerun_failure`（仅 SC ∧ phase=Generate ∧ 无在途 run ∧ durable 仍 Running）回落人工门（WaitingForHuman+门节点摘要携带原因+HumanGateOpened）；③manager 创建期复活臂 `recover_delegated_rerun_orphan`（F-23 臂之前）复用同源守卫。三条红测转绿（`sc_delegated_rerun_relay_is_not_drained_by_gate_node_kind_collision` / `..._relay_failure_falls_back_to_human_gate_durable` / `..._orphan_reopens_human_gate_on_manager_recreate`）；全量 3714 passed/0 failed/3 ignored；WS 手动路径与 F-23 Story 恢复零改动。**0018 现场处置（v48o @ d47ddf6d）**：日志 `[aria-recovery] delegated rerun orphan recovered to human gate`；durable running→waiting_for_human→terminated；追加 node_012（human_confirm，摘要「SC 返修接力启动失败，已回落人工门」）+node_013（completed/已终止），001-011 append-only 未回写；探针 approve（stale gate）→409 current_gate_id=node_012；abandon（node_012）→200 accepted。
- **F3（未修，engine 侧，不阻塞 P0）**：`POST /workspace-sessions/{id}/run-next`（advance 桥）panic：`provider_workspace_runner.rs:134 unreachable: legacy fake runner does not support pi`——advance 到 coding 的桥接只支持 Fake provider，真实 provider 链 panic（连接空回复）。规避：直接 `POST work-item-plans/{plan}/coding-attempts` 创建 attempt（0019 计划已由此通向 coding）。归属：workspace runner advance 桥；影响面在 advance→coding 启链（P0 不覆盖）。登记位置：`cadence/plans/2026-09-26_实施计划_WIGAutopilot_P1后台plan链_v1.0.md` Global Constraints「P1 前置必修」（commit 677f88f1）。

**关闸判定（2026-09-27 修订）**：F2 已闭合（修复 677f88f1+7662f8c0；0018 现场复活与放弃分支 v48o 实证）；原「因 F2 与 F3 不勾选」中 F2 项解除；F3 不阻塞 P0（影响面在 advance→coding 启链，已登记 P1 前置必修）→ **勾选 1.1–1.4**；替身清单全绿+真实链主通路+放弃分支全部实证；P0 仍不宣称覆盖 advance/coding 首启。

| 替身/自动化检查 | 负责任务 | 预期可观察结果 |
|---|---:|---|
| CAS 同键/异源/启停并发、旧数据默认 off、单 target 约束 | 2–4 | 唯一 revision / 409 / 422，手工多 target 不受扩权 |
| D6 unknown/server/client 与 revision 切换 | 4–5 | 未知/server 不发，client 保留既有单发 |
| workspace 与 coding HTTP/WS 竞争及双题原样 | 6–9、11 | 唯一赢家，完整 answers，消费前不删卡 |
| accepted 与 Delivered、HTTP 202+GET/同键重试、旧 run 410 | 6–9 | 未知回执不冒充 200、过期不可送新 run |
| observer 写拒、无 driver 的门/recovery、无自动启动 | 10–12 | 既有错误码，人工命令通过 REST，自动 provider starts=0 |

| 真实 provider/人工链 | 关闸前置与操作 | 合格判据 |
|---|---|---|
| 手动 plan 生成后关闭 driver | aria server + 可交互 provider；驾驶舱答两题 choice | 页面无需持有 driver，原 provider run 继续，刷新 choice 状态一致 |
| 人工 plan 门和 recovery | 同一计划分别覆盖确认/反馈/放弃与 compile recovery 中断 | 门不自动批准/推进，成功 compile 才 durable Confirmed |
| 手动启动 coding 后关闭 coding socket | Ready 后由人显式 StartCoding，驾驶舱回答 coding choice | 唯一 attempt 续接、无暗中自动首启、Final Confirm 人工不代点 |
| 默认 off 对照 | 旧 issue/无 enrollment 的手工链 | plan/advance/StartCoding 原准入、错误、恢复均保持 |

## 范围边界与自审索引

- REQ-WIGA-01：任务 2/3（默认 off、准确 source/target、重复选择）；REQ-WIGA-02：任务 2/3/4（CAS、关闭、Interactive，不提前物化 P2 的 `CodingStartRunPolicy`）；REQ-WIGA-05：任务 6–11（choice、门、recovery 与 observer），任务 12 双清单；REQ-WIGA-08：任务 4/5/11/12（同源 owner、退位、非 enrolled 对照）。
- REQ-ADV-05、REQ-CG-04、REQ-MTG-03 三份 MODIFIED delta：任务 1 注释和 Ready/门/单 target 例外，任务 2/3 单 target 限制与 Interactive，任务 10 门关闭不等于成功，任务 12 明确无自动首启。
- P1 的 EnsurePreparedPlan、StartPlanGeneration 与用户 design 后模式 UI；P2 的 attempt 策略物化、自动 advance、typed StartCoding 单发/无 socket amendment；P3 的完成信息/历史补读均**不在本计划**。P0 已登记 enrollment REST 入口，但不产生后台动作。
- 本计划不是运行记录；关闸任务所列 aria server 目前未启动。替身完成不代替真实链完成；落盘时没有声称这些测试已经执行或 P0 已实现。
