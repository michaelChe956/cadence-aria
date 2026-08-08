## ADDED Requirements

### Requirement: Group Work Item completion 只记录 Coder 已创建的提交

Group Work Item 在通过其单项 Code Review 后，编排服务 MUST NOT 对共享 worktree 执行自动暂存或自动创建 Git commit。服务 MUST 仅读取该 worktree 当前 `HEAD` 并将其记录为该 Work Item 的 terminal completion commit；该 Work Item 的完整 Git 证据为其 UnitRun `start_commit..completion_commit` 区间，不能以该末尾 commit 的单次父提交 diff 代替。

该行为 MUST NOT 依据文件或目录名称额外拒绝 Coder 的提交；提交范围责任由 Coder 使用当前 Work Item 的 `write_policy` 决定。该行为也 MUST NOT 新增服务端文件范围门禁或由服务端替 Coder 补做提交。

#### Scenario: Coder 已提交当前 Work Item

- **WHEN** Coder 已按当前 Work Item 写入策略创建提交，且该 Work Item 的 Code Review 已通过
- **THEN** 编排服务 MUST 记录 worktree 当前 `HEAD` 为 terminal completion commit，并以 UnitRun 起始提交至该 SHA 的完整区间作为 completion evidence；服务 MUST NOT 再执行自动暂存或创建额外提交

#### Scenario: worktree 存在范围外未跟踪内容

- **WHEN** Work Item 通过 Code Review 时共享 worktree 中存在范围外的未跟踪内容
- **THEN** 编排服务 MUST NOT 为了该 Work Item 自动暂存、提交或删除该内容

#### Scenario: rework 后的最终 HEAD 代表完整 Work Item 证据

- **WHEN** 同一 UnitRun 中 Coder 已创建首次提交，且在 Code Review rework 后又创建一个新的最终 HEAD
- **THEN** 编排服务 MUST 将最终 HEAD 记录为 completion commit，但后续范围、diff 和人工证据 MUST 覆盖该 UnitRun 的起始提交至最终 HEAD 的完整区间
