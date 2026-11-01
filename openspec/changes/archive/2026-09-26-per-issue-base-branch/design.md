# Design: per-issue-base-branch

> 依据：F-57/F-56 实证 + 用户三轮裁决（方案/默认值链/软限制 pivot）+ oracle 两轮+max 两轮裁决（pib-oracle.md/pib-oracle-2.md/pib-maxtask.md/pib-maxtask-2.md）。

## Context

issue_0002 的 design author 文件浏览泄漏到 `.worktrees/aria-issues/issue_0001/`（F-57）；coding fork base 由各上游入口以当前检出/HEAD 隐式确定（coding.rs:157-158、group.rs:105-106）；C1 核对树依赖共享 coding worktree、不可用时 fail-safe 跳过（被测试钉死）。

## Decisions

### D1 存储：issue 记录增 base_branch（Option<String>）

None=按默认链解析（main→master→无：读取 OK、生成/coding fail-closed 诊断）。REST 透传；UI 选择器预选默认值。创建后锁定：无修改通道。

### D2 分支列表与默认值

列表=全部本地 refs/heads（不隐式 fetch；远端跟踪引用随 fetch 漂移破坏锁定）。默认链 main→master→无默认强制手选。max-task 技术事实登记：已 fetch 的 remote-tracking ref 可直接 fork——技术可行不改变产品口径。

### D3 author 上下文限定：通道分级（硬边界 + 软限制 pivot）

- **host-served 通道**（kimi fs 桥）：路由 `git show <base>:<path>`/`git ls-tree`（树清单缓存可选）+ terminal 基线拒绝——**硬安全边界**，工作区/兄弟 worktree 不可达
- **provider 原生通道**（claude/codex/pi）：author prompt 注入基线限制教学块（prompts.rs 两族 builder，provider 无关、kimi 同注入冗余无害）——**软约束非边界**（用户 2026-09-25 裁决 pivot：可用性换保证降级；前轮「绝不降级 prompt」的技术论据仍真，pivot 是接受其风险而非驳倒）；残余风险的 AC 路径形态由 D4 核对 fail-closed 兜底，其余形态用户明示接受
- **注入落点**：prompt 组装层（builder），不在 adapter 层——教学属 prompt 组装，与 spawn 扼点层级不同；native adapter 零改动（原拒启分支未落地，clean cutover 零回滚）
- **防误读**：BaselineTreeRef 同一字段在 kimi=通道强制、native=prompt 软约束——文档注释必须写明两级语义，字段存在不构成 native 访问受限的证据
- 工具策略层仅既有写拒绝+spawn 守卫；「prompt 不承担安全边界」教义保留——教学块确实不是边界，此点须强调以防误读

### D4 coding fork 与核对同源

各上游入口隐式基线统一改读 issue.base_branch；C1 plan_baseline_tree 改按分支名直接取树（不依赖共享 worktree）+不可解析 fail-closed（**废弃 fail-safe 跳过——本条升级为 native 软限制残余风险的唯一硬兜底，若滑落则软限制零硬兜底**）。

## Deferred / 遗留演进

**base 专用检出根（native 通道沙箱）**：位于仓库工作区外、根外不可读的 base 检出作为 native 通道收束形态。
- **为什么推迟**：T2.1 审计证实四个真实 native provider 无一可验证「根外不可读」；用户 2026-09-25 裁决降级软限制（可用性优先）
- **启用条件**：OS 级沙箱包裹 native CLI + canary 六类回放验证根外不可读（审计 §5 方法）
- **再访触发**：软限制残余风险实害化（F-57 形态经 native 通道再发生且 C1 兜底不及的形态）/ native CLI 出现可验证隔离能力
- **随决策一并推迟**：专用检出根生命周期管理（创建/同步/清理——本期不创建任何检出根）

## Risks / Trade-offs

- native 会话 cwd=repository.path：CLI 启动预读（CLAUDE.md 等）读的是当前检出而非基线——内容源混淆在会话初始化即发生，早于任何 prompt；软约束对此面零作用，如实登记为残余
- T3.2 滑落=软限制零硬兜底（D4 前提）
- story/design 阶段无机械核对面（C1 仅 plan 期）；同路径异内容形态穿过 C1；非路径影响（prose 断言/抄内容）穿过
- 残余风险量化（~95%）为未验证估计——验证姿态=canary 六类回放（审计 §5），现有 tool-policy audit 无 native 路径凭证，需进程/OS trace 否则 unknown
- author 文件通道四形态（fs/terminal/MCP/resume）审计已全覆盖（pib-audit-report.md——Open Question 已答，引用关闭）

## Open Questions

（无——前述通道清单问题已由 T2.1 审计回答）
