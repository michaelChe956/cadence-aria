# Tasks

## 1. 单仓 target 投影、Enable 与 wire 收敛（REQ-WIGA-01、REQ-ROUTE-C5-TARGET）

- [ ] 1.1 先完成后端 BREAKING wire/model 收敛：删除 enrollment 记录／Enable 命令／`AutomationTargetDto`／冻结意图中与 target 冗余的 `logical_repository_id`，将 Enable `target` 收紧为必填；为 `PreparedPlanIntent`、`PlanGenerationIntent` 与 store/plan_generation 两条读路径增加显式 legacy reader，旧 JSON 缺 target 转为可诊断“旧代无绑定”并自动链 fail-closed，不因 serde 缺字段直接炸；所有后端编译消费从 `enrollment.target` 单向派生。同步 wire fixtures、旧 enrollment/意图读侧测试与 `RepositoryRouting::Legacy` 注释，不改名、不接入单仓 resolver。该任务必须先于 1.2，供单仓 Enable 使用真实 target，禁止伪造 logical 身份。

- [ ] 1.2 在 1.1 完成后，将 `GET /automation-target` 与 PUT Enable 的载体判定迁移为 C4 `RepositoryAuthorityResolver::resolve_for_issue` 优先：单仓分支以 `IssueRecord.repo_id → RepositoryRecord.id` 权威链构造 `SingleRepository{repository_id}` target（缺仓／仓未登记／错仓／跨载体 422，不伪造 logical 身份）；复用 C4 single-repository authority/conflict 检查，authority 冲突沿实际 `repository_routing_source_identity_mismatch`／`repository_routing_legacy_conflict` 错误字符串及 HTTP 409 映射 fail-closed；LC 分支保持既有单 target 约束。GET／PUT 必须调用同一 resolver，独立提交但不得脱离 1.1 部署或测试。

**验收映射：** A02“单仓目标可 Enable，错误仓／旧绑定被拒”；`web-runtime-repository-routing`、`work-item-group-autopilot` delta。

## 2. Enable 前完整角色链预检（REQ-WIGA-C5-PREFLIGHT）

- [ ] 2.1 将 `automation_gateway_preflight` 的 reviewer 谓词扩展为 author→coder→reviewer 逐角色 role-chain 预检（角色按真实 launch 派生：author_provider→plan author＋coder、reviewer_provider→plan/code reviewer；internal reviewer 从同一 reviewer 配置三值派生为 `Option<ProviderName>`，缺失为 `None`、不参与谓词且不回填 author；当前不存在 `ProviderName::InternalReviewer`，`ProviderConversationRole::InternalReviewer` 仅为会话角色标签；不新增 options 字段），GET 投影与 PUT Enable 调用同一函数、同一判定；错误码切换为 `automation_role_chain_unsupported`（422，逐角色列出违规与更换配置／目标提示），移除 `automation_gateway_reviewer_unsupported`；LC 分支复用 `ProviderRef::from_provider_name` 与 Codex 路由禁令同源谓词，单仓分支跳过 gateway 约束不误拒；拒绝零 enrollment 写入、零 provider 启动；运行期 503 仍走既有停等。

**验收映射：** A10“LC 不支持的 Pi coder 在授权和 admission 前稳定拒绝、单仓合法组合不误拒、503 通知停等无盲重”。

## 3. 单仓自动全链与前端切换（REQ-WIGA-C5-CHAIN、REQ-ADV-05、REQ-MTG-03）

- [ ] 3.1 扩展 Enrolled advance 的恰一 target 判定为双分支：advance 门前读取 issue 最新记录并调用当前 authority/carrier resolver；比较 carrier 与 `EnrollmentBindingIdentity.target`，再检查单仓 plan 全部工作项无 logical target 归属且 attempt 无 LC target 快照，或 LC attempt 唯一 target 等于 binding target；所有拒绝发生在 Ready／AutoStartOnce 等写入前。typed StartCoding 在已有 `paths`／attempt 上下文中、`with_current_enrollment_locked` 闭包内、`claim_coding_start` 前预解析 current carrier，并传入 `verify_current_enrollment` helper（允许内部签名增加 `current_carrier`）；单仓要求 snapshot 为 `None` 且 physical repository 身份相等，LC 比对 logical repository；resolver/ProductStore 错误映射为结构化 `StartCodingError`，保留 code/details；单仓链不注入 LC gateway／snapshot，prepare 沿既有 Legacy 分支。

- [ ] 3.2 完成 project 级 waiting item 的前端消费链：Rust project GET 返回无 issue 条目；observer 刷新 project list；projection 以结构化 `operation_id` 合并至同一 inbox；action facade/page sender 透传 project+operation resume variant；POST resume 成功后刷新 project，失败保留 durable waiting item。同步 `WorkItemPlanOptionsDialog.tsx` 在打开 automatic 时获取 target projection，处理 loading/error/stale（GET 失败禁止 stale automatic submit，manual 不阻断）；同步 role-chain details 保留与逐角色渲染；验证关闭页面与 coding socket 后单仓自动推进不阻断、manual/off 不被补偿扫描接管。

**验收映射：** A02“单仓自动 plan→Ready→typed 首启→coding→FinalConfirm 停等，错误仓／旧绑定拒绝”；A01 手动对照零回归。

## 4. Claude 初始化失败停等与恢复后继续（REQ-INIT-C5-RESUME）

- [ ] 4.1 在产品层实现 deterministic resume coordinator：扩展 `registration.rs`／`operation.rs`／`types.rs`，以冻结 input、固定 uuid5 namespace/name、project operation list、parent/resume/successor durable linkage、registry/guard/create/execute 原子顺序实现恢复；原 Failed 只读保留。明确状态分流：原 operation 非 Failed 拒绝；successor Created 执行一次，Running/Completed/Failed 只读返回，新的显式 command 才能再次恢复；project waiting projection 在无 successor 时展示原 Failed，successor Completed 后消隐，successor Failed 时展示最新链。补 `C1WaitingItemDto.operation_id`、`RepositoryInitializationFailureDiagnostics`、project/issue nullable 身份，新增 project waiting-items GET 与 resume POST；同步 `Cargo.toml` uuid v5 feature／`Cargo.lock`。前端由 3.2 消费。不得在 handler 内拼接旧随机 begin/execute 代替产品入口。

**验收映射：** A05“Claude 网关 503 时无旁路、无假成功，恢复后登记可继续”。

## 5. 联验、横贯红线与边界收口

- [ ] 5.1 完成 A01/A02/A05/A10 端到端联验与 §6.3 横贯检查：双载体手动对照零回归、单仓自动全链关页面跑到 FinalConfirm、零手工铺底／零直改数据／零重启服务脱困、通知→用户操作→自动继续闭环；确认与 C1（target/binding 写面）、C4（resolver/gateway 谓词本体）、C2（runner/门）零写面冲突，pi recipe 延期边界与 `RepositoryRouting::Legacy` 不改名口径在代码注释与文档中一致。

**验收映射：** 方案 v1.2 §5.4、§5.5、§6.1–6.3、§7；A01、A02、A05、A10。
