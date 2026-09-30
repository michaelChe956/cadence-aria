# Spec Delta

## ADDED Requirements

### Requirement: 自动化 target 的载体判定与单仓语义（REQ-ROUTE-C5-TARGET）

自动化 target 投影、Enable、prepare、Enrolled advance 与自动 StartCoding SHALL 以唯一 authority 解析 issue 的载体归属：issue 无逻辑代码库归属且无 legacy 别名逻辑代码库时为单仓（Single-Repository）载体，否则为逻辑代码库载体；authority 冲突 SHALL fail-closed 并沿用既有 `repository_routing_*` 稳定错误码与 4xx 映射。单仓载体的 target SHALL 为 issue 记录所属物理仓的真实 `RepositoryRecord.id`：issue 无所属物理仓、该物理仓未登记、同一 git 根存在别名记录或与 legacy 逻辑代码库成员来源冲突时 SHALL 拒绝，不得猜测或回退其他仓。单仓 target 的 wire 表示 MUST NOT 包含 logical codebase、logical repository 或 checkout 身份，自动化 target 投影对单仓 MUST NOT 返回 logical repository 身份。单仓载体的后续解析 SHALL 沿物理仓与单仓 shared worktree 路径，MUST NOT 经逻辑代码库 provider gateway 启动；逻辑代码库载体保持既有 gateway、authority-only 与单 target 约束不变。代码中的 `RepositoryRouting::Legacy` 分支名保持不变，其语义 SHALL 被理解为单仓载体而非待淘汰路径。

#### Scenario: 单仓投影返回真实物理仓 target

- **WHEN** 单仓 issue 的所属物理仓已登记，客户端请求自动化 target 投影
- **THEN** 响应的 target 为单仓类型且 repository 身份等于该 `RepositoryRecord.id`，logical repository 身份字段缺省，resolved options 与 prepare 同源

#### Scenario: 单仓 issue 缺所属仓或仓未登记

- **WHEN** 单仓 issue 无所属物理仓，或所属物理仓记录不存在
- **THEN** 投影与 Enable 返回明确 4xx 错误并指明缺失的仓库身份，不写 enrollment

#### Scenario: authority 冲突不回退

- **WHEN** 单仓物理仓与另一登记记录解析到同一 git 根，或与 legacy 逻辑代码库 active 成员来源身份相同
- **THEN** 系统返回 `repository_routing_source_identity_mismatch` 或 `repository_routing_legacy_conflict`（409），不选择任一记录继续

#### Scenario: 单仓链不经 gateway

- **WHEN** 单仓 enrollment 自动启动 plan author、reviewer 或 coding 角色
- **THEN** provider 按单仓直连路径启动，不产生逻辑代码库 gateway 审计记录，不创建逻辑代码库 snapshot
