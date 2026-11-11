# Spec Delta

## MODIFIED Requirements

### Requirement: 基准分支设置与锁定（REQ-PIB-01）

创建 Issue 时系统 SHALL 获取并展示默认仓库的**全部本地分支**（refs/heads，不隐式 fetch；需要远端基线时用户先建立本地跟踪分支——显式意图）供用户选择（UI 分支选择器 + REST 字段）。默认值 SHALL 按以下链确定：仓库存在 `main` 时默认 `main`；否则存在 `master` 时默认 `master`；两者皆不存在时 SHALL 无默认值并要求用户显式选择（未选择即 fail-closed 拒绝创建）。系统 SHALL 在创建时校验所选分支存在（不存在即 fail-closed 拒绝并提示）。`base_branch` 在 Issue 创建后 SHALL 锁定不可修改；需要不同基线时应创建新 Issue。存量 Issue 缺少该字段时 SHALL 按同一默认链解析（main 仓=main、master-only 仓=master，行为兼容；皆无仓可读取但生成与 coding SHALL 明确失败并诊断，不猜替代分支）。

上述基线生效语义在单仓继续成立。多仓会话的 **author 基线上下文面与 AC 基线核对面** SHALL 按计划已授权有效成员逐仓使用 checkout 当前 HEAD 所在分支，MUST NOT 用 `issue.base_branch` 或 main/master 默认链替代；detached HEAD、空仓或 Git 失败 SHALL 显式失败。系统 MUST 如实保留分面边界：多仓下 `issue.base_branch` 对上述两面不再生效，而 coder fork 与 advance 面仍按该字段的单仓语义运行；后两面不在本 change 的实现范围，多仓暂时不同源 SHALL 登记为后续独立 change 收敛的已知遗留缺口，MUST NOT 声称三面已同源。

#### Scenario: 创建时选择非默认分支

- **WHEN** 用户创建单仓 Issue 时从本地分支列表选择基准分支 `feature/x`
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

- **WHEN** 读取创建于本功能之前的单仓 Issue（无 base_branch 字段）
- **THEN** 按默认链解析（main/master 仓与既有行为一致）；皆无仓的存量 Issue 可读取，但其 story/design/plan 生成与 coding 启动明确失败并给出「基准分支不可解析」诊断，不回退不猜

#### Scenario: 多仓两面逐成员使用自身 HEAD 分支

- **WHEN** 已授权的 busi、gateway、frontend 分别检出不同本地分支，且 Issue 记录了与其中成员 HEAD 分支不同的 base_branch
- **THEN** author 基线上下文与 AC 核对 SHALL 分别使用三个成员各自 HEAD 所在分支的树，MUST NOT 统一套用 issue.base_branch 或猜 main/master；字段记录与锁定状态不被改写

#### Scenario: 多仓成员分支解析失败不猜替代

- **WHEN** 任一必需授权成员处于 detached HEAD、空仓或 Git 分支解析失败
- **THEN** 基线加载 SHALL 显式返回错误，MUST NOT 回退 main/master、使用其它成员分支或返回不完整成功树

#### Scenario: 多仓 fork 与 advance 的分面差异明确登记

- **WHEN** 对多仓 Issue 比较 author 上下文、AC 核对、coder fork 与 advance 的分支权威
- **THEN** 契约 SHALL 明示前两面不再使用 issue.base_branch、后两面仍用其单仓语义，后两面实现不由本 change 改动；多仓暂时不同源 MUST 作为已知遗留缺口保留，MUST NOT 呈现为全系统已改用逐仓 HEAD 或三面已同源

### Requirement: coding fork 与核对同源（REQ-PIB-03）

单仓 coding attempt 的 worktree 分支 SHALL 从 issue.base_branch fork；C1 的 AC×基线核对树 SHALL 直接按同一分支名取树，SHALL NOT 依赖共享 coding worktree；基线不可解析时 SHALL fail-closed 失败，不静默跳过，author 所见事实、核对所用与 coder 所 fork SHALL 同源。多仓下，本 change SHALL 只收敛 author 基线上下文与 AC 核对两面：基线 SHALL 覆盖计划已授权的全部有效成员，按物理成员身份、聚合根相对 checkout 前缀及仓内相对路径集合保留归属，逐成员读取自身 HEAD 分支树；MUST NOT 读取脏磁盘或兄弟 worktree 文件。任一必需成员基线不可解析、前缀重复或归属不唯一 SHALL 显式失败，MUST NOT 降为基线缺席或返回部分成功。coder fork/advance 仍按 REQ-PIB-01 登记的单仓语义运行，其多仓收敛不在本 change；MUST NOT 声称多仓三面已同源。

#### Scenario: fork 自所选基线

- **WHEN** base_branch=feature/x 的单仓 Issue 启动 coding
- **THEN** worktree 分叉自 feature/x；coder 的基线包含该分支全部内容

#### Scenario: 三面同源

- **WHEN** 同一文件路径分别被 author 引用、被 AC 核对检查、被 coder 作为基线，并区分单仓与多仓会话
- **THEN** 单仓三者的存在性判定 SHALL 全部基于同一 base_branch 树，provider 原生通道的物理可达集仍不构成通道层限制；多仓本 change 仅使 author 基线上下文与 AC 核对逐成员同源，coder fork/advance 的逐成员收敛不在本 change，MUST 明示其仍按 issue.base_branch 单仓语义运行的已知缺口，MUST NOT 声称多仓三面已经同源

#### Scenario: 核对基线不可解析不跳过

- **WHEN** 单仓 AC 核对执行时 issue.base_branch 的树不可解析（分支被删）
- **THEN** 核对以 fail-closed 失败并诊断，不静默跳过核对放行候选

#### Scenario: 真实三仓基线覆盖全部授权成员

- **WHEN** 真实三仓 Git 夹具中的 busi、gateway、frontend 各提交一个独有文件，计划授权三个有效成员而 issue.repo_id 只指向 busi
- **THEN** 基线加载结果 SHALL 包含三个成员的已提交路径、物理成员身份与聚合根相对 checkout 前缀，MUST NOT 只返回 busi 的平面树

#### Scenario: 磁盘额外 Git 目录不增加授权成员

- **WHEN** 聚合根磁盘还存在未被计划授权的 Git 目录，或成员不在 manifest、active、selection 形成的有效集合内
- **THEN** 基线 SHALL 只覆盖计划已授权有效成员，MUST NOT 通过扫描磁盘把额外目录扩充为成员

#### Scenario: 脏文件与兄弟分支文件不进入基线

- **WHEN** 三仓夹具包含未提交文件以及仅兄弟分支或兄弟 worktree 存在的文件，而这些文件不在相应成员选定 HEAD 分支树内
- **THEN** 加载结果 SHALL 不含这些文件；author 上下文与 AC 核对 MUST 使用相同的逐成员已提交树事实，MUST NOT 读取当前脏磁盘兜底

#### Scenario: 任一必需成员缺失使完整基线失败

- **WHEN** 三个必需授权成员中任一成员 checkout、分支或 Git 树不可解析，而其它两个成员可正常加载
- **THEN** loader SHALL 显式返回 Err，MUST NOT 返回只含两个成员的不完整成功结构，MUST NOT 降为 None 跳过核对

#### Scenario: 前缀重复或成员归属不唯一显式失败

- **WHEN** 两个授权成员的 checkout 前缀重复，或某个引用无法唯一确定成员归属
- **THEN** 基线加载或归属解析 SHALL 显式失败，MUST NOT 任意选仓、全仓同名搜索或降为基线缺席

#### Scenario: author 渲染按真实成员归属展示

- **WHEN** author 基线上下文展示多仓可用路径清单，且成员树内包含 src 等共同目录名
- **THEN** 分组标签 SHALL 来自真实成员身份与 checkout 前缀，路径 SHALL 使用对应成员前缀并属于完整基线结构，MUST NOT 从路径目录首段猜仓或把 src 标成成员仓

#### Scenario: 单仓与真正无仓语义保留

- **WHEN** 相同加载入口分别用于单仓 Issue 与真正无仓 Issue
- **THEN** 单仓 SHALL 保留 issue.base_branch 与仓内相对路径语义；真正无仓 SHALL 保留基线缺席语义，该缺席 MUST NOT 用于掩盖多仓必需成员加载失败
