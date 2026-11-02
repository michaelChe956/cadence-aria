# coding-workspace-completion Specification

## Purpose
在 Testing 阶段未纳入产品流程期间，解除 Coding Workspace completion gate 对 testing report 的依赖：完成判定以适用的 review 流程通过及全部非 testing gate 满足为准，不伪造 testing report 或成功状态；文件范围、runtime binding、handoff、terminal 完整性等非 testing 门禁继续生效，Testing 基础设施与既有 report 数据保持兼容。

## Requirements

### Requirement: Coding Workspace completion gate 不依赖 TestingReport

在 Testing 阶段未纳入产品流程期间，Coding Workspace completion gate 的判定 SHALL 以适用的 review 流程通过及全部非 testing gate 满足为准。

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

#### Scenario: 存量 testing report 状态不参与 gate 判定

- **WHEN** attempt 存在历史 testing report，其状态不是 Passed 或 PassedWithWarnings，但适用的 review 流程和全部非 testing gate 均满足
- **THEN** 系统 SHALL 忽略该 report 对 completion gate 判定的影响，且 MUST 保留原 report 数据不变

#### Scenario: 不阻塞亦不伪造

- **WHEN** required verification check 缺少 Passed testing report
- **THEN** 系统 MUST NOT 仅因 required verification check 缺少 Passed testing report 而阻塞 completion gate，也 MUST NOT 伪造 testing report 或 testing 成功状态。

#### Scenario: 界定 v2 终审流程

- **WHEN** schema v2 group attempt 进入最终 review 流程
- **THEN** 对于新的 schema v2 group attempt，适用的最终 review 流程为全部 Work Item 的独立 Code Review、基于每个 UnitRun `start_commit..completion_commit` 完整证据区间的组就绪检查和明确的人工 Final Confirm。

#### Scenario: 不启动内部评审

- **WHEN** schema v2 group attempt 的评审执行
- **THEN** 系统 MUST NOT 要求或启动 internal PR review、Group Reviewer、shard 或 reduction Provider 调用。

#### Scenario: 终确认仍过终门

- **WHEN** schema v2 group attempt 执行 Final Confirm
- **THEN** Final Confirm 仍执行已有的非 testing terminal gate，包括 completion binding、提交区间的文件范围与 shared worktree 清洁性。

#### Scenario: legacy 仍受绑定约束

- **WHEN** legacy group attempt 进入 terminal status
- **THEN** legacy group terminal status 继续受既有 authoritative plan binding 完整性规则约束。

### Requirement: 非 testing completion gate 与 terminal 完整性继续生效

解除 testing report 依赖后，Coding Workspace MUST 继续执行并强制满足当前路径适用的文件范围、runtime binding、handoff、unit 完成状态、completion commit、共享 worktree 清洁性及其他非 testing 一致性 gate。Group terminal status MUST 继续要求既有 `CodingAttemptPlanBinding`、`WorkItemPlanLineage` 与 authoritative plan revision 完整性；本 change MUST NOT 为无 binding legacy group 增加 terminal fallback。

#### Scenario: 文件范围校验失败

- **WHEN** internal PR review 已通过且不存在 testing report，但 changed files 违反 work item 或 runtime 声明的写入范围
- **THEN** 系统 MUST 拒绝完成并返回既有文件范围校验错误

#### Scenario: handoff 或 unit 完成条件缺失

- **WHEN** internal PR review 已通过且不存在 testing report，但可见 handoff 缺失、unit 未完成或 completion commit 缺失
- **THEN** 系统 MUST 拒绝完成并返回对应的既有非 testing 门禁错误

#### Scenario: 共享 worktree 不清洁

- **WHEN** internal PR review 已通过且不存在 testing report，但共享 worktree 未满足既有清洁性要求
- **THEN** 系统 MUST 拒绝完成并保持既有恢复/人工处置语义

#### Scenario: legacy group 缺少 authoritative binding

- **WHEN** legacy group completion gate 已通过，但 attempt 缺少 terminal status 所需的 `CodingAttemptPlanBinding`、`WorkItemPlanLineage` 或 authoritative plan revision
- **THEN** 系统 MUST 保持既有失败关闭行为，不得因 testing gate 放宽而允许该 group attempt 进入 terminal status

### Requirement: Testing 基础设施保持兼容

本行为变化 MUST NOT 删除或修改 Testing stage、tester 配置、TestingReport 持久化格式或现有 testing report 数据。系统 SHALL 保留 `VerificationGateResultMissing` 公共错误变体，以避免本 change 造成不必要的公共类型破坏。

#### Scenario: 读取既有 testing report

- **WHEN** 部署本 change 后读取历史 attempt 的 testing report
- **THEN** 系统 SHALL 按原数据模型返回 report，不得因完成门禁放宽而丢失或迁移该数据

#### Scenario: 不新增 testing gate 配置

- **WHEN** 创建或恢复 Coding Workspace attempt
- **THEN** 系统 MUST NOT 要求新增 testing gate 开关或配置字段，现有 API 和持久化结构 SHALL 保持兼容

### Requirement: Group Work Item completion 只记录 Coder 已创建的提交

Group Work Item 在通过其单项 Code Review 后，编排服务 MUST NOT 对共享 worktree 执行自动暂存或自动创建 Git commit。

#### Scenario: Coder 已提交当前 Work Item

- **WHEN** Coder 已按当前 Work Item 写入策略创建提交，且该 Work Item 的 Code Review 已通过
- **THEN** 编排服务 MUST 记录 worktree 当前 `HEAD` 为 terminal completion commit，并以 UnitRun 起始提交至该 SHA 的完整区间作为 completion evidence；服务 MUST NOT 再执行自动暂存或创建额外提交

#### Scenario: worktree 存在范围外未跟踪内容

- **WHEN** Work Item 通过 Code Review 时共享 worktree 中存在范围外的未跟踪内容
- **THEN** 编排服务 MUST NOT 为了该 Work Item 自动暂存、提交或删除该内容

#### Scenario: rework 后的最终 HEAD 代表完整 Work Item 证据

- **WHEN** 同一 UnitRun 中 Coder 已创建首次提交，且在 Code Review rework 后又创建一个新的最终 HEAD
- **THEN** 编排服务 MUST 将最终 HEAD 记录为 completion commit，但后续范围、diff 和人工证据 MUST 覆盖该 UnitRun 的起始提交至最终 HEAD 的完整区间

#### Scenario: execution 起点缺失不回填

- **WHEN** 某 execution 完成时没有已记录的 `start_commit`
- **THEN** 编排服务 MUST NOT 以基线分支 `HEAD` 回填，MUST 持久化起点缺失诊断并停等用户处理

#### Scenario: 读取并记录 HEAD

- **WHEN** Work Item 通过其单项 Code Review 后
- **THEN** 服务 MUST 仅读取该 worktree 当前 `HEAD` 并将其记录为该 Work Item 的 terminal completion commit；该 Work Item 的完整 Git 证据为其各 UnitRun execution 的 `start_commit..completion_commit` 区间，不能以该末尾 commit 的单次父提交 diff 代替。

#### Scenario: 认领前记录起点

- **WHEN** 每个 execution 启动（provider 认领前）
- **THEN** 每个 execution 的 `start_commit` MUST 取自该 execution 在 provider 认领前记录的真实 `HEAD`。

#### Scenario: 缺失停等并诊断

- **WHEN** completion 阶段发现起点缺失
- **THEN** completion 阶段 MUST NOT 以基线分支 `HEAD` 或首次 execution 起点回填缺失的起点，缺失时 MUST 持久化可诊断的不一致并停等，而不是猜测区间。

#### Scenario: 不按名称拒提交

- **WHEN** 编排服务处理 Coder 创建的提交
- **THEN** 该行为 MUST NOT 依据文件或目录名称额外拒绝 Coder 的提交；提交范围责任由 Coder 使用当前 Work Item 的 `write_policy` 决定。

#### Scenario: 不新增服务端门禁

- **WHEN** 编排服务处理该行为
- **THEN** 该行为也 MUST NOT 新增服务端文件范围门禁或由服务端替 Coder 补做提交。

### Requirement: 工作树本地产物写入仓库本地 exclude 且不改 .gitignore（REQ-CWS-01）

系统 SHALL 在工作树准备阶段，把平台本地产物目录写入**仓库本地** ignore 文件（`<common gitdir>/info/exclude`），使这些目录不出现在 `git status`；MUST NOT 修改 `.gitignore`（被跟踪文件，改它本身即属越界），MUST NOT 删除或改写仓库既有的 exclude 内容与注释。

写入 MUST 满足：

- **幂等**：重复执行 SHALL NOT 产生重复行（`.aria` 与 `.aria/` 视为等价）
- **并发安全**：同一仓库的并发写入 SHALL NOT 丢更新、SHALL NOT 产生截断文件（common gitdir 为多工作树共享，落点 MUST 加文件锁）
- **落点为 common gitdir**：一次写入对主检出与全部 linked worktree 生效；MUST NOT 依赖 `<repo>/.git` 为目录（linked worktree 的 `.git` 是文件）

#### Scenario: 本地产物不再弄脏 git status

- **WHEN** 工作树准备完成后，平台在本地产物目录（如 `.verify/`）写入文件
- **THEN** `git status --porcelain` SHALL 不显示这些目录；`.gitignore` 内容 SHALL 保持逐字节不变

#### Scenario: exclude 写入幂等且并发安全

- **WHEN** 同一仓库的多个工作树被并发创建
- **THEN** exclude 文件中每个平台目录 SHALL 恰好出现一行；仓库既有行 SHALL 原样保留；文件 SHALL NOT 被截断

#### Scenario: linked worktree 也能落到 common gitdir

- **WHEN** 目标仓路径本身是 linked worktree（其 `.git` 是文件而非目录）
- **THEN** exclude MUST 写入该仓的 common gitdir，MUST NOT 静默失败

### Requirement: 本地产物清单为单一事实源并与提交过滤共用（REQ-CWS-02）

平台本地产物目录清单 MUST 只有一处定义（单一事实源），并由「exclude 写入」与「Work Item 提交过滤」**共同使用**；MUST NOT 存在两份独立清单。

该清单 MUST 覆盖平台自产的**全部**产物路径（含证据令牌文件本身），使 exclude 失效时提交过滤仍能拦住它们。

该清单 MUST NOT 包含仓库内容目录（如 `cadence/`、`openspec/`、`.claude/`、`.agents/`、`.codex/`、`.pi/`、`.kimi-code/`、`.omp/`）—— 这些是仓库跟踪内容，排除它们会使新建文档从 `git status` 与 `git add` 消失。

#### Scenario: 令牌文件不会误入 Work Item 提交

- **WHEN** 工作树内存在平台证据令牌文件等本地产物
- **THEN** 提交过滤 SHALL 排除它，MUST NOT 把它暂存进 Work Item 提交

#### Scenario: 清单不含仓库内容目录

- **WHEN** 平台构建本地产物清单
- **THEN** 该清单 SHALL NOT 包含 `cadence/`、`openspec/`、`.claude/` 等仓库跟踪目录
