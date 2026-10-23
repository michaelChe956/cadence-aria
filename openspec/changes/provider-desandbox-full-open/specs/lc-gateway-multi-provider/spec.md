# 规范增量：LC 四 Provider 网关

<!-- 物理基线（2026-10-17）：原 lc-gateway-multi-provider 已由 controller 归档。真实主规范 openspec/specs/lc-gateway-multi-provider/spec.md:53-67 为 REQ-LCG-04（“Codex 仅受限安全模式”，原 change tasks 2.2），不在 aggregate-planning；write_boundary 门为 :9-51,113-123，Reviewer deny 为 :85-111。此 delta 对真实主规范七条要求均为 MODIFIED，不含双模式 ADDED。anchor 冲突见 design.md 依赖门，不冒领其未完成合同。 -->

## MODIFIED Requirements

### Requirement: 显式四家 provider gateway 映射

系统 SHALL 为 Claude Code、Codex、Pi、Kimi Code 显式解析 provider、dialect、wire adapter；Fake、未知及未来 provider 不得回落到另一家或 legacy direct。每个 action SHALL 独立保存 launch、resume、write_boundary 三态、exact version、投影摘要、证据引用与语义版本。write_boundary 的 Confirmed SHALL 表示当前宿主全开合同被真实验证，不再表示物理禁止写入。旧隔离语义证据不得自动升级。

#### Scenario: 显式 provider 映射
- **WHEN** LC 请求由四家受支持 provider 启动
- **THEN** 系统 SHALL 使用对应真实 adapter；未知 provider 与 Fake SHALL 在会话 spawn 前返回不支持原因，不切换 provider

#### Scenario: 未知 capability 阻断
- **WHEN** launch 或新版 write_boundary 为 Unknown/Denied，或版本、证据语义、投影与当前执行基线不一致
- **THEN** 系统 SHALL 在会话 spawn 前拒绝并返回核验原因，不伪造 Confirmed

#### Scenario: Fresh 与 resume capability 分离
- **WHEN** launch 与新版 write_boundary 已 Confirmed、resume 为 Unknown
- **THEN** fresh SHALL 继续校验其余门禁；明确 resume SHALL 在会话 spawn 前拒绝，不静默转 fresh

#### Scenario: 旧隔离证据迁移
- **WHEN** 读取缺少矩阵、缺少新语义标识或依旧只证明只读/target-only 的历史能力记录
- **THEN** 系统 SHALL 保留历史审计，按新语义未验证处理，并提供真实重验证；不得由旧 supported_actions、provenance 或旧 Confirmed 直接放行

#### Scenario: 明确废止 Unknown 迁移放行桥
- **WHEN** write_boundary 为 Unknown，且 legacy supported_actions 或旧 allow 列表包含本 action
- **THEN** 系统 SHALL 仍拒绝 validate 与 spawn 复验，仅当前语义 Confirmed 放行；legacy 必须真实重探针铸证，无迁移默认 allow，不以“不铸 Confirmed 但先允许启动”绕过本门

### Requirement: LC admission 与 spawn 前复验

LC 会话 SHALL 以 manifest canonical provider_context_root 为进程 cwd，并保留主规范的独立 member checkout/worktree target。validate 与真实会话 spawn 前 SHALL 分别检查 launch、新版 write_boundary、适用 resume、authority/policy/target/cwd/trust/投影/adapter 与适用 D4、受信执行合同；Reviewer 还须有效 B 档窗口。缺失/漂移零会话启动，不回落裸 input、无政策 bridge、备用 cwd/provider；未完成 anchor 的新规划身份规则不由本条提前实施。

#### Scenario: validate 后 authority 或投影漂移
- **WHEN** validate 后、spawn 前任一冻结事实、执行基线或 Reviewer 窗口有效性改变
- **THEN** 系统 SHALL 返回可审计 PolicyDrift/ProviderUnavailable 或窗口诊断，真实会话启动为零；有界版本观测与真实会话计数分开，不能把版本观测当新会话

#### Scenario: LC Coding target 独立于 root cwd
- **WHEN** LC Coding 以 member worktree 为 target
- **THEN** process cwd SHALL 为 canonical root，逻辑唯一交付写根恰为 target；target 不替代 cwd，该写根不宣称限制宿主进程物理可达范围

#### Scenario: 同步与流式入口
- **WHEN** LC 规划、split、Coding 或 Review 请求真实 provider
- **THEN** 同步/流式 SHALL 经同一 validated gateway policy，不支持或材料缺失失败关闭，不以恢复或全开功能绕过真实启动合同

#### Scenario: 双阶段能力复验不再验证禁止宿主写
- **WHEN** validate 或 spawn 复验消费 write_boundary Confirmed
- **THEN** 系统 SHALL 消费当前宿主全开语义证据，不再要求 root/非 target/.git/.aria 的宿主写尝试被 OS 拒绝；未知新证据仍拒绝，Reviewer 违规另由 B 档对账使本轮作废

### Requirement: 可验证的 provider policy projection

系统 SHALL 从不可由 caller 伪造的 gateway projection 将 action/role、岗位纪律、permission/approval、宿主执行基线、配置/MCP、trust、cwd/target 与逻辑 roots 映射到真实 CLI/RPC/ACP，记录精确摘要与证据引用。逻辑 roots 不是 OS 隔离证明；新版 write_boundary SHALL 验证全开执行及岗位/窗口合同，不以宿主写成功判能力失败。候选完整投影增强仍归 anchor 自身依赖工作流。

#### Scenario: Projection 与逻辑写根一致
- **WHEN** gateway 为任一 provider/action 生成 projection
- **THEN** adapter SHALL 仅消费绑定 envelope、provider exact version/dialect、role 与 target 的受信 projection，审计可追溯摘要和证据，不接受 caller 自造执行模型

#### Scenario: Boundary 证据缺失
- **WHEN** 没有当前执行基线的真实能力证据
- **THEN** write_boundary SHALL 保持 Unknown/Denied，不以空 roots、MCP 成功、git status、模型自述或旧隔离探针替代证据

### Requirement: Codex 仅受限安全模式

本条（REQ-LCG-04，保留原标题为修改锚）SHALL 采用四家全开基线：LC Codex Planning/Review/Coding 全相位 danger-full-access，平台不投影 read-only/workspace-write、不永久拒绝有效全开档。Direct 同步全开为“诉求1：四家全角色统一”的推论，非 LC 锚点逐字声明。never/on-request 审批独立；author danger 下按检测型 fileChange 纪律处理，不声称事前预防。

#### Scenario: danger-full-access 请求
- **WHEN** LC Codex 的有效 projection 使用 danger-full-access
- **THEN** 系统 SHALL 在其余准入事实与角色合同满足后允许启动，不返回旧 codex_danger_full_access_unsupported

#### Scenario: Codex Coding 受限权限未证实
- **WHEN** 旧受限档的 target-only/read-only 证据未成立，但当前宿主全开合同的版本、trust、D4 与真实执行证据齐备
- **THEN** 系统 SHALL 使用 danger-full-access 校验放行，不因废止的受限权限证明门拒绝；真实当前证据不足仍失败关闭，不换 cwd/provider

#### Scenario: Codex Planning/Review
- **WHEN** Codex 执行 Planning 或 Review，包括 Direct 与 LC
- **THEN** 系统 SHALL 使用 danger-full-access 与独立审批；author 在 fileChange 事件后中断 turn、失败关闭并 durable 审计，真机 danger+apply_patch 的 on-request/never 审批与事件观察按 REQ-ENV-09 验收；Reviewer 全工具面并受 B 档采信约束，不写 Decline 强制

#### Scenario: 旧受限档不得复活
- **WHEN** 旧配置、恢复记录或旁路请求 read-only/workspace-write 作为平台执行基线
- **THEN** 系统 SHALL 要求按新基线重新装配和核验，不静默沿用旧受限档、不把全开当失败后的宽松 fallback

### Requirement: Provider resume 与冻结指纹

系统 SHALL 仅在 action resume Confirmed、原生同 ID 恢复，且 policy/authority/canonical cwd/target/git-dir/worktree、exact version/dialect、新能力语义、岗位纪律、投影、trust 与适用 Aria MCP bundle 摘要一致时恢复。Kimi 自发现 MCP 非 Aria bundle。缺失/失配不续旧、不静默 fresh，仅用户明确新会话后完整准入；fingerprint 比较不创建会话进程。

#### Scenario: 原生恢复形态
- **WHEN** 请求 Claude resume、Codex thread/resume、Pi session-id/RPC 或 Kimi session/load
- **THEN** 系统 SHALL 验证原生同 ID 与完整冻结事实，不回填请求 ID 冒充确认；Reviewer 每次执行还须满足有效窗口合同；仅在对应 action resume=Confirmed 且原生 session id、版本/dialect、projection 与全部冻结指纹一致时恢复；四家 adapter SHALL 在 spawn 前按原生 session id 比对 durable `provider_start.lc_projection`/v2 resume 材料，缺失或任一摘要漂移即对被取代旧 run 追加 `superseded_policy_drift` 并以 typed 错误拒绝（零 child、零新 `provider_start`），不得清 resume id 转 fresh；Kimi ACP `session/load` 与其余三家同受此门约束，不因该 provider 的 Aria 注入 MCP bundle digest 校验惰性而豁免

#### Scenario: Resume capability 未确认
- **WHEN** resume 为 Unknown/Denied
- **THEN** 系统 SHALL 在会话 child 前拒绝，不伪称恢复或静默 fresh

#### Scenario: Fingerprint 漂移
- **WHEN** 任一冻结事实与旧会话失配，包括从旧隔离模型切换到全开基线
- **THEN** 系统 SHALL 将旧会话 superseded 并要求明确新会话操作；fingerprint 判定不创建 provider 会话，不以切换全开默许旧隔离指纹

### Requirement: 按角色守卫 tool policy 并同步 preflight

四家 provider SHALL 在 Direct/LC、同步/流式及 fresh/resume/retry 以受信角色合同守卫：author 按 REQ-ENV-09 落实岗位纪律（Codex danger 为检测型），Reviewer/Coder 全工具面且 Reviewer 有 B 档窗口。不得按自报角色升级或补回旧 reviewer deny；Kimi ACP policy 表达相同职责。preflight SHALL 与 gateway 同源，聚合所有可确定违规及 capability/projection 引用，gateway_required=false 不得绕过。

#### Scenario: 规划与评审角色必带写禁策略
- **WHEN** Claude/Codex/Pi 以 Orchestrator、WorkItemSplitter 或 Reviewer 启动
- **THEN** 系统 SHALL 按实际 author 职责落实岗位纪律，不再把三个角色一概要求 DenyFileWriteBuiltins；Reviewer 不带旧 deny，Codex author 用检测型 fileChange 纪律，Kimi 亦不维持旧角色恒拒

#### Scenario: Executor 与 Handoff 不得带通用 deny
- **WHEN** Executor/Coder 或 Handoff 启动
- **THEN** 系统 SHALL 不带旧通用 deny；若受信职责实际为 author，按 author 岗位合同而非角色字符串整体禁用 terminal/MCP；无身份不允许自报升级

#### Scenario: preflight 与 gateway 同一判定面
- **WHEN** 汇聚 provider/action、trust、投影、Codex 模式、新版 write_boundary、岗位与窗口合同
- **THEN** preflight SHALL 与 gateway 逐项同源并聚合 role/provider/action/reason 与证据引用，失配按漂移拒绝，不能沿用 Reviewer 必带 deny 的旧预检

#### Scenario: 工具策略角色不匹配
- **WHEN** author 的岗位限制缺失/非法，或 Reviewer/Coder 被补入旧只读/写工具 deny，或执行上下文不受信
- **THEN** 系统 SHALL 在 spawn 前给出可诊断错误，不静默删空岗位策略，也不恢复旧 reviewer deny

#### Scenario: Role-chain 有多个违规项
- **WHEN** 多个角色缺少映射、版本能力、trust、投影、执行/窗口合同或 adapter
- **THEN** preflight SHALL 一次返回全部可确定违规及引用，不启动 provider

#### Scenario: 禁用 gateway 标志的 LC 请求
- **WHEN** LC role-chain 带 gateway_required=false
- **THEN** 系统 SHALL 仍执行完整 gateway/preflight，不直连绕过

### Requirement: 四家真实 LC 会话验收证据

四家支持结论 SHALL 按 Story、Design、Plan/split、Coding、Review 与 fresh/resume 分别建立真实证据，记录 CLI exact version、原生参数、cwd/target、角色/action、政策/能力/投影/trust、审批及 pre/post 结果。宿主写可达性 SHALL 如实标记为全开，不冒称只读/target-only。Reviewer 还须证明快照恢复、通道锁与对账违规作废，并在证据中声明对账域为 tracked 与 untracked 非 ignored 文件、ignored 产物不在对账域，不得声称零写覆盖全部文件；缺证据格保持 Unknown/Denied。

#### Scenario: 单 provider/action E2E 通过
- **WHEN** 对应 action 完成真实会话并有完整新语义证据
- **THEN** 系统 SHALL 记录可追溯能力状态，fresh 与 resume 不互相替代；Coder 可自跑构建/测试/容器 CHECK，author 工具岗位规则独立验收

#### Scenario: E2E 越界或证据缺失
- **WHEN** 缺失必需审计/快照/真实能力证据，或失败关闭路径却创建会话
- **THEN** capability SHALL 不标记 Confirmed；Reviewer 自提交/.git/.aria 违规 SHALL 使该轮作废，恢复成功不改变违规事实；宿主普通写成功本身不再作为全开能力失败

