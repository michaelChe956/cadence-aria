# Spec Delta

> 基线：`openspec/specs/coder-owned-work-item-commit/spec.md` 现有三项 requirement 与 `coding-workspace-completion` 的「Group Work Item completion 只记录 Coder 已创建的提交」。REQ-CWC-04 起编号：该 capability 现有三项 requirement 无显式编号，视为 REQ-CWC-01 至 REQ-CWC-03，本 delta 顺延。本 delta 只新增，不修改既有文本；既有「编排服务不得自动暂存或提交」红线保持不变，平台确定性摘要只读取、不写仓库。

## ADDED Requirements

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
