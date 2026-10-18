# Spec Delta: fix-planning-session-anchor-recovery × logical-codebase-aggregate-planning

## MODIFIED Requirements

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
- **THEN** 系统 SHALL 继续按 snapshot 检测变化并拒绝旧会话，不因聚合根无 Git 而免检成员事实

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

### Requirement: 唯一规划 launch/resolver（REQ-PLN-07）

系统 SHALL 提供唯一规划 resolver，使 Story、Design、WorkItemPlan、review/revision、resume 与 WebSocket follow-up 的 context、canonical root cwd、独立 native target、业务材料/compile target、prompt、session audit 均来自同一 PlanningContextSnapshot/authority。native anchor 与业务 target SHALL 分离派生；禁止 issue.repo_id/first Story fallback、另写第二选仓算法或以 target worktree 覆盖 root cwd。

#### Scenario: 规划全链路一致
- **WHEN** 启动、run-next、revision、resume、review 或 WebSocket follow-up
- **THEN** 各环节 SHALL 同源派生 canonical root cwd/native anchor 与原业务 repository；合法 cwd≠业务 target 继续放行，单仓两字段保持原目录

#### Scenario: 聚合视图不得成为 IR 成员身份
- **WHEN** SC Plan 从生成 source 降为 IR 并进入 logical compile
- **THEN** 材料 SHALL 从原业务 repository 读取，IR target SHALL 是原有效 physical member ID；aggregate-root view ID SHALL NOT 写入 IR 或冒充成员 identity

## ADDED Requirements

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
