# WIG Autopilot P1 后台 Plan 链 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 交付 `work-item-group-autopilot` tasks.md §2.1–§2.4：人工确认 Design 后显式选择自动化、无页面准备并生成唯一绑定 plan、停在人工 choice/门/recovery，以及成功确认信息与 P1 双清单关闸。

**Architecture:** P0 已有的 issue 级 enrollment 是唯一授权来源；薄 `AutopilotOrchestrator` 从 durable enrollment、创建意图、plan/session、run 检查点 reconcile，只请求 `EnsurePreparedPlan` 与 `StartPlanGeneration`。共用 prepare 和唯一 `WorkspaceSessionManager` run 面，事件只唤醒、启动扫描与有界补偿兜底；信息从成功 compile/publication 和 durable Confirmed 派生，只读展示，人工门仍只由人解除。

**Tech Stack:** Rust 2024、Axum、Tokio、serde、既有文件锁/原子 JSON；React、TypeScript、Vitest、Testing Library。无新外部依赖。

**Spec:** `openspec/changes/work-item-group-autopilot/{proposal.md,design.md,tasks.md,specs/work-item-group-autopilot/spec.md,specs/work-item-plan-conversational-gate/spec.md,specs/multi-target-group-coding/spec.md}`；只展开 tasks.md §2 P1，不改 OpenSpec 契约、不实施 §3 P2 或 §4 P3。

## Global Constraints

- REQ-WIGA-01/02、REQ-MTG-03：Story/Design 仍由人生成确认；模式缺省「不自动化」，无 enrollment 旧 issue/旧 plan 不接管；自动化只认**恰一 logical repository**，零/多/身份漂移 fail-closed，手工多 target 路径不变。
- REQ-WIGA-02、REQ-CG-04：绑定 plan session 永远 `RunPolicy::Interactive`；不得用 `AutoIfValid` 作为授权，也不得自动回答 choice、批准/放弃人工门、选择 compile recovery；approve/关门/Completed 节点不等于 durable Confirmed。
- D1/D2：关闭 enrollment 后的新动作认领被挡住；已认领且正在运行的 run 不隐式 Abort；重开即使携新 payload 也不能把旧 plan/session 隐式重授权。P0 `compare_and_set` 已对 enabled 态异 payload 返回 Conflict；disabled 后携当前 revision 可改 payload 重开并保留 enrollment 身份和绑定，`bind_plan` 首次绑定使 revision +1，**不得重复修补或假设 revision 不变**。
- D1/REQ-WIGA-03：编排器不能持第二个 provider drive；重复/漏唤醒、关页、manager 回收、进程重启不得复用会 supersede 活 run 的 handler 入口；外部 provider 是否收到启动无法证明时停在可诊断人工恢复，绝不宣称 exactly-once。
- D5/REQ-WIGA-07：P1 只做绑定 plan 的成功确认 info、稳定 key 和待处理隔离；不做 P2 coding 完成信息，也不做 P3 K 窗口外近期历史补读、TTL、跨刷新/跨设备提醒策略；P0 人工门/recovery REST 与线 B Unsent 机制原样复用、不另建。
- 文件阅读以**当前 worktree 实际源码**为准（P0 原 plan 的拟议接口不是代码）；在 `.worktrees/feat-b-0808-add-monorepo` 根执行定向 `cargo test --locked --lib <filter> -- --nocapture` / `pnpm -C web exec vitest run <file>`；最后由集成负责人统一运行全量。**本文件只是计划，未声称已运行命令或已部署**。
- **P1 前置必修**：advance→run-next 桥对真实 provider panic（`provider_workspace_runner.rs:134` `legacy fake runner does not support pi`）——编排器 `AdvancePlan` 前必须修复，否则 enrolled 链无法经 advance 到 coding。

## Review Focus

1. **plan 已创建但 session/绑定未写、同时两个进程补偿：**恢复固定 plan/session，绝不覆盖 plan 内容或另建第二条链（Task 4、10 的中窗测试）。
2. **enabled→disabled→新 payload 重开与 bind revision+1：**旧 action 快照不能继承新许可、原绑定不得换源隐式续跑（Task 4、6 的授权测试）。
3. **活 run 与 manager 回收/旧进程 provider 不确定：**重复唤醒不 supersede；检查点/ledger 不可证明可安全恢复时显式停等人（Task 5、6、10）。
4. **choice 已提出、人工门含反馈/approve/abandon、compile recovery：**后台不得自动代答/关门/重试，驾驶舱无 driver 的 P0 REST 仍可人工处理（Task 7、10）。
5. **approve 点击、compile Failed/RecoveryRequired、缺 provenance、Confirmed 未落盘或重复刷新：**全部不产生成功误报；真正成功只一条 info 且不进待处理/批量动作（Task 8、9、10）。

---

## 文件责任、已核验 P0 接口与统一 P1 接口

**边界与责任（均已查实际源码）：**`src/product/models/automation.rs` 和 `src/product/issue_automation_store.rs` 保管 issue 授权；`src/web/handlers/automation_enrollment.rs` 的 GET/PUT/binding REST 已做 confirmed source 精确版本和单 target 校验；`src/web/handlers/lifecycle.rs::prepare_work_item_plan` 是当前人工 prepare，但使用 `id: None` 无法供后台重试；`src/product/lifecycle_store/{plan,workspace}.rs` 支持稳定 id 但前者可能覆盖既存 plan，后者只核对部分身份。`src/web/workspace_session/manager/{mod,runs,durable_projection}.rs` 是唯一 plan run 持有者，`create` 执行 `recover_on_creation`（可能把僵尸 Running 整流），`src/web/workspace_ws_handler/run/provider_run.rs::spawn_provider_run_from_handler` 与 manager 的 `start_run_from_attachment` 会 supersede 活 run。`src/product/workspace_engine/conversational_gate.rs::close_human_gate` 成功后才核验 durable Confirmed；`src/product/work_item_plan_store.rs::list_compile_transactions` 与 `src/product/work_item_plan_source_store.rs::get_publication_provenance` 是成功证明。前端 `web/src/components/lifecycle/WorkItemPlanOptionsDialog.tsx` 与 `useIssueLifecycleGeneration.ts` 是唯一现有 Design→Plan 发起路径；`web/src/hooks/useWorkspaceSessionObservers.ts`、`web/src/components/cockpit/CockpitShell.tsx`、`web/src/components/chat-workspace/cockpit/CockpitInbox.tsx` 分别是观察、计数/提醒、收件箱展示；只扩现有职责。

**P0 已落地，以下签名只消费、不改名/改参数/改语义：**

```rust
// src/product/models/automation.rs
pub struct IssueAutomationEnrollment {
    pub enrollment_id: String, pub selection_key: String,
    pub project_id: String, pub issue_id: String, pub enabled: bool,
    pub policy_revision: u64, pub source: EnrollmentSource,
    pub options: EnrollmentOptions, pub logical_repository_id: LogicalRepositoryId,
    pub prepare_intent_id: String, pub plan_id: Option<String>,
    pub session_id: Option<String>, pub created_at: String, pub updated_at: String,
}
pub enum EnrollmentWriteCommand {
    Enable { selection_key: String, source: EnrollmentSource,
             options: EnrollmentOptions, logical_repository_id: LogicalRepositoryId },
    Disable,
}
// src/product/issue_automation_store.rs
IssueAutomationStore::get(&self, project_id: &str, issue_id: &str)
    -> Result<Option<IssueAutomationEnrollment>, ProductStoreError>;
IssueAutomationStore::compare_and_set(&self, project_id: &str, issue_id: &str,
    expected_revision: Option<u64>, command: EnrollmentWriteCommand)
    -> Result<IssueAutomationEnrollment, EnrollmentError>;
IssueAutomationStore::bind_plan(&self, project_id: &str, issue_id: &str,
    expected_revision: u64, plan_id: &str, session_id: &str)
    -> Result<IssueAutomationEnrollment, EnrollmentError>;
IssueAutomationStore::ownership_for_session(&self, record: &WorkspaceSessionRecord)
    -> Result<AutomationOwnership, ProductStoreError>;
// src/web/choice_reply.rs、src/cross_cutting/choice_delivery.rs
pub struct ChoiceResponseRequest {
    pub command_id: String, pub expected_run_id: String,
    pub answers: Vec<ChoiceAnswerData>,
}
pub enum ChoiceReplyState { Submitting, Resolving, Delivered, Rejected, Expired }
// src/web/workspace_session/manager/choices.rs；P1 无需调用，列出实际签名防误改
WorkspaceSessionManager::claim_choice(&self, choice_id: &str,
    request: &ChoiceResponseRequest) -> Result<(ChoiceReplyStatus, bool), ChoiceReplyError>;
WorkspaceSessionManager::submit_claimed_choice(self: &Arc<Self>, choice_id: &str,
    request: &ChoiceResponseRequest) -> impl Future<Output = Result<ChoiceReplyStatus, ChoiceReplyError>>;
WorkspaceSessionManager::choice_status(&self, choice_id: &str, command_id: &str)
    -> Result<ChoiceReplyStatus, ChoiceReplyError>;
WorkspaceSessionManager::wait_choice_receipt(&self, choice_id: &str,
    command_id: &str, deadline: Duration)
    -> impl Future<Output = Result<ChoiceReplyStatus, ChoiceReplyError>>;
```

上述 choice 面只消费已落地 REST（不增 P1 回答接口）：`POST /api/workspace-sessions/{session_id}/choices/{choice_id}/response`、`GET /api/workspace-sessions/{session_id}/choices/{choice_id}/responses/{command_id}`、`POST /api/workspace-sessions/{session_id}/human-actions`；`HumanActionRequest::{Approve,Abandon,Feedback,CompileRecovery}` 已在 `src/web/types.rs`。enrollment REST 已在 `src/web/app.rs:259-266`：`PUT/GET /api/projects/{project_id}/issues/{issue_id}/automation-enrollment`，`POST /api/projects/{project_id}/issues/{issue_id}/automation-enrollment/binding`。手工 WorkItemPlan `prepare` 既有 POST 仍原契约。

**P1 拟新增、全任务统一定义（先定义再消费，不重定义 P0 类型）：**

```rust
// Task 1：只读有效目标和服务端 provider 已解析配置投影；
// target 使用与 P0 PUT 相同 preflight，provider 解析消费现有 provider_workspace_config。
pub struct AutomationTargetDto {
    pub logical_repository_id: String,
    pub resolved_options: EnrollmentOptions,
}
// GET /api/projects/{project_id}/issues/{issue_id}/automation-target?
//   author_provider=&reviewer_provider=&review_rounds=&superpowers_enabled=&
//   openspec_enabled=&include_integration_tests=&include_e2e_tests=&
//   force_frontend_backend_split=&require_execution_plan_confirm=

// Task 3：共用 prepare 数据面；此身份值对象放 product/models，避免 product -> web 依赖。
// manual `ids=None` 保持现有 sequential ID；bound `ids=Some` 不得覆盖既有 plan。
// src/product/models/automation.rs
pub struct PreparedPlanIds { pub plan_id: String, pub session_id: String }
// src/web/handlers/lifecycle/plan_preparation.rs
pub struct PreparedPlanRecords { pub plan: IssueWorkItemPlan, pub session: WorkspaceSessionRecord }
pub fn prepare_plan_records(state: &WebAppState, project_id: &str,
    issue_id: &str, request: PrepareWorkItemPlanRequest,
    ids: Option<PreparedPlanIds>) -> ApiResult<PreparedPlanRecords>;
// 此同步函数只做源/preflight/创建/已存在身份核验；由调用方在锁外执行
// ensure_workspace_context_message(...)。人工 REST 路径继续执行旧失败诊断、上下文、响应。

// Task 4：不可变创建意图写在该 issue 的 automation-plan-intent.json，
// 与 enrollment_id/prepare_intent_id 同源，记录冻结 source/options/target/plan/session；
// 先于 plan/session 持久化，在 enrollment 同一文件锁里执行。
pub struct PreparedPlanIntent {
    pub enrollment_id: String, pub prepare_intent_id: String,
    pub project_id: String, pub issue_id: String,
    pub source: EnrollmentSource, pub options: EnrollmentOptions,
    pub logical_repository_id: LogicalRepositoryId,
    pub plan_id: String, pub session_id: String,
}
// product store 只负责锁内 intent/bind；FnOnce 是 web/service 层注入的创建回调，
// 因而 product 层不引用 WebAppState、ApiResult 或 lifecycle handler。
pub fn ensure_plan_binding<F>(&self, project_id: &str, issue_id: &str,
    enrollment_id: &str, create: F) -> Result<IssueAutomationEnrollment, EnrollmentError>
where F: FnOnce(&IssueAutomationEnrollment, &PreparedPlanIntent) -> Result<(), EnrollmentError>;
// web 回调将 intent 转成 PrepareWorkItemPlanRequest，调用 prepare_plan_records(...,
// Some(PreparedPlanIds { plan_id: intent.plan_id.clone(), session_id: intent.session_id.clone() }))；
// bind_plan 原签名保留，锁内绑定分支提取私有 helper，避免嵌套锁。

// Task 5：P1 自动生成意图与真实 manager run 协调；不修改 P0 choice 门面。
pub struct PlanGenerationIntent {
    pub enrollment_id: String, pub plan_id: String,
    pub session_id: String, pub action_key: String,
    pub source: EnrollmentSource, pub options: EnrollmentOptions,
    pub logical_repository_id: LogicalRepositoryId,
    pub phase: PlanGenerationPhase,
}
pub enum PlanGenerationPhase { Claimed, EngineStarted, ProviderDispatched, NeedsHuman }
pub enum PlanGenerationOutcome { Running, AlreadyActive, WaitingForHuman, NeedsHuman }
pub async fn start_plan_generation_once(state: &WebAppState,
    enrollment: &IssueAutomationEnrollment) -> Result<PlanGenerationOutcome, String>;
// Task 6：有状态仅限于唤醒队列，动作/进度仍从上面 durable 事实推导。
pub struct AutopilotOrchestrator;
pub enum ReconcileOutcome { NoEnrollment, AwaitingHuman, Prepared, Generating, NeedsHuman }
pub async fn reconcile(&self, state: &WebAppState, project_id: &str,
    issue_id: &str) -> Result<ReconcileOutcome, String>;
// Task 8：只读确认完成事实，不保存通知表。
pub struct PlanConfirmedInfoDto {
    pub key: String, pub plan_id: String, pub session_id: String,
    pub occurred_at: String, pub title: String,
}
pub fn plan_confirmed_info(paths: &ProductAppPaths,
    enrollment: &IssueAutomationEnrollment)
    -> Result<Option<PlanConfirmedInfoDto>, ProductStoreError>;
```

P1 frontend 增加 `AutomationMode = "manual" | "automatic"`（默认 `manual`）、`getAutomationTarget`、`getAutomationEnrollment`、`putAutomationEnrollment` API helper；只在自动化分支写 P0 PUT，并**不**从浏览器调用普通 prepare。`selection_key` 必须是一次用户选择的稳定键：弹窗开启时生成并在本次提交重试间保存，不用每次点击都新生成；服务端仍以同键同 payload 幂等、异 payload Conflict 裁决。Task 1 的目标只读投影同时用 `provider_workspace_config` 解析成具体 provider/options，前端原样将 `resolved_options` 写入 P0 PUT，避免缺省 provider 再猜或假装 `fake`。`DesignSpec` DTO 没有 involved_repository_ids，也没有可直接从 issue `repo_id` 推出 logical target 的可靠字段；Task 1 新只读 target API 是必要投影，P0 PUT 继续独立校验精确源与 target。

**设计取舍：**推荐稳定意图+锁内唯一创建+manager 共用非 superseding 面（增加一份小的意图/检查点持久记录，但恢复可审计）；只用固定 id 然后调用 `create_issue_work_item_plan` 的方案会覆盖现有记录，拒绝。推荐 P1 info 作为现有 issue lifecycle 的可选只读投影、仅在当前 watched sessions 展示（重用观察刷新）；独立通知表是第二事实源，拒绝，K 外补读/TTL 按 tasks.md 明确留 P3。

## Task 1：2.1 自动化可用目标只读投影

**Files:** Create `src/web/handlers/automation_target.rs`；Modify `src/web/{handlers/mod.rs,app.rs}`；Test `src/web/handlers/automation_target.rs` 的 `#[cfg(test)]` 模块（沿用 `src/web/handlers/automation_enrollment.rs:401-627` 的单/零/多 target fixture 构造；若共用，搬到 `src/web/handlers/automation_enrollment_test_support.rs` 并在 Task 1 同步显式提交）。

**Interfaces:** Consumes: `RepositoryRouting::load_for_issue`、`logical_repository_ids_for_preflight`、`preflight_single_repository_candidate`、`IssueStore::get`、`provider_workspace_config`。Produces: `GET .../automation-target`（query 传用户 provider/options）→ `AutomationTargetDto` 含唯一 UUID 与服务端已解析 `EnrollmentOptions`；零/多、Legacy/FailClosed、provider 不可用、跨 issue 返回明确错误，不写 enrollment。

- [ ] **Step 1: 写失败测试。** 在新 handler 测试模块用 P0 `seed_fixture(member_count, true)` 同构 fixture（把共用 fixture 搬到现有测试支持模块而不改生产签名），测试真实 HTTP 行为：

  ```rust
  #[tokio::test]
  async fn automation_target_requires_exactly_one_logical_member() {
      let one = seed_fixture(1, true);
      let ok = get_automation_target(&one.router()).await;
      assert_eq!(ok.status(), StatusCode::OK);
      let body = response_json(ok).await;
      assert_eq!(body["logical_repository_id"], SINGLE_LOGICAL_ID);
      assert_eq!(body["resolved_options"]["author_provider"], "fake");
      for members in [0, 2] {
          let fixture = seed_fixture(members, true);
          let response = get_automation_target(&fixture.router()).await;
          assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
          assert!(!enrollment_file_exists(&fixture));
      }
  }
  ```

- [ ] **Step 2: 红灯。** `cargo test --locked --lib automation_target_requires_exactly_one_logical_member -- --nocapture`；预期 404/断言失败（路由尚未安装），而非凭源码字符串断言。
- [ ] **Step 3: 最小实现。** 只读 P0 PUT 使用的 routing+preflight：

  ```rust
  let routing = RepositoryRouting::load_for_issue(&paths, &project_id, &issue_id)
      .map_err(product_store_api_error)?;
  let RepositoryRouting::Logical { manifest, selection } = routing else {
      return Err(ApiError::validation("automation_enrollment_invalid_scope", "需要单一逻辑目标"));
  };
  let candidates = logical_repository_ids_for_preflight(&manifest, &selection);
  let SingleCandidatePreflightDecision::Eligible { repository_id } =
      preflight_single_repository_candidate(&candidates) else {
      return Err(ApiError::validation("automation_enrollment_invalid_scope", "必须恰一逻辑目标"));
  };
  Ok(Json(AutomationTargetDto {
      logical_repository_id: repository_id,
      resolved_options: resolve_enrollment_options_with_provider_workspace_config(&state, &query)?,
  }))
  ```

  函数进入时先 `IssueStore::get` 核验 issue；`query` 是 Task 1 新增的只读输入 DTO（与 `PrepareWorkItemPlanRequest` 的 provider/plan 选项同名同类型），`resolve_enrollment_options_with_provider_workspace_config` 也是本任务新增小函数，返回 `ApiResult<EnrollmentOptions>`，复用现有 `provider_workspace_config` 与 plan options 缺省；缺失/损坏按现有 `product_store_api_error`，不退化为零成员。禁止前端从 `repo_id` 推断 UUID 或猜默认 provider。
- [ ] **Step 4: 绿灯。** 同一过滤命令；预期单目标 200 与零/多 422，P0 `automation_enrollment_http_put_rejects_unconfirmed_or_stale_source` 原过滤命令也通过。
- [ ] **Step 5: 提交。** `git add src/web/handlers/automation_target.rs src/web/handlers/mod.rs src/web/app.rs && git commit -m "feat(wiga): expose validated single automation target"`；若抽共用 fixture，把 `src/web/handlers/automation_enrollment_test_support.rs` 和修改后的 `automation_enrollment.rs` 一并显式 `git add`。

## Task 2：2.1 Design 后模式选择与 P0 enrollment PUT

**Files:** Modify `web/src/components/lifecycle/{WorkItemPlanOptionsDialog.tsx,useIssueLifecycleGeneration.ts,IssueLifecycleWorkbench.generation.test.tsx}`、`web/src/api/{client.ts,types/lifecycle.ts,types.ts}`；Test `IssueLifecycleWorkbench.generation.test.tsx`。仅在 Design 卡已 confirmed 时显示自动化选择；不能改变 Story/Design 人工入口。

**Interfaces:** Consumes: Task 1 `GET automation-target` 的 `logical_repository_id/resolved_options`、P0 `GET/PUT automation-enrollment` 与 `EnrollmentWriteCommand::Enable` JSON（见 `src/web/handlers/automation_enrollment.rs:629-658`）、`StorySpec`/`DesignSpec.current_version`；Produces: `AutomationMode` 与前端 API helper；manual 分支保持 `prepareWorkItemPlan` 和 open-workspace，auto 分支只 PUT + refresh，后台随后认领。

- [ ] **Step 1: 写失败交互测试。** 在已有 test `prepares work item plan from design spec drawer and opens workspace` 保持 manual 断言，再补新用例（沿用 `lifecycleFetch()`；为 target/GET/PUT 扩展 fetch fixture 并保存 enrollment JSON，不能用 `vi.fn` echo 伪造后端判据）：

  ```tsx
  it("enrolls a confirmed design without manually preparing a plan", async () => {
    const fetchMock = lifecycleFetch({ automationTarget: SINGLE_LOGICAL_ID });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();
    render(<IssueLifecycleWorkbench />);
    await user.click(await screen.findByTestId("stage-tab-design"));
    await user.click(screen.getByRole("button", { name: "前端提示设计" }));
    await user.click(screen.getByRole("button", { name: "生成 Work Item" }));
    const dialog = await screen.findByRole("dialog", { name: "Work Item Plan 配置" });
    await user.click(within(dialog).getByRole("radio", { name: "自动化" }));
    await user.click(within(dialog).getByRole("button", { name: "启用自动化" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/automation-enrollment",
      expect.objectContaining({ method: "PUT" }),
    ));
    expect(fetchMock.mock.calls.some(([url]) => String(url).includes("work-item-plans:prepare"))).toBe(false);
  });
  ```

  再测未确认 Design 无自动 radio、重复点击同 selection_key+源+provider 返回同 enrollment/revision、异选项 409 留弹窗错误；既有 manual 用例原样通过。
- [ ] **Step 2: 红灯。** `pnpm -C web exec vitest run src/components/lifecycle/IssueLifecycleWorkbench.generation.test.tsx`；预期找不到「自动化」radio / PUT。
- [ ] **Step 3: 最小实现。** 扩 `WorkItemPlanOptionsFormValue.automation_mode`，`useState("manual")`，表单根据 mode 显示 `启用自动化`/既有 manual 按钮；`handleConfirmWorkItemPlanOptions` 在自动化分支先 GET 当前 enrollment revision、GET 唯一 target（后端最终以 P0 PUT 独立重验）、从同一 issue 的 `selectedColumns.story_spec` 取 Design 引用的已 confirmed Story 精确版本，提交完整 `Enable` 包体。`selection_key` 在弹窗实例中固定，失败重试不变；provider 默认必须经与后端同源的只读已解析配置投影或向 Task 1 的 read-only 响应增补已解析 provider 及审查回合/开关，前端无法解析则阻止自动提交，绝不猜默认。已存在 enabled enrollment 的相同选择重试传 GET 得到的现行 revision，P0 接受同键同 payload 返回原值；异 payload 让 P0 返回 409，UI 不自动 Disable/重开。服务端快照项显式 `run_policy=Interactive` 仅由后台 prepare 注入，PUT 不添加不存在的字段。
- [ ] **Step 4: 绿灯。** 同一 Vitest 文件；预期 manual 测试仍调用 prepare/open、auto 仅 PUT；provider 不可用/源未确认/零多 target 失败均可见且无自动 plan。选项 DTO/GET/PUT 实际类型与 Rust P0 JSON 对齐。
- [ ] **Step 5: 提交。** `git add web/src/components/lifecycle/WorkItemPlanOptionsDialog.tsx web/src/components/lifecycle/useIssueLifecycleGeneration.ts web/src/components/lifecycle/IssueLifecycleWorkbench.generation.test.tsx web/src/api/client.ts web/src/api/types/lifecycle.ts web/src/api/types.ts && git commit -m "feat(wiga): offer explicit design automation enrollment"`。

## Task 3：2.2 提取与人工同源的 prepare 数据面

**Files:** Create `src/web/handlers/lifecycle/plan_preparation.rs`；Modify `src/web/handlers/lifecycle.rs`（只移动 preflight/创建共用段并复用，保留人工 REST 契约）、`src/product/lifecycle_store/{plan.rs,workspace.rs}`（仅增加安全的 bound 创建/完整快照校验面，不改已有 public 签名）；Test `src/web/handlers/lifecycle_tests.inc.rs`。

**Interfaces:** Produces: 统一块 `PreparedPlanIds`、`PreparedPlanRecords`、`prepare_plan_records(state,project_id,issue_id,request,ids)`；manual `ids=None` 维持现有配置解析/单候选 preflight、失败诊断、上下文消息及响应；bound IDs 从 Task 4 传入并在创建前复核 frozen source/options/target。Consumes: `CreateIssueWorkItemPlanInput.id: Option<String>`、`LifecycleStore::create_workspace_session_with_id`、`ensure_workspace_context_message`（异步在创建临界区**之外**完成）、`provider_workspace_config`。

- [ ] **Step 1: 写失败测试。** 在 `src/web/handlers/lifecycle_tests.inc.rs` 的既有 `seed_prepare_work_item_plan_fixture(&paths, 1) -> (LifecycleStore,String,String)` 和 `WebAppState::new(root.path().to_path_buf(), WebRuntime::new_fake(...))` 上，使用返回的 Story/Design ID 构造 `PrepareWorkItemPlanRequest`。同稳定 ID 的两次 bound prepare，中间把第二次请求的 `include_e2e_tests` 修改后尝试同 ID 必须 IdentityMismatch，原请求再重试仍保留最初 plan 内容和创建时间；Task 4 单独模拟「plan 已写、session 未写」中窗。

  ```rust
  #[tokio::test]
  async fn prepare_bound_plan_reuses_identity_without_overwriting_content() {
      let root = TempDir::new().unwrap();
      let paths = ProductAppPaths::new(root.path().join(".aria"));
      let (lifecycle, story_id, design_id) = seed_prepare_work_item_plan_fixture(&paths, 1);
      let state = WebAppState::new(root.path().to_path_buf(),
          WebRuntime::new_fake(root.path().to_path_buf()));
      let request = plan_prepare_request(story_id, design_id);
      let ids = PreparedPlanIds {
          plan_id: "issue_work_item_plan_auto_fixture".into(),
          session_id: "workspace_session_auto_fixture".into(),
      };
      let first = prepare_plan_records(&state, PROJECT_ID, ISSUE_ID,
          request.clone(), Some(ids.clone())).unwrap();
      let mut changed = request.clone();
      changed.include_e2e_tests = Some(true);
      assert!(prepare_plan_records(&state, PROJECT_ID, ISSUE_ID,
          changed, Some(ids.clone())).is_err());
      let repeated = prepare_plan_records(&state, PROJECT_ID, ISSUE_ID,
          request, Some(ids)).unwrap();
      assert_eq!(repeated.plan.id, first.plan.id);
      assert_eq!(repeated.plan.created_at, first.plan.created_at);
      assert_eq!(lifecycle.list_issue_work_item_plans(PROJECT_ID, ISSUE_ID).unwrap().len(), 1);
  }
  ```

  本测试模块新增 `plan_prepare_request(story_id,design_id) -> PrepareWorkItemPlanRequest`：逐项填写现有 `src/web/types.rs:861-875` 的 `title`, `story_spec_ids`, `design_spec_ids`, `author_provider: Some("fake".into())`, `reviewer_provider: Some("fake".into())`, `review_rounds: Some(1)`, `superpowers_enabled: Some(false)`, `openspec_enabled: Some(false)`, `run_policy: Some(RunPolicy::Interactive)`, `include_integration_tests: Some(true)`, `include_e2e_tests: Some(false)`, `force_frontend_backend_split: Some(false)`, `require_execution_plan_confirm: Some(false)`；更深「已编译 plan 内容不得被 prepare 覆盖」用仓库已存在 `LifecycleStore::update_issue_work_item_plan` 单独断言。
- [ ] **Step 2: 红灯。** `cargo test --locked --lib prepare_bound_plan_reuses_identity_without_overwriting_content -- --nocapture`；预期新稳定创建接口尚未存在/重复覆盖触发失败。
- [ ] **Step 3: 最小实现。** `prepare_plan_records` 的 bound 分支在受 Task 4 enrollment 锁保护时**先查 plan 文件并核对 id/project/issue/source/options，不调用覆盖式 `create_issue_work_item_plan`**；session 既存时核对 id/project/issue/entity/type/provider/options/flow_kind/**Interactive**，不满足即 `IdentityMismatch`；只有不存在时用 `create_workspace_session_with_id` 创建。人工分支沿旧 `id:None` + `create_workspace_session`；原 `mark_single_candidate_prepare_failure`、上下文消息、响应投影语义不变，异步 `ensure_workspace_context_message` 必须放于 Task 4 锁外；不要把 `create_workspace_session_with_id` 当前只核对五个身份字段视为授权校验。
- [ ] **Step 4: 绿灯。** 同一过滤命令；补 `cargo test --locked --lib prepare_work_item_plan -- --nocapture` 验人工对照，任何现有默认 provider/rollout/preflight 行为变化都先修复。
- [ ] **Step 5: 提交。** `git add src/web/handlers/lifecycle/plan_preparation.rs src/web/handlers/lifecycle.rs src/product/lifecycle_store/plan.rs src/product/lifecycle_store/workspace.rs src/web/handlers/lifecycle_tests.inc.rs && git commit -m "refactor(wiga): share safe manual and bound plan preparation"`。

## Task 4：2.2 enrollment-bound 创建意图、唯一绑定与半提交恢复

**Files:** Modify `src/product/models/automation.rs`（**只增加** `PreparedPlanIntent`）、`src/product/issue_automation_store.rs`（新增方法/私有锁内 helper，P0 四个 public 签名不变）、`src/web/handlers/lifecycle/plan_preparation.rs`；Test `src/web/handlers/automation_enrollment.rs` 测试模块及 `src/product/issue_automation_store.rs` 测试模块。

**Interfaces:** Consumes Task 3 `prepare_plan_records(..., Some(ids))`，P0 enrollment GET/bind 语义；Produces `IssueAutomationStore::ensure_plan_binding`。稳定目标 ID 从**已持久的** `prepare_intent_id` 派生 `issue_work_item_plan_auto_{id}`/`workspace_session_auto_{id}`，无需靠扫描最近 plan。`PreparedPlanIntent` 先落盘，plan→session→绑定持久成功后才处理上下文异步准备。

- [ ] **Step 1: 写失败测试。** 在 P0 `seed_fixture(1,true)` 上 PUT、用 P1 存储面模拟中窗；重建 store/进程后两次补偿必须绑定同一 plan/session，plan 文件不得被覆盖。再执行 Disable→不同 source/options Enable 的正确 revision 重开，原 intent 与新快照不匹配则 fail-closed：

  ```rust
  #[tokio::test]
  async fn automation_prepare_recovers_plan_without_session_or_rebinding() {
      let fixture = seed_fixture(1, true);
      let enrolled = response_json(put_enrollment(&fixture.router(),
          enrollment_body(&fixture, 1, 1)).await).await;
      let intent_id = enrolled["prepare_intent_id"].as_str().unwrap();
      let expected_plan_id = format!("issue_work_item_plan_auto_{intent_id}");
      fixture.seed_intent_and_plan_without_session(&expected_plan_id);
      let recovered = fixture.ensure_enrolled_plan().await.unwrap();
      let again = fixture.ensure_enrolled_plan().await.unwrap();
      assert_eq!(recovered.plan_id, again.plan_id);
      assert_eq!(recovered.session_id, again.session_id);
      assert_eq!(recovered.plan_id.as_deref(), Some(expected_plan_id.as_str()));
      assert_eq!(fixture.plans().len(), 1);
      assert_eq!(fixture.sessions().len(), 1);
  }
  ```

  `seed_intent_and_plan_without_session` 用实际新持久意图+共用 prepare 的计划写入步骤模拟崩溃，不伪造绑定；另加两个真实线程/实例竞争同一 enrollment 的断言与异常 plan 文件内容不变。测试 fixture helper 在此任务测试模块提供具体文件落盘，不把伪 fixture 放生产。
- [ ] **Step 2: 红灯。** `cargo test --locked --lib automation_prepare_recovers_plan_without_session_or_rebinding -- --nocapture`；预期补偿无法恢复或重复创建。
- [ ] **Step 3: 最小实现。** 在 `automation-enrollment.json` 的同一 `with_exclusive_lock` 临界区重读 current，检查 enabled/enrollment_id、源版本仍 confirmed、target 恰一；查/写 `automation-plan-intent.json` 完整冻结快照（若已有值不一致返回 Conflict，严禁覆盖）；受同锁保护顺序核对/创建 plan、核对/创建 session、锁内绑定（提取 P0 `bind_plan` 的私有锁内分支，维持原 public 签名及 revision+1）。plan 已建但 session/绑定缺失补齐相同目标，损坏/读取失败原样报错。锁外 `ensure_workspace_context_message`；provider 配置绝不在第二次补偿时重新取环境默认。绑定后使用**新的** `policy_revision` 作后续动作观察，不把首次 Enable revision 固定为生成 revision。
- [ ] **Step 4: 绿灯。** 同一过滤命令，再运行 `cargo test --locked --lib issue_automation_store -- --nocapture` 与 `cargo test --locked --lib automation_enrollment_http -- --nocapture`；确认 enabled 异 payload 冲突、disabled 重开、bind +1 的 P0 测试仍过。
- [ ] **Step 5: 提交。** `git add src/product/models/automation.rs src/product/issue_automation_store.rs src/web/handlers/lifecycle/plan_preparation.rs src/web/handlers/automation_enrollment.rs && git commit -m "feat(wiga): recover one enrollment-bound plan and session"`。

## Task 5：2.2 生成动作检查点与 manager 非 superseding 共用面

**Files:** Modify `src/product/models/automation.rs`（追加 `PlanGenerationIntent/Phase`）、`src/product/issue_automation_store.rs`（持久检查点）、`src/web/workspace_session/manager/{runs.rs,durable_projection.rs}`、`src/web/workspace_ws_handler/{decisions/inbound.rs,run/provider_run.rs}`；Create `src/web/plan_generation.rs`；Modify `src/web/mod.rs`；Test `src/web/workspace_session/tests/part_07.rs`、`src/web/plan_generation.rs` 内部测试。

**Interfaces:** Consumes Task 4 已绑定 enrollment、`WorkspaceSessionRegistry::get_or_create`、`WorkspaceEngine::start_generation(ProviderConfigSnapshot,bool)`、manager 唯一 run 与 SC `reserve_single_candidate_provider_start` ledger；Produces `start_plan_generation_once` 和在同一 manager 临界区登记的非 superseding 认领。manual WS 路径经共用启动服务，但**仅手工**仍保留现有显式 supersede 语义，auto 有活 run 返回 `AlreadyActive` 不调用 `abort_active_run_from_attachment`/`start_run_from_attachment`。真实 provider drive 仍仅在 manager/run 层。

- [ ] **Step 1: 写失败测试。** 复用 `src/web/workspace_session/tests/part_07.rs::claim_manager` 和 `started_claim_run`（测试模块通过 `use super::{claim_manager, started_claim_run};` 引入），并在该测试模块新增 WebAppState fixture：由 claim_manager 所建 manager 注册进 `WebAppState`（复用 P0 `automation_enrollment` HTTP 测试的 state 构建模式，含绑定 session_auto_live 与已绑定 enrollment_1/plan_1）。断言自动认领无法替换该 run：

  ```rust
  #[tokio::test]
  async fn automation_generation_does_not_supersede_live_run() {
      let manager = claim_manager("session_auto_live");
      let (token, run_incarnation) = started_claim_run(&manager).await;
      let state = state_with_bound_enrollment(&manager, "enrollment_1", "plan_1");
      let outcome = start_plan_generation_once(&state, &bound_enrollment("enrollment_1", "plan_1")).await.unwrap();
      assert!(matches!(outcome, PlanGenerationOutcome::AlreadyActive));
      let active = manager.active_run_ref_for_test().unwrap();
      assert_eq!(active.token, token);
      assert_eq!(active.run_incarnation, run_incarnation);
  }
  ```

  `start_plan_generation_once` 与 `PlanGenerationOutcome::AlreadyActive` 是统一接口块中 P1 新定义的服务层面（签名固定为 `(&WebAppState, &IssueAutomationEnrollment)`）；`state_with_bound_enrollment`/`bound_enrollment` 为本测试模块新增 fixture helper；`active_run_ref_for_test` 为 manager 新增 `#[cfg(test)]` test-only 访问器（返回可比对 token/run_incarnation 的快照，见 Step 3）——**须在本任务 Files 与 Step 3 显式落地，禁止放入生产接口**。测试只通过该面，不再调用不存在的 `claim_automation_generation_run`/`AutomationRunClaim`。

- [ ] **Step 2: 红灯。** `cargo test --locked --lib automation_generation_does_not_supersede_live_run -- --nocapture`；预期缺共用非 supersede 原语。
- [ ] **Step 3: 最小实现。** 在 enrollment 文件锁下认领稳定键 `enrollment_id:plan_id:start_generation`，冻结 source/options/target、session id；engine 准入仅在 durable Prepare、Interactive 且无人工/失败状态时执行。`Claimed`（尚无 node/ledger）可安全恢复；`EngineStarted` 后 manager `create` 的 `recover_on_creation` **先**查/恢复既有 outline/ledger，不可裸调用 `start_generation` 造第二 node；检查点与 SC ledger 在重建时不能证明未触达外部 provider 则持久 `NeedsHuman`，只供人显式 recovery。`ProviderDispatched` + 现存活 run 只观察，不重复 spawn；`finish_run` 保持原 provider followup。不在 engine 长锁下认领；manager 短锁决定是否已有 run，auto 禁止调用会替换活 run 的原入口。**注意**现有 `reserve_single_candidate_provider_start` ledger 不含外部回执，不能单独当作安全重发证明。manual WS 复用共用 provider drive 实现而不改变其明确重跑授权/响应帧。
- [ ] **Step 4: 绿灯。** 同一过滤命令；`cargo test --locked --lib start_generation_locks_provider_and_creates_node -- --nocapture`、`cargo test --locked --lib workspace_choice_claim_run_finish_expires_old_commands -- --nocapture` 通过；新增中窗测试覆盖 manager zombie recovery 和 SC ledger 中断，不能只测空 run。
- [ ] **Step 5: 提交。** `git add src/product/models/automation.rs src/product/issue_automation_store.rs src/web/workspace_session/manager/runs.rs src/web/workspace_session/manager/durable_projection.rs src/web/workspace_ws_handler/decisions/inbound.rs src/web/workspace_ws_handler/run/provider_run.rs src/web/plan_generation.rs src/web/mod.rs src/web/workspace_session/tests/part_07.rs && git commit -m "feat(wiga): claim plan generation without superseding runs"`。

## Task 6：2.2 薄编排器、启动扫描与有界漏唤醒补偿

**Files:** Create `src/web/autopilot_orchestrator.rs`；Modify `src/web/{mod.rs,app.rs,state.rs}`、`src/web/handlers/automation_enrollment.rs`（成功 PUT 仅唤醒，不绑定运行生命周期）；Test `src/web/autopilot_orchestrator.rs` 内部 `#[cfg(test)]`。重启扫描用已存在 `ProjectStore::list` → `IssueStore::list(project_id)`，不扫旧 plan。

**Interfaces:** Consumes Task 4 `ensure_plan_binding`、Task 5 `start_plan_generation_once`，P0 `IssueAutomationStore::get`、`ProjectStore::list`、`IssueStore::list`；Produces `AutopilotOrchestrator::reconcile` 和由 `serve_web` 持有、随服务器生命周期关闭的后台扫描/补偿 task。事件只是 wake hint，不当权威 durable cursor；`build_web_router` 的 fake fixture 不因单次路由构造偷偷跑全局扫描。

- [ ] **Step 1: 写失败测试。** 建 fixture：两个已授权且同 issue 重复 wake，另有未授权手工 issue；关闭 browser/WS，不送事件并使用 `tokio::time::pause/advance`（若未启用 test-util 则用注入一次 tick 的纯 `reconcile_all_once`）模拟漏唤醒；计数事实只来自 durable plan/session，不测 callback 次数：

  ```rust
  #[tokio::test]
  async fn automation_reconcile_scans_only_enrolled_issues_and_reuses_binding() {
      let fixture = seeded_orchestrator_fixture().await;
      let worker = AutopilotOrchestrator::new(fixture.state.clone(),
          OrchestratorConfig { max_issues_per_tick: 32, ..Default::default() });
      worker.reconcile_all_once().await.unwrap();
      worker.reconcile_all_once().await.unwrap();
      let bound = fixture.automation_store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
      assert!(bound.plan_id.is_some());
      assert!(bound.session_id.is_some());
      assert_eq!(fixture.bound_plans().len(), 1);
      assert!(fixture.manual_issue_plans().is_empty());
  }
  ```

  `seeded_orchestrator_fixture()` 在本任务 `#[cfg(test)]` 模块中完整构造 `TempDir`、`ProductAppPaths`、`WebAppState`、一个 enabled enrollment 与一个未授权 issue，并提供上述观测方法；helper 必须写入真实 JSON/store 文件，不调用未定义的生产 API。`reconcile_all_once` 仅由 `AutopilotOrchestrator` 公开，接收自身 state，不再伪造路由回调。

  再测 Disable 先于动作认领零启动；Disable→不同 source/options 重新 Enable 且旧绑定存在时拒绝自动生成但不 Abort 在途 run；同 payload 重新 Enable 只按同冻结身份恢复。
- [ ] **Step 2: 红灯。** `cargo test --locked --lib automation_reconcile_scans_only_enrolled_issues_and_reuses_binding -- --nocapture`；预期无后台扫描/动作。
- [ ] **Step 3: 最小实现。** `reconcile(project,issue)` 每次读 fresh enrollment，缺失/disabled → `NoEnrollment`；当前 target/confirmed source/immutable intent 与绑定任一不匹配 → `NeedsHuman`，不得猜最近 plan。无绑定请求 `EnsurePreparedPlan`，有绑定只对 Prepare/未生成 session 请求 `StartPlanGeneration`；WaitingForHuman/choice/compile recovery/Confirmed/Failed/Terminated 不推进。`serve_web` 成功构建 state 后启动扫描；bounded tick（明确固定间隔如 2s、每轮最多 32 issue、公平游标在一轮内前进，满轮后重扫）覆盖丢事件；`EventHub` 可发唤醒但不给顺序承诺。锁内认领时和真正发起前再核当前 enrollment，Disable 后已认领 action 可以继续但未认领不准开始。后台错误留下可见 durable 检查点/诊断并继续扫描其他 issue，不吞掉读取错误为 off。
- [ ] **Step 4: 绿灯。** 同一过滤命令；再跑本模块 `cargo test --locked --lib autopilot_orchestrator -- --nocapture`，断言重复、漏唤醒、关闭/重开、换源、损坏 enrollment、停止门不出命令。
- [ ] **Step 5: 提交。** `git add src/web/autopilot_orchestrator.rs src/web/mod.rs src/web/app.rs src/web/state.rs src/web/handlers/automation_enrollment.rs && git commit -m "feat(wiga): reconcile enrolled plans on startup and bounded ticks"`。

## Task 7：2.2 无 driver choice/人工门/compile recovery 的停等人对照

**Files:** Modify `src/web/autopilot_orchestrator.rs`（只加判定及测试）、`src/web/handlers/{workspace_choice.rs,workspace_human_action.rs}`（必要时只补 P1 集成回归测试，不新建命令接口）、`web/src/pages/ChatCockpitPage.inbox.test.tsx`（验证驾驶舱原入口）；Test 同文件测试模块。P0 choice manager 面、REST `HumanActionRequest`、WS observer 只读均保持。

**Interfaces:** Consumes: P0 `ChoiceResponseRequest`、`ChoiceReplyState`、`HumanActionRequest::{Approve,Abandon,Feedback,CompileRecovery}`、`selectGateProjection`；Produces: P1 `ReconcileOutcome::AwaitingHuman` 在 pending choice、WaitingForHuman、compile `Failed/RecoveryRequired`、human gate turn busy 时不发下一动作，用户继续通过 P0 REST 手动解除。

- [ ] **Step 1: 写失败测试。** `enrolled_gate_fixture()` 必须在本任务测试模块中实现并返回真实 `TempDir`/`WebAppState`/`WorkspaceSessionManager`/HTTP router；沿用 P0 `workspace_choice.rs` 与 `workspace_human_action.rs` 的 engine fake 构造，仅替换必要的 enrollment/plan fixture，不把以下 helper 留在生产接口：

  ```rust
  #[tokio::test]
  async fn automation_reconcile_waits_for_choice_and_failed_compile() {
      let fixture = enrolled_gate_fixture().await;
      fixture.open_choice_with_two_questions().await;
      let before = fixture.provider_start_ledger().len();
      assert!(matches!(fixture.reconcile().await.unwrap(), ReconcileOutcome::AwaitingHuman));
      assert_eq!(fixture.provider_start_ledger().len(), before);
      fixture.human_answer_via_rest().await;
      fixture.fail_compile_after_human_approve().await;
      assert!(matches!(fixture.reconcile().await.unwrap(), ReconcileOutcome::AwaitingHuman));
      assert_eq!(fixture.provider_start_ledger().len(), before);
  }
  ```

  `open_choice_with_two_questions` 要实际注册两个 `WsOutMessage::ChoiceRequest`，`human_answer_via_rest` 逐个以 `ChoiceResponseRequest` 调用既有 REST 并等待 `Delivered`；`fail_compile_after_human_approve` 使用现有 finalizer failpoint。加 `Abandon` 后终态无额外 run，observer WS 写入仍拒。
 - [ ] **Step 2: 红灯。** `cargo test --locked --lib automation_reconcile_waits_for_choice_and_failed_compile -- --nocapture`；预期当前编排器未识别停点。
- [ ] **Step 3: 最小实现。** 编排器读取 durable session status/phase、pending choice 和最近 compile transaction status，把 `WaitingForHuman`、未交付 choice、`Failed/RecoveryRequired` 都作为停点，不自动调用 P0 REST；人工操作后只通过唤醒/补偿重读事实。P0 `workspace_human_action.rs:24-175` 已复用 manager/engine，禁止在 P1 衍生另一个 REST 或绕过 expected_gate_id。测试覆盖 gate Feedback/Approve/Abandon 和 recovery 的人手操作，不把 REST `200 Accepted` 当 Confirmed。
- [ ] **Step 4: 绿灯。** 同一过滤命令；`cargo test --locked --lib conversational_gate_approve_fails_closed_at_compile_finalizer_failpoint -- --nocapture` 和 `pnpm -C web exec vitest run src/pages/ChatCockpitPage.inbox.test.tsx` 对照。
- [ ] **Step 5: 提交。** `git add src/web/autopilot_orchestrator.rs src/web/handlers/workspace_choice.rs src/web/handlers/workspace_human_action.rs web/src/pages/ChatCockpitPage.inbox.test.tsx && git commit -m "test(wiga): keep automated plans waiting at human decisions"`。

## Task 8：2.3 成功 publication/compile + durable Confirmed 只读信息

**Files:** Create `src/web/plan_confirmed_info.rs`；Modify `src/web/{mod.rs,handlers/lifecycle.rs,types.rs}`；Test `src/web/plan_confirmed_info.rs` 的内部单元测试和 `src/web/handlers/lifecycle_tests.inc.rs` HTTP 投影用例。

**Interfaces:** Consumes: P0 `WorkItemPlanStore::list_compile_transactions(project_id,issue_id,plan_id)`、`WorkItemPlanSourceStore::get_publication_provenance(&scope,ref)`、`LifecycleStore::get_workspace_session(session_id)`、`IssueAutomationStore::get`；Produces: `plan_confirmed_info(paths,enrollment) -> Result<Option<PlanConfirmedInfoDto>,ProductStoreError>`，以及 `IssueLifecycleResponse.plan_confirmed_info: Vec<PlanConfirmedInfoDto>` additive 字段。事实 key `plan_confirmed:{plan_id}:{compile_id}`，时间来自成功事务 `committed_at`，不取会变的 `IssueWorkItemPlan.updated_at`。

- [ ] **Step 1: 写失败测试。** `compiled_plan_fixture()` 必须在本任务测试模块完整实现，并复用 `src/product/workspace_engine/tests/conversational_gate_close.rs::approval_fixture` 的真实 engine/store 构造：成功变体确实写入 publication provenance、compile transaction、Confirmed session；失败变体使用既有 finalizer failpoint 生成 Failed/RecoveryRequired，而非造 DTO。

  ```rust
  #[tokio::test]
  async fn plan_confirmed_info_requires_published_compile_and_confirmed_session() {
      let fixture = compiled_plan_fixture().await;
      let enrollment = fixture.bound_enrollment();
      assert!(plan_confirmed_info(&fixture.paths, &enrollment).unwrap().is_none());
      fixture.approve_and_persist_failed_compile().await;
      assert!(plan_confirmed_info(&fixture.paths, &enrollment).unwrap().is_none());
      fixture.recover_and_commit_compile().await;
      let first = plan_confirmed_info(&fixture.paths, &enrollment).unwrap().unwrap();
      let second = plan_confirmed_info(&fixture.paths, &enrollment).unwrap().unwrap();
      assert_eq!(first.key, second.key);
      assert_eq!(first.occurred_at, second.occurred_at);
      assert!(first.key.contains(&first.plan_id));
  }
  ```

  fixture 的 `bound_enrollment()` 返回真实 `IssueAutomationEnrollment`；`approve_and_persist_failed_compile`/`recover_and_commit_compile` 只封装已读到的 engine action 和持久写步骤，失败/无 provenance/session 未 Confirmed/普通非绑定 plan 均必须返回 None 或显式读取错误。
- [ ] **Step 2: 红灯。** `cargo test --locked --lib plan_confirmed_info_requires_published_compile_and_confirmed_session -- --nocapture`；预期无投影/失败判据。
- [ ] **Step 3: 最小实现。** 从 enrollment 精确绑定的 session/plan 读取，核对同 project/issue、`run_policy=Interactive`、`status=Confirmed`、`phase=Completed`、plan status Confirmed，且 tx `status=Committed && plan_commit_state=Committed && step_cursor="committed" && committed_at.is_some()`；tx 的 scope/compile_id、`publication_provenance_ref` 与 session 的 ref 一致，调用 source store 加载并验证 provenance hash/ref。多成功事务时只取与 session `compile_reservation.compile_id` 匹配的一条；引用或文件不可读不得报告成功。P0 `IssueWorkItemPlan` 无 `confirmed_at`，这里不新增第二确认时间，稳定 `occurred_at` 用 `tx.committed_at`。在 `issue_lifecycle` additive 投影仅从当前 enabled 精确绑定 enrollment 派生；禁用后历史 info 可按 P3 近期事实投影设计再扩，本任务不把禁用当假成功。
- [ ] **Step 4: 绿灯。** 同一过滤命令；`cargo test --locked --lib conversational_gate_approve_confirms_only_after_durable_compile -- --nocapture`，再跑新增 lifecycle HTTP 投影测试，确保重复 GET 同 key/时间。
- [ ] **Step 5: 提交。** `git add src/web/plan_confirmed_info.rs src/web/mod.rs src/web/handlers/lifecycle.rs src/web/types.rs src/web/handlers/lifecycle_tests.inc.rs && git commit -m "feat(wiga): derive plan confirmation from durable publication"`。

## Task 9：2.3 驾驶舱 info 与待处理计数/批量隔离

**Files:** Modify `web/src/api/types/lifecycle.ts`、`web/src/hooks/useWorkspaceSessionObservers.ts`、`web/src/components/cockpit/CockpitShell.tsx`、`web/src/components/chat-workspace/cockpit/CockpitInbox.tsx`、`web/src/pages/ChatCockpitPage.tsx`；Test `web/src/{hooks/useWorkspaceSessionObservers.test.tsx,components/cockpit/CockpitShell.test.tsx,pages/ChatCockpitPage.inbox.test.tsx}`（按已有文件若命名不同，使用现有 CockpitShell/Inbox 测试文件，不新建源码检查测试）。

**Interfaces:** Consumes Task 8 `IssueLifecycleResponse.plan_confirmed_info`，P0 observer `inbox/countedInbox`。Produces: 只读 `PlanConfirmedInfoItem` 与单独 `infoItems` 显示；`countedInbox`/actionable count 的 gate/stopped/error 集合不并入 info；首次获取当前 watched sessions 的既存 info 仅显示不弹提示，之后同页新事实 key 仅提示一次；跨刷新/key TTL/K 外历史不由 P1 保证。

- [ ] **Step 1: 写失败交互测试。** helper 对齐现状：`ChatCockpitPage.inbox.test.tsx` 使用 `ChatCockpitPage.test-utils` 的 helper（不存在 `lifecycleFetch`——该 helper 实际定义于 `IssueLifecycleWorkbench.test-utils.ts`，不引用）；`CockpitShell.test.tsx` 的 `renderShell` 现签名为 `renderShell({ inbox, onGoToInbox })`（CockpitShell.test.tsx:72）。因此 Step 1 先在 `CockpitShell.test.tsx` 测试支持区新增 typed fixture：`infoItem(key, planId)` 产出 `PlanConfirmedInfoItem`，`renderShell` 的 `inbox` 数组扩展接受 info 项。断言 info 分区可读、顶部和 favicon 的待处理数只数 gate、批量选择只有 gate：

  ```tsx
  it("shows plan confirmation without increasing actionable count", async () => {
    const info = infoItem("plan_confirmed:plan_1:compile_1", "plan_1");
    renderShell({ inbox: [info as never], onGoToInbox: () => {} });
    expect(await screen.findByText("Work Item Plan 已确认")).toBeInTheDocument();
    expect(screen.queryByText(/待处理 1 项/)).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /批量确认 1 项/ })).not.toBeInTheDocument();
  });
  ```

  `infoItem`/`renderShell` 的扩展参数必须按现有测试文件实际签名落地（在测试支持区补完整 typed fixture，禁止引用未定义的 `cockpitPageProps` 或 `lifecycleFetch`）；同一 key 重复 GET 不重复 toast，新 compile key 仅提示一次；gate/recovery 卡不被 info 遮挡。


  fixture 中 `session_1` 必须是实际 `watchedSessionIds` 内 enrollment-bound session；同一 key 重复 GET 后不重复 toast，再加入新 compile key 时仅新事实提醒；gate/recovery 卡不被 info 遮挡。UI 测试不能把无界历史塞进 watched 测试伪充 P3。
- [ ] **Step 2: 红灯。** `pnpm -C web exec vitest run src/pages/ChatCockpitPage.inbox.test.tsx`；预期无 info 文案/只读分区。
- [ ] **Step 3: 最小实现。** `useWorkspaceSessionObservers` 在既有 issue lifecycle refresh 中采集 `plan_confirmed_info`，只保留当前 watched session 的 info，按 durable `key` 去重；`CockpitShell` 分离 display info 与 `countedInbox`，计数/角标/危险操作仍只用旧 actionable。`CockpitInbox` 增独立「进度信息」只读 section（原「需要人工处理」不混入），带 plan session 下钻，不把 info 加入 `isSelectableGate`、`onBulkConfirm`。同页首次 hydration 将已有 key 初始化为已知，随后新 key 提示一次；错误/无对应 watched session 不造一条假 info；文案只称「Work Item Plan 已确认」，不称 coding 已完成或全交付。
- [ ] **Step 4: 绿灯。** 同一测试文件、`pnpm -C web exec vitest run src/components/cockpit/CockpitShell.test.tsx` 与 `pnpm -C web exec vitest run src/hooks/useWorkspaceSessionObservers.test.tsx`；需视觉实际打开浏览器核对 info/门同屏与计数隔离，不能仅依赖 DOM mock。
- [ ] **Step 5: 提交。** `git add web/src/api/types/lifecycle.ts web/src/hooks/useWorkspaceSessionObservers.ts web/src/components/cockpit/CockpitShell.tsx web/src/components/chat-workspace/cockpit/CockpitInbox.tsx web/src/pages/ChatCockpitPage.tsx web/src/hooks/useWorkspaceSessionObservers.test.tsx web/src/components/cockpit/CockpitShell.test.tsx web/src/pages/ChatCockpitPage.inbox.test.tsx && git commit -m "feat(wiga): show confirmed plan info outside actionable inbox"`。

## Task 10：2.4 P1 双清单关闸与文档实证

**Files:** Modify `src/web/autopilot_orchestrator.rs`、`src/web/plan_generation.rs`、`src/web/handlers/automation_enrollment.rs`、`src/web/plan_confirmed_info.rs` 的测试模块及 `web/src/components/lifecycle/IssueLifecycleWorkbench.generation.test.tsx`、`web/src/pages/ChatCockpitPage.inbox.test.tsx`；**两份证据满足后才** Modify `openspec/changes/work-item-group-autopilot/tasks.md:12-17` 勾 P1；同期维护用户可见项目既有说明（找到现有对应 README，不另造文档）。本任务计划制定时不改 OpenSpec。

**Interfaces:** Consumes Tasks 1–9 全部 P1 路径 + P0 human/choice REST；Produces 可审查的替身结果与真实 provider 关页/重启记录，不向 §3/§4 出口写空实现。若 aria 未部署或 provider 不可用，真实链明确**未关闸**，不得勾 `2.4` 或宣称 P1 通过。

- [ ] **Step 1: 写失败全路径测试。** 在新编排器现有真实 store/router fake fixture 中注入 `AfterIntentSaved`、`AfterPlanSaved`、`AfterSessionSaved`、`AfterEngineStarted` 可控一次性中断，重启/补偿后检查同一事实及不能证明的 provider 分诊：

  ```rust
  #[tokio::test]
  async fn automation_p1_crash_windows_keep_one_plan_and_never_reissue_provider() {
      for window in [CrashWindow::AfterIntentSaved, CrashWindow::AfterPlanSaved,
                     CrashWindow::AfterSessionSaved, CrashWindow::AfterEngineStarted] {
          let fixture = seeded_orchestrator_fixture().await;
          fixture.interrupt_once(window).await;
          fixture.restart_and_reconcile().await;
          assert_eq!(fixture.bound_plans().len(), 1);
          assert_eq!(fixture.sessions_for_bound_plan().len(), 1);
          assert!(fixture.provider_start_count() <= 1);
          assert_eq!(fixture.automation_session().run_policy, RunPolicy::Interactive);
      }
  }
  ```

  人工门/compile Failed 时另断言无 info；恢复成功后 info key/occurred_at 相同；新增未选/manual 和多 target 的对照。所有中窗测试须实际中断持久写/manager 构造，不以 `mockReturnValueOnce` 假冒磁盘重启。
- [ ] **Step 2: 红灯。** `cargo test --locked --lib automation_p1_crash_windows_keep_one_plan_and_never_reissue_provider -- --nocapture`；预期尚有至少一个真实中窗断言失败；若此前 Tasks 4–6 已完全覆盖，保留新增不同窗口的边界断言，不制造假红灯。
- [ ] **Step 3: 最小实现与真实链操作。** 修复只在 P1 共用创建/检查点/扫描/投影对应文件，不改 P0 已落地签名；启动 aria 服务并实际部署当前 P1 二进制及 web 资源，确认 GET `/api/health` 与 provider 可用。真实操作者先人工生成/确认 Story、Design，再选自动化；关 plan workspace 与 driver，观察唯一绑定 plan/session、唯一 plan provider run、choice 在驾驶舱多问题人工回复、计划门手动 Feedback/Approve/Abandon 分支及一次 compile fail→recovery 人工处理；重启服务后按 durable 事实恢复，成功 Confirmed 且仅一条 info，角标/批量计数不含 info。原手工 issue、零/多 target 对照完整跑一遍；不测试 P2 coding 自动首启。
- [ ] **Step 4: 绿灯及双清单。** 定向 Rust `cargo test --locked --lib automation_p1 -- --nocapture`、`cargo test --locked --lib plan_confirmed_info -- --nocapture`、定向 Vitest 上述两个文件；**最后由集成负责人**统一全量检查。分别记录下表每行真实命令、时间、输出路径/截图/事实 ID 与成败；替身与真实链不得互填。真链无服务部署则保留「未执行：aria 未运行」，切勿把 fake provider 结果抄过去。
- [ ] **Step 5: 提交。** 仅当测试及真实链关闸通过、OpenSpec §2.1–2.4 证据齐备后：`git add src/web/autopilot_orchestrator.rs src/web/plan_generation.rs src/web/handlers/automation_enrollment.rs src/web/plan_confirmed_info.rs web/src/components/lifecycle/IssueLifecycleWorkbench.generation.test.tsx web/src/pages/ChatCockpitPage.inbox.test.tsx openspec/changes/work-item-group-autopilot/tasks.md && git commit -m "test(wiga): close P1 plan-chain replacement and real-chain gates"`。若既有用户文档确实改动，同时显式加其**实际路径**，不使用 `git add .`；真实链未过不勾任务，不提交误导关闸宣称。

## P1 两份关闸清单（执行时逐项记录事实）

| 替身/自动化证据 | Task | 期望可观察结果 |
|---|---:|---|
| Design 未确认/默认 manual、同键重复与异 payload、单/零/多 target、原手动路径 | 1/2/4/10 | 未授权零动作、同 enrollment/revision、冲突 409/422、manual 仍可生成 |
| 准备 intent/plan/session/bind 四中窗、并发 worker、关闭/重开、计划内容不被覆盖 | 3/4/6/10 | 恰一 plan/session；新 payload 与旧绑定不匹配 fail-closed |
| 漏事件/无 WS/manager 回收/活 run/ledger 已开始的不确定副作用 | 5/6/10 | 继续原动作或明确人工分诊，不 supersede、不造第二 provider |
| choice 多问题、门反馈/approve/abandon、compile fail/recovery/不自动批准 | 7/10 | P0 REST 人工可操作，编排器保持等待，observer 写入仍拒 |
| publication/compile 真成功+Confirmed、Failed/recovery/无 provenance、重复刷新 | 8/9/10 | 一个稳定 info key/occurred_at、待处理计数与批量行为不变 |

| 真实 provider/人工链证据 | Task | 必须实际观察 |
|---|---:|---|
| 部署 P1 aria、浏览器实际模式选择、人工 Story/Design 确认与手工对照 | 2/10 | 前后端资源同版，真实 provider 可用；旧/manual 不被后台接管 |
| 选单 target auto→关闭 plan workspace/driver→重启→重看 durable 文件与会话 | 4/5/6/10 | 一次绑定、同一 plan/session、生成续接且无页面驱动 |
| 人工 choice、计划门反馈/批准或放弃、compile 失败及人工 recovery | 7/10 | 无 driver 驾驶舱手动解除；停点期间未自动批准/重试 |
| compile/publication 最终成功、plan durable Confirmed 与信息展示 | 8/9/10 | 同一事实仅一次提醒；info 不增待处理、不提供批量动作 |

**当前依赖状态：**撰写时服务器 aria **未运行**，故真实链未关闸。P0 `/human-actions` 路由及 handler 已落地（`src/web/app.rs:363-366`、`src/web/handlers/workspace_human_action.rs:24-175`），P1 复用并在真实链重验；若实施中发现真实服务端当前版本未部署 P0 面，先部署/核实，不允许 P1 发明替代门面。线 B amendment Unsent/补投递机制已落地但本计划不触碰（P2 才依赖）。

## 自审与追溯

- §2.1 / REQ-WIGA-01/02、REQ-MTG-03 → Task 1–2、4、10；§2.2 / REQ-WIGA-03、REQ-CG-04 → Task 3–7、10；§2.3 / REQ-WIGA-07 → Task 8–9、10；§2.4 → Task 10 双清单。REQ-WIGA-02 中 coding attempt 物化属于 §3 P2，本 P1 只保持 Interactive、授权撤销与手工零回归，不假装已实现 AutoStartOnce。
- 全部五条 Review Focus 有对应失败前/通过后**行为测试**；任务每个五步、指明 Files/Interfaces/显式提交；所有共用符号在统一接口块或 P0 已有文件定义；P0 已落地 public 函数签名只消费，不追写原 P0 plan 的拟议替身。
- P1 不规划 AdvancePlan、StartCodingOnce、amendment 改造、coding 完成信息、K 窗口外历史/TTL/跨刷新提醒。真链待部署，计划尚未执行任何测试/提交，关闸状态不能写成通过。
