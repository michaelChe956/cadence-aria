# change 实施 per-issue-base-branch 计划 v1.0

> 按 superpowers:executing-plans / subagent-driven-development 执行。

**Goal:** issue 基准分支全链锚定——设置锁定/author 上下文基线限定（F-57 根治）/coding fork+核对三面同源。

**Architecture:** issue 模型+REST/UI（T1）→ 通道审计+分级路由（T2，最重）→ fork/核对同源（T3）→ 门禁回放（T4）。后端 Rust+前端 TS。

**Spec:** `openspec/changes/per-issue-base-branch/`（双裁决定案版）；裁决依据 pib-oracle.md/pib-maxtask.md；实证 F-57（design source id repository_0001/status.html）。

## Global Constraints

- 默认链 main→master→无默认强制手选；存量 None 同链（皆无仓=读取 OK/生成 coding fail-closed）。
- 列表仅本地 refs/heads 不隐式 fetch；远端基线=用户先建本地跟踪分支。
- 通道层路由为安全边界（host-served→git show/ls-tree；native→工作区外专用检出根；收不住拒启）；prompt/cwd/禁写/事后日志不承载。
- C1 核对改按分支名取树+不可解析 fail-closed（废弃 fail-safe 跳过）。
- 测试命令照仓规；前端 pnpm build 收尾。

## Review Focus

1. 通道漏网（terminal/MCP/resume 某形态未收）→ T2.1 审计完整性+每通道用例
2. 专用检出根仍含 .worktrees/（位置错误）→ T2.2 根路径断言
3. 存量皆无仓误回退 main → T2.3/存量场景
4. 核对跳过死灰复燃 → T3.2 不可解析红用例
5. 修改通道漏设（REST PATCH 未拒）→ T1.1 锁定断言

---

### Task 1: 模型与创建入口
- [ ] 1.1 issue 记录 base_branch（Option）+REST 透传+创建校验（五用例红绿，REQ-PIB-01 场景 1-3/5-6）。
- [ ] 1.2 UI 分支选择器（本地列表+默认预选+皆无无默认）+锁定呈现（场景 4）。

### Task 2: author 上下文基线限定（F-57 根治）
- [ ] 2.1 全通道审计报告（四形态逐 provider 落点+已知漏洞核查+旁路检测）——2.2 依据。
- [ ] 2.2 分级路由实施（host-served git show+树缓存/native 工作区外检出根+根外不可读/收不住拒启）；红绿三用例（REQ-PIB-02 场景 1-2/4）。
- [ ] 2.3 基线消失 fail-closed+存量皆无仓失败诊断（场景 3+存量）。

### Task 3: coding fork 与核对同源
- [ ] 3.1 上游入口全清单隐式基线→issue.base_branch+创建校验（场景 1）。
- [ ] 3.2 C1 核对按分支名取树+fail-closed 不跳过（场景 2-3）。

### Task 4: 门禁收口
- [ ] 4.1 全量四门禁+strict+vitest；F-56/F-57 现场回放记录。

## Self-Review

REQ-PIB-01→T1、02→T2、03→T3；Review Focus 五条全挂；T2.1→T2.2 硬依赖；预算收口留接手（C1 接力先例）。
