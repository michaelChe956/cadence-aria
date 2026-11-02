# Spec Delta

## MODIFIED Requirements

### Requirement: 聚合规划上下文（REQ-PLN-01）

系统 SHALL 将规划类 workspace 上下文从单仓库改为逻辑代码库聚合：包含 logical_codebase 引用、成员 inventory（id/alias/path/role/profile 摘要）、focus 成员与索引 revision；prompt 注入紧凑成员清单与预算，不注入全部成员源码；超预算时确定性截断并标记未检索成员。逻辑代码库 provider 进程的 `cwd` SHALL 固定为 canonical `provider_context_root`，target/member/checkout identity SHALL 独立传递；单仓 provider cwd 行为保持不变。

#### Scenario: 聚合上下文注入

- **WHEN** 为逻辑代码库下的 Issue 启动 Story/Design/Work Item 计划生成
- **THEN** provider SHALL 从聚合根（provider_context_root）只读启动，prompt 注入紧凑成员清单与预算控制，不注入全部成员源码；target 可为选定 member/checkout，且允许 cwd 与 target 不同

#### Scenario: 预算超限

- **WHEN** 成员 inventory/摘要/证据超出 token 或 byte 预算
- **THEN** 系统 SHALL 按确定性顺序截断并标记未检索成员，不撑爆 context window，且不得以切换到成员 cwd 规避预算

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
