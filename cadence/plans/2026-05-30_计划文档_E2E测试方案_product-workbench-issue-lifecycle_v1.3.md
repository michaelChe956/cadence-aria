# product-workbench-issue-lifecycle 真实场景端到端 E2E 测试方案

## 文档信息

- 文档类型：计划文档 / E2E 测试方案
- 版本：v1.3
- 日期：2026-05-30
- 适用分支：`product-workbench-issue-lifecycle`
- 工作区：`.worktrees/product-workbench-issue-lifecycle`
- 目标代码库：`/home/michael/workspace/github/naruto`
- 方案范围：只定义测试方案、执行步骤、验收口径与风险；不执行测试，不修改业务代码。

## 1. 当前分支进度理解

当前分支已经具备从产品工作台到 Coding Workspace 的完整用户入口：

- `/` 进入 `/workbench`，默认页面为 `IssueLifecycleWorkbench`。
- 用户可在 UI 中创建 Project、添加 Repository、创建 Issue。
- Issue 必须绑定 Repository；Repository path 会 canonicalize 并保存到产品索引。
- Issue 可生成 Story Spec Workspace，confirmed Story 可生成 Design Spec Workspace，confirmed Design 可生成 Work Item Workspace。
- `/workbench/workspace/:sessionId` 是 Story/Design/Work Item 的对话式 Workspace，通过 WebSocket 调真实 streaming provider。
- Workspace 的 `准备上下文` 阶段支持“发送”上下文和显式“开始生成”，不会把普通上下文输入误当作开始执行。
- Workspace 会持久化消息、artifact、timeline、node detail、provider execution event，并在确认后更新 Story/Design/Work Item 状态。
- Work Item confirmed 后，用户可从 drawer 进入 `/workbench/coding/:attemptId`。
- Coding Workspace 已包含 worktree prepare、Coding、Testing、Rework Analyst、Code Review、Review Request、Internal PR Review、Final Confirm 等阶段。
- Coding Workspace 每个 LLM 阶段前有 Stage Gate，用户可“立即开始”、切换 provider 或中止。
- Coding Workspace 真实 provider 通过默认 registry 调用本机 `codex` 与 `claude` 命令；不要设置 `ARIA_PROVIDER_MODE=fake`。

当前需要审核的边界：

- 用户期望最终由 Aria 自己创建 Pull Request 或 Merge Request，并在 PR/MR 描述中包含本次变更总结、审核结论和后续人工处理信息。
- 当前代码中 `ReviewRequest` 是否已经具备真实 PR/MR 创建能力需要在 E2E 中验证；如果最终只产生 `git_branch_only`、只有 pushed branch、没有 PR/MR URL，则本轮按产品能力缺口记录为失败或阻塞。
- 测试执行者不得手动执行 `git add`、`git commit`、`git push`、`gh pr create` 或类似会改变目标仓库本地/远端状态的命令；所有写入、提交、推送和 PR/MR 创建必须由 Aria 通过产品流程完成。
- `naruto` 目前是极简仓库，没有 Python 工程结构；Work Item 必须明确 Python 文件布局和验证命令，避免 Coding Workspace 只能依赖仓库探测兜底。
- 当前 Design Spec 创建入口的 `design_kind` 只有 `frontend` 枚举；本测试按现有产品入口使用它承载通用技术设计，不把枚举名作为业务失败条件。

## 2. 测试目标

1. 从浏览器模拟人工点击，完整走通 Project -> Repository -> Issue -> Story Spec -> Design Spec -> Work Item -> Coding -> 测试 -> review -> push/review request -> final confirm。
2. 使用真实 provider，而不是 fake provider，验证 Story、Design、Work Item 产物和 Coding 阶段都来自真实 provider 执行。
3. 在 `/home/michael/workspace/github/naruto` 中落地 Python 代码与测试，且测试覆盖 `n=1`、`n=2`、`n=3`、`n=5`、`n=10`。
4. 验证 Aria 的浏览器可观测证据：Workspace artifact、provider 状态、timeline、Coding testing report、review report、git push/review request 信息。
5. 验证最终 PR/MR 由 Aria 自动创建，PR/MR 描述包含变更总结、测试结果、review 结论和交由人工后续处理的说明。
6. 不通过手工改 `naruto` 或手工 git/PR 操作来制造成功；失败必须记录为 provider 输出问题、产品工作流问题或目标仓库环境问题。

## 3. 必选业务场景

Issue 标题：

```text
爬楼梯问题
```

Issue 描述：

```text
实现爬楼梯问题：给定 n 阶楼梯，每次可以爬 1 或 2 阶，返回到达楼顶的不同方法数。请使用 python 实现函数 climb_stairs(n: i32) -> i32，并补充测试覆盖 n=1、n=2、n=3、n=5、n=10。
```

业务验收值：

| n | 期望返回 |
|---|----------|
| 1 | 1 |
| 2 | 2 |
| 3 | 3 |
| 5 | 8 |
| 10 | 89 |

Python 落地建议：

- 函数签名在 Python 中映射为 `def climb_stairs(n: int) -> int`。
- 推荐实现文件：`climb_stairs.py`。
- 推荐测试文件：`test_climb_stairs.py`。
- 推荐验证命令：`python -m unittest -v test_climb_stairs.py`。
- Work Item artifact 必须包含“验证命令”段落，并显式写入上面的命令，保证 Coding Workspace 能读取计划命令。

## 4. 执行前置条件

### 4.1 Cadence Aria 工作区

```bash
cd /home/michael/workspace/github/cadence-aria/.worktrees/product-workbench-issue-lifecycle
git status --short --branch
cargo check --locked
```

期望：

- 当前分支为 `product-workbench-issue-lifecycle`。
- 工作区无未预期改动。
- `cargo check --locked` 通过。

### 4.2 Naruto 目标仓库

```bash
cd /home/michael/workspace/github/naruto
git status --short --branch
git remote -v
```

期望：

- 当前分支为 `main`。
- 工作区干净。
- `origin` 可由 Aria 使用当前凭据创建分支并创建 PR/MR。
- 执行者不手动创建分支、不手动 push、不手动创建 PR/MR；只观察和审核 Aria 的结果。

### 4.3 Provider 与工具

```bash
codex --version
claude --version
pnpm --version
cargo watch --version
playwright-cli --help
```

期望：

- `codex`、`claude` 均已登录且可执行。
- `pnpm` 可用。
- `cargo-watch` 可用；缺失时按项目规则安装宿主机组件，不使用 Docker 绕过。
- `playwright-cli` 可用。

## 5. 服务启动方案

使用源码开发服务，不使用旧 `web/dist` 静态产物。

终端 A，在 Aria worktree 根目录：

```bash
cargo watch -w src -w Cargo.toml -w Cargo.lock -x "run --locked -- web --workspace . --host 127.0.0.1 --port 4317"
```

终端 B，在 `web/` 目录：

```bash
pnpm dev
```

健康检查：

```bash
curl --noproxy '*' -sS http://127.0.0.1:4317/api/health
curl --noproxy '*' -sS -I http://127.0.0.1:5173/
curl --noproxy '*' -sS http://127.0.0.1:5173/api/health
```

期望：

- 后端返回 `{"status":"ok"}`。
- 前端返回 `200 OK`。
- Vite proxy `/api/health` 返回 `{"status":"ok"}`。

## 6. Playwright CLI 操作原则

- 使用 `playwright-cli open http://127.0.0.1:5173/workbench` 打开真实浏览器。
- 每次点击前先用 `playwright-cli snapshot` 确认可访问名称和 refs。
- 优先按 role/name 操作；没有稳定 ref 时再用当前 snapshot ref。
- 不使用固定 sleep 判断 provider 完成；等待页面出现具体状态：
  - Workspace：`开始生成`、`确认通过`、`流程已完成`、Artifact 非空。
  - Coding：`Stage Gate 立即开始`、`测试通过`、`approve`、`review request 已创建`、`确认完成`。
- 真实 provider 阶段若出现权限请求，必须从浏览器点击允许或拒绝，不在终端代替 UI 操作。
- 每个阶段保留必要截图、snapshot、console、requests 作为失败诊断材料。
- 不使用终端替 Aria 执行任何写入型 git 或 PR/MR 命令；终端只允许做只读审计，且以浏览器可见证据为主。

## 7. 主链路测试步骤

### E2E-L4-01 创建 Project

步骤：

1. 打开 `http://127.0.0.1:5173/workbench`。
2. 点击 `新建 Project`。
3. 填写 Project 名称：`E2E Naruto Climb Stairs 20260530-<HHMMSS>`。
4. 填写 Project 描述：`真实 provider E2E：naruto 爬楼梯问题`。
5. 点击 `创建 Project`。

断言：

- 左侧 Projects 出现新 Project。
- 页面标题仍为 `Issue 生命周期工作台`。
- 当前 Project 显示为刚创建的名称。

### E2E-L4-02 添加 Naruto 代码库

步骤：

1. 点击 `添加代码库`。
2. 填写代码库名称：`naruto`。
3. 填写本地路径：`/home/michael/workspace/github/naruto`。
4. Policy 选择 `manual-write`。
5. Provider 选择 `Codex`。
6. 点击 `添加代码库`。

断言：

- 代码库列表出现 `naruto`。
- 路径显示 `/home/michael/workspace/github/naruto`。
- `新建 Issue` 按钮变为可用。

### E2E-L4-03 创建 Issue

步骤：

1. 点击 `新建 Issue`。
2. Issue 标题填写 `爬楼梯问题`。
3. Issue 描述填写第 3 节必选描述。
4. 代码库选择 `naruto`。
5. 点击 `创建 Issue`。

断言：

- Issue 列出现 `爬楼梯问题`。
- 选中 Issue 后详情区 preview 包含 `climb_stairs` 和 `n=10`。
- Story Spec、Design Spec、Work Item 区域均为 `暂无内容`。

### E2E-L4-04 生成并确认 Story Spec

步骤：

1. 在 Issue 卡片点击 `生成 Story Spec`。
2. 自动进入 `/workbench/workspace/<session_id>`。
3. 打开 `Provider 配置`，确认 Author/Reviewer 使用真实 provider。建议：
   - Author：`Codex`
   - Reviewer：`Claude Code`
   - 启用交叉审核：开启
   - 审核轮次：1
4. 在 `补充上下文` 输入框发送：

```text
请把 Python 映射明确写进 Story：函数名 climb_stairs，Python 签名 def climb_stairs(n: int) -> int；验收值必须包含 1->1、2->2、3->3、5->8、10->89。
```

5. 确认页面仍停留在 `prepare_context`，然后点击 `开始生成`。
6. 如出现权限请求，从浏览器点击允许。
7. 等待进入人工确认阶段，打开 `Artifact` 查看产物。
8. 若 artifact 明显遗漏验收值，发送修改意见；否则点击 `确认通过`。
9. 点击 `返回` 回到 workbench。

断言：

- Story Workspace 的聊天记录中能看到补充上下文。
- Artifact 中包含目标函数、输入输出、验收值和边界说明。
- 返回看板后 Story Spec 卡片出现，状态为 confirmed 或 drawer 显示 `已确认`。
- Story version 记录存在，作者/审核 provider 信息可见。

### E2E-L4-05 生成并确认 Design Spec

步骤：

1. 在 Story Spec 卡片打开 drawer。
2. 点击 `生成 Design Spec`。
3. 进入 Design Workspace。
4. 打开 `Provider 配置`，继续使用真实 provider。
5. 发送补充上下文：

```text
Design 必须面向极简 Python 仓库，避免引入额外依赖。建议使用 climb_stairs.py + test_climb_stairs.py + unittest。请写清楚算法为 DP/迭代，时间 O(n)、空间 O(1)。
```

6. 点击 `开始生成`。
7. 如出现权限请求，从浏览器点击允许。
8. 等待 Artifact 非空并进入人工确认。
9. 确认 Design 说明文件布局、算法、测试策略后点击 `确认通过`。
10. 返回 workbench。

断言：

- Design Spec 卡片出现并 confirmed。
- Artifact 说明 Python 文件布局和 unittest 验证策略。
- Design 关联来源 Story。

### E2E-L4-06 生成并确认 Work Item

步骤：

1. 在 Design Spec 卡片打开 drawer。
2. 点击 `生成 Work Item`。
3. 进入 Work Item Workspace。
4. 打开 `Provider 配置`，继续使用真实 provider。
5. 发送补充上下文：

```text
Work Item 必须可直接驱动 Coding Workspace。请明确文件改动：
- 新增 climb_stairs.py，提供 def climb_stairs(n: int) -> int。
- 新增 test_climb_stairs.py，使用 unittest 覆盖 n=1、n=2、n=3、n=5、n=10。
- 验证命令必须写在标题为“验证命令”的段落中，命令为 python -m unittest -v test_climb_stairs.py。
```

6. 点击 `开始生成`。
7. 如出现权限请求，从浏览器点击允许。
8. 等待 Artifact 非空并进入人工确认。
9. 打开 Artifact，确认包含 `验证命令` 和 `python -m unittest -v test_climb_stairs.py`。
10. 若命令缺失，发送修改意见要求补齐；否则点击 `确认通过`。
11. 返回 workbench。

断言：

- Work Item 卡片出现。
- Work Item drawer 显示可 `开始 Coding`。
- Work Item 的 plan status 为 confirmed。
- Work Item artifact 可被 Coding Workspace 读取到验证命令。

### E2E-L4-07 创建 Coding Attempt 并准备运行

步骤：

1. 打开 Work Item drawer。
2. 点击 `开始 Coding`。
3. 进入 `/workbench/coding/<attempt_id>`。
4. 确认顶部显示：
   - stage 为 `prepare_context`
   - base branch 为 `main` 或当前 naruto 分支
   - branch 为 `aria/work-items/<work_item_id>/attempt-1`
5. 在 Coding Provider 配置面板中建议设置：
   - Coder：`Codex`
   - Tester：`Codex`
   - Analyst：`Codex`
   - Code Reviewer：`Claude Code`
   - Internal Reviewer：`Codex`
6. 在 `补充 Coding 上下文` 输入：

```text
请只修改 naruto 仓库的业务文件，不提交 .aria、__pycache__、*.pyc 或测试输出日志。完成后测试命令必须是 python -m unittest -v test_climb_stairs.py。
```

7. 点击 `发送上下文`。

断言：

- Coding chat 中出现用户上下文。
- 状态仍允许点击 `开始 Coding`。
- Provider 配置已按角色显示真实 provider。

### E2E-L4-08 运行 Coding Workspace 全流程

步骤：

1. 点击 `开始 Coding`。
2. Worktree prepare 完成后，遇到每个 Stage Gate 时点击 `立即开始`，避免等待倒计时。
3. Coding 阶段如出现 provider stream，观察是否创建/修改 Python 文件。
4. Testing 阶段观察 Testing Report。
5. Rework Analyst 阶段如果判定 `needs_fix`，继续通过后续 Gate，让流程回到 Coding 修复。
6. Code Review 若 request changes，继续通过 Rework -> Coding 修复。
7. Review Request 阶段观察 Git 面板：
   - commit sha 非空
   - push 状态为 `pushed`
   - branch 为 `aria/work-items/<work_item_id>/attempt-1`
   - PR/MR URL 非空，且明确由 Aria 创建
   - PR/MR 描述包含本次变更总结、测试结果、review 结论和后续人工处理说明
8. Internal PR Review 通过后，等待 Final Confirm。
9. 点击 `确认完成`。

断言：

- Timeline 至少包含 `准备 worktree`、`代码编写`、`执行测试`、`分析官判定`、`代码审查`、`发起 review request`、`内部 PR 审查`、`最终确认`。
- Tests 面板显示 `python -m unittest -v test_climb_stairs.py`，状态 passed，exit code 0。
- Review 面板 Code Review verdict 为 approve，Internal PR Review verdict 为 approve。
- Git 面板显示 commit、remote、push pushed、review request id 和 PR/MR URL。
- 打开或查看 PR/MR 详情时，标题对应 `爬楼梯问题`，描述包含实现摘要、测试命令/结果、review 结论和交由人工处理的说明。
- Coding Attempt 状态最终为 completed。
- 回到 workbench 后 Work Item 卡片显示 `completed` 或 latest attempt 为 `completed · final_confirm`。

### E2E-L4-09 目标仓库落地验证

在浏览器完成后，从终端只做只读审计和验证，不手工修代码，不执行 checkout、commit、push、PR/MR 创建等会改变状态的命令：

```bash
cd /home/michael/workspace/github/naruto
git status --short
git ls-remote --heads origin 'aria/work-items/*'
git log --oneline --decorate -n 5
```

期望：

- 当前主工作区没有被污染；不要求主工作区存在 `climb_stairs.py` 或 `test_climb_stairs.py`。
- 远端存在 Aria pushed branch。
- 业务改动应在 Aria 创建的 Coding worktree/branch 中完成。
- PR/MR 证据优先来自 Aria UI；如需外部只读核对，只允许使用 `gh pr view` 或 Web 页面查看，不允许 `gh pr create`。

如需审计 Aria worktree：

```bash
git -C <coding_worktree_path> status --short
git -C <coding_worktree_path> diff --stat <base_branch>...HEAD
cd <coding_worktree_path>
python -m unittest -v test_climb_stairs.py
```

期望 diff：

- 包含 `climb_stairs.py`。
- 包含 `test_climb_stairs.py`。
- 不包含 `.aria/`、`__pycache__/`、`*.pyc`、测试输出日志。

## 8. 真实 PR/MR 验收口径

本轮采用严格 PR/MR 口径：

- Aria 必须自己完成代码提交、分支 push 和 Pull Request 或 Merge Request 创建。
- 执行者不得通过终端或第三方 CLI 手工补 `git push`、`gh pr create`、GitLab MR 创建或任何等价写操作。
- PR/MR URL 必须在 Aria UI 中可见，或能从 Aria 的 Review Request 详情中读取。
- PR/MR 描述必须包含本次变更总结、测试命令与结果、自动 review / internal review 结论，以及“后续交由人工处理”的说明。

通过标准：

- Review Request 阶段显示 created/succeeded。
- `external_url` 或等价 PR/MR 链接非空。
- 链接指向目标仓库的真实 PR/MR。
- PR/MR head branch 为 `aria/work-items/<work_item_id>/attempt-1` 或 Aria 实际展示的 attempt branch。
- PR/MR base branch 为 `main` 或测试开始时 Aria 识别的目标 base branch。
- PR/MR 描述符合上述内容要求。

失败/阻塞标准：

- 如果 Aria 只完成 commit/push，但 Review Request 类型为 `git_branch_only` 且没有 PR/MR URL，本 E2E 不通过，记录为产品能力缺口。
- 如果 UI 只给出“请人工创建 PR/MR”的 instructions，本 E2E 不通过，记录为产品能力缺口。
- 如果执行者手工创建了 PR/MR，该 PR/MR 不能作为本 E2E 通过证据。

## 9. 失败分类与处置

| 失败点 | 归类 | 处置 |
|--------|------|------|
| Project/Repository/Issue 创建失败 | 产品 API/UI 缺陷 | 保留 Network 请求与响应，停止主链路 |
| Workspace 生成产物缺失业务字段 | Provider 输出质量问题或 prompt 缺陷 | 先用人工修改意见返修一次；仍失败则记录 |
| 确认后看板不同步 | 产品状态同步缺陷 | 保留 API `/api/issues/<id>/lifecycle` 响应 |
| Work Item 缺少验证命令 | Provider 输出质量问题，影响 Coding | 必须返修 Work Item，不进入 Coding |
| Coding worktree prepare 失败 | Git 环境或产品 Git 服务缺陷 | 检查 `naruto` 状态、branch、remote |
| Testing 未执行指定命令 | Work Item 解析或 Tester Loop 缺陷 | 保留 work item markdown 和 testing report |
| Review Request push 失败 | Git remote/auth 缺陷 | 保留 Git 面板、stderr、review request JSON |
| Review Request 未创建 PR/MR | 产品能力缺口或 provider/tooling 配置缺陷 | 保留 UI 证据、review request JSON，不手工补 PR/MR |
| PR/MR 描述缺少总结或审核信息 | 产品模板或 provider 输出缺陷 | 保留 PR/MR 页面和 Aria review 记录，记录为失败 |
| Provider 长时间无输出 | Provider 执行失败或超时 | 用浏览器中止，记录 provider 与阶段 |
| 最终没有 PR/MR URL | 产品能力缺口 | 按第 8 节记录为失败或阻塞 |

## 10. 最终通过标准

本方案通过必须同时满足：

- 浏览器中完整走通 Project -> Repository -> Issue -> Story -> Design -> Work Item -> Coding Attempt。
- Story、Design、Work Item 均由真实 provider 生成并被人工确认。
- Coding Attempt 状态为 completed。
- Testing Report 中指定 Python unittest 命令 passed。
- Review Request 阶段由 Aria 成功 commit、push 分支并创建真实 PR/MR。
- PR/MR 描述包含变更总结、测试结果、review 结论和交由人工后续处理的说明。
- `naruto` 的实现文件和测试文件存在于 coding branch/worktree。
- 测试覆盖 5 个指定 n 值，且返回值正确。
- 无 `.aria/`、`__pycache__/`、`*.pyc`、测试输出日志进入业务提交。
- 不存在执行者手工 git 写操作或手工创建 PR/MR 的补成功行为。

## 11. 执行记录模板

测试执行后在同目录新增状态记录或在本文件追加记录，建议格式：

```markdown
## 执行记录 YYYY-MM-DD HH:mm

- 执行人：
- Aria commit：
- Naruto base branch / commit：
- Project ID：
- Repository ID：
- Issue ID：
- Story Spec ID：
- Design Spec ID：
- Work Item ID：
- Coding Attempt ID：
- Coding branch：
- Commit SHA：
- Review request / PR：
- 测试命令：
- 测试结果：
- 结论：通过 / 阻塞 / 失败
- 失败证据：
- 后续动作：
```

## 12. 已确认审核项

1. 最终必须由 Aria 创建真实 Pull Request 或 Merge Request；只产生 pushed branch 或 `git_branch_only` 不满足本轮期望。
2. 本轮真实 provider 组合采用建议值：Workspace Author `Codex`、Reviewer `Claude Code`；Coding Coder/Tester/Analyst/Internal Reviewer `Codex`、Code Reviewer `Claude Code`。
3. 执行者不手工做 git 写操作或 push；只观察和审核 Aria 如何提交、push、创建 PR/MR。
4. 如果真实 provider 生成 Story/Design/Work Item 质量不足，允许最多一次通过 UI “发送修改意见”返修后继续。
