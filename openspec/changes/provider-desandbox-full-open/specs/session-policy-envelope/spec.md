# 规范增量：会话政策与全角色宿主执行

<!-- 来源：openspec/specs/session-policy-envelope/spec.md:9-70,109-140；原标题仅作匹配锚。Direct 全开为诉求1推论，Codex 作者纪律为检测型；可验收行为均在下方要求与场景。 -->

## MODIFIED Requirements

### Requirement: 两层政策结构（REQ-ENV-01）

系统 SHALL 持久化集中政策 artifact，按每次 LC provider run 冻结 policy/revision/digest、action、target、逻辑 read/write roots、provider/config/capability 与受信执行合同。同步与流式真实入口均适用；既有流式 legacy 直连仅允许在受信 builder、岗位守卫和等价审计齐全时保留过渡拓扑，同步栈不适用此例外。全开能力不得绕过 authority 校验。

#### Scenario: 启动逻辑代码库 provider run
- **WHEN** LC 规划、编码、评审或初始化启动真实 provider（过渡例外除外）
- **THEN** envelope SHALL 由唯一 authority resolver 从集中政策解析并校验，缺失或不一致时 spawn 前失败关闭

#### Scenario: legacy 直连例外启动
- **WHEN** Story/Design/SC 首轮与修订、WorkItemPlan legacy 或 coding reviewer 使用既有流式直连
- **THEN** 系统 SHALL 经 builder 冻结新执行合同与 REQ-ENV-09 author 岗位规则，通过 adapter 守卫和启动审计；Reviewer 不再要求旧 deny，仍须满足 B 档窗口，不允许裸输入绕过

#### Scenario: coding Coder 直连例外启动
- **WHEN** Coder Executor 使用既有流式直连
- **THEN** 系统 SHALL 经受信 builder 构造全工具宿主执行上下文、通过角色守卫，并以既有 execution_event_audit 记录启动；不得带 author 岗位 deny 或绕过工厂

#### Scenario: LC 冷启动生成有效 envelope
- **WHEN** authority、manifest/checkout、rules/policy 与成员 index 所需材料齐备并请求运行
- **THEN** 系统 SHALL 生成稳定 target、policy revision/digest、能力语义及执行合同的冻结 envelope，仅使用该校验结果启动

#### Scenario: policy 或 authority 不一致时拒绝启动
- **WHEN** authority 无法唯一解析、政策材料缺失、digest/revision 不一致、target 跨 kind 或实际 capability 未通过准入
- **THEN** provider SHALL 零会话启动并返回可操作核验/准备/重试等待；不得回落成员仓路径、旧 pointer、历史布局或裸 input，不写假能力，不自动重试未知副作用；允许操作后复用原编排链，不建第二套 durable operation 状态机

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

envelope SHALL 区分规划、编码、评审 action 的业务职责，保留历史 PlanningReadOnly/ReviewReadOnly 标识时不得将名称投影成物理只读。规划/评审逻辑交付写根为空，Coding 唯一交付写根恰为当前 target；逻辑写根约束平台交付范围，不限制宿主 terminal/MCP 的物理可达性。Reviewer 的临时验证变化 SHALL 经 B 档恢复与采信判定。

#### Scenario: coding target 与 envelope 不一致
- **WHEN** coding target 与冻结 envelope 不一致
- **THEN** 系统 SHALL 拒绝启动，spawn 前复核 canonical path/git-dir/worktree identity，不能借全开忽略 TOCTOU

#### Scenario: 逻辑写根不冒充执行隔离
- **WHEN** 规划/评审 terminal 能写宿主路径或 Coder 能访问 target 外路径
- **THEN** 系统 SHALL 如实报告宿主全开，不将空/单一逻辑 roots 当物理阻断；平台交付范围校验与 Reviewer 违规作废继续有效

### Requirement: 禁止降级（REQ-ENV-05）

系统 SHALL 对缺少当前宿主全开执行证据、岗位/窗口合同、版本、authority 或投影的 provider 路由失败关闭，不切换 provider、不静默换执行模型。Codex danger-full-access SHALL 为四家全开基线而非降级；旧只读/target-only 能力或 declared 来源不等于新版真实验证。capability SHALL 保留 exact version、dialect、证据等级与语义标识。

#### Scenario: 选择 coding provider
- **WHEN** 路由选择的 provider 不满足新执行合同或当前版本能力未验证
- **THEN** 系统 SHALL 阻塞并返回可操作能力缺失，不回落旧受限档、不自动换 provider；证据齐备的 danger-full-access 不得因旧永久拒绝规则阻断

### Requirement: 非编码角色的 built-in 文件写工具拒绝（REQ-ENV-09）

系统 SHALL 仅对 author 保留编辑代码文件的岗位纪律：具备过滤机制的 provider 不配置相应内建工具；Codex danger 下为检测型，观察 fileChange 后中断 turn、失败关闭并持久审计，不承诺预防写入。Reviewer/Coder 不带该 deny，terminal/MCP/skills 全角色一致。作者文档走平台通道、不由作者提交 Git；岗位规则独立于 sandbox/审批，不宣称约束同 UID 其它写通道。

#### Scenario: 非编码角色启动时 built-in 写工具不可用
- **WHEN** author 首轮、feedback revision、WorkItemPlan 普通/fresh/resume/serial/batch 或 legacy/gateway 入口启动
- **THEN** Pi/Claude SHALL 使用工具过滤、Kimi 在代码编辑 fs 服务落实作者岗位限制；Codex SHALL 使用检测型纪律而非虚构预防机制，发生 author fileChange 时中断当前 turn、将本轮结果标为失败且不得采信，并先持久化事件与处置记录；不得改用 read-only、拒绝全部 commandExecution 或禁 terminal/MCP/skills

#### Scenario: codex 审批分类
- **WHEN** Codex 收到审批请求
- **THEN** 系统 SHALL 按 wire 形态确定性应答与审计；自发现 MCP 维持受信自动批准，普通 commandExecution 维持权限模式与 ApprovalBridge，Reviewer 不再统一拒绝 commandExecution/fileChange；author 的检测型 fileChange 纪律不依赖审批请求出现，不以应答 decline 宣称在 danger 下强制预防文件变化，所有会话仍用 danger-full-access
- **AND** 未知形态 SHALL 协议合法拒绝（elicitation 返回 -32601 与 data，item 返回 decline）并记 protocol_warning；连续至少 3 次未知形态终止会话并记 unknown_approval_storm，任何请求不得静默不应答，driver choice 与既有提问语义不变

#### Scenario: Codex danger 与 apply_patch 真机岗位探针
- **WHEN** 在固定真实 Codex CLI 版本下，以 author 的 danger-full-access 分别配置 on-request 与 never 并执行受控 apply_patch
- **THEN** 验收 SHALL 保存两档真实审批请求“出现或未出现”的原始观察、fileChange 事件、实际文件变化、中断 turn 与 durable 审计结果；审批未出现不得写成被拒成功，fileChange 观察到后必须失败关闭本轮，不能把已发生写入描述为事前预防
- **AND** 若 fileChange 不可观测或不能可靠中断，系统 SHALL 判该版本检测型岗位机制未通过，不把 prompt、静默日志或模型自述作为通过证据

#### Scenario: 策略缺失或非法 fail-closed（双向守卫）
- **WHEN** author 岗位规则缺失/非法，或 Reviewer/Coder 被装配 author deny 或旧只读执行档
- **THEN** 系统 SHALL 在 spawn 前失败关闭并返回原因，不以清空 author policy 取得全开，也不补回 Reviewer deny；角色归属来自受信装配，不能由 prompt/env/ACP 自报

#### Scenario: resume 与修订策略一致（冻结记录比对）
- **WHEN** author 或 Reviewer 修订/resume
- **THEN** 系统 SHALL 以原生会话 ID 检索该会话最近启动记录，精确比对岗位工具语义/物理投影摘要、规则版本、provider exact version/dialect 与执行基线；记录缺失或失配则拒绝续接、标记 superseded 并进入显式新会话路径，不混用旧 tp-v1 受限档记录

#### Scenario: 启动与审批的 durable 审计
- **WHEN** author 的 pi/claude/codex 岗位策略启动或作出审批决策
- **THEN** 系统 SHALL 经 engine sink 向 LifecycleStore tool-policy-run-audit 分区串行追加 JSONL，key=(workspace_session_id, role_run_seq)，run seq 持久单调、行 seq 单调；首行恰为一条 provider_start，后续允许 approval_decision/file_change_detected/protocol_warning/session_terminated
- **AND** file_change_detected（Author 检测型岗位，T13）为第五类 canonical 事件：只能出现在 provider_start 首行之后，携带 item_id、summary（path+changeType）、source_method（item wire method）、disposition（处置冻结文本：turn 中断、run Failed、本轮输出不可采信、已发生写入不回滚）与 policy_digest；检测事实与处置记录不得因后续事件、恢复或会话终结丢失
- **AND** adapter SHALL 在 start 返回前完成握手并确认 native session ID（Pi 生成/传入，Claude fresh 等 init 有界确认、resume 已知，Codex 等 thread/start 应答），确认失败拒绝；启动审计包含最终参数、sandbox/approval、岗位规则与摘要、原生 ID、版本/dialect，写失败返回错误并终止子进程；后续审计失败沿既有 kill 链终止，读取坏行跳过并警告，不伪称 pre-spawn 审计
- **AND** approval_decision SHALL 包含 category/server/tool/request/decision/reason/policy digest；Coder/非岗位策略与 Kimi 仍经 execution_event_audit，Reviewer 窗口与作废事实另按 B 档持久化，不能因退出旧策略分区丢失采信证据

#### Scenario: Coder 与非策略路径零变化
- **WHEN** Coder 或聚合初始化 Executor 启动
- **THEN** 系统 SHALL 保留全工具面、Codex danger-full-access 与独立审批映射，不注入 author deny；MCP 与未知审批仍按确定性表应答和审计，不以“零变化”保留旧无响应缺陷

#### Scenario: kimi 既有对齐维持
- **WHEN** Kimi 作为任意角色启动
- **THEN** 系统 SHALL 保留 ACP 原生与既有 execution_event_audit 形态，但工具行为改为全角色 Host 单路径：Reviewer 可 Terminal/FsWrite，author 仅拒编辑代码文件工具，规划角色不再整体拒绝或禁 terminal；不得以“既有 policy 不得放宽”复活旧限制

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
