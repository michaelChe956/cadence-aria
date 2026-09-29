# Proposal

## Why

单仓（Single-Repository）是正当的一等载体，单仓手动 plan＋coding 路径已经跑通；但自动化入口只接受逻辑代码库（LC）：`GET automation-target` 与 `PUT automation-enrollment` 的 Enable 分支对 `RepositoryRouting::Legacy` 直接返回 422，Enrolled advance 要求“恰一 logical target”，StartCoding 用 `enrollment.logical_repository_id` 与 attempt snapshot 互证。C1 已冻结双载体 `EnrollmentTarget`，C4 已提供 `RepositoryAuthorityResolver::resolve_for_issue`，但单仓 target 没有任何消费者，只放开入口会形成“可 Enable、后续断链”的假自动化。同时 Enable 前只预检 LC reviewer，LC 下必被 gateway 拒绝的 coder 仍能授权（发现 #10）。本 change 按方案 v1.2 §5.4 C5 合同打通单仓自动化全链并补齐完整角色链预检，满足 A02／A10。

## What Changes

- 自动化 target 投影与 Enable 接受单仓载体：载体由 C4 唯一 authority 判定；单仓 target 为 `SingleRepository{repository_id}`，`repository_id` 必须等于 `IssueRecord.repo_id`，且解析到真实 `RepositoryRecord.id`；不生成、不填充任何 logical 替身 ID。
- enrollment 及冻结意图的载体身份以 `EnrollmentTarget` 为唯一权威：单仓 enrollment 不再携带伪 logical repository id；prepare、plan 生成意图、Enrolled advance、typed StartCoding、owner 投影、驾驶舱等待项与成功信息全部按同一 target 核对，跨载体或身份漂移 fail-closed。
- 单仓 prepare／worktree／advance 沿单仓既有路径（物理仓解析、单仓 shared worktree），不注入 LC gateway、不创建 LC snapshot；LC 分支保持现有 gateway 与单 target 约束，并补上“attempt 唯一 target 必须等于 enrollment target”的核对。
- Enable 前完整角色链预检：按实际 launch 派生 author→coder→reviewer，每个角色按其载体的真实启动路径判定；LC 静态不支持组合在授权前稳定 422 并指明角色与更换配置／目标操作，零 enrollment 写入、零 provider 启动；单仓不套 gateway 约束；运行期动态不可用（503）进入既有通知停等，不自动切换 provider、不盲重。
- **BREAKING（wire）**：reviewer 专用错误码 `automation_gateway_reviewer_unsupported` 由覆盖全部角色的 `automation_role_chain_unsupported`（422，逐角色列出违规）取代；enrollment、Enable 命令、plan 准备／生成意图与 automation-target 投影中与 `target` 冗余的 `logical_repository_id` 字段删除，Enable 必须携带 `target`。前端 TS 类型、提交 payload、错误展示同步切换；存量 JSON 多余字段按忽略读取，不做数据迁移。
- Claude 单仓登记初始化失败的可操作停等：失败 operation 进入驾驶舱通知，展示步骤证据与“网关恢复后继续”操作（以冻结输入重新提交新的初始化 operation，不复活旧记录）；pi 初始化 recipe 本轮不做。
- 代码枚举 `RepositoryRouting::Legacy` 保留原名，仅以文档注释澄清其语义为“单仓载体（Single-Repository）”。
- 修改 REQ-ADV-05／REQ-MTG-03 中“恰一 logical repository 才可自动首启”的口径为“恰一精确 target（单仓物理仓或一个 logical repository）”，多 target 人工红线不变。

## Capabilities

### New Capabilities

无。

### Modified Capabilities

- `work-item-group-autopilot`：自动授权的单 target 范围扩展到单仓载体；新增完整角色链预检与单仓全链自动推进要求。
- `web-runtime-repository-routing`：新增 enrollment 载体判定与单仓 target 语义（真实 `RepositoryRecord.id`、禁止伪 logical ID、单仓不走 LC gateway）。
- `multi-target-group-coding`：REQ-MTG-03 的“单 target enrollment 唯一例外”从“一个 logical repository”扩展为“一个精确 target（单仓物理仓或一个 logical repository）”，多 target 红线不变。
- `work-item-plan-advance`：REQ-ADV-05 的自动首启前提改为“恰一精确 target 且与 enrollment target 一致”，补单仓 enrolled advance 场景。
- `repository-initialization-progress`：新增单仓 Claude 初始化失败的通知与“恢复后继续”产品操作。

## Non-Goals

- 不实现 pi 初始化 recipe，不引入第三个初始化 provider，不改 `repos.json` 或以任何直写登记数据绕过网关不可用；登记遗留在网关重新挂接后另行立项。
- 不重命名 `RepositoryRouting::Legacy`，不为单仓创建逻辑代码库 manifest／selection／snapshot，不把单仓降级或伪装为 LC。
- 不放宽 LC gateway 与单 target 约束，不引入多 target 自动 fan-out。
- 不自动补偿历史 issue；默认仍不自动化，manual／off 不被补偿扫描接管；story／design、人工计划门、choice、FinalConfirm 仍由人处理。
- 不在 enrollment options 中新增独立 coder provider 配置；coder 按既有派生规则取值。
- 不重做 C1 的 target／binding 写面、C2 的 runner／门或 C4 的 resolver，只消费其合同。

## Impact

- 后端：`src/product/models/automation.rs`、`src/product/issue_automation_store.rs`、`src/web/handlers/{automation_target.rs,automation_enrollment.rs,automation_gateway_preflight.rs,lifecycle/plan_preparation.rs,repository_registration.rs}`、`src/web/{advance_plan.rs,coding_start.rs,autopilot_orchestrator.rs,plan_confirmed_info.rs,error.rs}`、`src/product/logical_codebase/repository_routing.rs`（注释）、`src/product/repository_store/`。
- 前端：`web/src/api/types/lifecycle.ts`、`web/src/components/lifecycle/{useIssueLifecycleGeneration.ts,WorkItemPlanOptionsDialog.tsx}`、驾驶舱 inbox 通知与相关测试夹具。
- 依赖：C1 `enrollment-recovery-surface`（complete，需先归档）、C4 `logical-codebase-bootstrap-and-repair-surface`（complete）；与 C2 `coding-execution-resilience-and-product-triage` 无写面重叠。
- 上位输入：`cadence/designs/2026-09-28_方案设计_真实链缺口修复总方案_v1.2.md` §4.1 GAP-I／GAP-J／单仓入口行、§4.3 #10、§5.4、§6.1–6.3（A01／A02／A05／A10）。
