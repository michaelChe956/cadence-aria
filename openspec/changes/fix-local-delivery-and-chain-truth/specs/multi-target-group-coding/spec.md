# Spec Delta

> 修订基线：`openspec/specs/multi-target-group-coding/spec.md` 现行 REQ-MTG-04。MODIFIED 为完整替换文本，在原文基础上把「已推送」判定扩展为统一的「已交付」谓词。REQ-MTG-06 至 REQ-MTG-10 为新增。
>
> 与未归档的 `fix-multi-repo-group-coding-start`（`coding-start` capability，REQ-CS-04/06）的关系：REQ-MTG-09、REQ-MTG-10 不修改 REQ-CS-06，只补充它未覆盖的 attempt 状态守卫；两者并存，见 proposal 时序说明。

## MODIFIED Requirements

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

## ADDED Requirements

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
