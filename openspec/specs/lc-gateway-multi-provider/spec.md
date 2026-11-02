# lc-gateway-multi-provider Specification

## Purpose

为 LC 的 Story、Design、Plan、Coding 与 Review 会话提供四家 provider 一致的证据驱动准入、权限投影、恢复与审计契约；证据不足时在进程启动前失败关闭。

## Requirements

### Requirement: 显式四家 provider gateway 映射

系统 SHALL 为 Claude Code、Codex、Pi、Kimi Code 显式解析 ProviderRefType、ProviderDialect 与 wire dialect，并为每个 provider/action 使用 durable capability 证据；Fake、未知值及未来 provider SHALL NOT 回落到 Claude、legacy direct path 或其它 provider。每个 action 行 SHALL 独立记录 launch、resume、write_boundary 的 Confirmed、Denied(reason) 或 Unknown 状态，以及 exact version、dialect、projection digest、证据引用与 provenance。旧记录缺少此矩阵时 SHALL 解释为 Unknown，不得由旧 supported_actions 或 provenance 自动升级。

#### Scenario: 显式 provider 映射
- **WHEN** LC 请求由 Claude Code、Codex、Pi 或 Kimi Code 启动
- **THEN** 系统 SHALL 使用该 provider 的显式 ref、dialect、wire dialect 和 adapter；未知 provider 与 Fake SHALL 在 spawn 前返回 provider_unsupported_for_gateway_launch

#### Scenario: 未知 capability 阻断
- **WHEN** provider/action 的 launch 或 write_boundary 为 Unknown/Denied，或证据版本与当前 CLI 不符
- **THEN** 系统 SHALL 在 provider spawn 前拒绝并返回对应诊断，不写入成功 capability、不切换 provider

#### Scenario: Fresh 与 resume capability 分离
- **WHEN** provider/action 的 launch 已 Confirmed、resume 为 Unknown，且用户请求 fresh session
- **THEN** fresh launch MAY 继续校验其余门禁后启动；当用户明确请求 resume 时系统 SHALL 因 resume 非 Confirmed 在 spawn 前拒绝，且不得静默改为 fresh

#### Scenario: 旧隔离证据迁移
- **WHEN** 读取缺少矩阵、缺少新语义标识或依旧只证明只读/target-only 的历史能力记录
- **THEN** 系统 SHALL 保留历史审计，按新语义未验证处理，并提供真实重验证；不得由旧 supported_actions、provenance 或旧 Confirmed 直接放行

#### Scenario: 明确废止 Unknown 迁移放行桥
- **WHEN** write_boundary 为 Unknown，且 legacy supported_actions 或旧 allow 列表包含本 action
- **THEN** 系统 SHALL 仍拒绝 validate 与 spawn 复验，仅当前语义 Confirmed 放行；legacy 必须真实重探针铸证，无迁移默认 allow，不以“不铸 Confirmed 但先允许启动”绕过本门

### Requirement: LC admission 与 spawn 前复验

每个LC provider会话 SHALL 使用manifest provider_context_root canonical path作为process working_directory并传独立native target。workspace Story/Design/Plan的PlanningReadOnly native target SHALL 按REQ-PLN-08固定aggregate-root，业务材料/compile target保留原成员规则；Coding target仍member checkout/worktree。validate与真实会话spawn前 SHALL 分别重新核launch、新版write_boundary、适用resume、authority/policy/target/cwd/trust/投影/adapter/availability、适用D4与受信执行合同；Reviewer还须有效B档窗口。漂移拒绝，不裸input/legacy bridge/provider fallback。

#### Scenario: validate 后 authority 或投影漂移
- **WHEN** validate后、会话spawn前任一冻结事实、执行基线或Reviewer窗口有效性变化
- **THEN** 系统 SHALL 返回可审计PolicyDrift/ProviderUnavailable或窗口诊断，adapter session-start与会话child为零；版本观测单独遵下述60秒TTL合同，不缓存其它事实，不把版本观测当新会话

#### Scenario: LC Coding target 独立于 root cwd
- **WHEN** LC Coding以member worktree为target请求启动
- **THEN** process cwd SHALL 为canonical root，逻辑唯一交付写根恰为target；聚合root不成为Coding写根，target不替cwd，该写根不宣称限制宿主进程物理可达范围

#### Scenario: 同步与流式入口
- **WHEN** LC规划/split/Coding/Review入口请求真实provider
- **THEN** 同步/流式 SHALL 经同validated gateway policy；不支持/材料缺失fail-closed，不以本恢复功能或全开基线绕过真实启动合同替无native合同的同步split启动streaming fresh

#### Scenario: PlanningReadOnly aggregate native 具体化
- **WHEN** workspace Story/Design/Plan处于PlanningReadOnly且同一authority/snapshot有效
- **THEN** native envelope target SHALL 为canonical aggregate-root、writable roots为空，business repository/material/IR target保持原有效成员规则；此为原成员target表述的规划具体化，不影响Coding

#### Scenario: 双阶段能力复验不再验证禁止宿主写
- **WHEN** validate 或 spawn 复验消费 write_boundary Confirmed
- **THEN** 系统 SHALL 消费当前宿主全开语义证据，不再要求 root/非 target/.git/.aria 的宿主写尝试被 OS 拒绝；未知新证据仍拒绝，Reviewer 违规另由 B 档对账使本轮作废

### Requirement: 可验证的 provider policy projection

系统 SHALL 从不可由caller伪造的gateway完整projection将action/role、岗位纪律、permission/approval、宿主执行基线、配置/MCP、trust、cwd/native target与逻辑roots映射到真实CLI/RPC/ACP权限，并记录精确digest。LC workspace在session spawn前 SHALL 准备候选full projection，与旧full冻结基准比较，spawn时审计/复验同源；logical roots不是OS隔离证明，新版write_boundary SHALL 验证全开执行及岗位/窗口合同被真实验证，不以宿主写成功判能力失败，不以空常量域basis或registry未知version冒充full材料。

#### Scenario: Projection 与逻辑写根一致
- **WHEN** gateway为provider/action生成projection
- **THEN** adapter SHALL 仅消费绑定envelope/provider exact version/dialect/role/native target的projection，audit可追digest/evidence；业务target不能污染规划native聚合身份

#### Scenario: Boundary 证据缺失
- **WHEN** 宿主全开执行与岗位/窗口合同缺当前执行基线的真实能力证据
- **THEN** write_boundary SHALL 为Unknown/Denied并阻断action，不以空roots、MCP成功、git status、模型自述或旧隔离探针替证据

#### Scenario: candidate 与 native 审计同源
- **WHEN** workspace准备fresh或resume candidate
- **THEN** current version/trust/config/MCP/boundary材料 SHALL 来自同provider真实只读facts，候选full fingerprint与spawn/audit投影一致；不得以空常量域basis或registry未知version冒充full材料，不将旧basis与full混比

### Requirement: Codex 仅受限安全模式

LC gateway SHALL 永久禁止 Codex `danger-full-access`。Planning/Review 仅可使用经当前版本验证的 `read-only` sandbox 与 `on-request` approval；Coding 仅可在 `workspace-write`、process cwd=root、protocol cwd=target、实际写边界 target-only（以及明确允许的临时目录）、trust 与 D4 均得到当前版本证据时放行。任何证据缺失、配置漂移或协议 cwd 不兼容 SHALL fail-closed；UI、resume、permission mode 不得覆盖此硬门。

#### Scenario: danger-full-access 请求
- **WHEN** LC Codex projection 或既有配置请求 danger-full-access
- **THEN** 系统 SHALL 返回稳定错误 `codex_danger_full_access_unsupported` 并在 spawn 前拒绝

#### Scenario: Codex Coding 受限权限未证实
- **WHEN** workspace-write 的实际可写集合、protocol cwd=target、trust、D4 或版本证据不完整
- **THEN** 系统 SHALL 拒绝 Codex Coding，不得回退 danger-full-access、换 cwd 或换 provider

#### Scenario: Codex Planning/Review
- **WHEN** LC Codex 执行 Planning 或 Review
- **THEN** 仅在当前版本确认 read-only + on-request、工具/审批 wire 与 boundary evidence 后启动

#### Scenario: 旧受限档不得复活
- **WHEN** 旧配置、恢复记录或旁路请求 read-only/workspace-write 作为平台执行基线
- **THEN** 系统 SHALL 要求按新基线重新装配和核验，不静默沿用旧受限档、不把全开当失败后的宽松 fallback

### Requirement: Provider resume 与冻结指纹

系统 SHALL 仅在action resume=Confirmed、原生恢复真实同ID且policy/authority/canonical cwd/native target/git/worktree/provider exact version/dialect、新能力语义、岗位纪律、tool policy、full projection、trust与适用Aria MCP digest一致时恢复LC旧session。Kimi自发现MCP不是Aria bundle。不支持/失配/缺完整旧冻结记录 SHALL 不续旧、不静默fresh；用户明确新会话后才完整fresh admission。比较自身不创建会话进程，有界只读--version按下述独立观测合同。

#### Scenario: 原生恢复形态
- **WHEN** 请求Claude --resume、Codex thread/resume、Pi session-id/RPC或Kimi session/load
- **THEN** resume能力/版本/dialect/fullprojection指纹 SHALL 一致且原生确认同ID，不回填请求ID伪确认；Reviewer每次执行还须满足有效窗口合同

#### Scenario: Resume capability 未确认
- **WHEN** resume能力为Unknown/Denied
- **THEN** 系统 SHALL 会话child前typed拒收，不伪称恢复或静默fresh

#### Scenario: Fingerprint 漂移
- **WHEN** policy/cwd/native target/trust/capability/tool/projection/适用MCP与旧冻结事实失配，包括从旧隔离模型切换到全开基线
- **THEN** 系统 SHALL 标记旧native superseded并要求明确fresh，adapter session-start/session child为零；同一revision driver不继续start_streaming，不以切换全开默许旧隔离指纹

#### Scenario: 缺完整旧 map
- **WHEN** native/audit/旧运行存在但fullmap不可验证或basis不兼容
- **THEN** 系统 SHALL 耐久waiting＋显式fresh，不用当前fingerprint覆旧会话，不借audit兜底捞已superseded ID

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

### Requirement: LC 版本只读观测与60秒缓存

LC当前exact-version观测 SHALL 允许有界只读CLI --version，进程内成功结果TTL固定60秒且非滑动；同key并发miss/到期 SHALL single-flight一条观测。缓存仅版本事实，key隔离provider/命令canonical executable identity/参数/supplier域；失败/超时/取消不缓存，不回退过期成功。probe不是会话进程，仍与adapter start/session child分别计数；TTL更长防护更钝，不默改参数。

#### Scenario: 并发 burst 压一条
- **WHEN** 同key当前版本缓存缺失/到期，多caller同时需要观测
- **THEN** 系统 SHALL 只执行一条有界version probe，共享同结果；不同key互不串行、不串值；不因观测创建native会话

#### Scenario: 非滑动TTL边界
- **WHEN** 成功观测完成后age小于60秒或达到60秒
- **THEN** age<60秒 SHALL 复用而不延长到期，age>=60秒 SHALL 再观测；进程重启不复用旧缓存，命令identity变化失效；最多60秒版本变化识别滞后如实呈现

#### Scenario: 失败与取消不保留过期值
- **WHEN** 观测失败/超时/取消
- **THEN** 系统 SHALL 不缓存失败、不使用已到期旧成功，不把候选当准入通过；caller取消后不会复活已拒绝旧run

#### Scenario: 三种spawn口径
- **WHEN** 验收mismatch或native握手失败
- **THEN** 证据 SHALL 分列version probe、adapter session-start、session child/native创建；mismatch后两项为零，握手后错ID须kill/reap且不得冒称零child；direct原版本cache行为不改
