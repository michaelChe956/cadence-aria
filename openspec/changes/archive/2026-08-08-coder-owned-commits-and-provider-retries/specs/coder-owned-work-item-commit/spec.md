## Purpose

让最了解当前 Work Item 需求和写入策略的 Coder 对其提交内容负责，并避免编排服务把共享 worktree 的无关改动自动纳入提交。

## ADDED Requirements

### Requirement: Coder 必须按当前 Work Item 写入策略创建提交

当 Coder 完成一个会修改目标仓库的 Work Item 时，Coder prompt MUST 要求其在同一 worktree 内检查完整 Git 状态，并仅按当前 Work Item 的规范性 `write_policy` 暂存允许的改动后创建提交。prompt MUST 要求 Coder 报告暂存文件清单、提交 SHA 与提交后的 Git 状态。

该规则 MUST NOT 通过静态目录黑名单表达；某个目录或文件是否可提交 MUST 仅由当前 Work Item 的 `write_policy` 决定。

#### Scenario: 写入策略允许的文件由 Coder 精确提交

- **WHEN** Coder 完成一个允许修改 `src/feature.rs` 的 Work Item，且 worktree 中同时存在不属于该策略的未跟踪文件
- **THEN** Coder MUST 只暂存该 Work Item 写入策略允许的改动并创建提交，且其报告 MUST 包含暂存文件清单、提交 SHA 与提交后的 Git 状态

#### Scenario: 写入策略显式允许目录内的生成文件

- **WHEN** 当前 Work Item 的 `write_policy` 显式允许某个生成目录或其中的文件
- **THEN** Coder MAY 按该策略将该允许范围内的改动纳入提交，系统 MUST NOT 因目录名称自动拒绝该提交

#### Scenario: Coder 不得以全量暂存代替范围判断

- **WHEN** Coder 为一个 Work Item 准备提交
- **THEN** prompt MUST 禁止使用不区分当前 Work Item 范围的全量暂存命令，并要求先核对完整 Git 状态与精确暂存清单

### Requirement: Coder 不得为清理共享 worktree 删除未被当前 Work Item 授权的内容

Coder prompt MUST 要求 Coder 不得使用广泛删除或清理命令处理不属于当前 Work Item 的未跟踪内容、生成物或其他残留。遇到无法按当前 `write_policy` 解释的改动时，Coder MUST 保留该改动并在结果中报告，而不是为了获得干净状态删除它。

#### Scenario: 遇到范围外未跟踪目录

- **WHEN** Coder 在提交前发现一个不属于当前 Work Item `write_policy` 的未跟踪目录
- **THEN** Coder MUST 不删除该目录，MUST 不将其暂存，并 MUST 在结果中报告该状态

### Requirement: Work Item 的提交证据必须覆盖该 UnitRun 的完整提交区间

当 Coder 拥有提交职责时，系统 MUST 以 UnitRun 的不可变 `start_commit` 和 Coder 完成后的只读 `completion_commit` 共同表示 Work Item 的 Git 证据。该 Work Item 的改动文件、diff 引用和人工审查材料 MUST 从 `start_commit..completion_commit` 区间派生，MUST NOT 只从末尾 `completion_commit` 的单次提交派生。

当 `start_commit` 与 `completion_commit` 相同，系统 MUST 将其表示为无可观察 Git 增量的空区间，MUST NOT 把该提交相对其父提交的文件归属给当前 Work Item。该观测 MUST 与 Coder 的原始输出证据一起供人工查看，但 MUST NOT 触发服务端补提交、路径黑名单或新增提交范围门禁。

#### Scenario: Coder rework 产生多个提交

- **WHEN** Coder 首次完成 Work Item 创建提交 `C1`，独立 Reviewer 要求返修，Coder 随后创建提交 `C2`
- **THEN** 该 Work Item 的 completion evidence MUST 覆盖 `start_commit..C2`，并包含 `C1` 与 `C2` 的改动和提交引用，而不是只检查 `C2`

#### Scenario: 未观察到新的 Coder 提交

- **WHEN** Coder 完成后当前 `HEAD` 与 UnitRun 的 `start_commit` 相同
- **THEN** 系统 MUST 持久化空提交区间和 Coder 原始输出引用，MUST NOT 将起始提交相对父提交的改动归属给当前 Work Item，也 MUST NOT 自动创建提交
