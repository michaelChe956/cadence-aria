# 规范增量：会话政策与全角色宿主执行

<!-- 来源：openspec/specs/session-policy-envelope/spec.md:9-70,109-140；原标题仅作匹配锚。Direct 全开为诉求1推论，Codex 作者纪律为检测型；可验收行为均在下方要求与场景。 -->

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
### Requirement: 禁止降级（REQ-ENV-05）

系统 SHALL 对缺少当前宿主全开执行证据、岗位/窗口合同、版本、authority 或投影的 provider 路由失败关闭，不切换 provider、不静默换执行模型。Codex danger-full-access SHALL 为四家全开基线而非降级；旧只读/target-only 能力或 declared 来源不等于新版真实验证。capability SHALL 保留 exact version、dialect、证据等级与语义标识。

#### Scenario: 选择 coding provider
- **WHEN** 路由选择的 provider 不满足新执行合同或当前版本能力未验证
- **THEN** 系统 SHALL 阻塞并返回可操作能力缺失，不回落旧受限档、不自动换 provider；证据齐备的 danger-full-access 不得因旧永久拒绝规则阻断

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

## ADDED Requirements
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
