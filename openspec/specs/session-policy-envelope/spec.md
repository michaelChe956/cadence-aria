# session-policy-envelope Specification

## Purpose

为逻辑代码库流程的每次 provider run 冻结集中政策/目标/配置快照；真实 adapter 只接受 validated launch policy；路由级 fail-closed；禁止无政策 fallback（experimental + supervised）。

## Requirements

### Requirement: 两层政策结构（REQ-ENV-01）

系统 SHALL 持久化 `AggregatePolicyArtifact`（集中政策正文 + digest + revision）作为事实来源；每次 provider run 生成不可变 `SessionPolicyEnvelope`（policy_id/revision/digest、action、target、read/write roots、provider dialect、config artifact 引用、canonical `working_directory`）。本契约覆盖逻辑代码库流程的**全部真实 provider 启动入口**：同步栈 `ProviderAdapter::run(AdapterInput)`（含 work-item split 引擎直接调用）与流式栈 `StreamingProviderAdapter::start(StreamingProviderInput)`（含聚合初始化、聚合规划 provider_drive、coding provider_stream、review）。**例外（显式过渡例外；范围=流式栈）**：既有 legacy 直连入口——流式栈 workspace/coding 引擎的 author/revision/review 直连 `provider.start`（Story/Design/SingleCandidate(SC) 首轮与修订、WorkItemPlan legacy、coding reviewer 直连）**与 coding Coder(Executor) 直连**——允许保留现状拓扑；此类启动必须满足：（a）输入由 engine builder 工厂构造并按角色携带 REQ-ENV-09 工具策略（Reviewer 角色带策略，Executor/Coder 不带）；（b）受 adapter 层双向角色守卫保护；（c）接受按角色适用的启动审计——**策略角色（作者/评审）的 pi/claude/codex 启动接 REQ-ENV-09 的 durable_tool_policy_audit 分区；Executor/Coder 与非策略路径、kimi（全部角色）以既有 execution_event_audit 通道为等价审计（role/provider-specific 既有形态豁免，kimi 零改动为用户裁决）**。**同步栈（AdapterInput 直连）不适用本例外**，维持本 requirement 原文要求（逻辑仓既有 gateway guard fail-closed；非逻辑仓保留 API 无生产调用方，重激活前置=REQ-ENV-09 策略绑定）。全量 gateway 迁移为后续独立 change 的路线项，完成前本例外持续有效。

逻辑代码库任一真实 provider run（规划、编码、评审或初始化）使用的 policy/target 引用 SHALL 由唯一 LC authority resolver 解析并校验，且 SessionPolicyEnvelope SHALL 冻结 policy_id、revision、digest、target identity、canonical `working_directory` 与 provider capability 快照。逻辑代码库的 `working_directory` SHALL 等于唯一 authority 的 canonical `provider_context_root`；`target`/`worktree` 继续表示本次会话被授权操作的成员、checkout 或 worktree，允许 cwd 与 target 不同；单仓 legacy 入口保留原有 cwd/target 映射。resolver 无法唯一解析、policy 材料缺失、digest/revision 不一致、root identity 漂移或目标跨 kind 时，启动 SHALL 在 provider spawn 前 fail-closed，返回可诊断、可操作的核验/准备/重试等待项；不得回落到成员仓路径、项目级历史布局、旧 pointer 或裸 provider input。用户完成允许操作后，系统 SHALL 复用原始登记/编排链继续，不创建第二套 durable operation 状态机。

#### Scenario: 启动逻辑代码库 provider run

- **WHEN** 逻辑代码库流程启动任何真实 provider run（规划/编码/评审/初始化；本 requirement 例外入口除外）
- **THEN** envelope SHALL 由 resolver 从 policy artifact 解析并校验，冻结 canonical root cwd 与独立 target identity，缺省或不一致时 fail-closed 拒绝启动

#### Scenario: legacy 直连例外启动

- **WHEN** workspace 引擎 author/revision/review 以 legacy 直连启动 provider
- **THEN** 该启动 SHALL 携带经 builder 工厂设置的角色工具策略（REQ-ENV-09），通过 adapter 守卫，并写入 durable 启动审计；LC root cwd 与独立 target 由同一 authority 校验，不得以裸输入绕过策略

#### Scenario: coding Coder 直连例外启动

- **WHEN** coding Coder（Executor）以 legacy 直连启动 provider
- **THEN** 该启动 SHALL 经 engine builder 工厂构造（Executor 禁带工具策略，REQ-ENV-09；携带策略即拒），通过 adapter 双向守卫，并以既有 execution_event_audit 通道接受按角色适用的等价启动审计（非 durable_tool_policy_audit 分区）；LC root cwd 与 target worktree 独立校验，不得以裸输入绕过 builder 工厂

#### Scenario: LC 冷启动生成有效 envelope

- **WHEN** 逻辑代码库完成 authority、manifest/checkout、root rules/policy 与成员 index 所需材料并请求 provider run
- **THEN** resolver SHALL 生成包含 canonical root working directory、稳定目标、政策 revision/digest 和 capability 快照的 envelope，provider 仅能以该校验过的 envelope 启动

#### Scenario: LC root-cwd 生成有效 envelope

- **WHEN** 逻辑代码库启动规划、编码、评审或初始化 provider run
- **THEN** envelope SHALL 冻结 canonical root working directory、稳定 target identity、政策 revision/digest 与 provider capability 快照，且允许 cwd 与 target 不同；provider 仅能以校验过的 envelope 启动

#### Scenario: policy 或 authority 不一致时拒绝启动

- **WHEN** canonical root、policy digest、target identity、规则引用或 capability 与准入快照不一致
- **THEN** provider SHALL 零启动并返回可审计等待项；不得回落成员仓路径、项目级历史布局、旧 pointer 或裸 provider input

#### Scenario: 过渡审计与同步边界
- **WHEN** 过渡 Direct 入口启动，或保留的同步 API 拟重新激活
- **THEN** author 的 pi/claude/codex 岗位策略 SHALL 使用 durable_tool_policy_audit；Coder/非岗位策略与 Kimi 使用既有 execution_event_audit，Reviewer 另有 B 档持久窗口记录；同步 LC 仍须 validated gateway，非 LC 同步 API 激活前也须绑定新岗位/执行合同，过渡例外不得扩张

### Requirement: 适配器只接受 validated launch policy（REQ-ENV-02）

LC 真实 adapter SHALL 仅接受经 gateway 校验的 launch policy，禁止裸输入与无政策 legacy fallback；逻辑代码库固定关闭 fallback，Fake/测试经类型或 registry 隔离。REQ-ENV-01 既有流式例外仅以受信 builder、更新后的岗位守卫与等价审计代替 gateway 构造，不得成为全开无政策启动。

#### Scenario: 尝试无政策启动
- **WHEN** 非过渡例外的 LC 入口以裸 input 或 legacy fallback 请求真实 provider
- **THEN** 系统 SHALL 失败关闭，Fake/测试路径保持隔离

#### Scenario: 例外入口缺失策略
- **WHEN** legacy author 缺少岗位规则，或任一例外入口缺少受信执行合同、必要审计或 Reviewer 窗口
- **THEN** adapter SHALL 在 spawn 前拒绝；Reviewer 不带旧写工具 deny 本身不构成缺失策略

### Requirement: action 与写根约束（REQ-ENV-03）

系统 SHALL 使 envelope 区分 `PlanningReadOnly` / `CodingTargetWrite` / `ReviewReadOnly`；Planning/Review 的 writable roots 为空，Coding 的唯一 writable root 恰为当前 target worktree。canonical root working directory 是 provider 启动与目录发现边界，不得据此把 coding writable root 扩大为聚合根；root 约束仍是“配置目标 + best-effort”，不宣称 OS 级不可写。

#### Scenario: coding target 与 envelope 不一致

- **WHEN** coding run 的 target、worktree 或独立 working directory 与 envelope 不一致
- **THEN** 启动 SHALL 被拒绝（fail-closed）；spawn 前复验 canonical root、git-dir、worktree identity 与 target snapshot

#### Scenario: 逻辑写根不冒充执行隔离
- **WHEN** 规划/评审 terminal 能写宿主路径或 Coder 能访问 target 外路径
- **THEN** 系统 SHALL 如实报告宿主全开，不将空/单一逻辑 roots 当物理阻断；平台交付范围校验与 Reviewer 违规作废继续有效

### Requirement: resume 一致性（REQ-ENV-04）

系统 SHALL 仅当 policy digest、target/git identity、canonical working directory、provider exact version/dialect、capability snapshot与适用的完整LC projection/trust/tool/MCP冻结事实一致时允许LC resume；任一失配或冻结材料不可验证 SHALL 拒绝旧native、标记superseded并等待明确新会话，只有用户显式fresh后重新完整admission。Planning/Coding/Review/initialization继续原action门，direct单仓语义不变。

#### Scenario: 续接旧 provider session
- **WHEN** 尝试续接LC旧provider session且任一冻结指纹不一致
- **THEN** 系统 SHALL 拒绝旧native并保存superseded waiting，不能静默fresh；fingerprint判定本身无provider会话进程，显式fresh后才重新validate/revalidate

#### Scenario: cwd 或 target 改变时拒绝 resume
- **WHEN** LC canonical root cwd、native target/git identity、policy或capability变化
- **THEN** 系统 SHALL 拒绝旧会话且零adapter session-start/协议child，不以cwd/member fallback放行；PlanningReadOnly的native target按REQ-PLN-08固定aggregate-root，业务材料target独立保持

#### Scenario: 版本观测与会话 spawn 口径
- **WHEN** LC candidate准备或spawn前复验需要当前exact version
- **THEN** 系统 SHALL 允许有界只读CLI --version及60秒非滑动TTL、同键single-flight；版本probe与adapter start/session child SHALL 分别计数，有效缓存age小于60秒、到期再观测，失败/超时/取消不缓存且不使用过期成功，不缓存其它admission事实

#### Scenario: 原生握手后失败如实收口
- **WHEN** 已创建会话child后发现原生恢复ID缺失/错ID或runtime协议失败
- **THEN** 系统 SHALL kill/reap并记录未恢复，不冒称零child或伪回填请求ID；不得自动清旧ID转fresh

### Requirement: 禁止降级（REQ-ENV-05）

系统 SHALL 对缺少当前宿主全开执行证据、岗位/窗口合同、版本、authority 或投影的 provider 路由失败关闭，不切换 provider、不静默换执行模型。Codex danger-full-access SHALL 为四家全开基线而非降级；旧只读/target-only 能力或 declared 来源不等于新版真实验证。capability SHALL 保留 exact version、dialect、证据等级与语义标识。

#### Scenario: 选择 coding provider
- **WHEN** 路由选择的 provider 不满足新执行合同或当前版本能力未验证
- **THEN** 系统 SHALL 阻塞并返回可操作能力缺失，不回落旧受限档、不自动换 provider；证据齐备的 danger-full-access 不得因旧永久拒绝规则阻断

### Requirement: 配置来源隔离（REQ-ENV-06）

系统 SHALL 使逻辑代码库流程的真实 provider 使用 Aria-owned、权限受控的 settings/MCP bundle；审计 user/project/local/env/子仓 MCP 合并优先级，隔离未批准的配置与凭据；记录最终 argv 与配置 digest 到 run 审计；托管 settings（managed-settings）优先级高于 Cadence 注入时列为已知 gap。**例外（用户裁决 2026-09-04）**：provider **自发现通道**——provider CLI 原生读取的项目级/用户级配置（项目 `.mcp.json`、`.kimi-code/mcp.json`、`.codex/config.toml` 等）——为受信任通道，**不受 Aria bundle 管控与配置来源隔离约束**；该通道的配置内容由用户负责，其带来的配置来源审计弱化为显式接受的边界。Aria 主动注入场景（注入 settings/MCP bundle 时）的管控、脱敏与审计要求维持不变。LC root-cwd 的根原生配置不被复制、改写或强制纳入 Aria bundle 审计；但 Codex/Kimi 为可用而执行的 root-specific trust 登记 SHALL 记录其 root、动作与结果，失败 SHALL fail-closed。

#### Scenario: 真实 provider 启动

- **WHEN** 逻辑代码库流程启动真实 provider 且 Aria 注入配置
- **THEN** 使用经校验的 Aria 配置产物，最终参数与配置 digest 写入 run 审计记录；未批准配置/凭据不注入；检测 `/status` Setting sources 是否含 managed settings，含时在 run 审计显式标注「managed-settings 活跃，Aria 注入可能被覆盖」

#### Scenario: provider 自发现通道

- **WHEN** provider CLI 经自身配置发现机制从 LC root 或用户级原生机制加载 MCP server（不经 Aria 注入）
- **THEN** 该通道 SHALL NOT 受本 requirement 的 bundle 管控；系统不对其配置来源作隔离或审计声明，root-specific trust 登记另按 REQ-REG-14 审计

#### Scenario: provider 自发现 root 配置

- **WHEN** provider CLI 从 canonical LC root 或用户级原生机制发现 MCP/rules/skills 配置且未使用 Aria 注入 bundle
- **THEN** 系统 SHALL 允许该受信任自发现通道继续运行，不将其误报为 Aria bundle 注入；root-specific trust 前提与结果可供启动审计关联

#### Scenario: Aria 注入仍受控

- **WHEN** 逻辑代码库 provider 启动使用 Aria 注入 settings/MCP bundle
- **THEN** 原有 allowlist、凭据脱敏、最终 argv 与 digest 审计和 managed-settings gap 标注 SHALL 保持不变

### Requirement: 每仓最小指针（REQ-ENV-07）

系统 SHALL 保留既有向每个成员仓发布极薄 AGENTS.md/CLAUDE.md 最小指针（logical codebase ID、repo ID、canonical policy locator、「未加载集中政策前禁止写」声明）的受控 publication 通道；该通道经独立 worktree/branch 发布并生成 ReviewRequest（非自动 PR），可回滚，处理仓内已有文件的合并。envelope 为权威政策输入，pointer 仅负责发现；pointer 未发布前，既有「完整读取目标根 AGENTS.md/CLAUDE.md」语义为「读取 envelope 校验的聚合政策」。本 root-cwd recipe SHALL NOT 新增、复制或强制发布成员 pointer，也不得将 pointer 作为根规则缺失的 fallback；有效 LC session 的权威规则来源是 canonical root policy/rules 与 envelope，pointer 缺失时继续按既有配置策略阻塞或标记。

#### Scenario: 指针受控发布

- **WHEN** 经既有独立受控通道发布每仓最小指针
- **THEN** 使用独立 branch + ReviewRequest（非污染主 checkout、非自动 PR）；已有 CLAUDE.md/AGENTS.md 时按无冲突策略合并或明确冲突，root recipe 不代替该通道发布

#### Scenario: 指针缺失

- **WHEN** 目标仓最小指针缺失或与政策不一致
- **THEN** coding 由 envelope 校验的聚合政策驱动；指针缺失时按配置策略阻塞或标记，不将指针当作政策正文执行，也不回退成员规则替代根规则

#### Scenario: root-cwd 不发布新成员指针

- **WHEN** 新 LC 完成 root recipe 并准备启动 provider session
- **THEN** 系统 SHALL 直接消费 canonical root 的规则/policy；不得为了 root-cwd discovery 向成员仓复制或新增 thin pointer，成员原有受控 pointer 通道状态不被删除

#### Scenario: 指针缺失不改变 root authority

- **WHEN** 成员 pointer 缺失或与 policy 不一致
- **THEN** coding/planning SHALL 依据 envelope 校验的 root policy 继续按配置策略阻塞或标记，不把成员 pointer 正文当作规则事实源

### Requirement: kimi ACP mcpServers 走 envelope 受控注入（REQ-ENV-08）

系统 SHALL 使逻辑代码库流程的 kimi provider 真实会话（`session/new` 与 `session/load`）的 `mcpServers` 参数来自 Aria-owned、权限受控的 settings/MCP bundle：allowlist 校验、配置 digest 记录、凭据脱敏、argv 审计；无 bundle 时为空数组（argv 与 digest 记入既有 run 审计通道=execution_event_audit 体系，非 tool-policy 分区）。resume 时（Aria 注入的）bundle digest 不一致 SHALL 拒绝。**例外**：kimi CLI 原生自发现（项目级 `mcp.json`，经用户一次性交互 Trust 后自动加载）属 REQ-ENV-06 例外条款定义的受信任自发现通道，不受本 requirement 管控；本 requirement 的 digest 漂移拒绝仅适用于 Aria 注入的 `mcpServers`。Kimi CLI 从 canonical LC root 经 root-specific workspace trust 自发现的通道不被 root-cwd change 拦截；workspace trust 登记与 Aria 注入 bundle 是两条独立事实和审计通道。

#### Scenario: kimi 注入受控 bundle

- **WHEN** kimi 真实会话初始化且 envelope 提供 bundle
- **THEN** `mcpServers` 由 bundle 派生，argv 与 digest 记入 run 审计；无 bundle 时为空数组

#### Scenario: kimi 自发现不受控

- **WHEN** kimi CLI 经原生机制加载已信任 canonical LC root 的项目级 MCP 配置
- **THEN** 该加载 SHALL NOT 受本 requirement 管控或拒绝；root-specific trust 登记按 REQ-REG-14 另行审计

#### Scenario: resume digest 漂移拒绝

- **WHEN** `session/load` 时（Aria 注入的）bundle digest 与冻结值不一致
- **THEN** 拒绝加载并报告差异，启动新会话且旧会话标记 superseded（对齐 REQ-ENV-04）

#### Scenario: Kimi 原生 root MCP 自发现

- **WHEN** Kimi CLI 已登记当前 canonical LC root trust，并从根项目配置自发现 MCP
- **THEN** 系统 SHALL 允许该 native MCP 通道运行，不要求把配置复制为 Aria bundle，也不得将其误判为受控注入

#### Scenario: Kimi Aria bundle 保持受控

- **WHEN** Kimi 真实会话初始化或 resume 且 envelope 提供 Aria bundle
- **THEN** `mcpServers` SHALL 按原有 bundle allowlist/digest/脱敏/审计规则处理；注入 digest 漂移 SHALL 拒绝 resume 并新建会话

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

#### Scenario: Codex danger 与 apply_patch 真机岗位探针
- **WHEN** 在固定真实 Codex CLI 版本下，以 author 的 danger-full-access 分别配置 on-request 与 never 并执行受控 apply_patch
- **THEN** 验收 SHALL 保存两档真实审批请求“出现或未出现”的原始观察、fileChange 事件、实际文件变化、中断 turn 与 durable 审计结果；审批未出现不得写成被拒成功，fileChange 观察到后必须失败关闭本轮，不能把已发生写入描述为事前预防
- **AND** 若 fileChange 不可观测或不能可靠中断，系统 SHALL 判该版本检测型岗位机制未通过，不把 prompt、静默日志或模型自述作为通过证据

#### Scenario: Reviewer 不继承 author 规则
- **WHEN** 任一家 provider 的 workspace/coding Reviewer、repair、retry 或受支持 legacy review 入口启动
- **THEN** 系统 SHALL 提供完整工具、terminal、MCP 与 skills；必须通过有效 B 档窗口与平台通道锁，未接线 provider 拒绝担任 Reviewer；不重新启用已取消的 Group Reviewer 流程

### Requirement: blocked／rework 下的受限政策读取与重新授权（REQ-ENV-C2-POLICY）

当逻辑代码库 coding attempt 处于 blocked 或 rework 等待面时，产品面 SHALL 提供“读取政策”与“重新授权”操作。读取政策 SHALL 仅通过唯一 LC authority resolver 解析该 attempt 当前 SessionPolicyEnvelope 冻结的 policy_id、revision 与 digest，返回与 digest 一致的政策正文；resolver 无法唯一解析、digest 不一致或材料缺失时 SHALL fail-closed 并返回可诊断等待项，MUST NOT 回落到成员仓路径、项目级历史布局或绝对路径猜测。重新授权 SHALL 仅在用户确认后为该 attempt 的下一次 Coder／Reviewer 返修 run 签发 evidence 授权，授权 SHALL 绑定 attempt、role 与 policy digest，并仅在该 run 处于运行态期间有效。错误 role、过期或已替换的授权、错 attempt、digest 不匹配或越出 manifest 成员范围的读取 SHALL 继续被拒绝。系统 MUST NOT 把政策绝对路径或正文写入 context note 作为授权手段。

#### Scenario: 返修 coder 读取同 digest 政策

- **WHEN** 用户在 blocked 页面点击“重新授权”后发起返修，返修 Coder 在运行中读取政策
- **THEN** Coder 读到的政策正文 digest 与 envelope 冻结的 digest 一致，返修继续

#### Scenario: 页面读取政策

- **WHEN** 用户在 rework 等待面点击“读取政策”
- **THEN** 页面显示 resolver 返回的政策正文及其 policy_id、revision、digest，不暴露宿主绝对路径

#### Scenario: 错 role 或过期授权被拒

- **WHEN** 非被授权 role 的 run 或授权已过期／已被新授权替换的 run 发起读取
- **THEN** 读取被拒绝，attempt 保持停等并提示重新授权

#### Scenario: resolver 无法解析时停等

- **WHEN** resolver 无法唯一解析 policy 或 digest 与 envelope 不一致
- **THEN** 读取与重新授权均 fail-closed，系统落地核验等待项，不回落任何备用路径

### Requirement: 权威根政策正文与冻结引用一致（REQ-ENV-12）

逻辑代码库 recipe 发布的每个最终 `AggregatePolicyArtifact` SHALL 在 manifest 的 canonical `provider_context_root` 下，以 `policy_id` 的安全相对路径提供与 artifact.policy_text 字节一致的正文；原始 UTF-8 字节 SHA-256 SHALL 等于 artifact digest，revision SHALL 与 `policy_id` 一致。发布不完整时 MUST NOT 完成最终 receipt；下游 SHALL 继续消费同一权威 policy 引用，不引入成员仓、内部 JSON 路径或桩正文的替代入口。

#### Scenario: envelope 引用可读取的权威根正文

- **WHEN** 逻辑代码库 recipe 已完成并请求 planning、coding、review 或 revision provider run
- **THEN** authority resolver 返回的 `policy_id`/revision/digest SHALL 与 scoped policy artifact 一致，provider 通过 canonical root 下 `policy_id` 可读取同一原始正文；envelope、gateway、admission 与 receipt 消费契约 SHALL 保持不变，readiness 仅增加 REQ-BOOT-06 定义的桩识别

#### Scenario: 发布目标缺失、冲突或摘要不一致

- **WHEN** recipe 最终发布时无法安全建立 canonical root 下的政策路径，路径存在不同正文、symlink/越界，或落盘正文与 artifact/digest/revision 不一致
- **THEN** 系统 SHALL 在签发最终 receipt 前失败关闭并留下可重试诊断，不替换用户文件、不从成员仓或旧桩正文补齐；下游现有身份与 digest 漂移拒绝行为 SHALL 保持不变

#### Scenario: 政策升级与历史冻结引用

- **WHEN** 用户显式重跑 recipe 并发布更高 revision 的最终政策
- **THEN** 新正文 SHALL 以新 `policy_id` 原样落盘且旧正文保留；新 envelope SHALL 使用当前权威 artifact 的 revision/digest，旧 envelope 的 resume SHALL 按既有指纹一致性契约判定，不把旧引用改写为新正文

#### Scenario: 单仓策略路径保持不变

- **WHEN** 传统单仓登记创建或启动 provider session
- **THEN** 既有单仓 policy、cwd、四命令和 `git_finalize` 契约 SHALL 保持不变，不要求 canonical LC root 的政策文件

### Requirement: 多仓评审与 author 的会话唯一身份（REQ-ENV-13）

多仓库逻辑代码库下，评审角色与 author 角色的运行身份 MUST 为每个 workspace 会话新生成的唯一 id（UUID 形态），与该会话一一绑定。同一会话跨评审轮次、跨 repair MUST 使用同一个 id；存量会话缺少该身份时 MUST 补生成一次并持久化，MUST NOT 每次启动重新生成。会话删除即解绑，MUST NOT 依赖独立身份映射存储的清理。

评审身份 MUST 参与 `reviewer-verify` 的 canonical 摘要域；author 的 `resource_scope_id` MUST 由同一会话身份派生。

单仓库代码库 MUST 沿用既有窗口编号身份（`{attempt}-{role_run}-w01`）与既有 author 派生方式，canonical 摘要与改造前逐字节一致。

#### Scenario: 多仓同会话跨轮身份恒定

- **WHEN** 多仓会话先后启动第 1 轮评审与第 2 轮评审（含 repair）
- **THEN** 两轮 reviewer 摘要域取值 MUST 相同，且 MUST 为该会话的 UUID

#### Scenario: author 身份与评审身份同源

- **WHEN** 多仓会话启动 author run
- **THEN** 其 `resource_scope_id` MUST 由同一会话 UUID 派生

#### Scenario: 存量会话来自身份只生成一次

- **WHEN** 多仓存量会话首次被读取且尚无身份值
- **THEN** 系统 MUST 生成一个身份并持久化，后续读取 MUST 复用该值，MUST NOT 生成第二个

#### Scenario: 会话删除即解绑

- **WHEN** 多仓会话被删除
- **THEN** 该身份 MUST 不再被任何持久记录引用，MUST NOT 存在需单独清理的身份映射

#### Scenario: 单仓身份不变

- **WHEN** 单仓会话进入评审并启动 author run
- **THEN** reviewer 身份 MUST 仍为窗口编号，author 身份 MUST 仍为既有 session 派生，canonical 摘要 MUST 与改造前逐字节一致

### Requirement: 多仓文档环节评审不开评审窗（REQ-ENV-14）

多仓库逻辑代码库下，Story / Design / Work Item 三环节评审 MUST NOT 开启评审窗、MUST NOT 生成评审窗记录、MUST NOT 执行基线快照 / 对账 / 恢复，MUST NOT 产出 `plan_review_window_*` 类错误码或窗口注意力提示。

该类评审是只读材料评审，不进入需要验证执行面的审查窗口档；coding 侧审查仍受既有审查窗口约束。

多仓评审无法解析材料集合或材料渲染失败时，系统 MUST 以 `plan_review_material_unresolved: <原因>` 显式阻断该轮，MUST NOT 启动 provider。

单仓库代码库 MUST 保留既有评审窗（开启、关闭、注意力提示、崩溃后的零锁恢复）与 `plan_review_window_*` 错误码，行为逐项不变。

#### Scenario: 多仓无窗口记录与窗口错误码

- **WHEN** 多仓会话完成任意一轮三环节评审
- **THEN** MUST NOT 产生评审窗记录、窗口事件、窗口注意力提示或 `plan_review_window_*` 错误码

#### Scenario: 多仓材料缺失显式阻断

- **WHEN** 多仓评审无法解析材料集合或材料渲染失败
- **THEN** 系统 MUST 以 `plan_review_material_unresolved: <原因>` 阻断该轮，MUST NOT 启动 provider，MUST NOT 静默以空材料继续

#### Scenario: 多仓做基线对账恢复

- **WHEN** 多仓文档环节评审开始或中断后恢复
- **THEN** 系统 MUST NOT 建立基线快照、MUST NOT 执行对账与恢复，恢复 MUST 直接复用既有多仓材料解析路径

#### Scenario: 单仓窗口与零锁恢复不变

- **WHEN** 单仓会话进入评审，或在评审中途崩溃后恢复
- **THEN** 评审窗记录与 window id、窗口早返回路径、注意力提示、`plan_review_window_*` 错误码与零锁恢复 MUST 与改造前一致

### Requirement: root-cwd 会话启动契约（REQ-ENV-10）

逻辑代码库的 planning、coding、review、revision、split 与 initialization provider session SHALL 从同一 canonical LC root 启动；启动审计、envelope、gateway 与 recipe receipt SHALL 记录并可复验该 root identity。目标成员、checkout 或 worktree SHALL 通过独立 target identity 传递，不得把 provider cwd 当作目标仓库或授权写根。

#### Scenario: 所有 LC 入口统一 root cwd

- **WHEN** LC provider 启动 Story/Design/Plan、ChoiceFollowup、Revision、ReviewOnly、split、Coder 或初始化 turn
- **THEN** provider process cwd SHALL 是 manifest `provider_context_root` 的 canonical path；对应 target SHALL 独立记录，单仓入口行为 SHALL 不变

#### Scenario: 双工厂 root identity 漂移

- **WHEN** 登记、recipe、workspace/coding gateway 或 aggregate provider factory 得出的 canonical root 不一致
- **THEN** 系统 SHALL 在 provider spawn 前 fail-closed，记录不一致字段和摘要，不允许一个 root 生成配置而另一个 root 消费配置

### Requirement: cwd 与 target 的合法分离形态（REQ-ENV-11）

系统 SHALL 允许逻辑代码库 session 的 canonical `working_directory` 与 target worktree/member checkout 不同，只要 working directory 位于 authority 允许范围且 target 通过 canonical path、成员归属、git-dir/worktree identity、policy action 与 capability 复验。`writable_roots` SHALL 继续表达逻辑授权：Planning/Review 为空，Coding 仅为 target worktree。

#### Scenario: root cwd 与成员 target 均有效

- **WHEN** LC provider 从非 Git canonical root 启动，target 指向其下已登记成员的 main checkout 或 worktree
- **THEN** gateway SHALL 放行 cwd≠target 的合法形态；provider cwd 用于根配置发现，target resolver 仍以成员 Git 身份和 policy 目标校验操作范围

#### Scenario: 分离形态越权

- **WHEN** working directory 越出 authority、target 不属于 manifest、git-dir/worktree 身份漂移，或 Coding writable root 不是 target worktree
- **THEN** 系统 SHALL 在 spawn 前拒绝，且不得用扩大 root、成员 fallback 或 direct provider 启动规避校验

### Requirement: 统一宿主执行与受信环境（REQ-ENV-HOST-01）

四家 provider 的全角色、Direct/LC、同步/流式及 ACP/MCP 子执行 SHALL 继承当前用户宿主环境执行，平台不构造、不覆盖子进程环境（调用方显式 env map 仅作覆盖层），不施加应用层 bwrap/netns/no-new-privs。执行合同由平台受信装配并冻结，不能被自报 role、prompt 或环境升级；HOME 为用户宿主 HOME，凭据随宿主环境可达，安全由用户自管，平台不作凭据隔离或物理防写承诺。

#### Scenario: 无 bwrap 的全角色启动
- **WHEN** 宿主没有 bwrap 而具备该 provider 的必要 CLI 与运行前提
- **THEN** Author、规划、Reviewer 与 Coder SHALL 不因缺 bwrap 失败；全部记录宿主非隔离事实，不声称物理防写或凭据不可达

#### Scenario: 运行环境与清理边界
- **WHEN** 平台构造或清理 provider 运行环境
- **THEN** 系统 SHALL 记录环境策略版本而不泄密、不将环境具体值写入摘要或日志；子进程环境继承宿主，平台不注入；只能清理本轮运行前创建并登记的私有目录和资源，绝不依据 HOME/XDG 值递归清除 owner 目录、共享缓存或用户容器

#### Scenario: 平台受信 Git 不执行仓库注入代码
- **WHEN** 平台启动 Git 操作，包括快照、跨 target 检查、交付与补偿
- **THEN** 系统 SHALL 禁用仓库 hooks 与 fsmonitor 桥，不因 provider 写入本地配置而自动执行它；Coder 窗口外合法提交保持可用，受信 Git 与 provider 自己执行命令的信任边界如实区分

### Requirement: Reviewer B 档快照恢复与对账采信（REQ-ENV-REVIEW-B）

所有 Reviewer 运行 SHALL 采用能力全开与 B 档软铁律：可恢复基线快照、平台写入/提交通道锁、违规整轮作废。零写对账域 SHALL 限于 tracked 与 untracked 非 gitignore 忽略文件；ignored 产物不在域内，系统 SHALL 在报告与审计面显式披露该边界。系统 SHALL 在平台记录未被篡改前提下持久化违规事实，恢复先前现场并独立判定报告可采信；无独立 UID，不承诺恶意同 UID 篡改必可检测或外部副作用可回滚。

#### Scenario: 对账域由基线冻结的 ignore 规则界定
- **WHEN** 审查窗口建立可恢复基线
- **THEN** 对账域 SHALL 恰为 tracked 文件 ∪ untracked 且当时未被 gitignore 忽略的文件（即 `git status` 常规口径）；被忽略产物（`node_modules/`、`target/`、构建输出）SHALL NOT 进入基线采集、恢复动作或违规判定
- **AND** 对账域 SHALL 在基线时刻冻结：恢复与违规判定 MUST NOT 采用 Reviewer 期间被改写的 ignore 规则重新划域；worktree 内 `.gitignore` 属 tracked 域内文件，Reviewer 改动它本身 SHALL 按域内差异处理（证据落盘、恢复、本轮作废）
- **AND** 报告、窗口记录与审计面 SHALL 显式披露「ignored 产物不在对账域（已知边界）」，MUST NOT 声称零写覆盖全部文件

#### Scenario: 完整审查窗口
- **WHEN** Reviewer 将读取材料并开始验证
- **THEN** 系统 SHALL 先锁平台 stage/commit/push 与补偿通道并确认 writer 静止，再持久化可恢复基线（含既有 dirty/staged 与 untracked 非 ignored 文件；ignored 产物不采集、不进基线），之后采材料和启动；未知 writer、锁或快照失败停等，不采污染材料

#### Scenario: ignored 产物不在对账域
- **WHEN** Reviewer 的构建/测试在 `node_modules/`、`target/` 或其它 gitignore 忽略路径创建、修改或残留文件
- **THEN** 系统 SHALL NOT 将 ignored 产物计入基线规模、恢复动作或违规判定；报告与窗口记录 SHALL 注明「ignored 产物不在对账域」，且 SHALL 如实声明已知代价——对 ignored 产物的评审修改不被发现、不被恢复、不构成违规
- **AND** 恢复动作 SHALL NOT 删除、清空或改写 ignored 产物：域外条目不得当作 Reviewer 残留清除，Coder 既有 ignored 依赖/构建目录必须原样保留
- **AND** 对账域内（tracked 与 untracked 非 ignored）仍为严格零写：任何差异 SHALL 先落证据、恢复后判违规并使本轮作废；对账域内临时验证产物按「正常验证恢复」处理，语义不变

#### Scenario: 正常验证恢复
- **WHEN** Reviewer 构建/测试在对账域内产生临时文件且正常退出
- **THEN** 系统 SHALL 停止登记 writer、恢复 worktree 基线并清理本轮私有资源，保留原有 Coder dirty；只有对账域内恢复完整、对账无违规、边界已披露与记录完整才接受报告，恢复成功本身不代表可采信

#### Scenario: Reviewer 自提交或元数据违规
- **WHEN** 观察到 Reviewer 自行提交、写 Git/平台元数据、绕过平台通道或其它规定的违规
- **THEN** 系统 SHALL 在恢复前持久化不可由恢复清除的事实，使整轮报告作废、告警并进入人工处置；worktree 恢复不洗白该轮，平台不得借该报告推进交付

#### Scenario: 三个对账域不得混同
- **WHEN** 审查结束执行恢复与采信核对
- **THEN** 系统 SHALL 分开判定 worktree 恢复域、Git 观察域与平台合法写入域；worktree 恢复域的比对范围 SHALL 恰为 tracked 与 untracked 非 ignored 文件（ignored 产物不在域内，其改动不可见为已接受代价）；平台合法写入域内报告/journal 写入不误判 Reviewer 违规，Reviewer 写元数据不冒充平台合法写入；单一 .git/.aria 摘要相等不足以采信

#### Scenario: 取消崩溃与人工恢复
- **WHEN** 审查取消、超时、进程崩溃、恢复失败或 journal 不一致
- **THEN** 系统 SHALL 以持久窗口续接停止与恢复，完成前不释放冲突写通道、不启动后续 provider；无法确认安全恢复时提供人工门；任一违规轮恢复后也必须启动新审查轮，不能沿用旧报告或以污染态重冻基线

### Requirement: HostCoding 自行 CHECK 与补充验证边界（REQ-ENV-CHECK-01）

Coder SHALL 在宿主 HostCoding 环境自行执行计划 CHECK（构建、测试、依赖与容器），平台不新增通用命令 runner。环境能力声明仅说明 JVM/Node/网络/容器等前提，不是授权或路由指令；A′ verification-triage SHALL 仅补充外部硬件、人工检查或临时缺项，不取代主路径、不自动改计划。

#### Scenario: 工具链齐备的自跑 CHECK
- **WHEN** 工具链、依赖网络、容器引擎与登录态符合已声明前提
- **THEN** Coder SHALL 直接执行原计划命令并记录 cwd、实际命令、退出码与测试数量；平台不得以旧沙箱/只读 grammar 拦截

#### Scenario: 前提缺失与失败分类
- **WHEN** 依赖、认证、权限、环境能力缺失或测试断言失败
- **THEN** 系统 SHALL 如实分类与呈现证据，不统称沙箱失败、不伪造通过；补充验证与重跑沿既有人工批准/唤回 Coder 路径，不清除 finding 或放宽平台交付授权
