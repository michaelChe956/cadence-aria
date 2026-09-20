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

一期推进 SHALL 为人工显式：每个 target-attempt 的 coding provider 启动唯一入口 SHALL 保持显式 `StartCoding` 入站命令，SC admission 的 target-attempt 在经 advance 置 Ready 之前收到 StartCoding SHALL fail-closed 拒绝并返回 `SC_CODING_REQUIRES_ADVANCE`（per-attempt 适用，判定=advance record 与该 attempt 的绑定关系，未绑定或未 Ready 一律拒绝）。advance durable record SHALL 保持每 plan 恰一条（幂等不变）；其与 target-attempt 的绑定 SHALL 以 additive 方式扩展承载多 target-attempt 集（既有单 target 记录的 `attempt_id` 承载语义不变、零迁移）。系统 MUST NOT 自动顺序拉起、依赖驱动拉起或以任何编排动作隐式启动跨 target 的 target-attempts——推进顺序 SHALL 由人决定（可串行或并行对各自 Ready 的 target-attempt 发 StartCoding）。跨 target-attempt 依赖就绪门自动编排为二期 defer 项：触发条件=一期交付后的真实多仓使用证据，届时凭证据另行立项，本红线在本 change 内不被任何「简化版编排」变体突破。

#### Scenario: per-attempt 守卫零变化

- **WHEN** 某 SC admission 的 target-attempt 尚未被 advance 绑定置 Ready 时收到 StartCoding
- **THEN** 系统以 `SC_CODING_REQUIRES_ADVANCE` fail-closed 拒绝，该 attempt 与 session 状态不变，无 provider 启动

#### Scenario: 手动逐 target 推进

- **WHEN** 多 target 分流创建完成后全部 target-attempt 就绪
- **THEN** 无任何 provider 被自动启动；人对各自 target-attempt 显式发送 StartCoding（先后或并行）方进入执行

#### Scenario: 绑定关系 additive 扩展兼容存量

- **WHEN** 读取既有单 target 场景的 advance record 并判定其 attempt 的 Ready 状态
- **THEN** 判定语义与本 change 前逐字节等价（单值 attempt_id 精确匹配路径保留），多 target 集绑定仅对新场景生效

#### Scenario: 无自动跨 attempt 编排

- **WHEN** 某 target-attempt 完成、失败或进入任意状态转换
- **THEN** 系统不据此自动启动、排队或建议启动任何其他 target-attempt（编排缺席是一期红线）

### Requirement: group 级聚合只读视图与终态（REQ-MTG-04）

系统 SHALL 为每个 plan 提供 group 级聚合只读视图，与拆分能力同批交付：内容至少包括 per-target 投影（目标仓库身份、该 target-attempt 的状态与当前 stage、分支、head commit、推送状态、ReviewRequest 状态、失败/阻塞原因）与聚合终态。聚合终态 SHALL 三值：全部交付（每个 target-attempt 均达 REQ-COD-06 完成级别——Completed 且目标仓分支已推送且 ReviewRequest 已生成）、部分（任一 target 未满足，SHALL 显式呈现 partial failure 与未满足项，MUST NOT 伪装全局成功）、未启（无 target-attempt，或全部 target-attempt 均未达 provider 启动——判定优先于「部分」：仅当存在任一 target-attempt 已达 provider 启动且非全部交付时方落「部分」）。判定口径 SHALL 与 `per-repo-coding-execution` 的每仓交付聚合（每 target 取最新 attempt、Completed+已推送才算交付）一致。该视图 SHALL 从 target-attempt/unit durable 事实确定性派生，SHALL NOT 形成第二套状态机；各 target-attempt 的 group coding workspace 仍是其执行观察面（REQ-GCE-04 语义 per-attempt 适用），聚合视图 SHALL NOT 承载任何决策、启动或中止操作面。

#### Scenario: 聚合视图呈现 per-target 与终态

- **WHEN** 客户端读取多 target plan 的聚合视图
- **THEN** 每个 target 可读到其 attempt 状态/stage、分支、commit、推送与评审状态及失败/阻塞原因，并读到三值聚合终态；数据均派生自 target-attempt durable 事实

#### Scenario: partial failure 显式呈现

- **WHEN** 某 target-attempt 未达完成级别（失败、未推送或未完成）而另一 target 已交付
- **THEN** 聚合终态为部分，未满足 target 与原因显式呈现，系统不将该 plan 呈现为全部交付

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
