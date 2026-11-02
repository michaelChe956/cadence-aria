# logical-codebase-aggregate-planning Specification

## Purpose
规划类产物（Issue / Story / Design / Work Item 计划）使用聚合上下文：provider 从聚合根只读启动（全量 experimental + supervised，best-effort 只读），注入成员清单、目标仓 profile、索引摘要与政策 envelope。

## Requirements

### Requirement: 聚合规划上下文（REQ-PLN-01）

系统 SHALL 将规划 workspace 上下文组织为逻辑代码库聚合，包含 logical_codebase、成员 inventory（id/alias/path/role/profile 摘要）、focus 与索引 revision；从 canonical 聚合根启动并使用全角色宿主基线，不能由聚合 cwd 推导只读。prompt SHALL 注入紧凑成员清单与预算而非全部源码，超预算按已有确定性次序截断并标记未检索成员，具体阈值沿用已确认设计与 Plan。

#### Scenario: 聚合上下文注入
- **WHEN** 为 LC Issue 启动 Story/Design/Work Item 计划生成
- **THEN** provider SHALL 从 canonical provider_context_root 以宿主全开基线启动，注入紧凑成员清单和预算，不注入全部成员源码；author 岗位工具规则单独生效

#### Scenario: 预算超限
- **WHEN** inventory/摘要/证据超出 token 或 byte 预算
- **THEN** 系统 SHALL 按既有确定性顺序截断并标记未检索成员，不撑爆 context window，不借去沙箱重定义截断策略

### Requirement: Issue 逻辑代码库归属与参与仓库（REQ-PLN-02）
系统 SHALL 通过 `project_id` 表达 Issue 的逻辑代码库归属，通过 `IssueCodebaseSelection`（include/exclude/focus）表达参与仓库；`IssueRecord.repo_id` 仅作迁移期 focus/primary 投影；focus 可为多值、必须在 include 集合内，include/exclude 重叠时 exclude 优先。

#### Scenario: 创建逻辑代码库 Issue
- **WHEN** 创建逻辑代码库下的 Issue
- **THEN** 可指定参与/排除/重点仓库，服务端校验成员归属与优先级，DTO 返回参与仓库摘要

#### Scenario: 成员变更
- **WHEN** 规划后成员被删除/停用
- **THEN** 系统 SHALL 使相关 Issue selection 与 planning snapshot 失效或标记，阻塞指向已删除成员的新工作项

### Requirement: 规划上下文快照（REQ-PLN-03）

系统 SHALL 每次规划 run 固化 PlanningContextSnapshot（membership_revision、每仓 checkout identity/revision/dirty/availability、index revision、policy digest、canonical working directory、target identity 与 access fingerprint），作为 context/cwd/prompt/audit 的唯一依据。resume、revision、follow-up 与 split planning SHALL 使用同一 snapshot 合同；失配 SHALL 拒绝旧 native 会话并持久等待用户明确 fresh，不能自动重建后启动。

#### Scenario: 恢复/续接规划会话
- **WHEN** 恢复或续接规划会话且 cwd、target、policy 或成员/index 快照任一指纹不一致
- **THEN** 系统 SHALL 拒绝旧 native session、保存 superseded 等待；用户明确新会话后才重新完整 admission 并重建当前上下文，不沿用可能过时/越权的内容

#### Scenario: Plan stale 重连等待
- **WHEN** Plan 轮廓或 SC author/manual turn 在重连时发现 planning snapshot 已过期
- **THEN** 系统 SHALL 投影同一等待事实且 provider session-start/native child 计数为零，不自动 OutlineRebuild、增 SC attempt、扣预算或降到 legacy；显式 fresh 后按原 run-context 继续

#### Scenario: 聚合 native target 下成员事实漂移
- **WHEN** native target 固定聚合根但成员 checkout identity/revision/dirty/availability 或 membership/index 发生变化
- **THEN** 系统 SHALL 继续按 snapshot 检测变化并拒绝旧会话，不因聚### Requirement: 唯一规划 launch/resolver（REQ-PLN-07）

系统 SHALL 提供唯一规划 resolver，使 Story、Design、WorkItemPlan、review/revision、resume 与 WebSocket follow-up 的 context、canonical root cwd、独立 native target、业务材料/compile target、prompt、session audit 均来自同一 PlanningContextSnapshot/authority。native anchor 与业务 target SHALL 分离派生；禁止 issue.repo_id/first Story fallback、另写第二选仓算法或以 target worktree 覆盖 root cwd。

#### Scenario: 规划全链路一致
- **WHEN** 启动、run-next、revision、resume、review 或 WebSocket follow-up
- **THEN** 各环节 SHALL 同源派生 canonical root cwd/native anchor 与原业务 repository；合法 cwd≠业务 target 继续放行，单仓两字段保持原目录

#### Scenario: 聚合视图不得成为 IR 成员身份
- **WHEN** SC Plan 从生成 source 降为 IR 并进入 logical compile
- **THEN** 材料 SHALL 从原业务 repository 读取，IR target SHALL 是原有效 physical member ID；aggregate-root view ID SHALL NOT 写入 IR 或冒充成员 identity

: 唯一规划 launch/resolver（REQ-PLN-07）

系统 SHALL 提供唯一规划 launch/resolver，使 Story、Design、WorkItemPlan、review/revision、resume 与 WebSocket follow-up 的 context、canonical root cwd、独立 target、prompt、session audit 均来自同一 `PlanningContextSnapshot`；禁止任何 `issue.repo_id`、first Story fallback 或以 target worktree 覆盖 root cwd。

#### Scenario: 规划全链路一致

- **WHEN** 启动、run-next、revision、resume、review 或 WebSocket follow-up
- **THEN** 各环节 SHALL 使用同一 snapshot 派生 root cwd 与 target；cwd 与 target 不一致但均通过 authority/gateway/checkout 校验时 SHALL 放行，单仓两字段继续映射为原有目录

### Requirement: Story 聚合视野（REQ-PLN-04）
系统 SHALL 使 Story 生成时列出涉及仓库（`involved_repositories`，引用 member ID）；AI 不确定涉及仓库时产生 blocker 向用户确认，不得猜测或默认塞给 primary。

#### Scenario: Story 涉及跨仓改动
- **WHEN** Story 涉及跨仓改动（如订单服务 + 库存服务 + 前端）
- **THEN** Story SHALL 明确列出涉及仓库并描述各自改动范围；AI 无法确定涉及仓库时产生 blocker

### Requirement: Design 聚合视野与改动顺序（REQ-PLN-05）

系统 SHALL 使 Design 显式携带 logical_codebase_ref 与 involved_repositories，不回落 issue.repo_id；跨仓关系由 AI 从聚合索引按需检索（启发式，非语义级服务图）。Design SHALL 表达改动顺序作为 Work Item depends_on 依据；多仓 involved_repository SHALL 必须提供 change_order，缺失即 blocker change_order_required_for_logical_codebase；单仓 change_order 可选。scope/change_order SHALL NOT 被解释为迁移 PlanningReadOnly native target 的授权。

#### Scenario: Design 表达跨仓改动顺序
- **WHEN** Design 涉及跨仓接口/契约变更
- **THEN** Design SHALL 表达改动顺序（如公共契约→provider→consumer），作为 Work Item depends_on 依据

#### Scenario: 多仓 Design 缺失 change_order 即 blocker
- **WHEN** 逻辑代码库 Issue 的 Design 涉及多个 involved_repository 但未提供 change_order
- **THEN** 系统 SHALL 拒绝其进入确认/compile，保留 blocker change_order_required_for_logical_codebase；规划 native anchor 固定不取消业务 gate

#### Scenario: 合法 scope 回写不迁移 native anchor
- **WHEN** 同一 Design 从初始空 scope 收敛为四仓 involved/change_order，且环境与 snapshot 未变
- **THEN** native target SHALL 仍为 canonical aggregate root，评审/反馈修订不得仅因 scope 回写 supersede 原 native 会话；业务材料和 compile target 按原规则派生

### Requirement: 规划只读边界（REQ-PLN-06）

规划 provider 会话 SHALL 为只读语义的 best-effort（`best_effort_configured`：Aria-owned 配置 + cwd + pre/post 检测）；仅在固定 provider/OS 越界写 fixture 通过后才可升级为 `production_verified_readonly`；未达到该级别时不得宣称「物理上无法写入」；疑似越权写入需被检测并标记。

#### Scenario: 规划运行
- **WHEN** 规划 run 完成
- **THEN** 系统 SHALL 报告 best-effort 只读状态（配置目标 + 前后检测），成员主 checkout 与聚合根无规划产生的已知写入；硬只读需 PreToolUse deny（Write/Edit when plan）或路由 Codex `:read-only`，Claude plan 模式仅为 prompt 层

#### Scenario: Kimi 规划 Terminal 可执行
- **WHEN** Kimi 的 Orchestrator、WorkItemSplitter 或 Handoff 请求普通 argv 或 shell terminal，并满足当前权限审批
- **THEN** 系统 SHALL 正常执行，不因角色全拒、只读命令表、bwrap 缺失或解释器限制拒绝；若该运行承担 author 职责，编辑代码文件内建工具岗位规则仍适用

#### Scenario: 作者产出与规划身份保持
- **WHEN** author 生成 Story/Design/Plan 或请求编辑代码文件内建工具
- **THEN** 文档 SHALL 走平台产物通道且不由作者提交 Git；具备过滤机制的 provider 不配代码编辑内建工具，Codex 按检测型 fileChange 纪律中断并使该轮失败，不能宣称事前防写；terminal/MCP/skills 不额外禁用，聚合快照、authority 与业务成员归属继续有效

### Requirement: 唯一规划 launch/resolver（REQ-PLN-07）

系统 SHALL 提供唯一规划 resolver，使 Story、Design、WorkItemPlan、review/revision、resume 与 WebSocket follow-up 的 context、canonical root cwd、独立 native target、业务材料/compile target、prompt、session audit 均来自同一 PlanningContextSnapshot/authority。native anchor 与业务 target SHALL 分离派生；禁止 issue.repo_id/first Story fallback、另写第二选仓算法或以 target worktree 覆盖 root cwd。

#### Scenario: 规划全链路一致
- **WHEN** 启动、run-next、revision、resume、review 或 WebSocket follow-up
- **THEN** 各环节 SHALL 同源派生 canonical root cwd/native anchor 与原业务 repository；合法 cwd≠业务 target 继续放行，单仓两字段保持原目录

#### Scenario: 聚合视图不得成为 IR 成员身份
- **WHEN** SC Plan 从生成 source 降为 IR 并进入 logical compile
- **THEN** 材料 SHALL 从原业务 repository 读取，IR target SHALL 是原有效 physical member ID；aggregate-root view ID SHALL NOT 写入 IR 或冒充成员 identity

### Requirement: 逻辑 issue 的 selection 初始写入

多仓 project 创建 issue 时服务端自动写入 all_members 代码库选区，保证 Story/Design/WorkItemPlan 生成可达。

#### Scenario: 自动 all_members
- **WHEN** 多仓 project 创建 issue（请求体与单仓一致，repository_id 为 primary repository，须属于 manifest active member）
- **THEN** 服务端原子化创建 issue 并写入覆盖全部 active member 的 selection；selection 写入失败时删除刚创建的 issue 并返回 422 issue_selection_write_failed（删除亦失败时记录 orphan 告警并返回 500）；不存在只建 issue 不建 selection 的中间态

#### Scenario: primary repository 语义
- **WHEN** 多仓 issue 携带 repository_id
- **THEN** 该字段语义为 primary repository（UI 归属与兼容投影），不影响 selection 成员范围（恒为 all_members）

### Requirement: 多仓 Design change_order 强制

多仓 Design 涉及多个 involved repository 时必须提供 change_order，缺失即 blocker。

#### Scenario: 缺 change_order 被拒
- **WHEN** 多仓 Design 涉及多个 involved repository 且未提供 change_order
- **THEN** 生成被拒并产生 blocker change_order_required_for_logical_codebase，不得进入 compile；模型层不再保留"缺失不强制"的相反语义

### Requirement: candidate 与 compile 同源有效图（REQ-PLN-09）

系统 SHALL 在 candidate/reviewer 前置门及 Final Compile 使用同源结构规则与上下文digest：有效图边为 canonical Inputs 的 provider→consumer 与已确认change_order的前置仓全部WI→后置仓全部WI之并集。声明Dependencies SHALL 单独保留并按原规则校验一致性，不将未参与执行的声明边冒称实际边。同端点多来源 SHALL 全部保留；自依赖、未知引用、成员归属、Plan Intent与物理target同构分别验证。纯结构检查 SHALL 无selection写回、WI/transaction创建或provider调用，外层只加载一次授权/成员事实；logical→physical ID映射不得改变规则。

#### Scenario: R1 两种边相撞
- **WHEN** gateway序3/frontend序4，WI-005归gateway且真实Inputs要求WI-004(frontend)
- **THEN** candidate门确定拒收，记录WI-004→WI-005 canonical_input与WI-005→WI-004 change_order两种实际来源，坏候选不评审/确认；机器指令保AC/真实Inputs/仓序，交原授权author重组，Final Compile同规则复验

#### Scenario: 同仓与单仓原语义
- **WHEN** 两WI同target或单仓Design允许空change_order
- **THEN** 不添加同仓自动边，单仓编译行为不变；多仓缺顺序仍原blocker，不临时取消自动边逃过检测

#### Scenario: 成员与物理身份
- **WHEN** IR物理target不在授权成员、Plan Intent归属不符或成员事实失效/不可读
- **THEN** 拒绝候选并保具体typed结构或operational原因，不把聚合native target当业务target、不选首仓、不在reviewer纯门重复selection写回

### Requirement: 来源 witness 与语义保护（REQ-PLN-10）

有效图阻断 SHALL 携实际邻接边可验证的确定性有向cycle witness及每跳来源、contract/字段定位与仓序索引；未排序节点集合不得直接渲染成有向环。机器建议 SHALL 使用完整已读事实，保持真实供需/AC/能力/授权仓序与写面；唯一性只对可完整枚举局部域给证据，拓扑/删边空间不能因只生成一个建议就称唯一。开放语义等价由原reviewer判断；无环不是业务正确证明。

#### Scenario: witness 每跳真实
- **WHEN** Kahn返回含不在环上的阻断残余节点
- **THEN** 系统从实际邻接边提取稳定有向witness、逐跳可核验来源，不把sort/dedup节点串画箭头，不穷举全部最小删边集

#### Scenario: 保留真实供需的多解
- **WHEN** 改归属、拆分验证与历史恢复存在多个保持业务要求的候选
- **THEN** author收到各具体约束/证据，不要求用户选择仓UUID；删/替依赖仅有同供需已满足证据才成为安全候选，否则只给保护约束下靶向重组指令

### Requirement: 规划 native 锚点生命周期（REQ-PLN-08）

LC Story/Design/WorkItemPlan 的 PlanningReadOnly native launch anchor SHALL 固定 canonical aggregate root（native member/checkout ID 为空、writable roots 为空）；focus/involved/change_order/Plan targets SHALL 只作为业务规划事实，不迁移 native anchor。原业务材料、compile target、selection 守卫、direct 单仓及 Coding member/write-root 语义 SHALL 保持。

#### Scenario: 三类规划锚点稳定
- **WHEN** Story focus、Design involved/change_order 或 Plan target/focus 从空/唯一变为多值或改变
- **THEN** PlanningReadOnly native target/cwd SHALL 保持 aggregate root；同源业务 repository 仍按原有效成员/材料规则，不自动默认首仓

#### Scenario: selection 失效或 focus 越界
- **WHEN** manifest/selection 不成对、selection invalidated 或 focus 不在合法 include
- **THEN** 系统 SHALL 在 native launch 前 fail-closed；固定 root SHALL NOT 绕过原 selection 守卫

#### Scenario: SC 成员材料到编译不回归
- **WHEN** LC SC Plan 的原业务目标为唯一成员，该成员 language.md 存在而聚合根对应材料不存在
- **THEN** 系统 SHALL 仍从原成员消费材料，source→IR→确认 compile 保持有效成员映射且 native envelope 为 aggregate root，不因 native anchor 改变制造材料等待或 work_item_target_missing

#### Scenario: direct 与 Coding 不变
- **WHEN** 传统单仓或 Coding/WorkItem 写面启动
- **THEN** 系统 SHALL 保持原目录/target/member/write-root 与 direct native 恢复行为，不把 Coding writable root 扩到聚合根

### Requirement: 多仓评审范围与代码库形态门禁（REQ-PLN-11）

系统 SHALL 以 `IssueRecord.logical_codebase_id` 作为代码库形态的唯一判定键：`None` 为单仓库代码库，`Some(_)` 为多仓库逻辑代码库。

多仓库逻辑代码库下，Story / Design / Work Item 三环节评审的评审范围 MUST 等于该 issue 选定的真实仓库集合（1~N 个成员仓）。系统 MUST NOT 引入「主仓 / 主成员（primary）」概念，MUST NOT 回落到 manifest 首成员、`IssueRecord.repo_id` 投影或任何 primary 锚定结果，MUST NOT 因无法解析集合而静默缩小范围。

单仓库代码库（`logical_codebase_id == None`）下，评审窗口、评审目标与 cwd 解析、主成员早返回路径、基线语义、门控、容错与文案 MUST 与改造前逐项一致。

#### Scenario: 多仓评审范围取 issue 选定仓库集合

- **WHEN** 多仓 issue 选定 2 个成员仓并进入 Story / Design / Work Item 评审
- **THEN** 评审 MUST 覆盖且仅覆盖这 2 个仓，评审材料与 reviewer prompt MUST NOT 出现「主仓 / 主成员 / primary」措辞，MUST NOT 产生主成员锚定事件

#### Scenario: 多仓不得回落 manifest 首成员或 repo 投影

- **WHEN** 多仓 issue 的评审链路解析仓库集合失败
- **THEN** 系统 MUST 以可诊断错误阻断该轮评审，MUST NOT 选取 manifest 首成员或 `IssueRecord.repo_id` 投影继续

#### Scenario: 单仓形态行为不变

- **WHEN** 单仓 issue（`logical_codebase_id` 为 `None`）进入三环节评审
- **THEN** 评审窗口记录与 window id、评审目标与 cwd、窗口早返回路径、基线语义、门控与文案 MUST 与改造前一致

### Requirement: 评审材料仓库清单全量注入与超限阻断（REQ-PLN-12）

多仓三环节评审材料 MUST 全量注入该 issue 选定仓库集合的成员清单（每仓至少含 id、alias、路径、role、profile 摘要）。系统 MUST NOT 按 soft/hard 预算裁剪清单，MUST NOT 以 `omitted_member_ids`、`omitted_member_count`、`truncated` 等缺员标记呈现被省略成员。

评审材料绝对上限 MUST 为 500 KiB。渲染结果超过该上限时，系统 MUST 以明确错误 `planning_material_over_budget` 阻断该轮评审，MUST NOT 启动 provider。

#### Scenario: 全量清单无缺员

- **WHEN** 多仓 issue 选定 3 个成员仓并构建评审材料
- **THEN** 渲染材料 MUST 含全部 3 个成员仓，材料 MUST NOT 含任何 omitted 或 truncated 形式的缺员标记

#### Scenario: 超上限显式阻断

- **WHEN** 渲染后的评审材料超过 500 KiB
- **THEN** 系统 MUST 返回 `planning_material_over_budget` 并阻断本轮评审，MUST NOT 启动 provider，MUST NOT 丢弃成员后继续

#### Scenario: 单仓材料与 prompt 不变

- **WHEN** 单仓 issue 构建评审材料
- **THEN** 系统 MUST NOT 注入成员清单块，材料与 reviewer prompt MUST 与改造前字节一致

### Requirement: 评审材料稳定注入面（REQ-PLN-13）

多仓评审的仓库清单与逐仓实况 MUST 位于不随会话历史压缩丢失的稳定注入面（canonical inputs 区），MUST 在长会话多轮评审后仍被完整重放。系统 MUST NOT 让该材料只存在于消息尾部。

材料块 MUST 保持既有段边界与既有聚合 scope 标记可解析，MUST NOT 引入会截断后续材料段的形态。

#### Scenario: 长会话压缩后材料仍在

- **WHEN** 多仓评审会话经历 3 轮以上评审并触发历史压缩
- **THEN** 该轮 reviewer prompt MUST 仍包含完整仓库清单与逐仓实况

#### Scenario: 材料块不破坏既有段解析

- **WHEN** 材料块写入稳定注入面
- **THEN** 既有材料段边界与聚合 scope 标记 MUST 仍可被解析，后续材料段 MUST NOT 被截断

#### Scenario: 单仓不注入成员材料

- **WHEN** 单仓 issue 构建 reviewer prompt
- **THEN** 系统 MUST NOT 注入成员清单或逐仓实况，MUST NOT 使该会话被判定为聚合规划会话

### Requirement: 逐仓实况与逐仓基线（REQ-PLN-14）

多仓评审材料 MUST 为每个成员仓渲染固定实况行：`职责`、`关键目录`、`入口`、`构建/测试命令`、`基线` 各一行。无法识别的项 MUST 写「未识别」并保留该行，MUST NOT 省略行或整仓。

逐仓基线 MUST 取自该仓 checkout 的当前 HEAD 所在分支（`git rev-parse --abbrev-ref HEAD`），MUST NOT 猜测 `main` / `master`。单仓基线解析规则 MUST 与改造前一致，含不可解析时 fail-closed 终止该轮。

#### Scenario: 多仓逐仓实况齐全

- **WHEN** 多仓 issue 含一个 Rust 仓与一个 Vue 仓并构建评审材料
- **THEN** 材料 MUST 出现两段逐仓实况，每段含职责、关键目录、入口、构建/测试命令、基线五行；缺项 MUST 显示「未识别」

#### Scenario: detached HEAD 或空仓只影响本仓

- **WHEN** 某成员仓处于 detached HEAD 或无任何提交
- **THEN** 该仓基线行 MUST 显示「基线未识别」及诊断，其余仓 MUST 继续渲染，该轮评审 MUST 继续

#### Scenario: 单仓基线仍 fail-closed

- **WHEN** 单仓 issue 的基线分支不可解析
- **THEN** 系统 MUST 按改造前语义 fail-closed 终止该轮，MUST NOT 降级为「基线未识别」

### Requirement: 多仓评审 must-fix 必查类别（REQ-PLN-15）

多仓三环节 reviewer 的 must-fix 边界规则 MUST 追加以下四类必查项：需求漏项（需求或范围未被覆盖）、仓库选择不合理（列出但不涉及 / 涉及但未列）、验收标准不可判定、跨仓影响缺失。

单仓 reviewer 的 must-fix 规则文案 MUST 逐字不变。

#### Scenario: 四类必查出现在多仓 reviewer 规则

- **WHEN** 构建多仓 Story / Design / Work Item reviewer prompt
- **THEN** must-fix 边界规则段 MUST 含需求漏项、仓库选择不合理、验收标准不可判定、跨仓影响缺失四类

#### Scenario: 漏项夹具产出 must_fix

- **WHEN** 多仓夹具的需求漏掉某涉及仓的改动，且 reviewer 判定该漏项成立
- **THEN** 该轮结论 MUST 含 must_fix 级别的 finding

#### Scenario: 单仓文案逐字不变

- **WHEN** 构建单仓三环节 reviewer prompt
- **THEN** must-fix 边界规则段 MUST 与改造前逐字一致

### Requirement: 多仓 finding 完整性与旧轮证据保留（REQ-PLN-16）

多仓评审中，任一 finding 缺少非空 `evidence` 或缺少非空 `required_action` 时，系统 MUST NOT 将其收入 findings；该轮 MUST 判定为需要人工（`needs_human`）并携带诊断码 `malformed_findings`。

多仓评审的旧轮历史摘要 MUST 保留每个 finding 的 `message` 与 `evidence`；旧记录缺失该字段时 MUST 以显式占位值呈现，MUST NOT 静默丢弃。

单仓的 findings 解析与旧轮摘要 MUST 与改造前一致。

#### Scenario: 缺字段即不收

- **WHEN** 多仓评审输出含一条 `evidence` 为空的 finding
- **THEN** 该 finding MUST NOT 进入 findings，该轮 MUST 判为 `needs_human` 且诊断码 MUST 为 `malformed_findings`

#### Scenario: 旧轮保留 message 与 evidence

- **WHEN** 多仓评审进入第 2 轮并压缩第 1 轮历史
- **THEN** 第 1 轮摘要 MUST 含该轮每条 finding 的 message 与 evidence

#### Scenario: 单仓同输入行为不变

- **WHEN** 单仓评审输出同样缺少 `evidence` 的 finding
- **THEN** 行为 MUST 与改造前一致（照收该 finding），旧轮摘要格式 MUST 不变

### Requirement: 多仓 Work Item splitter 目录摘要与显式失败（REQ-PLN-17）

多仓 Work Item 拆分的目录摘要 MUST 分层呈现（顶层条目加一级子项，总行数受限），MUST NOT 使用单层截断 30 条。

目录读取失败时 MUST 以显式文本 `repository_structure_unavailable: <原因>` 进入材料与诊断；空仓 MUST 以 `repository_structure_empty: <路径>` 呈现，MUST NOT 以伪内容冒充空仓。

单仓拆分的目录摘要 MUST 与改造前逐字一致。

#### Scenario: 多层仓出现二级条目

- **WHEN** 多仓成员仓含多层目录并参与 Work Item 拆分
- **THEN** 目录摘要 MUST 含顶层条目与其一级子项

#### Scenario: 读取失败显式报错

- **WHEN** splitter 读取某成员仓目录失败
- **THEN** 材料 MUST 出现 `repository_structure_unavailable: <原因>`，MUST NOT 呈现为空仓伪内容

#### Scenario: 单仓摘要逐字不变

- **WHEN** 单仓 issue 执行 Work Item 拆分
- **THEN** 目录摘要 MUST 与改造前逐字一致（含既有 30 条截断与空仓文案）
