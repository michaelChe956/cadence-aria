# Tasks

## 1. 单仓 target 投影、Enable 与 wire 收敛（REQ-WIGA-01、REQ-ROUTE-C5-TARGET）

- [ ] 1.1 将 `GET /automation-target` 与 PUT Enable 的载体判定迁移为 C4 `RepositoryAuthorityResolver::resolve_for_issue` 优先：单仓分支以 `IssueRecord.repo_id → RepositoryRecord.id` 权威链构造 `SingleRepository{repository_id}` target（缺仓／仓未登记／错仓／跨载体 422，不伪造 logical 身份）；LC 分支保持既有单 target 约束；authority 冲突沿 `repository_routing_*` 稳定码 fail-closed（对应 `work-item-group-autopilot` REQ-WIGA-01、`web-runtime-repository-routing` REQ-ROUTE-C5-TARGET）。
- [ ] 1.2 执行 BREAKING wire 收敛：删除 enrollment 记录／Enable 命令／`AutomationTargetDto`／冻结意图（`PreparedPlanIntent`、`PlanGenerationIntent`）及前端 `lifecycle.ts`、`useIssueLifecycleGeneration.ts` 中与 target 冗余的 `logical_repository_id`，Enable 必须携带 `target`；存量 JSON 多余字段按忽略读取、旧 enrollment 按"旧代无绑定"解释（自动链 fail-closed、手动链零回归）；`RepositoryRouting::Legacy` 仅补文档注释澄清单仓语义（对应 REQ-WIGA-01、REQ-ROUTE-C5-TARGET）。

**验收映射：** A02"单仓目标可 Enable，错误仓／旧绑定被拒"；`web-runtime-repository-routing`、`work-item-group-autopilot` delta。

## 2. Enable 前完整角色链预检（REQ-WIGA-C5-PREFLIGHT）

- [ ] 2.1 将 `automation_gateway_preflight` 的 reviewer 谓词扩展为 author→coder→reviewer 逐角色 role-chain 预检（角色按真实 launch 派生：author_provider→plan author＋coder、reviewer_provider→plan/code reviewer、internal reviewer 固定系统角色；不新增 options 字段），GET 投影与 PUT Enable 调用同一函数、同一判定；错误码切换为 `automation_role_chain_unsupported`（422，逐角色列出违规与更换配置／目标提示），移除 `automation_gateway_reviewer_unsupported`；LC 分支复用 `ProviderRef::from_provider_name` 与 Codex 路由禁令同源谓词，单仓分支跳过 gateway 约束不误拒；拒绝零 enrollment 写入、零 provider 启动；运行期 503 仍走既有停等（对应 `work-item-group-autopilot` REQ-WIGA-C5-PREFLIGHT）。

**验收映射：** A10"LC 不支持的 Pi coder 在授权和 admission 前稳定拒绝、单仓合法组合不误拒、503 通知停等无盲重"。

## 3. 单仓自动全链与前端切换（REQ-WIGA-C5-CHAIN、REQ-ADV-05、REQ-MTG-03）

- [ ] 3.1 扩展 Enrolled advance 的恰一 target 判定为双分支：单仓 enrollment 要求 plan 全部工作项无 logical target 归属且 attempt 无 LC target 快照；LC enrollment 补"attempt 唯一 target 必须等于 enrollment target"核对；typed StartCoding 的 `verify_current_enrollment` 互证按 enrollment target 分支（单仓 attempt `target_snapshot` 必须 `None` 且 repository 身份等于 issue 当前权威仓；LC 比对 logical repository）；单仓链不注入 LC gateway／snapshot，prepare 沿既有 Legacy 分支（对应 `work-item-plan-advance` REQ-ADV-05、`multi-target-group-coding` REQ-MTG-03、`work-item-group-autopilot` REQ-WIGA-C5-CHAIN）。
- [ ] 3.2 会话 owner 投影、驾驶舱等待项、plan 已确认与"待最终确认"信息对双载体统一按 target 身份派生展示；前端 `WorkItemPlanOptionsDialog.tsx`、错误展示与通知夹具同步单仓 target 与新错误码；验证关闭页面与 coding socket 后单仓自动推进不阻断、manual/off 不被补偿扫描接管（对应 REQ-WIGA-C5-CHAIN）。

**验收映射：** A02"单仓自动 plan→Ready→typed 首启→coding→FinalConfirm 停等，错误仓／旧绑定拒绝"；A01 手动对照零回归。

## 4. Claude 初始化失败停等与恢复后继续（REQ-INIT-C5-RESUME）

- [ ] 4.1 为 `Failed` 终态的单仓初始化 operation 增加 project 级驾驶舱等待项（稳定事实 key、失败／已完成步骤、结构化诊断、可重试性与"恢复后继续"动作说明），并实现 `POST .../repository-initializations/{operation_id}/resume`：校验 Failed 终态后以冻结 `input` 构造确定性新 operation（同 command_id 幂等重放返回同一 operation），走完整既有 Claude 初始化步骤；原失败记录只读；同路径互斥按既有规则拒绝；网关仍不可用时再次 Failed 停等、不伪造成功、不直写登记数据、不引入 pi recipe（对应 `repository-initialization-progress` REQ-INIT-C5-RESUME）。

**验收映射：** A05"Claude 网关 503 时无旁路、无假成功，恢复后登记可继续"。

## 5. 联验、横贯红线与边界收口

- [ ] 5.1 完成 A01/A02/A05/A10 端到端联验与 §6.3 横贯检查：双载体手动对照零回归、单仓自动全链关页面跑到 FinalConfirm、零手工铺底／零直改数据／零重启服务脱困、通知→用户操作→自动继续闭环；确认与 C1（target/binding 写面）、C4（resolver/gateway 谓词本体）、C2（runner/门）零写面冲突，pi recipe 延期边界与 `RepositoryRouting::Legacy` 不改名口径在代码注释与文档中一致。

**验收映射：** 方案 v1.2 §5.4、§5.5、§6.1–6.3、§7；A01、A02、A05、A10。
