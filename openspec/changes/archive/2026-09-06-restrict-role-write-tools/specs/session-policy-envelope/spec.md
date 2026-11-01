# session-policy-envelope Delta — restrict-role-write-tools

## MODIFIED Requirements

### Requirement: 两层政策结构（REQ-ENV-01）

系统 SHALL 持久化 `AggregatePolicyArtifact`（集中政策正文 + digest + revision）作为事实来源；每次 provider run 生成不可变 `SessionPolicyEnvelope`（policy_id/revision/digest、action、target、read/write roots、provider dialect、config artifact 引用）。本契约覆盖逻辑代码库流程的**全部真实 provider 启动入口**：同步栈 `ProviderAdapter::run(AdapterInput)`（含 work-item split 引擎直接调用）与流式栈 `StreamingProviderAdapter::start(StreamingProviderInput)`（含聚合初始化、聚合规划 provider_drive、coding provider_stream、review）。**例外（本 change 修订，显式过渡例外；范围=流式栈）**：既有 legacy 直连入口——流式栈 workspace/coding 引擎的 author/revision/review 直连 `provider.start`（Story/Design/SingleCandidate(SC) 首轮与修订、WorkItemPlan legacy、coding reviewer 直连）**与 coding Coder(Executor) 直连**——允许保留现状拓扑；此类启动必须满足：（a）输入由 engine builder 工厂构造并按角色携带 REQ-ENV-09 工具策略（Reviewer 角色带策略，Executor/Coder 不带）；（b）受 adapter 层双向角色守卫保护；（c）接受按角色适用的启动审计——**策略角色（作者/评审）的 pi/claude/codex 启动接 REQ-ENV-09 的 durable_tool_policy_audit 分区；Executor/Coder 与非策略路径、kimi（全部角色）以既有 execution_event_audit 通道为等价审计（role/provider-specific 既有形态豁免，kimi 零改动为用户裁决）**。**同步栈（AdapterInput 直连）不适用本例外**，维持本 requirement 原文要求（逻辑仓既有 gateway guard fail-closed；非逻辑仓保留 API 无生产调用方，重激活前置=REQ-ENV-09 策略绑定）。全量 gateway 迁移为后续独立 change 的路线项，完成前本例外持续有效。

#### Scenario: 启动逻辑代码库 provider run

- **WHEN** 逻辑代码库流程启动任何真实 provider run（规划/编码/评审/初始化；本 requirement 例外入口除外）
- **THEN** envelope SHALL 由 resolver 从 policy artifact 解析并校验，缺省或不一致时 fail-closed 拒绝启动

#### Scenario: legacy 直连例外启动

- **WHEN** workspace 引擎 author/revision/review 以 legacy 直连启动 provider
- **THEN** 该启动 SHALL 携带经 builder 工厂设置的角色工具策略（REQ-ENV-09），通过 adapter 守卫，并写入 durable 启动审计；不得以裸输入绕过策略

#### Scenario: coding Coder 直连例外启动

- **WHEN** coding Coder（Executor）以 legacy 直连启动 provider
- **THEN** 该启动 SHALL 经 engine builder 工厂构造（Executor 禁带工具策略，REQ-ENV-09；携带策略即拒），通过 adapter 双向守卫，并以既有 execution_event_audit 通道接受按角色适用的等价启动审计（非 durable_tool_policy_audit 分区）；不得以裸输入绕过 builder 工厂

### Requirement: 适配器只接受 validated launch policy（REQ-ENV-02）

系统 SHALL 使逻辑代码库流程的真实 provider adapter 只接受经 `LogicalCodebaseProviderGateway` 构造的 `ValidatedSessionLaunchPolicy` 启动；关闭真实 provider 的无政策 fallback（legacy `run_streaming` 默认 bridge、coding retry `allow_legacy_stream_fallback: true`）与裸 `StreamingProviderInput`/`AdapterInput` 直接启动，逻辑代码库调用固定 `allow_legacy_stream_fallback=false`；Fake/测试路径经 registry 分层或编译期构造限制隔离，不依赖运行时 `if provider != Fake`。**例外（本 change 修订）**：REQ-ENV-01 定义的 legacy 直连入口不受「必须经 gateway 构造 ValidatedSessionLaunchPolicy」约束，但必须满足 REQ-ENV-01 例外条款的（a）（b）（c）三项替代约束；除该例外外，禁止任何裸 input 直启与 legacy fallback。

#### Scenario: 尝试无政策启动

- **WHEN** 逻辑代码库流程试图以裸 input 或 legacy fallback 启动真实 provider（REQ-ENV-01 例外入口除外）
- **THEN** 系统 SHALL 拒绝并返回 fail-closed 错误；Fake/测试路径被类型或 registry 层隔离

#### Scenario: 例外入口缺失策略

- **WHEN** legacy 直连例外入口的作者/评审角色启动未携带 REQ-ENV-09 策略
- **THEN** adapter 守卫 SHALL 在 spawn 前拒绝（fail-closed）

### Requirement: 配置来源隔离（REQ-ENV-06）

系统 SHALL 使逻辑代码库流程的真实 provider 使用 Aria-owned、权限受控的 settings/MCP bundle；审计 user/project/local/env/子仓 MCP 合并优先级，隔离未批准的配置与凭据；记录最终 argv 与配置 digest 到 run 审计；托管 settings（managed-settings）优先级高于 Cadence 注入时列为已知 gap。**例外（本 change 修订，用户裁决 2026-09-04）**：provider **自发现通道**——provider CLI 原生读取的项目级/用户级配置（项目 `.mcp.json`、`.kimi-code/mcp.json`、`.codex/config.toml` 等）——为受信任通道，**不受 Aria bundle 管控与配置来源隔离约束**；该通道的配置内容由用户负责，其带来的配置来源审计弱化为显式接受的边界。Aria 主动注入场景（注入 settings/MCP bundle 时）的管控、脱敏与审计要求维持不变。

#### Scenario: 真实 provider 启动

- **WHEN** 逻辑代码库流程启动真实 provider 且 Aria 注入配置
- **THEN** 使用经校验的 Aria 配置产物，最终参数与配置 digest 写入 run 审计记录；未批准配置/凭据不注入；检测 `/status` Setting sources 是否含 managed settings，含时在 run 审计显式标注「managed-settings 活跃，Aria 注入可能被覆盖」

#### Scenario: provider 自发现通道

- **WHEN** provider CLI 经自身配置发现机制加载 MCP server（不经 Aria 注入）
- **THEN** 该通道 SHALL NOT 受本 requirement 的 bundle 管控；系统不对其配置来源作隔离或审计声明

### Requirement: kimi ACP mcpServers 走 envelope 受控注入（REQ-ENV-08）

系统 SHALL 使逻辑代码库流程的 kimi provider 真实会话（`session/new` 与 `session/load`）的 `mcpServers` 参数来自 Aria-owned、权限受控的 settings/MCP bundle：allowlist 校验、配置 digest 记录、凭据脱敏、argv 审计；无 bundle 时为空数组（argv 与 digest 记入既有 run 审计通道=execution_event_audit 体系，非 tool-policy 分区）。resume 时（Aria 注入的）bundle digest 不一致 SHALL 拒绝。**例外（本 change 修订）**：kimi CLI 原生自发现（项目级 `mcp.json`，经用户一次性交互 Trust 后自动加载）属 REQ-ENV-06 例外条款定义的受信任自发现通道，不受本 requirement 管控；本 requirement 的 digest 漂移拒绝仅适用于 Aria 注入的 `mcpServers`。

#### Scenario: kimi 注入受控 bundle

- **WHEN** kimi 真实会话初始化且 envelope 提供 bundle
- **THEN** `mcpServers` 由 bundle 派生，argv 与 digest 记入 run 审计；无 bundle 时为空数组

#### Scenario: kimi 自发现不受控

- **WHEN** kimi CLI 经原生机制加载项目级 MCP 配置
- **THEN** 该加载 SHALL NOT 受本 requirement 管控或拒绝

#### Scenario: resume digest 漂移拒绝

- **WHEN** `session/load` 时（Aria 注入的）bundle digest 与冻结值不一致
- **THEN** 拒绝加载并报告差异，启动新会话且旧会话标记 superseded（对齐 REQ-ENV-04）

## ADDED Requirements

### Requirement: 非编码角色的 built-in 文件写工具拒绝（REQ-ENV-09）

系统 SHALL 按角色施加经校验的工具黑名单策略（本期矩阵：作者家族与评审家族=拒绝 built-in 文件写工具；编码 Coder=无黑名单；聚合初始化 provider turns=Executor 无黑名单）：计划作者家族会话（Story/Design/SingleCandidate(SC) 首轮 author、author feedback revision、WorkItemPlan author 普通/fresh/resume/serial/batch）与评审家族会话（Story/Design reviewer、SC reviewer、review repair、coding CodeReviewer/InternalReviewer、group review）的 **built-in 文件写工具与 built-in 文件写升级审批** SHALL 不可达/被拒绝；Coder 与聚合初始化 SHALL 维持既有全工具面。**保护范围声明**：本 requirement 保护的是 built-in 工具面；bash/terminal 与 provider 自发现 MCP 工具为**显式信任逃逸面**（MCP 工具可能具备写能力，经 REQ-ENV-06 例外条款由用户裁决信任），不在本 requirement 保护范围内。策略以语义意图表达、经各 provider translator 翻译为 canonical 物理片段在启动时过滤；对 pi/claude/codex，黑名单之外的既有非写工具面 SHALL NOT 因本策略被禁用（读取、bash/terminal 只读命令、自发现 MCP/extension 工具、`ask_user`）。各 provider permission 映射、ApprovalBridge 语义与 driver choice 兜底不变。**术语**：本 requirement 中「审计」区分 `durable_tool_policy_audit`（策略会话 JSONL 分区）与 `execution_event_audit`（既有非 durable 通道，用于 Coder/非策略路径与 kimi）；未注明种类时指 durable_tool_policy_audit。

#### Scenario: 非编码角色启动时 built-in 写工具不可用

- **WHEN** 任一 provider 的作者或评审会话启动（含首轮、修订、fresh、resume；含 legacy 直连例外与经 gateway 传递到 adapter 的启动）
- **THEN** pi SHALL 注入 `--exclude-tools edit,write`；claude code SHALL 注入 `--disallowedTools Edit,Write,NotebookEdit`；codex 直连路径的 thread/start 与 thread/resume SHALL 携带 `read-only` 沙箱与 `on-request` 审批并按「codex 审批分类」场景决策；kimi SHALL 维持既有 client services 角色策略（Orchestrator `FsWrite` 无条件拒绝、Reviewer 只读、WorkItemSplitter 全拒，均为既有形态，不收紧也不放宽）

#### Scenario: codex 审批分类

- **WHEN** codex 会话收到审批请求
- **THEN** 系统 SHALL 按 wire 形态与确定性应答表分类（审计通道逐类标注）：MCP 工具调用（`_meta.codex_approval_kind="mcp_tool_call"`，属自发现信任通道）SHALL 在所有会话（含 Coder）自动批准——策略会话记 `approval_decision` 于 durable_tool_policy_audit，Coder 记 execution_event_audit；策略会话（作者/评审）的 `commandExecution`/`fileChange` 审批 SHALL 拒绝并记 durable_tool_policy_audit；Coder 的 `commandExecution`/`fileChange` 维持 ApprovalBridge 既有上抛链路（execution_event_audit）；未知形态 SHALL 按应答表以协议合法动作拒绝（elicitation 形态→JSON-RPC error `-32601`+data、item 形态→`{"decision":"decline"}`）并记 `protocol_warning`（策略会话→durable 分区；Coder→execution event 通道）；同一会话连续 ≥3 次未知形态 SHALL 终止会话（`session_terminated`，reason_code=`unknown_approval_storm`，落点同 `protocol_warning` 规则）；任何形态 SHALL NOT 静默不应答
- **AND** ApprovalBridge 既有审批语义 SHALL 零变化

#### Scenario: 策略缺失或非法 fail-closed（双向守卫）

- **WHEN** pi/claude/codex 上作者/评审角色（Orchestrator/WorkItemSplitter/Reviewer）会话启动时策略缺失、为空、携带非支持意图，或 Executor/Handoff 角色会话携带策略
- **THEN** 系统 SHALL 在 provider 进程 spawn 前拒绝启动并返回 fail-closed 错误；不得静默降级为无限制工具面

#### Scenario: resume 与修订策略一致（冻结记录比对）

- **WHEN** 作者或评审会话经修订或 resume 路径重启（含携带既有 session id）
- **THEN** 系统 SHALL 在 spawn 前按 provider_session_id 在该 workspace 会话审计分区内检索最近 `provider_start` 记录，比对 tool_policy canonical digest、provider version 与 adapter dialect 三元组；任一不一致或记录缺失时 SHALL 拒绝 resume、标记 superseded 并新建会话。digest 输入=规范前缀 `tp-v1`+provider 名+canonical 物理 token 序列+审批分类规则版本常量；version 于每次启动经 adapter CLI 版本探测获取（不可得→策略会话启动 fail-closed），dialect 为 adapter 代码常量；比较为精确字符串相等

#### Scenario: 启动与审批的 durable 审计

- **WHEN** 策略会话在 pi/claude/codex 上完成启动或作出审批决策
- **THEN** 系统 SHALL 经 engine 提供的审计 sink 向 LifecycleStore `tool-policy-run-audit/` 分区追加 JSONL 记录（文件 key=(workspace_session_id, role_run_seq)，`role_run_seq` 由 engine 按 run 分配且随 run 记录持久化单调；行 seq 文件内单调递增；`provider_start` 恰为首行一条，`approval_decision`/`protocol_warning`/`session_terminated` 任意多条跟随；append 经 sink 内互斥串行化；**时序与 kill 责任：adapter 须在 `start` 返回前完成启动握手并确认 native session id（pi=预生成/传入，claude=resume 已知·fresh 等首个 init 事件有界超时，codex=thread/start 应答；确认失败→启动 fail-closed），`provider_start` 于 start 返回前写入，写失败=adapter 在返回前终止子进程并返回错误；`approval_decision` 等后续事件写失败=经 engine 既有 provider task kill 链终止会话**）：`provider_start`（最终 argv/沙箱+审批参数原文、canonical 片段与 digest、workspace_session_id、provider_session_id[启动后取自 adapter 原生会话 id]、provider version/dialect）与 `approval_decision`（category/server_name/tool_name/request_id/decision/reason_code/policy_digest）；`provider_start` 追加失败 ⇒ adapter 返回错误并沿 engine provider task 既有 kill 链终止会话、run 判失败（gateway 与直连路径一致）；读取端坏行=跳过+`protocol_warning`，写入端失败=错误传播；系统 SHALL NOT 声称 pre-spawn 审计。Coder/非策略会话的审批决策走既有 execution event 通道（非 durable 分区）

#### Scenario: Coder 与非策略路径零变化

- **WHEN** coding Coder 或聚合初始化（Executor）启动
- **THEN** SHALL 维持既有全工具面与沙箱/审批档位（无黑名单注入，codex 维持 `danger-full-access` 与 permission_mode 映射）；commandExecution/fileChange 审批既有 bridge 链路零变化
- **AND** 行为改进边界（仅此两处非零变化，回归锁定）：策略会话与全角色的 MCP/fileChange/未知审批从现状静默不应答改为按应答表应答+事件（MCP=accept+审计；fileChange=策略会话 decline/Coder 走 bridge；未知=拒绝应答+`protocol_warning`）

#### Scenario: kimi 既有对齐维持

- **WHEN** kimi 作为任意角色 provider 启动
- **THEN** 其 client services 策略 SHALL 保持现状；本 change SHALL NOT 收紧或放宽 kimi 任何既有工具面（kimi 不接 tool-policy 审计分区与 argv 注入，为 provider-specific 既有形态例外）
