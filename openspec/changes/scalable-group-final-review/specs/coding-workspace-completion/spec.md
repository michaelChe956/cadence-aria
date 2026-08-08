## MODIFIED Requirements

### Requirement: Coding Workspace completion gate 不依赖 TestingReport

在 Testing 阶段未纳入产品流程期间，Coding Workspace completion gate 的判定 SHALL 以适用的 review 流程通过及全部非 testing gate 满足为准。系统 MUST NOT 仅因 required verification check 缺少 Passed testing report 而阻塞 completion gate，也 MUST NOT 伪造 testing report 或 testing 成功状态。

对于新的 schema v2 group attempt，适用的最终 review 流程为全部 Work Item 的独立 Code Review、基于每个 UnitRun `start_commit..completion_commit` 完整证据区间的组就绪检查和明确的人工 Final Confirm；系统 MUST NOT 要求或启动 internal PR review、Group Reviewer、shard 或 reduction Provider 调用。Final Confirm 仍执行已有的非 testing terminal gate，包括 completion binding、提交区间的文件范围与 shared worktree 清洁性。legacy group terminal status 继续受既有 authoritative plan binding 完整性规则约束。

#### Scenario: schema v2 group 缺少 testing report

- **WHEN** schema v2 group attempt 的所有 unit 已完成、每项独立 Code Review 已通过、组就绪检查完整、用户已 Final Confirm，且绑定的 verification plan revision 含 required verification check 但 attempt 没有 testing report
- **THEN** 系统 SHALL 允许最终完成流程成功，不得返回 `VerificationGateResultMissing`，且 MUST NOT 启动 internal PR review 或 Group Reviewer Provider

#### Scenario: legacy group completion gate 缺少 Passed testing report

- **WHEN** legacy group attempt 的 unit、handoff 与其他 completion gate 前置条件满足，用户完成适用的人工最终确认，verification plan 含 required gate，一个 plan 仅有非 Passed testing report、另一个 plan 没有 matching testing report
- **THEN** `run_group_completion_gates` SHALL 成功，且系统 SHALL 保留原 testing report 数据不变
- **AND THEN** 本场景 MUST NOT 绕过 group terminal status 对 authoritative plan binding 的既有完整性要求

#### Scenario: single attempt 缺少 testing report

- **WHEN** single-attempt 已通过适用的 review 流程和其他非 testing 完成门禁，work item 引用了含 required gate 的 verification plan，但 attempt 没有 testing report
- **THEN** 系统 SHALL 允许最终完成流程继续，不得仅因缺少 testing report 阻塞完成
