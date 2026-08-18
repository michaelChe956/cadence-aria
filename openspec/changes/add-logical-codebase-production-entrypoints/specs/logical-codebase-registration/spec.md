## ADDED Requirements

### Requirement: 登记生产 HTTP 入口

登记批次能力通过生产 HTTP 端点可用：预检、提交、查询、恢复、取消，全部为同步短操作（请求内完成并返回最终或部分结果）。

#### Scenario: 预检冻结快照
- **WHEN** 调用登记预检端点提交聚合根与候选路径
- **THEN** 服务端先执行聚合根准入校验（非 git 根、成员越界、symlink 逃逸、嵌套 worktree、根所有权冲突，返回对应 aggregate_root_* 稳定错误码），再执行候选分类；预检结果（含每候选的路径、git 根、来源身份、预检修订号）作为冻结快照持久化，返回 preflight_id；快照过期（24h）或不存在时提交返回 404 registration_preflight_not_found

#### Scenario: 同步提交
- **WHEN** 调用提交端点携带 preflight_id 与确认路径列表
- **THEN** 服务端以冻结快照重建确认输入并同步执行 TOCTOU 复验与逐项成员登记，返回批次最终状态（completed 或 partial_failed）与单项结果；登记期间成员主 checkout 无 git 写副作用

#### Scenario: 漂移语义
- **WHEN** 提交或恢复时某成员仅预检修订号变化
- **THEN** 该项标记 needs_attention 并跳过，批次继续
- **WHEN** 提交或恢复时某成员路径、git 根或来源身份变化
- **THEN** 整批中止并返回 409 registration_batch_conflict

#### Scenario: manifest 首批原子创建与 root 一致性
- **WHEN** 提交时 project 尚无 manifest
- **THEN** 以提交的聚合根为 provider_context_root 原子创建 manifest 并登记首批成员
- **WHEN** 提交的聚合根与既有 manifest 的 provider_context_root 不一致
- **THEN** 返回 409 aggregate_root_mismatch，不产生任何成员变更

#### Scenario: 恢复与取消
- **WHEN** 对 partial_failed 批次调用 resume
- **THEN** 同步重跑未完成项，漂移语义同上
- **WHEN** 对 Queued/PartialFailed 批次调用 cancel
- **THEN** 批次标记终态 cancelled；对已终态批次调用返回 409 registration_batch_not_cancelable

### Requirement: 登记端点的前端入口

#### Scenario: 登记向导
- **WHEN** 用户在多仓 project 的逻辑代码库页发起成员登记
- **THEN** 向导引导：填写聚合根 → 展示候选分类（eligible/non_git/duplicate/nested/needs_attention/missing/outside_root）→ 勾选确认（勾选 needs_attention 项视为显式确认）→ 同步提交（带 loading）→ 展示批次结果与单项状态，partial_failed 可恢复
