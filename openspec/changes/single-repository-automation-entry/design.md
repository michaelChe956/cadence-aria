# Design

## Context

本设计承接 `proposal.md`，把方案 v1.2（`cadence/designs/2026-09-28_方案设计_真实链缺口修复总方案_v1.2.md`）§4.1 的 GAP-I／GAP-J／单仓自动化入口行、§4.3 的 #10、§5.4 C5 合同、§5.5 共享边界与 §6 双载体验收落为 C5 架构边界。现状是"半条链"：

- C1 已冻结双载体 `EnrollmentTarget`（`src/product/logical_codebase/types.rs:23-31`）与版本化绑定（`EnrollmentBindingIdentity.target`，`src/product/models/automation.rs:136-146`），但单仓 target 没有任何消费者：`GET /automation-target`（`src/web/handlers/automation_target.rs:87-105`）与 PUT Enable 的 `validate_enrollment_scope`（`src/web/handlers/automation_enrollment.rs:336-347`）对非 `RepositoryRouting::Logical` 直接 `invalid_scope` 422。
- C4 已交付唯一 authority：`RepositoryAuthorityResolver::resolve_for_issue`（`src/product/logical_codebase/repository_routing.rs:236-302`）对单仓 issue 返回 `Ok(None)` 并明确"不得把物理仓伪装为 LC target"。
- Enable 前预检只覆盖 reviewer（GAP-F 遗产，`src/web/handlers/automation_gateway_preflight.rs:31-47`），LC 下必被 gateway 拒绝的 coder 仍能授权（发现 #10）。
- enrolled advance 的单 target 判定只认 logical（`src/web/advance_plan.rs:196-205` 按 `units_by_target` 要求恰一 logical target，单仓 plan 的 units 全部 `unattributed` 必被拒）；`verify_current_enrollment` 的 attempt 互证同样只比 `logical_repository_id`（`src/web/coding_start.rs:491-502`）。
- 单仓 Claude 初始化已有真实步骤链与 Failed 终态（`src/product/repository_store/operation.rs:220-267`、`initializer.rs:103` 的 `provider_unavailable`），但失败后没有通知停等与"恢复后继续"产品动作（GAP-J）；pi recipe 按裁决延期。

上位追溯：§3.1 通知→用户操作→原链继续闭环、§3.3 操作安全约束；§5.4 四条实施合同（target union 全链迁移／role-chain 预检／Claude 初始化停等／前端默认行为）；§5.5 要求 C5 只消费 C1 target／绑定写面与 C4 resolver，不改 LC gateway 谓词本体、不复制生命周期实现；§7 定 C5 为 3–5 人日、验收 A01/A02/A05/A10、pi 延期不构成阻断。

## Goals / Non-Goals

**Goals:**

- 让自动化 target 投影与 Enable 接受单仓载体：载体由 C4 唯一 authority 判定，单仓 target 为 `SingleRepository{repository_id}`，`repository_id` 等于 issue 记录的 `repo_id` 解析出的真实 `RepositoryRecord.id`，全链不生成任何 logical 替身身份。
- 把 enrollment 与冻结意图的载体身份收敛到 `EnrollmentTarget` 单一权威：删除与 target 冗余的 `logical_repository_id` wire 字段（BREAKING），Enable 必须携带 `target`。
- Enable 前完整角色链预检：按真实 launch 派生 author→coder→reviewer 逐角色核验；LC 静态不支持组合稳定 422（`automation_role_chain_unsupported`，逐角色列出违规）；单仓不套 gateway 约束；动态 503 仍走既有停等。
- 单仓 enrollment 沿既有单仓路径完成与 LC 同等的自动链（prepare→生成→人工门→advance→typed 首启→FinalConfirm 前停等），prepare／Enrolled advance／StartCoding 各自复核 enrollment target 与当前权威载体一致，漂移 fail-closed 零副作用。
- Claude 单仓登记初始化失败的可操作停等：失败 operation 进驾驶舱通知（步骤证据＋结构化诊断），"网关恢复后继续"以冻结输入重提新 operation，原失败记录只读。
- 前端 TS 类型、提交 payload、错误展示同步切换；默认仍不自动化，manual／off 不被补偿扫描接管。

**Non-Goals:**

- 不实现 pi 初始化 recipe、不引入第三初始化 provider、不改 `repos.json` 或任何直写登记数据（proposal Non-Goals、§4.1 GAP-J 延期边界）。
- 不重命名 `RepositoryRouting::Legacy`（仅文档注释澄清语义），不为单仓创建逻辑代码库 manifest／selection／snapshot，不把单仓伪装为 LC。
- 不重做 C1 的 target／binding 写面（只消费 `EnrollmentTarget`、`load_current_enrollment_binding`、command ledger）、C2 的 runner／门、C4 的 resolver（只消费 `resolve_for_issue` 与 `repository_routing_*` 错误码）。
- 不放宽 LC gateway 与单 target 约束，不引入多 target 自动 fan-out；不在 enrollment options 新增独立 coder provider 配置（coder 按既有派生规则取值）。
- 不自动补偿历史 issue；预检不探活、不把动态不可用判为静态不支持。

## Decisions

### 1. 载体判定 resolver-first，单仓 target 用真实物理仓身份

GET 投影与 PUT Enable 的载体判定从"`RepositoryRouting::load_for_issue` else 分支 422"改为先调 C4 `RepositoryAuthorityResolver::resolve_for_issue`（`repository_routing.rs:236`）：

- `Ok(None)`（单仓 issue、无 legacy 别名 LC）→ 单仓分支：读取 `IssueRecord.repo_id`（缺失→422 指明缺仓库身份），经 `find_repository`（`src/web/handlers/lifecycle/plan_preparation.rs:84` 同源物理仓解析）解析真实 `RepositoryRecord.id`，target 固定为 `EnrollmentTarget::SingleRepository { repository_id }`，且要求用户提交的 target 与之逐字节相等。
- `Ok(Some(resolution))` → LC 分支：保持既有 `preflight_single_repository_candidate` 单 target 约束（`automation_enrollment.rs:348-351`），target 为 `LogicalCodebase` 双级身份。
- resolver／routing 返回的冲突 → 沿既有错误字符串 `repository_routing_source_identity_mismatch`／`repository_routing_legacy_conflict` 与 HTTP 409 映射（实际产生点 `repository_routing.rs:414-418`、`:446-449`、`:555-587`；映射 `src/web/handlers/support_parts/product_store_error.inc.rs:49-97`、`src/web/error.rs:122-125`），不将其误称为 `RepositoryRoutingErrorCode::stable_code` enum 变体，不选择任一记录继续，不猜“最新”。

**选择理由：** authority 判定只有 C4 一个写面，C5 若在 GET/PUT 里重新按 manifest/selection 自判会重建双轨（§5.5 防撞要求）。物理仓身份只来自 `IssueRecord.repo_id → RepositoryRecord.id` 权威链，杜绝伪造 logical ID 或反向降级。

**防护：** `AutomationTargetDto`（`automation_target.rs:26-33`）单仓投影只含 `enrollment_target`（单仓形态）与 `resolved_options`，不返回任何 logical 身份字段；错仓、跨载体、仓未登记一律 422 且零 enrollment 写入、零 provider 启动。

### 2. 载体身份收敛到 target union（BREAKING wire）

与 target 冗余的 `logical_repository_id` 从以下写面删除，`EnrollmentTarget` 成为唯一权威：

- `IssueAutomationEnrollment.logical_repository_id`（`automation.rs:56`）与 `EnrollmentWriteCommand::Enable.logical_repository_id`（`automation.rs:114`）→ Enable 必须携带 `target`（`Option` 收紧为必填）；读侧旧 JSON 的该字段按 serde 忽略。
- 冻结意图 `PreparedPlanIntent.logical_repository_id`（`automation.rs:318/334`）与 `PlanGenerationIntent.logical_repository_id`（`automation.rs:355/377`，`same_identity` 参与漂移判定）→ 改为 `target: EnrollmentTarget`，从 `enrollment.target` 派生；旧意图文件重派生不一致即按既有 fail-closed 语义拒绝，不覆盖。
- `AutomationTargetDto.logical_repository_id`（`automation_target.rs:28`）与 enrollment GET/PUT DTO、前端 `web/src/api/types/lifecycle.ts`（`AutomationEnrollment`／`AutomationEnrollmentEnableCommand` 的 `logical_repository_id`，lifecycle.ts:298/314）→ 删除；`useIssueLifecycleGeneration.ts:318` 的提交 payload 只回传 `enrollment_target`。
- reviewer 专用错误码 `automation_gateway_reviewer_unsupported`（`automation_gateway_preflight.rs:19`、`src/web/error.rs:202`）→ 由 `automation_role_chain_unsupported`（422）取代。

**存量数据：** 旧 enrollment JSON（有 `logical_repository_id` 无 `target`）不做迁移，按 C1 既定语义解释为"无版本化绑定的旧代 enrollment"——自动链 fail-closed（`load_current_enrollment_binding`，`src/web/advance_plan.rs:37-64`），用户重新 Enable 或 rebind 携带 target 后进入新链；手动链零影响。

**选择理由：** 保留冗余字段意味着每个消费点都要做"两字段一致性"校验，且单仓 enrollment 无合法 logical 值可填——只有删字段才能让"单仓不携带任何 logical 身份"成为类型级不变量（REQ-WIGA-01）。

### 3. 完整角色链预检：同一谓词、两分支、GET/PUT 同判定

把 `automation_gateway_preflight.rs` 的单 reviewer 谓词扩展为逐角色 role-chain 预检函数，GET 投影与 PUT Enable 调用同一函数（现状两处各自内联调用 reviewer 谓词，`automation_target.rs:108-114` 与 `automation_enrollment.rs:367-373`）：

- **角色派生**：按真实 launch 路径的同一规则，不新增配置——plan author 与 coder 取 `EnrollmentOptions.author_provider`（`ProviderConfigSnapshot` 冻结的 author，coder 无独立配置项），plan reviewer 与 code reviewer 取 `reviewer_provider`；internal reviewer 从同一 reviewer 配置三值派生，类型为 `Option<ProviderName>`（`CodingRoleProviderConfigSnapshot.internal_reviewer`／`internal_reviewer_config()`）。存在 reviewer provider 时派生该 provider，缺失为 `None`，不参与谓词且不得回填 author。当前 `ProviderName` 没有单独的 internal-reviewer provider 枚举变体；`ProviderConversationRole::InternalReviewer`（`src/product/models/provider.rs:43`）只是会话角色标签，不是 provider。
- **LC 载体**：每个 (角色, provider) 组合过既有静态谓词——`ProviderRef::from_provider_name`（`provider_gateway.rs:143`，无 gateway dialect 即 `provider_unsupported_for_gateway_launch`）＋ Codex 路由禁令（`CODEX_DANGER_FULL_ACCESS_*`，`provider_gateway.rs:1111`，与 `enforce_route_policy` 同源）。任一角色违规 → 422 `automation_role_chain_unsupported`，错误逐角色列出（角色、provider、原因）并提示更换配置／目标。
- **单仓载体**：MUST NOT 施加 gateway 约束——resolver 判定为单仓时跳过 gateway 谓词，仅保留既有 provider 可用性校验（`provider_workspace_config`，`automation_target.rs:56-63`）；本机可用即授权（A10"单仓不误拒"）。
- **静态≠动态**：预检只判确定性静态不支持，不探活、不实例化 provider；Enable 后运行期 503 走既有 `BlockedProviderUnavailable` 停等与通知（`autopilot_orchestrator.rs:379-388` 终态交生成准入分诊），不自动切换 provider、不盲重。

**防护：** 拒绝路径零 enrollment 写入、零 provider 启动（422 在 `validate_enrollment_scope`／投影阶段先于任何 store 写）；测试运行 `test_provider_enabled` 仅豁免 Fake，不放行 Pi/KimiCode（沿用 `automation_gateway_preflight.rs:38-40` 语义）。

**选择理由：** 谓词本体（dialect 映射、路由禁令）与真实 gateway 执行同源，预检不产生第二套支持矩阵；把 reviewer 谓词泛化为 role-chain 而非新增 coder 专用码，错误一次列出全部违规角色，避免用户逐次试错（#10 根因是"只查一角色"）。

### 4. 单仓自动链沿既有路径，逐门核对 target 一致性

自动链骨架（orchestrator reconcile：`ensure_enrolled_plan` → 人工停点 → `coding_chain_stage` advance→Ready→AutoStartOnce，`autopilot_orchestrator.rs:75-136/445-569`）载体无关，C5 只在各核对点按 union 分支：

- **prepare**：`prepare_plan_records` 的 Legacy 分支已支持单仓（`plan_preparation.rs:79-89`：`repo_id`→物理仓→单候选）；`ensure_enrolled_plan`（`plan_preparation.rs:258`）前置 `load_current_enrollment_binding` 已按 target union 校验版本化绑定，冻结意图（决策 2）改从 `enrollment.target` 派生后自然覆盖单仓。
- **Enrolled advance**：`advance_plan.rs:196-205` 的恰一 target 判定扩展为两分支——单仓 enrollment 要求 `units_by_target.by_target` 为空且 `unattributed` 非空（plan 全部工作项无 logical target 归属）且 attempt 无 `target_snapshot`；LC enrollment 要求 `by_target.len()==1` 且唯一 target 等于 enrollment target（补齐"attempt 唯一 target 必须等于 enrollment target"核对，spec 场景"逻辑代码库 attempt target 与 enrollment 不符"）。group 创建侧 `validate_group_single_target`（`src/product/coding_attempt_store/group_validation.rs:510-550`）的 Legacy 分支已保证单仓 units 全 `None`——复用不重写。
- **typed StartCoding**：`verify_current_enrollment`（`coding_start.rs:451-504`）的 attempt 互证按 enrollment target 分支——单仓：attempt `target_snapshot` 必须为 `None`（`Some` 即身份漂移拒绝），且 enrollment target 的 `repository_id` 必须等于 issue 当前权威 `repo_id` 解析结果；LC：`snapshot.logical_repository_id == binding target` 的 logical repository（替换现 `enrollment.logical_repository_id` 比较，:491-502）。`verify_current_binding`（`coding_start.rs:510-543`）已按 union 比较 origin 冻结 target 与当前 binding——零改动复用。
- **单仓不注入 LC 设施**：单仓 enrollment 的 prepare／advance／StartCoding 全程不创建 LC manifest/selection/snapshot、不产生 gateway 审计（REQ-ROUTE-C5-TARGET"单仓链不经 gateway"）；provider 按单仓直连路径启动，worktree 沿单仓 shared worktree 路径（`advance_ready_only.rs` 的 Legacy group 路径零变化）。
- **投影与通知**：`plan_confirmed_info`、驾驶舱等待项（`C1WaitingItemDto.target`，`plan_confirmed_info.rs:148` 已是 union）、成功信息对两种载体同一派生，展示当前 target 身份。

**防护：** 任一核对点载体漂移（issue 改属 LC 而 enrollment 是单仓、或反之）→ fail-closed 零副作用，等待项提示重新绑定；manual／off enrollment 在 reconcile 入口即返回 `NoEnrollment`（`autopilot_orchestrator.rs:86-91`），补偿扫描不接管。

### 5. GAP-J：Claude 初始化失败停等与"恢复后继续"

单仓登记继续使用现有 Claude 初始化真实步骤链与 `git_finalize` 语义（`ClaudeRepositoryInitializer`，`initializer.rs:33`；步骤命令由 `RepositoryInitializationStepKind::command()` 定义，`types.rs:140`）。失败面补齐为可操作停等：

- **失败事实已 durable**：worker 失败把 operation 置 `Failed` 并记录 `failed_step`／`error`（`operation.rs:220-267`），provider 不可用带 `provider_unavailable` 原因码（`initializer.rs:103`）；冻结输入持久在 `RepositoryInitializationOperation.input`（`types.rs:181-186`）。新增 `parent_operation_id`、`resume_command_id`、`superseded_by`（或等价 successor 关联）作为 durable linkage，旧 JSON 以默认值读入。
- **通知等待项**：扩展既有 waiting-items 投影家族（`C1WaitingItemDto` 的字段约定：kind/reason/completed_steps/actions，`plan_confirmed_info.rs:140-159`）增加 project 级 `repository_initialization_failed` 等待项。`C1WaitingItemDto` 显式携带 `operation_id` 与 `RepositoryInitializationFailureDiagnostics`（failed step、reason code、provider、changed paths、retryable），project 条目 `issue_id` nullable/缺省，稳定事实 key 为 project+operation；由驾驶舱复用同一 inbox 投影消费。原 Failed 无 successor 时展示，后继 Completed 后原 waiting item 稳定消隐，后继 Failed 时展示最新失败链而不重复生成无关联条目。
- **产品层恢复后继续动作**：`POST /api/projects/{project_id}/repository-initializations/{operation_id}/resume`（路由命名沿既有 `.../resume` 约定，`app.rs:189/209`；携带 `command_id`）只做 HTTP 映射，调用产品层 deterministic coordinator。coordinator 校验原 operation 为 `Failed`，按冻结 `input` 及固定 `Uuid::NAMESPACE_URL`、UTF-8 name `cadence/repository-initialization/v1\\0{original_operation_id}\\0{command_id}` 派生**新的** operation；Task 6 同步 `Cargo.toml` uuid `v5` feature 与 `Cargo.lock`。operation store 增加 project list，扫描 `repository_initializations_root(project_id)`，校验 id/project/shape/state 并按 `(created_at, operation_id)` 排序，损坏记录显式诊断。coordinator 以 registry/guard→create→execute 的顺序串起完整 Claude 步骤，不在 handler 调随机 `begin_initialization` 代替恢复。
- **状态分流与互斥**：同 `(failed_operation_id, command_id)` 重放返回同一 successor；successor `Created` 只允许一次执行，`Running` 只读返回而不重复启动，`Completed` 只读返回成功事实，`Failed` 只读返回失败事实，用户必须再次使用新的 command 才能恢复。并发互斥复用 `repository_initialization_runs` registry；同 git 根已有登记或正在初始化 → 既有冲突语义拒绝。
- **边界**：网关仍不可用时 successor 再次以 `Failed` 终态停等并保留诊断/parent 链，不伪造成功、不直写登记数据；pi recipe 本轮不做。

**选择理由：** resume 是“新事实”而非复活旧记录，满足审计（A05“无假成功”）；产品层 coordinator 让 deterministic id、冻结 input、关联事实、operation list、互斥与执行具有单一原子入口；复用完整步骤链保证恢复后仍走真实 Claude 初始化，无跳步旁路。

### 6. 前端切换、project waiting item 消费与默认行为

前端同步 BREAKING 收敛：`web/src/api/types/lifecycle.ts` 删除 `logical_repository_id` 字段并按新错误码展示逐角色违规原因；`useIssueLifecycleGeneration.ts` Enable payload 只回传服务端投影的 `enrollment_target`。`WorkItemPlanOptionsDialog.tsx` 打开自动模式时先获取 target projection，处理 loading/error/stale，manual 模式不受 target GET 失败阻断。

project 级失败等待项走完整消费链：`GET /api/projects/{project_id}/repository-initializations/waiting-items` → `useWorkspaceSessionObservers.ts` 刷新 → `workspace-cockpit-projection.ts` 合并为无 issue 的 project item → `CockpitInbox.tsx` 展示 diagnostics 与“网关恢复后继续” → `cockpit-action-routing.ts` 的 project+operation `C1RecoveryActionPayload` 与 `CockpitActionFacade` → `ChatCockpitPage.tsx` sender → `POST /api/projects/{project_id}/repository-initializations/{operation_id}/resume`。item identity 使用结构化 `operation_id` 与 project，不从展示字符串反解析；动作成功后刷新 project，失败保留 durable waiting item，不乐观隐藏。`ApiRequestError.details` 保留或转换为结构化 role-chain view model。

默认行为不变：未 Enable／Disable 的 issue 走手动链，补偿扫描零动作（A01 手动对照）。存量 JSON 多余字段按忽略读取，旧 target 缺失通过 legacy reader 转为可诊断 fail-closed，不阻断手动路径。

## Failure Handling and Migration

- **载体／身份漂移**：issue 权威载体与 enrollment target 载体或身份不一致（含 issue 改属 LC、repo_id 变更、attempt 携带 LC snapshot 而单仓 enrollment）→ 相关动作 fail-closed 零副作用，等待项提示重新绑定；不自动迁移、不猜最新。
- **authority 冲突**：resolver 返回实际错误字符串 `repository_routing_source_identity_mismatch`／`repository_routing_legacy_conflict`（产生于 `src/product/logical_codebase/repository_routing.rs:414-418`、`:446-449`、`:555-587`，HTTP 409 映射于 `src/web/handlers/support_parts/product_store_error.inc.rs:49-97` 与 `src/web/error.rs:122-125`）→ fail-closed，不选择任一记录继续；这些字符串不是 `RepositoryRoutingErrorCode::stable_code` enum 变体。
- **预检拒绝**：422 稳定码逐角色列违规，零 enrollment 写入、零 provider 启动；GET 与 PUT 同判定。
- **运行期 503**：Enable 后动态不可用走既有通知停等与“恢复后重试”，不自动切换 provider、不盲重、不回滚 enrollment。
- **旧 durable 数据**：无 `target` 的旧 enrollment／旧意图由显式 legacy reader 转为“旧代无绑定”可诊断结果，自动链拒绝、手动链零回归；重新 Enable／rebind 携带 target 进入新链，旧记录只读。
- **init resume 再失败**：保留 successor 的最新失败诊断与 parent 链并再次停等；successor Completed 时原 waiting item 稳定消隐；原失败与历次 resume 记录均可追溯，Repository 记录只在真实完成时创建。
- **兼容边界**：本 change 只改 C5 直接冲突的 requirement；C2 coding 侧恢复、C4 LC 冷启动不在此重复实现。

## Contract Traceability

| 方案 v1.2 依据 | 本设计落点 |
|---|---|
| §4.1 GAP-I（target union 全链） | 决策 1/2/4：resolver-first 投影与 Enable、冗余字段删除、prepare/advance/StartCoding 逐门核对 |
| §4.1 GAP-J（Claude 停等、pi 延期） | 决策 5：Failed 通知等待项＋冻结输入 resume；Non-Goals 明确 pi/repo.json 边界 |
| §4.1 单仓自动化入口行（全链承诺） | 决策 4/6：单仓沿既有路径自动到 FinalConfirm 前停等，页面关闭不阻断 |
| §4.3 #10（LC Pi coder 可 Enable） | 决策 3：author→coder→reviewer 逐角色预检、422 逐角色列出、单仓不误拒 |
| §4.5 GAP-F 已修边界 | 决策 3 复用 reviewer 谓词本体扩展，不重做 reviewer 预检 |
| §5.4 C5 合同四条 | 决策 1-2（target 迁移）、3（role-chain）、5（Claude 入口）、6（前端默认） |
| §5.5 共享边界 | 决策 1 消费 C4 resolver、决策 2/4 消费 C1 target/binding 写面、决策 3 不改 LC gateway 谓词本体 |
| §6.1-6.3 A01/A02/A05/A10 与零绕行 | 验收映射见 tasks.md；手动对照零回归、单仓自动全链、网关停等无旁路、role 准入零 spawn |
| §7 工期与阻断 | 依赖 C1（已 complete）与 C4 resolver（已交付）；pi 延期不构成开发阻断 |

## Risks / Trade-offs

- **BREAKING wire 需前后端按顺序切换：** 先落后端 wire/model、resolver、project waiting HTTP，再切前端类型与消费链；Task 1 至 Task 7 完成前禁止前端先于后端部署。存量 JSON 由显式 legacy reader／serde default 诊断读取，保证读侧不炸。
- **单仓无 gateway 预检意味着运行期失败只能停等：** 与手动单仓链的既有行为一致（手动链本就直连），A10 只要求"合法组合不误拒"，不承诺单仓零失败。
- **resolver-first 增加只读判定的读取面：** GET/Enable 各多一次 issue/project 权威读；均为既有 store 只读操作，可接受。
- **advance/StartCoding 逐门核对分支增多：** 分支只做"按 enrollment target 的载体分派＋身份相等断言"，不引入新状态；`validate_group_single_target` 与 `verify_current_binding` 两个既有核对点零改动，控制回归面。
- **resume 确定性 operation_id 依赖 store 幂等语义：** 该语义已有既有实现覆盖（`operation.rs:24-41` 同记录幂等／异记录拒绝）；uuid5 派生保证跨重放稳定，hash 输入含原 operation_id 防止跨 operation 串档。
