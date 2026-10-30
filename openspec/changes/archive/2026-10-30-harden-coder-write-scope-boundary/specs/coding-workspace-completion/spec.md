# Spec Delta: harden-coder-write-scope-boundary

## ADDED Requirements

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
