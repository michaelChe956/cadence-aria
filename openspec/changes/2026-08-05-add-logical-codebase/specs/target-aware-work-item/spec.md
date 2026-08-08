# target-aware-work-item Specification

## Purpose

Work Item 携带唯一 `target_repository_id`，贯穿拆分/编译/校验/运行链路；缺失或非法 target 一律 blocker；禁止回落到 primary。

## ADDED Requirements

### Requirement: target 必填与校验集（REQ-TGT-01）
系统 SHALL 使每个 Work Item 持久化唯一的 `target_repository_id`，其语义类型为 `LogicalRepositoryId`（稳定 UUID 逻辑身份，不可歧义指向 checkout 或物理投影）；target 必须属于该 Issue 冻结的 `IssueCodebaseSelection` 有效成员集合（include 且未 exclude、未删除/停用），而不只是 Project 全体成员。执行时由 member 解析为 `RepositoryCheckoutId`，再定位到 `RepositoryRecord.id` + canonical_path + git_dir_identity。

#### Scenario: 拆分/编译产生 Work Item
- **WHEN** 拆分或编译产生 Work Item
- **THEN** 每个 item 均有合法 target，且 target 属于该 Issue selection 有效集合；指向被 Issue 排除或已删除成员的 target 被拒绝

### Requirement: 缺失 target 即 blocker（REQ-TGT-02）
系统 SHALL 使拆分引擎输出缺少 `target_repository_id` 或 target 非法时产生 blocker 进入人工确认；不发布任何可执行 item；原始输出、候选、blocker、人工补 target、再编译/重试全程可审计。

#### Scenario: provider 未给出 target
- **WHEN** provider 输出未明确 target 的工作项
- **THEN** 该项被标记为 blocker，不生成可执行 Work Item；人工补充合法 target 后重新校验与编译

### Requirement: target 贯穿编译链路（REQ-TGT-03）
系统 SHALL 使 `target_repository_id`（语义类型 `LogicalRepositoryId`）贯穿 Outline schema → Draft → compile transaction → `LifecycleWorkItemRecord` → runtime binding；删除「从第一个 Story 推导单一仓库并填充所有 item」的默认路径（当前 `compile_support.rs:25-39,118-123` 的实现须改造）；多个 Work Item 可共享同一合法 target（同一仓库多个任务），但不得统一回落 primary。

#### Scenario: 跨仓 Story 拆分为多工作项
- **WHEN** 跨仓 Story/Design 被拆分为多个工作项
- **THEN** 各 item 的 target SHALL 分别保留其声明的合法成员仓（可同仓可异仓），不再全部落到同一仓库；编译 transaction 先验证全部 target 再一次性发布，禁止部分 item 已落库

### Requirement: 跨仓顺序表达（REQ-TGT-04）
系统 SHALL 通过现有 work item 级 `depends_on` 表达跨仓执行顺序（执行顺序图，非服务调用图）；校验无环、未知 ID、同一 plan、target 成员有效性；完成级别明确定义（本地 commit 或 push 分支）。

#### Scenario: Design 指定改动顺序
- **WHEN** Design 指定「先改公共契约 → 再改 provider → 最后改 consumer」
- **THEN** 对应 Work Item 通过 `depends_on` 建立先后关系，下游 item 在上游完成前不可执行；存在环或非法依赖时校验失败

### Requirement: 按仓分组展示（REQ-TGT-05）
系统 SHALL 使 Issue 下 Work Item 按 `target_repository_id` 分组展示。

#### Scenario: 查看多仓 Issue 工作项
- **WHEN** 查看多仓 Issue 的工作项列表
- **THEN** 工作项按仓库分组，每组标注仓库名与状态
