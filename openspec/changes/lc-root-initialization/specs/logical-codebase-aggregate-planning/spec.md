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

系统 SHALL 每次规划 run 固化 `PlanningContextSnapshot`（membership_revision、每仓 checkout revision/dirty/availability、index revision、policy digest、canonical working directory、target identity 与 access fingerprint），作为 context/cwd/prompt/audit 的唯一依据。resume、revision、follow-up 与 split planning SHALL 使用同一 snapshot 合同。

#### Scenario: 恢复/续接规划会话

- **WHEN** 恢复或续接规划会话且 cwd、target、policy 或成员快照任一指纹不一致
- **THEN** 系统 SHALL 拒绝沿用旧 session，标记旧会话 superseded 或要求重建，并以新的 root cwd 与 target 重新生成上下文

### Requirement: 规划只读边界（REQ-PLN-06）

规划 provider 会话 SHALL 为只读语义的 best-effort（`best_effort_configured`：Aria-owned 配置 + canonical root cwd + target 声明 + pre/post 检测）；仅在固定 provider/OS 越界写 fixture 通过后才可升级为 `production_verified_readonly`；未达到该级别时不得宣称“物理上无法写入”。非编码角色 SHALL 保持 built-in 文件写工具拒绝，provider-native MCP 写能力仍按既有受信任自发现边界处理。

#### Scenario: 规划运行

- **WHEN** 规划 run 完成
- **THEN** 系统 SHALL 报告 best-effort 只读状态（配置目标、canonical root cwd、独立 target 与前后检测），成员主 checkout 与聚合根无规划产生的已知写入；硬只读证据不足时不得扩大 writable root 或自动转为编码路径

### Requirement: 唯一规划 launch/resolver（REQ-PLN-07）

系统 SHALL 提供唯一规划 launch/resolver，使 Story、Design、WorkItemPlan、review/revision、resume 与 WebSocket follow-up 的 context、canonical root cwd、独立 target、prompt、session audit 均来自同一 `PlanningContextSnapshot`；禁止任何 `issue.repo_id`、first Story fallback 或以 target worktree 覆盖 root cwd。

#### Scenario: 规划全链路一致

- **WHEN** 启动、run-next、revision、resume、review 或 WebSocket follow-up
- **THEN** 各环节 SHALL 使用同一 snapshot 派生 root cwd 与 target；cwd 与 target 不一致但均通过 authority/gateway/checkout 校验时 SHALL 放行，单仓两字段继续映射为原有目录
