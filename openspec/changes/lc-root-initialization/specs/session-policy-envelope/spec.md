# Spec Delta

## MODIFIED Requirements

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

### Requirement: action 与写根约束（REQ-ENV-03）

系统 SHALL 使 envelope 区分 `PlanningReadOnly` / `CodingTargetWrite` / `ReviewReadOnly`；Planning/Review 的 writable roots 为空，Coding 的唯一 writable root 恰为当前 target worktree。canonical root working directory 是 provider 启动与目录发现边界，不得据此把 coding writable root 扩大为聚合根；root 约束仍是“配置目标 + best-effort”，不宣称 OS 级不可写。

#### Scenario: coding target 与 envelope 不一致

- **WHEN** coding run 的 target、worktree 或独立 working directory 与 envelope 不一致
- **THEN** 启动 SHALL 被拒绝（fail-closed）；spawn 前复验 canonical root、git-dir、worktree identity 与 target snapshot

### Requirement: resume 一致性（REQ-ENV-04）

系统 SHALL 仅当 policy digest、target identity、canonical working directory、provider version/dialect、capability snapshot 全部一致时允许 resume；任一维度变更时新建会话并使旧会话 superseded；对 planning/coding/review/initialization 均适用。

#### Scenario: 续接旧 provider session

- **WHEN** 尝试续接旧 provider session 且 policy digest、target identity、canonical working directory、provider version/dialect 或 capability snapshot 指纹不一致
- **THEN** 系统 SHALL 拒绝 resume 并启动新会话，旧会话标记 superseded

#### Scenario: cwd 或 target 改变时拒绝 resume

- **WHEN** 尝试续接旧 provider session 且 canonical root working directory、target、政策或 capability 指纹任一不一致
- **THEN** 系统 SHALL 拒绝 resume 并启动新会话，旧会话标记 superseded

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

## ADDED Requirements

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
