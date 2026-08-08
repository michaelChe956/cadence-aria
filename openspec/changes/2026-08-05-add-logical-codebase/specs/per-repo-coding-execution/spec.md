# per-repo-coding-execution Specification

## Purpose

Coding 在目标仓独立 worktree 执行（主 checkout 不动），shared worktree 键升级为 `(project, issue, repository)`，attempt 冻结 target 快照，mixed-target group 一律拒绝；P1 交付为推分支 + 人工评审（ReviewRequest），自动 PR 后置。

## ADDED Requirements

### Requirement: 单目标仓 worktree 执行（REQ-COD-01）
系统 SHALL 由 Cadence 在目标仓显式创建独立 worktree（`git worktree add`）并将 provider cwd 钉死到该 worktree；主 checkout 的 HEAD 与工作区不被修改；worktree 路径统一为 `<repo>/.worktrees/aria-issues/{issue}`（与现有实现一致）；「主 checkout 不被修改」限定为 HEAD 与工作区，不误述为零 Git 副作用（worktree 会变更 git 元数据）。

#### Scenario: 对 Work Item 启动 coding
- **WHEN** 对 Work Item 启动 coding
- **THEN** 系统 SHALL 先显式创建目标仓 worktree，再以该 worktree 作为 provider cwd 启动；主 checkout 的 HEAD/工作区不受影响

#### Scenario: worktree 隔离不依赖 provider 自觉
- **WHEN** provider 尝试越出 worktree 写入主 checkout 或其他路径
- **THEN** P1 系统 SHALL 通过 pre/post 越界检测与配置约束（best-effort）标记并阻断；不宣称 OS 级不可达；硬隔离需 provider/OS sandbox（后置）

### Requirement: attempt 冻结 target 快照（REQ-COD-02）
系统 SHALL 使 `CodingExecutionAttempt` 持久化不可变 `target_repository_id`/checkout/revision 与 policy digest；创建、恢复、重放一律使用冻结快照；旧 attempt 缺快照时 display-only 或人工恢复，不得从活 Work Item/`Issue.repo_id` 重新猜测。

#### Scenario: 创建与恢复 coding attempt
- **WHEN** 创建、恢复或重放 coding attempt
- **THEN** 系统 SHALL 使用 attempt 内冻结的 target 快照；快照缺失时 fail-closed 拒绝启动或进入人工恢复

### Requirement: shared worktree 按三元键（REQ-COD-03）
系统 SHALL 使 Issue shared worktree 的存储、锁、获取/释放/迁移/删除接口全部升级为 `(project, issue, repository)` 键；旧 `issue-shared-worktree.json` 提供迁移与恢复策略；同一仓库多个 Work Item 可共享同一仓库级 worktree 并串行，不同仓库可并行。

#### Scenario: 异仓并行
- **WHEN** 同一 Issue 的两个 Work Item 分属不同仓库
- **THEN** 两个仓库的 worktree 可并行创建与执行，互不阻塞

#### Scenario: 同仓串行
- **WHEN** 同一 Issue 同一仓库的多个 Work Item
- **THEN** 复用同一仓库级 shared worktree，由该仓库锁串行化

### Requirement: mixed-target group 一律拒绝（REQ-COD-04）
系统 SHALL 使多成员 Issue 下创建 mixed-target WorkItemGroup 被一律拒绝并返回稳定错误码（在创建、恢复、replay 三处一致）；同 target 的 group 允许；不做自动按仓拆分。

#### Scenario: 尝试 mixed-target group
- **WHEN** 用户尝试对涉及多个目标仓库的 Work Item 建立同一 group
- **THEN** 创建 SHALL 被拒绝并返回稳定错误码；拆分能力不在本 change 范围

### Requirement: 跨仓只读证据检索（REQ-COD-05）
系统 SHALL 使 Coder 可经受控检索接口获取其他成员仓的只读证据（改 A 的接口时查 B 的调用点），证据带 ACL/snapshot/token 预算/审计；Coder 不持有聚合根；跨仓调用点检索为启发式符号/字符串近似（CodeGraph 非 Java 语义级），非精确调用图；「无法写入其他仓」为 P1 best-effort 表述，不宣称 OS 级强制。

#### Scenario: coding 中检索跨仓证据
- **WHEN** coding 过程中 Coder 需要了解其他仓的调用关系
- **THEN** 通过受控检索接口获取只读证据注入上下文；证据记录来源 snapshot 与预算；Coder 的写入配置目标仅为目标 worktree

### Requirement: 每仓独立交付（推分支 + 人工评审）（REQ-COD-06）
系统 SHALL 使每仓独立 branch/commit/push 并持久化 `ReviewRequest`（GitBranchOnly，含分支/推送状态，无外部 PR URL）；Issue 完成 = 所有必需 Work Item 完成（以已推分支为完成级别），partial failure 显式呈现，不伪装全局成功；自动创建 GitHub/GitLab PR 不在本 change 范围。

#### Scenario: 多仓 Issue 各仓交付
- **WHEN** 多仓 Issue 各仓 coding 完成
- **THEN** 各仓 SHALL 产出独立分支/提交/推送与 ReviewRequest 状态；某仓失败时显式呈现 partial failure，不标记 Issue 为已完成

#### Scenario: 交付完成级别
- **WHEN** 判定 Work Item/Issue 完成
- **THEN** 以「目标仓分支已推送且 ReviewRequest 已生成」为完成级别；未推送/推送失败的 item 不得视为已交付
