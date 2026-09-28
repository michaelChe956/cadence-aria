# Spec Delta

## MODIFIED Requirements

### Requirement: Work Item 的提交证据必须覆盖该 UnitRun 的完整提交区间

当 Coder 拥有提交职责时，系统 MUST 以 UnitRun 的不可变 `start_commit` 和 Coder 完成后的只读 `completion_commit` 共同表示 Work Item 的 Git 证据。该 Work Item 的改动文件、diff 引用和人工审查材料 MUST 从 `start_commit..completion_commit` 区间派生，MUST NOT 只从末尾 `completion_commit` 的单次提交派生。

每一次新 execution（包括首次执行、重试 execution 与 restart 后的新 execution）MUST 在 provider 认领该 execution 之前记录当时工作树的真实 `HEAD` 作为自身 `start_commit`；新 execution MUST NOT 继承前一 execution 的 `start_commit`，也 MUST NOT 在完成时以基线分支或首次 execution 的起点回填。重连或恢复同一 execution MUST NOT 改写已记录的 `start_commit`。

当 `start_commit` 与 `completion_commit` 相同，系统 MUST 将其表示为无可观察 Git 增量的空区间，MUST NOT 把该提交相对其父提交的文件归属给当前 Work Item。该观测 MUST 与 Coder 的原始输出证据一起供人工查看，但 MUST NOT 触发服务端补提交、路径黑名单或新增提交范围门禁。

#### Scenario: Coder rework 产生多个提交

- **WHEN** Coder 首次完成 Work Item 创建提交 `C1`，独立 Reviewer 要求返修，Coder 随后创建提交 `C2`
- **THEN** 该 Work Item 的 completion evidence MUST 覆盖 `start_commit..C2`，并包含 `C1` 与 `C2` 的改动和提交引用，而不是只检查 `C2`

#### Scenario: 未观察到新的 Coder 提交

- **WHEN** Coder 完成后当前 `HEAD` 与 UnitRun 的 `start_commit` 相同
- **THEN** 系统 MUST 持久化空提交区间和 Coder 原始输出引用，MUST NOT 将起始提交相对父提交的改动归属给当前 Work Item，也 MUST NOT 自动创建提交

#### Scenario: 零提交重跑不倒算人工 WIP

- **WHEN** 前一 execution 之后用户在工作树中留下人工提交，随后一次重试 execution 未产生任何新提交即完成
- **THEN** 该重试 execution 的 `start_commit` MUST 等于其认领前的真实 `HEAD`，区间为空；人工提交 MUST NOT 被归属给当前 Work Item

#### Scenario: 重连同一 execution 不改写起点

- **WHEN** 某 execution 已记录 `start_commit` 后连接断开并重连、或 runner 恢复该 execution
- **THEN** 该 execution 的 `start_commit` MUST 保持不变
