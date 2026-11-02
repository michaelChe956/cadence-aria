# coder-owned-work-item-commit Specification

## Purpose

让最了解当前 Work Item 需求和写入策略的 Coder 对其提交内容负责，并避免编排服务把共享 worktree 的无关改动自动纳入提交。

## Requirements

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

### Requirement: 平台提交责任声明优先于仓库规则（REQ-CWC-04）

Coder 的最终 prompt（首次、增量、返修与 provider 重试路径）SHALL 包含平台提交责任声明：平台编排下当前 Work Item 的提交责任归 Coder，该责任优先于目标仓库规则文件（如 `AGENTS.md`、`CLAUDE.md`）中的「不自动提交」类规则；同时声明禁止执行 `git push`（任何形式，含强制推送变体），交付分支的推送由平台执行，未推送或推送失败不算任务失败，不得为此重试或报告失败。该声明 SHALL 在 prompt 最终组装后追加，MUST NOT 改变 Work Item 投影的渲染内容、投影版本或执行上下文哈希，在途 run 的校验 MUST NOT 因此失败。该声明 SHALL 对所有 coding attempt 注入，不限于逻辑代码库 attempt。

#### Scenario: 仓库规则要求不自动提交

- **WHEN** 目标仓库的 `AGENTS.md` 声明不得自动提交，而当前 Work Item 由平台编排并要求 Coder 提交
- **THEN** Coder 的最终 prompt 明确声明提交责任归 Coder 且优先于该仓库规则，Coder 按 `write_policy` 创建提交

#### Scenario: 各 prompt 路径一致

- **WHEN** Coder 以首次、增量、返修或 provider 重试路径启动，且无论走投影渲染路径还是回落路径
- **THEN** 各路径的最终 prompt 均含该声明，且声明追加不改变既有执行上下文哈希

#### Scenario: 声明不改变哈希

- **WHEN** 一个在本 change 前已创建并校验过执行上下文哈希的 unit run 在本 change 后继续运行
- **THEN** 其哈希校验结果不变，run 不因声明追加而失效

### Requirement: Coder 交付汇报契约（REQ-CWC-05）

Coder 的最终 prompt SHALL 包含交付汇报契约：交付摘要只包含四类事实——实际改动的文件、提交 SHA、验证命令及其完整输出、`git status`。环境异常（路径或挂载命名空间差异等）只允许写入摘要末尾固定小节「环境备注」，且 SHALL 一行带过，MUST NOT 占据摘要主体，MUST NOT 借此省略任何交付事实。prompt SHALL 同时声明文件工具与 shell 的路径命名空间可能不同，一律以 prompt 给定的 worktree 路径为准。该契约与 REQ-CWC-04 的声明 SHALL 在同一段平台指令中承载，随同一次追加注入。

#### Scenario: 环境异常仅进入环境备注

- **WHEN** Coder 在执行中遇到文件工具与 shell 路径命名空间不一致
- **THEN** 契约要求其把该异常写入摘要末尾「环境备注」一行，而交付摘要主体仍完整包含改动文件、提交 SHA、验证输出与 `git status`

#### Scenario: 环境异常不得省略交付事实

- **WHEN** Coder 以环境异常为由想省略验证输出或提交 SHA
- **THEN** 契约明确禁止；缺失的交付事实 SHALL 如实写明未能获得及原因，不得以环境叙事代替

### Requirement: 平台确定性交付摘要（REQ-CWC-06）

Group Work Item 完成并记录 terminal completion commit 后，系统 SHALL 依据 durable 事实生成「交付事实（平台生成）」并持久化到该 attempt 的对话时间线，与 Coder 自述内容分离：至少含完成提交 SHA、该 UnitRun `start_commit..completion_commit` 区间的变更文件清单、该运行已记录的验证证据、handoff 标识。任一事实缺失时 SHALL 显示「未记录」，MUST NOT 推断、补写或以 Coder 自述替代；起点缺失仍按既有规则停等，不得为生成摘要而回填。摘要生成 MUST NOT 暂存、提交或修改目标仓库，生成失败 MUST NOT 阻断 Work Item 完成，但 SHALL 留下可见告警。同一 UnitRun 重入不得产生重复摘要。

#### Scenario: 完成后可见平台摘要

- **WHEN** 某 Work Item 通过 Code Review 并记录 completion commit
- **THEN** 该 attempt 的对话时间线出现一条平台生成的交付事实，含完成提交、变更文件、验证证据与 handoff 标识，且标明由平台生成

#### Scenario: 摘要不受 Coder 叙事污染

- **WHEN** Coder 的自述摘要与 durable 事实不一致，或被环境叙事占满
- **THEN** 平台摘要仍只反映 durable 事实，二者并列可查，平台摘要不引用 Coder 自述内容

#### Scenario: 缺失事实显示未记录

- **WHEN** 该运行没有记录到验证证据，或 handoff 尚未发布
- **THEN** 对应项显示「未记录」，系统不推断也不补写

#### Scenario: 重入幂等且不写仓库

- **WHEN** 同一 UnitRun 的完成流程因恢复而重入
- **THEN** 时间线中该运行只有一条平台摘要，目标仓库的暂存区、提交与工作区内容保持不变
