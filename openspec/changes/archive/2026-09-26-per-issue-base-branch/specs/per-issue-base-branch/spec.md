# per-issue-base-branch Delta

## Purpose

issue 级基准分支的设置、锁定与全链锚定：author 生成上下文、coding worktree fork、AC 基线核对三面同源于 issue.base_branch，杜绝跨分支上下文泄漏（F-57）与基线错位（F-56）。host-served 通道获得通道层硬边界；provider 原生通道以 prompt 软限制运行（用户 2026-09-25 裁决接受残余风险），其 AC 路径引用形态由 REQ-PIB-03 的核对 fail-closed 兜底。

## ADDED Requirements

### Requirement: 基准分支设置与锁定（REQ-PIB-01）

创建 Issue 时系统 SHALL 获取并展示默认仓库的**全部本地分支**（refs/heads，不隐式 fetch；需要远端基线时用户先建立本地跟踪分支——显式意图）供用户选择（UI 分支选择器 + REST 字段）。默认值 SHALL 按以下链确定：仓库存在 `main` 时默认 `main`；否则存在 `master` 时默认 `master`；两者皆不存在时 SHALL 无默认值并要求用户显式选择（未选择即 fail-closed 拒绝创建）。系统 SHALL 在创建时校验所选分支存在（不存在即 fail-closed 拒绝并提示）。`base_branch` 在 Issue 创建后 SHALL 锁定不可修改；需要不同基线时应创建新 Issue。存量 Issue 缺少该字段时 SHALL 按同一默认链解析（main 仓=main、master-only 仓=master，行为兼容；皆无仓可读取但生成与 coding SHALL 明确失败并诊断，不猜替代分支）。

#### Scenario: 创建时选择非默认分支

- **WHEN** 用户创建 Issue 时从本地分支列表选择基准分支 `feature/x`
- **THEN** Issue 记录 base_branch=feature/x；后续 story/design/plan 生成与 coding worktree 均锚定该分支

#### Scenario: 默认值按链确定

- **WHEN** 用户未显式选择而仓库存在 main（或仅存在 master）
- **THEN** 默认值分别为 main（或 master），选择器预选该值

#### Scenario: 皆无 main/master 强制手选

- **WHEN** 仓库分支列表不含 main 也不含 master（如仅 release/*）
- **THEN** 无默认值，用户必须显式选择；未选择提交创建即被拒绝

#### Scenario: 分支不存在拒绝创建

- **WHEN** 创建时输入的分支在默认仓库不存在
- **THEN** 创建被拒绝并提示分支名；不产生半配置 Issue

#### Scenario: 创建后锁定

- **WHEN** 已有 Issue 的 base_branch 被尝试修改
- **THEN** 修改被拒绝（明确提示改基线需新建 Issue）；已生成产物与基线的对应关系不被破坏

#### Scenario: 存量兼容

- **WHEN** 读取创建于本功能之前的 Issue（无 base_branch 字段）
- **THEN** 按默认链解析（main/master 仓与既有行为一致）；皆无仓的存量 Issue 可读取，但其 story/design/plan 生成与 coding 启动明确失败并给出「基准分支不可解析」诊断，不回退不猜

### Requirement: author 上下文基线限定（REQ-PIB-02）

story/design/plan 生成期间的仓库文件浏览与读取 SHALL 以 issue.base_branch 的树内容为基线语义，按通道分级落地：**host-served 文件通道**（如 provider 桥接的 fs 服务）SHALL 路由为 `git show <base>:<path>` / `git ls-tree <base>` 形态（不 checkout、不触工作区）并对其 terminal 通道维持基线拒绝——该通道层为硬安全边界，工作区与兄弟 worktree 路径不可达；**provider 原生文件通道**（host 无法在通道层转码/收束者，如 claude/codex/pi）SHALL 在 author prompt 注入基线限制教学块（仅以 base_branch 树内容为「既有事实」来源、不得访问 `.worktrees/` 及仓库工作区外路径的指引）——该限制为软约束，不构成安全边界，其 AC 路径引用形态的后果由 REQ-PIB-03 的核对 fail-closed 拦截，其余形态为用户明示接受的残余风险。工具策略层仅维持既有写拒绝与 spawn 守卫纵深，SHALL NOT 承载基线语义；prompt 教学块不因注入而获得边界地位。host-served 通道的 author 文件访问 SHALL NOT 到达仓库工作区、其他分支检出或兄弟 issue worktree 路径。基线分支在生成时不可解析（被删/改名）SHALL fail-closed 终止生成并给出明确诊断（两种通道同此语义）。

#### Scenario: host-served 通道看不到兄弟 worktree

- **WHEN** plan author 经 host-served 通道（如 kimi fs 桥）在生成中浏览仓库寻找「既有交付物」
- **THEN** 其可见集为 base_branch 树内容，`.worktrees/aria-issues/*` 等工作区路径不可达——F-57 形态在该通道层不再可能；provider 原生通道见软限制场景

#### Scenario: 引用与所见一致

- **WHEN** author 在 spec 中引用某仓库文件作为「既有」事实
- **THEN** 该文件必存在于 base_branch 树（该保证由 plan 期核对（REQ-PIB-03）承载，不依赖 author 侧访问受限）

#### Scenario: 基线消失 fail-closed

- **WHEN** 生成时 base_branch 已被删除或不可解析
- **THEN** 生成终止并报「基准分支不存在」诊断，不回退 main、不猜替代分支

#### Scenario: provider 原生通道的软限制注入

- **WHEN** provider 原生文件通道（host 无法在通道层收束者，如 claude/codex/pi）的 author 会话携带基线锚点启动
- **THEN** 其 prompt 注入基线限制教学块（仅以 base_branch 树内容为「既有事实」来源、不得访问 `.worktrees/` 及仓库工作区外路径的指引）；该教学块为软约束，不构成访问边界，author 的物理可达集不受限——残余风险由用户显式裁决接受

#### Scenario: 软限制漏网的 AC 路径硬兜底

- **WHEN** provider 原生通道的 author 绕过 prompt 软约束，将基线外路径写入 plan 的 AC/验证计划
- **THEN** C1 的 AC×基线机械核对以 fail-closed 拦截该候选（acceptance_path_not_in_baseline，MustFix），不放行

### Requirement: coding fork 与核对同源（REQ-PIB-03）

coding attempt 的 worktree 分支 SHALL 从 issue.base_branch fork（`create_branch` 的 base 取 base_branch——现行各上游入口以当前检出/HEAD 隐式确定基线的形态 SHALL 统一改为读取 issue.base_branch）；C1 的 AC×基线核对树 SHALL 取 issue.base_branch 的树，且 SHALL NOT 依赖共享 coding worktree 解析（直接按分支名取树）；基线分支不可解析时核对 SHALL fail-closed 失败（不静默跳过核对——本条为 provider 原生通道软限制残余风险的唯一硬兜底，其有效性以该 fail-closed 语义为前提）。author 所见、核对所用、coder 所 fork 三者 SHALL 为同一基线。

#### Scenario: fork 自所选基线

- **WHEN** base_branch=feature/x 的 Issue 启动 coding
- **THEN** worktree 分叉自 feature/x；coder 的基线包含该分支全部内容

#### Scenario: 三面同源

- **WHEN** 任一文件路径分别被 author 引用、被 AC 核对检查、被 coder 作为基线
- **THEN** 三者的存在性判定全部基于同一 base_branch 树，不再出现「author 看得到/核对判不在」或「AC 要求基线外文件」的错位；author 物理可达集在 provider 原生通道不受通道层限制，存在性判定仍以 base_branch 树为唯一口径

#### Scenario: 核对基线不可解析不跳过

- **WHEN** AC 核对执行时 issue.base_branch 的树不可解析（分支被删）
- **THEN** 核对以 fail-closed 失败并诊断，不静默跳过核对放行候选
