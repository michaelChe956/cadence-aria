# multi-target-group-coding Specification

## Purpose
mixed-target WorkItemGroup 按 target 分流执行的一期契约：每个目标仓库一个独立 target-attempt（REQ-COD-02 单快照不变式 per-attempt 保全、单值持久化 schema 零改动）、建组/advance 编排按 target 显式分流（单 target 场景零变化）、一期推进为人工显式（无自动跨 attempt 编排）、group 级聚合只读视图/终态与拆分同批交付、attempt 增殖审计与恢复一致性。跨 attempt 依赖就绪门自动编排为二期，凭一期证据另行立项。

## Requirements

### Requirement: 按 target 分流创建与 per-(plan,target) 唯一性（REQ-MTG-01）

建组与 advance 初始化 SHALL 按 target 显式分流：解析 authoritative plan binding 后按 unit 的 `target_repository_id` 分组——单一 target（含无 target 且 selection focus 唯一）SHALL 走既有建组路径且行为与本 change 前完全一致（单 attempt、收敛路由、零变化）；多 target SHALL 为每个目标仓库分别建立独立的 target-attempt（scope 为 WorkItemGroup、绑定同一 plan），MUST NOT 以单一 attempt 承载多 target。target-attempt 唯一性 SHALL 为 per-`(plan, target)`：同一 `(plan, target)` 至多一个 target-attempt，重复建组/advance/重放 SHALL 命中既有 target-attempt 而非新建。units SHALL 按 target 归属分入各自 target-attempt 的 coding units；无 target 归属且 selection focus 不唯一 SHALL 保持 fail-closed 拒绝并返回稳定错误码。创建、恢复、replay 三处校验与全部路由消费面 SHALL 一致适用本分流规则；「group 内存在多 target」本身 SHALL NOT 构成拒绝理由。

#### Scenario: mixed-target group 按 target 分流创建

- **WHEN** 对涉及多个目标仓库的 Work Item 建立 WorkItemGroup 并推进建组
- **THEN** 系统为每个目标仓库分别建立独立 target-attempt，各自冻结该 target 的快照并承载该 target 的 units，不返回多目标拒绝错误

#### Scenario: 单 target 场景零变化

- **WHEN** group 全部 units 属同一目标仓库（或无 target 且 focus 唯一）
- **THEN** 建组、路由、advance 与执行行为与本 change 前完全一致：单 attempt、单快照、既有收敛路径不变

#### Scenario: per-(plan,target) 唯一与幂等命中

- **WHEN** 同一 plan 的建组或 advance 因重放/重复命令再次解析出同一 target 集合
- **THEN** 每 `(plan, target)` 仍恰一个 target-attempt（命中既有实例返回其 durable 状态），不新建、不重复初始化

#### Scenario: 无唯一 target 归属 fail-closed

- **WHEN** units 均无 `target_repository_id` 且 issue codebase selection focus 不唯一
- **THEN** 建组被拒绝并返回稳定错误码（TargetMissing 语义），不静默选择任一 target

### Requirement: per-target 冻结快照与单 worktree 不变式（REQ-MTG-02）

每个 target-attempt SHALL 各自冻结其目标仓库的 `AttemptTargetSnapshot`（三层身份映射、revision、policy digest、membership revision），REQ-COD-02 的「创建、恢复、重放一律使用冻结快照、不得从活 Work Item 重新猜测」语义 SHALL per-attempt 原样适用。`CodingExecutionAttempt` 的单值持久化字段（worktree_path、head_commit、stage、target_snapshot）SHALL 保持单值语义：每 attempt 恰对应一个 worktree、一个 head、一个 target 快照，MUST NOT 数组化或以集合字段承载多 target。target-attempt 的路由权威 SHALL 为其自身冻结快照（恢复、重放、半启动恢复按各自快照与 worktree 独立运转），MUST NOT 从 plan binding 的 units 重新收敛单一 target。同 target 的串行（REQ-COD-03 同仓串行与三元键 worktree lock）与 attempt 内 unit 级约束（REQ-GCE-01 共享 worktree 单 active unit）SHALL 原样适用；异 target 的 target-attempts SHALL 可并行存在与执行（各自 worktree lock 隔离）。

#### Scenario: 每 target-attempt 各持冻结快照

- **WHEN** 多 target 分流创建后创建、恢复或重放任一 target-attempt
- **THEN** 该 attempt 使用其自身冻结的目标快照与 worktree，与他 target-attempt 互不依赖、互不阻塞

#### Scenario: 单值不变式拒绝多 target 承载

- **WHEN** 任何代码路径试图使单一 attempt 同时承载多 target（快照集合、多 worktree）
- **THEN** 该形态不存在于持久化 schema 与执行模型中（单值字段零改动红线），多 target 只能经分流为多 target-attempt 表达

#### Scenario: 异 target 并行

- **WHEN** 同一 plan 的两个 target-attempt 分别处于执行中
- **THEN** 两者各自经 `(project, issue, repository)` 三元键 worktree lock 隔离，可并行推进；同 target 的第二个 active attempt 仍被拒绝（per-(issue,target) 单 active）

### Requirement: 手动推进与启动门（REQ-MTG-03）

**多 target** 推进 SHALL 以人工显式为默认：每个 target-attempt 的 coding provider 首启唯一入口 SHALL 保持显式 `StartCoding` 入站应用命令，SC admission 的 target-attempt 在经 advance 置 Ready 之前收到 StartCoding SHALL fail-closed 拒绝并返回 `SC_CODING_REQUIRES_ADVANCE`（per-attempt 适用，判定=advance record 与该 attempt 的绑定关系，未绑定或未 Ready 一律拒绝）。advance durable record SHALL 保持每 plan 恰一条（幂等不变）；其与 target-attempt 的绑定 SHALL 以 additive 方式扩展承载多 target-attempt 集（既有单 target 记录的 `attempt_id` 承载语义不变、零迁移）。在无精确自动化授权（无 enrollment、enrollment 关闭或不匹配）时，系统 MUST NOT 自动顺序拉起、依赖驱动拉起或以任何编排动作隐式启动跨 target 的 target-attempts——多 target 的推进顺序 SHALL 由人决定（可串行或并行对各自 Ready 的 target-attempt 发 StartCoding），不能以「循环中一次仅启动一个」规避禁止批量编排。

仅 `work-item-group-autopilot` 显式 opt-in enrollment 精确绑定授权 target（一个单仓物理仓——plan 全部工作项无 logical target 归属、attempt 无逻辑代码库 target 快照；一个逻辑代码库单成员；或 logical codebase 成员集合）且 plan 实际桶满足 ∅≠P⊆A、无未归属 WI 时，服务端方可经共享 plan-run 驱动自动推进：以独立共用 StartCoding 命令与 Enrolled advance（`AutoStartOnce` 单发许可）首启与续驱全部实际桶，首启与续驱 SHALL 沿 `CodingPlanSelection::deterministic_start_command_id` 稳定命令身份；全部自动桶（尤其非首桶）SHALL 以 Enrolled origin 启动并逐桶核对 policy/enrollment/binding；上游完成挂点 SHALL 自动启动下游就绪桶。撤销、绑定不符、越权成员、同一 plan 出现授权外 attempt 或 terminated SHALL 零 provider、禁止降级 Manual。这不构成手动顺序的变更：手动与无授权场景的推进语义保持本 requirement 第一段不变；无授权集合的跨 target-attempt 依赖就绪门自动编排仍为 defer 项，本红线不被任何「简化版编排」变体突破。

#### Scenario: per-attempt 守卫零变化

- **WHEN** 某 SC admission 的 target-attempt 尚未被 advance 绑定置 Ready 时收到 StartCoding
- **THEN** 系统以 `SC_CODING_REQUIRES_ADVANCE` fail-closed 拒绝，该 attempt 与 session 状态不变，无 provider 启动

#### Scenario: 手动逐 target 推进

- **WHEN** 多 target 分流创建完成后全部 target-attempt 就绪，且该 plan 无精确自动化授权
- **THEN** 无任何 provider 被自动启动；人对各自 target-attempt 显式发送 StartCoding（先后或并行）方进入执行

#### Scenario: 绑定关系 additive 扩展兼容存量

- **WHEN** 读取既有单 target 场景的 advance record 并判定其 attempt 的 Ready 状态
- **THEN** 判定语义与本 change 前逐字节等价（单值 attempt_id 精确匹配路径保留），多 target 集绑定仅对新场景生效

#### Scenario: 无自动跨 attempt 编排

- **WHEN** 某 target-attempt 完成、失败或进入任意状态转换，且无精确自动化授权
- **THEN** 系统不据此自动启动、排队或建议启动任何其他 target-attempt

#### Scenario: 集合授权的共享 plan-run 自动驱动

- **WHEN** 成员集合 enrollment 精确绑定该 plan（实际桶 P⊆授权集合 A），plan 已确认
- **THEN** 服务端经 Enrolled advance 置各实际桶 Ready、以 AutoStartOnce 单发与稳定命令身份自动首启全部桶；上游完成后自动启动下游就绪桶，各桶 origin 均为当前 binding 的 Enrolled

#### Scenario: 撤销或绑定不符零启动

- **WHEN** 自动驱动过程中 enrollment 被撤销、绑定被篡改、发现越权成员或 plan 已 terminated
- **THEN** 后续自动首启/续驱零 provider、不降级 Manual，停在该点等待显式处理

#### Scenario: 单 target enrollment 的唯一例外

- **WHEN** enrolled plan、权威 target 及冻结 attempt 均精确指向同一 logical repository，advance 已将该唯一 attempt 置 Ready
- **THEN** 有效 per-attempt opt-in 可通过独立 StartCoding 单发首启；advance 本身仍不启动 provider，AI choice/人工门停等保留（正常批准按 `work-item-group-autopilot` REQ-WIGA-02 自动通过）

#### Scenario: 单仓 enrollment 的唯一例外

- **WHEN** 单仓 enrollment 的 target 等于 issue 所属物理仓，plan 全部工作项无 logical target 归属，advance 已将唯一 attempt 置 Ready
- **THEN** 有效 per-attempt opt-in 可通过独立 StartCoding 单发首启；advance 本身仍不启动 provider，AI choice/人工门停等保留（正常批准按 `work-item-group-autopilot` REQ-WIGA-02 自动通过）

#### Scenario: 多 target enrollment 不能泛化为逐个自动发送

- **WHEN** 同一 plan 出现授权集合外的 target-attempt、集合 enrollment 的 plan 混入手动仓 attempt，或 enrollment 范围与冻结 attempt 不符
- **THEN** 所有对该 attempt 的自动首启均拒绝，不因单次只有一个 attempt 就突破授权边界

### Requirement: group 级聚合只读视图与终态（REQ-MTG-04）

系统 SHALL 为每个 plan 提供 group 级聚合只读视图，与拆分能力同批交付：内容至少包括 per-target 投影（目标仓库身份、该 target-attempt 的状态与当前 stage、分支、head commit、推送状态、ReviewRequest 状态、失败/阻塞原因）与聚合终态。聚合终态 SHALL 三值：全部交付（每个 target-attempt 均达 REQ-COD-06 完成级别——Completed、目标仓分支已推送或未配置远端而本地交付、且 ReviewRequest 已生成）、部分（任一 target 未满足，SHALL 显式呈现 partial failure 与未满足项，MUST NOT 伪装全局成功）、未启（无 target-attempt，或全部 target-attempt 均未达 provider 启动——判定优先于「部分」：仅当存在任一 target-attempt 已达 provider 启动且非全部交付时方落「部分」）。

「已交付」谓词 SHALL 单一定义：最新 ReviewRequest 的推送状态为 `pushed` 或 `skipped_not_configured`。plan 级聚合、issue 级交付聚合与前端交付徽标 SHALL 共用该谓词，MUST NOT 各自直接比较 `pushed`。推送状态为 `skipped_not_configured` 的 target SHALL 在 per-target 投影中与 `pushed` 区分呈现，聚合终态为全部交付时仍须能看出哪些 target 为本地交付。

判定口径 SHALL 与 `per-repo-coding-execution` 的每仓交付聚合（每 target 取最新 attempt、Completed 且满足已交付谓词才算交付）一致。该视图 SHALL 从 target-attempt/unit durable 事实确定性派生，SHALL NOT 形成第二套状态机；各 target-attempt 的 group coding workspace 仍是其执行观察面（REQ-GCE-04 语义 per-attempt 适用），聚合视图 SHALL NOT 承载任何决策、启动或中止操作面。

#### Scenario: 聚合视图呈现 per-target 与终态

- **WHEN** 客户端读取多 target plan 的聚合视图
- **THEN** 每个 target 可读到其 attempt 状态/stage、分支、commit、推送与评审状态及失败/阻塞原因，并读到三值聚合终态；数据均派生自 target-attempt durable 事实

#### Scenario: partial failure 显式呈现

- **WHEN** 某 target-attempt 未达完成级别（失败、未推送或未完成）而另一 target 已交付
- **THEN** 聚合终态为部分，未满足 target 与原因显式呈现，系统不将该 plan 呈现为全部交付

#### Scenario: 本地交付计入全部交付

- **WHEN** 多 target plan 的各 target 最新 attempt 均为 Completed，其中一个 target 的最新 ReviewRequest 推送状态为 `pushed`，另一个为 `skipped_not_configured`
- **THEN** 聚合终态为全部交付，且后者在 per-target 投影中显示为本地交付而非已推送

#### Scenario: 未配置远端不掩盖真实推送失败

- **WHEN** 一个 target 的推送状态为 `skipped_not_configured`，另一个 target 的推送状态为 `failed`
- **THEN** 聚合终态为部分，失败 target 作为未满足项显式呈现，本地交付的 target 不被误判为失败

#### Scenario: 聚合视图只读派生

- **WHEN** 聚合视图被读取或刷新
- **THEN** 其内容由 target-attempt/unit 状态确定性派生，不存在独立持久化的聚合状态机；经该视图不能触发任何决策或执行动作

#### Scenario: 同批交付

- **WHEN** 按 target 分流创建能力验收
- **THEN** 聚合只读视图（含前端呈现）已同批交付可用，不存在「拆分已交付而聚合视图缺席」的中间交付态

### Requirement: attempt 增殖审计与恢复一致性（REQ-MTG-05）

按 target 分流的每次 target-attempt 创建 SHALL 落 durable 审计记录：至少包括 plan 身份、目标仓库身份、target-attempt 身份、分流依据（authoritative binding 解析依据/revision）与触发入口（advance 命令键或建组入口）。恢复、重放与检索 SHALL 可追溯每个 target-attempt 的分流来源；同一 plan 的多 attempt 检索语义 SHALL 按 `(plan, target)` 消解（MUST NOT 依赖「取最早」等在增殖后有歧义的检索语义表达唯一性）。恢复矩阵 SHALL 覆盖创建、恢复、replay、半启动恢复、断连重连 × 单 target（回归零变化）与多 target（每 attempt 独立等价单 target 语义）的组合，作为本 capability 验收口径。增殖审计面 SHALL 作为二期（跨 attempt 依赖就绪门自动编排）立项的证据来源——审计只记录事实，MUST NOT 承载调度或编排逻辑。

#### Scenario: 分流创建留审计

- **WHEN** 多 target 分流创建任一 target-attempt
- **THEN** durable 审计记录落盘且含 plan、target、attempt 身份与分流依据，恢复/重放后仍可追溯

#### Scenario: 恢复矩阵等价语义

- **WHEN** 多 target 场景下任一 target-attempt 经历创建、恢复、replay、半启动恢复或断连重连
- **THEN** 其语义与单 target attempt 等价（按自身快照与 worktree 独立运转），不因其他 target-attempt 的存在或状态而变化

#### Scenario: 审计不承载编排

- **WHEN** 审计面记录 attempt 状态与增殖事实
- **THEN** 其内容仅用于追溯与二期立项证据，不触发任何自动推进、排队或依赖判定

### Requirement: plan 级聚合最终确认（REQ-MTG-11）

系统 SHALL 提供无 body 的 `POST /api/projects/{project_id}/issues/{issue_id}/work-item-plans/{plan_id}/coding-final-confirm`：对当前 plan-run 冻结 targets（≥2 才呈现聚合按钮）按 target 顺序逐个调用既有且不修改的最终确认函数，跳过已 Completed、首失败即停，返回逐仓结果并明确失败仓。`GET …/coding-start` 的 plan 聚合进度 SHALL 新增 `final_confirm` 四态（not_ready/ready/partial/done，含 ready/confirmed attempt 集与 blocking 原因）与复用 A1 类型的 `attention_items`；GET 只读持久事实，git/owner/范围检查仍由既有确认函数执行；逐 attempt 原确认按钮 SHALL 保留。

#### Scenario: 全齐后一次确认全部

- **WHEN** 多仓全部 attempt 待最终确认且 readiness 完整（GET final_confirm.state=ready），用户点「确认本计划全部完成（M 仓/N 项）」
- **THEN** 恰发出一个 POST，后端按序逐仓确认全部成功，返回 outcome=all_confirmed 与每仓 results（status=confirmed）

#### Scenario: 首失败停止并显示失败仓

- **WHEN** 聚合确认中第 2 个仓确认失败
- **THEN** 第 3 个及之后的仓未被调用（status=not_attempted），响应 outcome=partially_confirmed 且第 2 仓的 error_code/message 可见，用户可重试

#### Scenario: 未齐不出现聚合按钮

- **WHEN** 存在未完成的执行、或剩余 attempt 未全部 WaitingForHuman＋FinalConfirm、或 units 未全 Completed、或 readiness snapshot 不满足身份匹配＋Complete＋diagnostics 空
- **THEN** final_confirm.state 为 not_ready（或 partial 且剩余未就绪），聚合确认不可用，blocking 指明未齐的 attempt 与原因

#### Scenario: 响应形状与幂等边界

- **WHEN** POST 执行完成
- **THEN** 响应为 `{plan_id,outcome:all_confirmed|partially_confirmed|not_ready,results:[{attempt_id,target_repository_id,status:confirmed|already_completed|failed|not_attempted,error_code?,message?}]}`；不加 command_id/generation 防重放（B1-e 不做，已知风险）

### Requirement: 只读 plan 总览与待办直达（REQ-MTG-12）

系统 SHALL 提供只读 plan 总览页 `/workbench/projects/$projectId/issues/$issueId/coding-plans/$planId`：按 order_index 列出全部 WI（逻辑 ID、summary 缺省用 ID、仓库、unit 进度、attempt 状态、依赖/等待原因、待办徽标、详情链接），标题显示执行/确认 N/M。`usePlanCodingProgress` SHALL 统一返回 `{data,loading,error,refetch}` 并支持可选轮询（总览可见时 5 秒）。待办徽标 SHALL 复用同源 attention 事实（A1），点击按 `target+focus{gate_id,choice_id}` 进入对应 attempt 真实详情并可返回；总览页除 plan 聚合确认外 MUST NOT 提供业务写操作。

#### Scenario: 一页列全 WI 并进入真实详情

- **WHEN** 用户打开多仓 plan 的总览页
- **THEN** 全部 WI 按序列出并显示各仓状态与待办徽标；点击某 WI 行进入该 attempt 的真实对话/审查/进度详情（复用既有 WS 单详情切换），返回后仍在总览

#### Scenario: 徽标直达可处理卡片

- **WHEN** 用户点击某 WI 的 choice/gate 待办徽标
- **THEN** 页面导航到对应 attempt 并定位/滚动到该 choice 或 gate 卡片（focus 生效），处理入口可用；多 WI 共一 attempt 时注明 attempt 历史范围，不伪造精确筛选

#### Scenario: 轮询见非当前仓进展

- **WHEN** 总览页保持可见且另一仓的 attempt 状态推进
- **THEN** 5 秒轮询刷新后该仓状态更新，无需手动刷新；加载/失败有明确提示

### Requirement: 仪表盘 attempt 全态状态条（REQ-MTG-13）

coding 仪表盘 SHALL 在保留 unit 拓扑判据不变的前提下新增独立 attempt 状态展示（`selectCodingDashboardAttempts`）：多仓取聚合 target_attempts、单仓取当前 session，按 `{attemptId,repositoryName,status,stage,attentionKinds}` 完整覆盖 11 个 attempt 状态；WaitingForHuman＋FinalConfirm SHALL 显示「等待最终确认」，MUST NOT 统一叫分诊。

#### Scenario: 11 态全覆盖且最终确认不叫分诊

- **WHEN** 仪表盘接收 11 种 attempt 状态的参数化夹具（created/running/waiting_for_human/blocked/awaiting_manual_recovery/awaiting_plan_amendment/applying_plan_amendment/amendment_apply_failed/completed/failed/aborted，含 WaitingForHuman＋FinalConfirm）
- **THEN** 每种状态显示既有 ATTEMPT_STATUS_LABELS/STAGE_LABELS 的正确文案，最终确认态显示「等待最终确认」；unit 已完成与 attempt 待确认可同时呈现

#### Scenario: 单仓显示当前 attempt 且拓扑不变

- **WHEN** 单仓 attempt 的 coding 页渲染仪表盘
- **THEN** 状态条显示当前 session 的 attempt；unit 六态拓扑判据与投影和改前一致

### Requirement: 多 target 逐仓交付面板（REQ-MTG-06）

work_item_group attempt 的 coding 页面在该 plan 绑定不少于 2 个 target-attempt 时，SHALL 对每个 target 渲染一行只读交付信息，至少含仓库名称、分支与基线分支、worktree 路径、attempt 状态、stage、head commit、推送状态、ReviewRequest 标识与失败/阻塞原因；推送状态为 `skipped_not_configured` 的行 SHALL 显示「本地交付(无远端)」。不满足该判据时，页面 SHALL 保持现有单 attempt 顶部信息，DOM 与文案零变化。该面板 MUST NOT 承载决策、启动、中止、重推或恢复操作。

#### Scenario: 多仓逐仓渲染

- **WHEN** work_item_group 的 plan 绑定两个 target-attempt，一个推送状态为 `pushed`，另一个为 `skipped_not_configured`
- **THEN** 页面渲染两行，分别含各自仓库名、分支、worktree、状态、stage、提交与推送状态，其中后一行显示「本地交付(无远端)」，两行互不覆盖

#### Scenario: 单仓页面零变化

- **WHEN** coding 页面的 plan 只绑定一个 target-attempt，或 attempt scope 不是 work_item_group
- **THEN** 页面不渲染逐仓面板，顶部分支、worktree、提交与推送状态的呈现与本 change 前一致

#### Scenario: 面板无操作入口

- **WHEN** 逐仓面板被渲染
- **THEN** 面板内不存在决策、启动、中止、重推或恢复的操作入口

### Requirement: plan 聚合进度 target 项的逐仓事实字段（REQ-MTG-07）

plan 聚合进度的 target 项（`target_attempts[]`）SHALL 在保留既有五个字段（`target_repository_id`、`attempt_id`、`attempt_status`、`worktree_path`、`lease_state`）不变的前提下，additive 增加九个字段，字段名为 snake_case，类型均为「字符串或 null」，null 表示无对应事实：`repository_name`、`branch_name`、`base_branch`、`stage`、`head_commit`、`push_status`、`push_error`、`review_request_id`、`manual_recovery_reason`。各字段的来源与取值域见下列场景，字段名与类型一经本 requirement 冻结，前后端 MUST NOT 各自变更。

#### Scenario: 字段来源与取值域

- **WHEN** 客户端读取某 target 项
- **THEN** `repository_name` 为目标仓展示名，解析自 checkout 路径末段，解析不出时回落为目标仓标识字符串；`branch_name`、`base_branch`、`stage`、`head_commit` 取自该 target 最新绑定 attempt，`stage` 为 snake_case 取值；`push_status`、`push_error`、`review_request_id` 取自该 attempt 最新一条 ReviewRequest，`push_status` 取值域为 `not_pushed`、`pushed`、`failed`、`skipped_not_configured`；`manual_recovery_reason` 为 attempt 记录的人工恢复原因，开放值集，原样透传

#### Scenario: 数据契约 additive

- **WHEN** 旧客户端读取带有九个新字段的 plan 聚合进度，或新客户端读取缺少新字段的旧响应
- **THEN** 既有五个字段的名称、类型与语义不变，旧客户端忽略新字段后功能不受影响；新客户端按 null 处理缺失字段

#### Scenario: 无 ReviewRequest 或成员已删

- **WHEN** 某 target 尚无 ReviewRequest，或其冻结的成员 attempt 已被独立删除
- **THEN** 无 ReviewRequest 时 `push_status`、`push_error`、`review_request_id` 为 null；成员已删时 attempt 派生字段为 null，`attempt_status` 沿既有的已中止投影；系统不猜测或补造事实

#### Scenario: 人工恢复原因透传

- **WHEN** 某 target-attempt 处于 awaiting_manual_recovery，原因码为 `coding_runner_failed_while_running`
- **THEN** 该 target 项的 `manual_recovery_reason` 原样为该原因码，逐仓面板将其作为失败/阻塞原因呈现

#### Scenario: 客户端对未知取值的容错

- **WHEN** 客户端遇到未知的 `push_status` 或 `stage` 取值
- **THEN** 客户端回落显示原文，不报错也不丢行

### Requirement: 执行状态以所属 attempt 状态为守卫（REQ-MTG-08）

plan 聚合进度中每个 Work Item 的 `execution_state` SHALL 以其所属 target-attempt 的状态为守卫：仅当 attempt 状态为 Running，并且 provider 启动事实成立（启动 claim 相位已达 provider 可能已启动、active 指针指向该 unit、unit 为 Running）时，才 MAY 投影为 `running`。attempt 状态不是 Running 时 MUST NOT 投影为 `running`。该守卫 SHALL 仅作用于读取与展示层，MUST NOT 改写 unit 状态、启动 claim 相位或 active 指针等底层事实：AMR 下 unit 保持 Running 是恢复复用的前提。

#### Scenario: Running 的 attempt 仍显示执行中

- **WHEN** attempt 为 Running，active unit 为 Running 且 provider 启动 claim 成立
- **THEN** 该 unit 的 `execution_state` 仍为 `running`，既有投影语义不变

#### Scenario: attempt 非 Running 不得显示 running

- **WHEN** attempt 为 Created（启动已被接受但尚未经 admission 进入 Running）、Blocked、WaitingForHuman、Failed、Aborted 或 awaiting_manual_recovery，且 unit 仍为 Running
- **THEN** 该 unit 的 `execution_state` 不为 `running`；Created 回落既有的 `initialized`

#### Scenario: 底层事实不被改写

- **WHEN** 投影因 attempt 状态而不显示 `running`
- **THEN** 该 unit 的持久化状态、启动 claim 与 active 指针保持原值；显式恢复后同一 unit 从原位继续，无需重建

### Requirement: attempt 级执行状态取值（REQ-MTG-09）

`execution_state` 值域 SHALL 在既有取值基础上 additive 增加四个取值：`awaiting_manual_recovery`（attempt 为 awaiting_manual_recovery，`waiting_reason` 透传该 attempt 的人工恢复原因，开放值集）、`waiting_human`（attempt 为 Blocked 或 WaitingForHuman）、`failed`（attempt 为 Failed）、`aborted`（attempt 为 Aborted）。attempt 级取值 SHALL 优先于 unit 级的等待与执行投影；已完成的 unit 仍为 `completed`，plan-run 失效仍为 `invalidated`，二者优先级不变。

#### Scenario: AMR 的 attempt 不显示执行中

- **WHEN** 某 target-attempt 处于 awaiting_manual_recovery，其 active unit 仍为 Running 且启动 claim 已达 provider 可能已启动
- **THEN** 该 unit 的 `execution_state` 为 `awaiting_manual_recovery`，`waiting_reason` 为该 attempt 的人工恢复原因，界面 MUST NOT 显示 provider 运行中

#### Scenario: 其他 attempt 状态的取值

- **WHEN** attempt 为 Blocked、WaitingForHuman、Failed 或 Aborted，且其下 unit 未完成
- **THEN** 该 unit 的 `execution_state` 依次为 `waiting_human`、`waiting_human`、`failed`、`aborted`

#### Scenario: 已完成事实不被覆盖

- **WHEN** 某 unit 已 Completed，而其所属 attempt 因后续阶段（如推送）转入 awaiting_manual_recovery
- **THEN** 该 unit 的 `execution_state` 仍为 `completed`，其依赖方的等待判定不受影响

### Requirement: 多 target 仪表盘按 attempt 状态覆盖（REQ-MTG-10）

聚合进度绑定不少于 2 个 target-attempt 时，仪表盘对未完成 unit 的六态投影 SHALL 受所属 attempt 状态覆盖：attempt 为 awaiting_manual_recovery 或 Failed 时，其下未完成的 unit SHALL 呈现为失败态；已完成的 unit 保持完成态；其余 attempt 状态沿用现有 unit 状态映射。单 target 与 v1 单 attempt 的仪表盘路径 SHALL 零变化。

#### Scenario: 仪表盘把 AMR 画成失败

- **WHEN** 多 target 的 plan 中，某 attempt 为 awaiting_manual_recovery，其下一个 Running unit 与一个 Pending unit 尚未完成
- **THEN** 仪表盘将这两个未完成 unit 呈现为失败态，同 plan 其他 attempt 的 unit 及已完成 unit 的呈现不变

#### Scenario: 单 target 路径零变化

- **WHEN** 聚合进度只绑定一个 target-attempt，或页面为 v1 单 attempt
- **THEN** 仪表盘的六态映射与本 change 前一致
