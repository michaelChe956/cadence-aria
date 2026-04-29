# Aria 产品化端到端入口与 Workflow Engine 设计

## 文档信息

- 文档类型：技术方案
- 版本：v1.0
- 日期：2026-04-29
- 适用仓库：`cadence-aria`
- 目标测试项目：`/Users/michaelche/Documents/git-folder/github-folder/naruto`
- 目标机器：
  - macOS Intel
  - Win11 WSL Ubuntu

## 1. 背景

当前 Aria 已具备一批底层能力，包括 daemon 基础生命周期、REPL discovery、wire protocol、runtime units、OpenSpec 约束编译、provider adapter、fake provider 链路测试等。

但这些能力尚未形成面向用户的正式端到端入口：

- `aria repl` 当前更接近 daemon discovery stub，不是 Claude Code CLI / Codex CLI 风格的自然语言交互会话。
- `daemon new_task` 当前主要完成 task/intake 初始化，没有接管完整 N00-N28 workflow。
- planning、coding、testing、final closure 等 runtime chain 主要存在于库函数和 fake provider 测试中，尚未通过 CLI/daemon 产品化。
- 真实 Claude Code、Codex provider adapter 已有基础能力，但还没有被正式端到端任务入口稳定编排。

因此，当前问题不应只按“补 REPL 命令”处理，而应拆成三阶段：先跑通真实端到端，再沉入 daemon workflow engine，最后构建自然语言 REPL。

## 2. 目标与非目标

### 2.1 目标

第一优先级是新增非交互式 E2E 调试入口，使 Aria 能通过一条命令在 `naruto` 测试 worktree 中完成真实 JWT 登录功能开发闭环。

正式 E2E 输入需求为：

```text
做一个用户登录功能，具备前端页面，后端服务，用 JWT 即可。不需要 SQLite，不需要数据库，内存处理即可。
```

端到端链路必须使用真实 Claude Code、Codex、superpowers、OpenSpec。外部人工不直接替 Aria 开发或测试 `naruto`，只审计 Aria 产物。

第二优先级是把第一阶段的同步 runner 下沉为 daemon workflow engine，使 daemon 真正负责 workflow 状态、恢复、事件流、gate、provider 调度和 artifact index。

第三优先级是重新设计 REPL，使其成为 Claude Code CLI / Codex CLI 风格的自然语言 agent 会话，而不是内部 wire protocol shell。

### 2.2 非目标

本设计不要求一次性完成所有最终产品能力。

第一阶段不要求 daemon 具备完整 workflow engine，也不要求 REPL 具备完整交互体验。第一阶段只要求通过产品化入口跑通真实 E2E，并保留后续迁移到 daemon 的清晰边界。

本设计不要求人工测试 `naruto` 开发内容。`naruto` 的构建、测试、验证应由 Aria workflow 内部触发并记录证据。

## 3. 总体路线

采用三阶段路线：

1. 阶段 A：实现 `aria task run` 非交互式 E2E 入口。
2. 阶段 C：实现 daemon workflow engine，并让 `task run` 迁移为 daemon 客户端。
3. 阶段 REPL：实现 Claude Code / Codex 风格自然语言交互会话。

路线取舍如下：

- 阶段 A 最贴近当前目标，优先验证 Aria 是否能真实驱动 `naruto` 完成登录功能。
- 阶段 C 是长期正确架构，避免 CLI、REPL、daemon 各自实现 workflow。
- REPL 依赖 C 的 daemon engine 才能获得稳定价值，因此不应优先做成协议命令控制台。

## 4. 阶段 A：非交互式 E2E 入口

### 4.1 用户入口

新增命令：

```bash
aria task run \
  --workspace /Users/michaelche/Documents/git-folder/github-folder/naruto-aria-e2e-login \
  --request "做一个用户登录功能，具备前端页面，后端服务，用 JWT 即可。不需要数据库，内存处理即可" \
  --non-interactive
```

可选参数：

```bash
--change-id aria-login-jwt
--providers real
--timeout 3600
--report json
```

默认行为：

- `--providers real` 使用真实 Claude Code 和 Codex。
- `--non-interactive` 不在中途等待人工输入。
- 遇到必须人工确认的 hard gate 时，任务状态落为 `blocked_by_gate`，并生成 blocked report，而不是挂起等待。

### 4.2 组件划分

阶段 A 新增或整理 5 个边界清晰的组件：

1. `TaskRunCommand`
   - 解析 CLI 参数。
   - 识别 workspace、request、change id、provider mode、timeout、report mode。
   - 负责命令层错误提示，不承载 workflow 业务逻辑。

2. `TaskRunOrchestrator`
   - 第一阶段的同步编排器。
   - 负责按顺序调用 OpenSpec bootstrap、planning chain、execution chain、final closure chain。
   - 不直接绑定 CLI stdout，后续可迁移进 daemon `WorkflowEngine`。

3. `ProviderFactory`
   - 根据节点 contract 选择 Claude Code 或 Codex。
   - 生产真实 `CliProviderAdapter`。
   - 测试中可注入 fake/scripted provider。

4. `TaskRunStore`
   - 负责 `.aria/runtime/tasks/<task_id>/` 下的状态、artifact、provider run、stdout/stderr ref、structured output ref、报告落盘。
   - 输出证据优先落盘，不只依赖终端文本。

5. `TaskRunReport`
   - 负责最终人类可读摘要和机器可读 JSON 摘要。
   - 包含 task id、change id、final status、provider run refs、OpenSpec change 路径、testing report 路径、final summary 路径、target diff 摘要。

### 4.3 执行流程

阶段 A 的同步流程如下：

1. 校验 workspace 是 git worktree，并存在 `openspec/config.yaml`。
2. 校验目标 worktree 状态，避免在脏工作区上不明确地执行。
3. 创建 task id 与 change id。
4. 初始化 `.aria/runtime/tasks/<task_id>/`。
5. 创建或 bootstrap OpenSpec change。
6. 运行 planning chain，生成或写回 spec、design、tasks。
7. 运行 execution chain，触发 coding、testing、code review、rework。
8. 运行 final closure chain，生成 final review 和 final summary。
9. 写入最终 report。
10. CLI 输出摘要和关键路径。

### 4.4 gate 策略

阶段 A 默认面向无人值守 E2E，因此 gate 策略必须确定：

- soft gate：如果 workflow 有足够默认策略，可自动采用保守默认继续。
- hard gate：如果必须人工确认，任务状态写为 `blocked_by_gate`。
- blocked report 必须说明 gate id、阻塞节点、需要用户确认的问题、已完成产物和下一步恢复建议。

阶段 A 不实现长时间等待用户交互，也不把 REPL 作为必要依赖。

### 4.5 验收标准

执行 `aria task run ... --non-interactive` 后，必须满足以下证据要求：

- `naruto-aria-e2e-login/openspec/changes/<change_id>/` 存在。
- `naruto-aria-e2e-login/.aria/runtime/tasks/<task_id>/` 存在。
- provider run records 存在，并能追踪 stdout、stderr、structured output ref。
- testing report 存在，记录 Aria 触发的验证命令、工作目录、结果和失败信息。
- final summary 或 blocked report 存在。
- `git diff` 显示登录功能相关变更；如果任务失败或被 gate 阻塞，报告必须解释未产生完整 diff 的原因。

## 5. 阶段 C：daemon Workflow Engine

### 5.1 目标

阶段 C 将阶段 A 的同步 runner 下沉到 daemon，使 daemon 成为唯一 workflow 执行者。CLI 和未来 REPL 都只是客户端。

目标命令仍保持简单：

```bash
aria task run --workspace /path/to/naruto-worktree --request "做一个用户登录功能..."
```

但内部行为变为：

1. CLI 连接或启动 daemon。
2. CLI 提交 task。
3. daemon 接管 workflow。
4. CLI 订阅事件并等待完成。
5. daemon 持久化状态与产物。
6. CLI 输出最终报告。

### 5.2 daemon 模块

阶段 C 新增 5 个 daemon 核心模块：

1. `WorkflowEngine`
   - 驱动 N00-N28 节点。
   - 负责 intake、OpenSpec、planning、execution、testing、review、integration、final closure 的节点推进。
   - 不处理 CLI 文本，只处理结构化 task。

2. `WorkflowStateStore`
   - 持久化 task state、node state、gate state、event cursor、provider run refs、artifact index。
   - 支持 daemon 重启后从最近稳定节点恢复。

3. `ProviderScheduler`
   - 根据节点 contract 选择 Claude Code 或 Codex。
   - 控制 provider run 串行或有限并发。
   - 统一记录 stdout/stderr、structured output、失败分类、retry 信息。

4. `GateManager`
   - 负责 approval gate。
   - 非交互模式下将 hard gate 转为 `blocked_by_gate`。
   - 交互模式下接收 CLI/REPL 的 approve、reject、reply，并恢复 workflow。

5. `EventBus`
   - 向 CLI/REPL 推送 task、node、provider、artifact、testing、gate、completion 事件。
   - 支持 replay cursor 和客户端断线重连。

### 5.3 客户端关系

阶段 C 后，`task run` 和 REPL 均走 daemon：

```text
CLI / REPL
  -> daemon wire command
  -> WorkflowEngine
  -> Runtime Units
  -> ProviderScheduler
  -> Task Store / EventBus
  -> CLI / REPL display
```

CLI 不再直接调用 runtime units。REPL 也不直接调用 runtime units。workflow 行为只有 daemon 一份实现。

### 5.4 恢复策略

daemon 启动时读取 `.aria/runtime/session.json` 和 task state：

- 已完成节点不重复执行。
- 未开始节点可继续执行。
- 正在运行的 provider run 恢复为 `unknown_recovered` 或 `failed_recoverable`。
- workflow 根据 provider run 状态决定是否重试、阻塞或失败。
- event cursor 支持客户端重新订阅。

### 5.5 验收标准

阶段 C 完成后应满足：

- `aria daemon run` 可以执行真实任务，而不只是处理基础 wire command。
- `aria task run` 在 daemon 模式下能跑完整登录 E2E。
- daemon 中断后重启，能恢复 task 状态。
- CLI 能实时显示 workflow 进度。
- 阶段 A 的 E2E 验收不失效，底层执行器从同步 runner 切到 daemon engine。

## 6. 阶段 REPL：自然语言 agent 会话

### 6.1 定位

REPL 是 Aria 的自然语言交互式任务会话，体验接近 Claude Code CLI / Codex CLI。

REPL 不是 daemon wire protocol shell，不暴露 `hello`、`attach`、`new_task` 这类内部命令。

### 6.2 启动方式

```bash
aria
```

或：

```bash
aria repl --workspace /path/to/naruto-worktree
```

默认输入就是自然语言任务：

```text
> 做一个用户登录功能，具备前端页面，后端服务，用 JWT 即可，不需要数据库
```

### 6.3 交互模型

REPL 收到自然语言输入后：

1. 识别这是新任务、补充说明，还是当前 gate 的回答。
2. 自动连接或启动 daemon。
3. 将任务提交给 daemon workflow engine。
4. 实时显示 workflow 进度。
5. 如果需要用户补充，直接用自然语言提问。
6. 如果需要 gate，显示清晰的批准、拒绝或补充信息选项。
7. 任务完成后展示摘要、diff、测试结果和产物路径。

### 6.4 slash commands

REPL 只保留少量辅助 slash commands：

```text
/status      查看当前任务状态
/artifacts   查看当前任务产物
/diff        查看目标 worktree diff 摘要
/tests       查看最近测试报告
/approve     批准当前 gate
/reject      拒绝当前 gate
/resume      恢复最近任务
/new         开始新任务
/quit        退出
```

这些命令是用户体验层，不应泄漏 daemon 内部协议。

### 6.5 展示策略

REPL 默认展示摘要事件，不全量刷 Claude/Codex 原始 stdout：

- 展示 task 创建、OpenSpec 写回、provider run 开始/完成、测试失败/通过、review 结果、final summary。
- 原始 stdout/stderr 通过 artifact 路径引用。
- 用户可通过 `/artifacts` 和 `/tests` 查看证据路径。

### 6.6 REPL 验收标准

REPL 完成后应满足：

- 用户可以像使用 Claude Code/Codex 一样输入自然语言任务。
- REPL 自动提交 task 到 daemon workflow engine。
- 用户不需要理解 N00-N28 或 wire protocol。
- `/status`、`/diff`、`/artifacts`、`/tests` 可用于调试观察。
- gate 通过自然语言提示和 slash command 处理。

## 7. 测试策略

### 7.1 阶段 A 测试

优先使用 TDD 实现以下测试层：

- CLI 参数解析测试：覆盖 `task run`、workspace、request、change id、non-interactive、report mode。
- orchestrator fake provider 测试：验证 OpenSpec、planning、execution、final closure 调用顺序和产物落盘。
- provider factory 测试：验证 Claude Code/Codex 节点选择和 adapter 配置。
- blocked gate 测试：验证 hard gate 在 non-interactive 下落为 `blocked_by_gate`。
- 真实 E2E 手动门禁：在 `naruto-aria-e2e-login` 中运行真实 provider 链路。

### 7.2 阶段 C 测试

阶段 C 需要覆盖：

- daemon workflow 状态推进。
- event bus 事件顺序。
- task state 持久化与恢复。
- provider run 恢复分类。
- CLI 提交任务并等待 daemon 完成。
- daemon 中断恢复后继续或明确失败。

### 7.3 REPL 测试

REPL 测试重点：

- 自然语言输入被识别为 task request。
- slash command 不泄漏 wire protocol。
- gate 提示和 approve/reject 能恢复 workflow。
- event stream 展示稳定且不刷屏。

## 8. 实施顺序

推荐实施顺序：

1. 为阶段 A 写详细 implementation plan。
2. TDD 补 `aria task run` CLI 入口。
3. TDD 补 `TaskRunOrchestrator` fake provider 链路。
4. 接入真实 `CliProviderAdapter` 和 provider factory。
5. 在 `naruto-aria-e2e-login` 跑真实 JWT 登录 E2E。
6. 将阶段 A 的 orchestrator 边界作为阶段 C `WorkflowEngine` 的迁移输入。
7. 实施 daemon workflow engine。
8. 基于 daemon engine 实施自然语言 REPL。

## 9. 设计决策

### 9.1 为什么先 A

当前最重要的问题是验证 Aria 能否真实完成端到端任务。A 的反馈最快，能最早暴露真实 provider、OpenSpec、target worktree、测试命令、产物落盘之间的集成问题。

### 9.2 为什么后 C

C 是长期正确架构，但范围更大。如果直接做 C，容易在 daemon 恢复、事件流、gate、scheduler 等基础设施上消耗大量时间，却迟迟无法验证真实登录 E2E。

### 9.3 为什么不先做旧式 REPL

协议命令式 REPL 只能帮助调试 daemon wire protocol，不能提供用户期望的 Claude Code/Codex 风格体验。并且在 daemon 尚未具备 workflow engine 前，REPL 即使能 `new_task`，也无法自然完成真实端到端任务。

### 9.4 为什么 REPL 要依赖 daemon engine

自然语言 REPL 需要可恢复的长任务、事件流、gate、artifact 查询、diff 展示和 provider 调度。如果 REPL 自己实现 workflow，会与 CLI 和 daemon 产生重复逻辑。统一走 daemon engine 可以保持行为一致。

## 10. 风险与缓解

### 10.1 真实 provider 输出不稳定

风险：Claude Code/Codex 可能不稳定输出 sentinel JSON。

缓解：

- 强化 prompt template 的输出约束。
- 保留 raw stdout/stderr refs。
- 对 structured output parse error 给出明确 provider error route。

### 10.2 `naruto` 测试命令不明确

风险：Aria 不知道目标项目应运行哪些验证命令。

缓解：

- planning 或 execution setup 阶段必须产出 acceptance targets。
- testing report 必须记录命令和工作目录。
- 如果无法确定测试命令，任务失败并报告 `testing_command_unresolved`。

### 10.3 gate 导致无人值守 E2E 卡住

风险：端到端测试中途等待人工确认。

缓解：

- 阶段 A 默认 `--non-interactive`。
- hard gate 写为 `blocked_by_gate` 并生成 blocked report。
- 后续 REPL/daemon gate manager 支持恢复。

### 10.4 同步 runner 与 daemon engine 重复

风险：阶段 A 的 runner 变成临时分叉实现。

缓解：

- 阶段 A 的 `TaskRunOrchestrator` 不依赖 CLI 输出。
- 使用接近 daemon 的 task state 和 artifact store。
- 阶段 C 直接迁移 orchestrator 边界，而不是重写业务流程。

## 11. 结论

本设计确认采用 A -> C -> REPL 的路线：

- 先通过 `aria task run` 跑通真实端到端 JWT 登录用例。
- 再将同步 runner 下沉为 daemon workflow engine。
- 最后基于 daemon engine 构建 Claude Code/Codex 风格自然语言 REPL。

这条路线优先解决当前端到端调试诉求，同时为长期产品化架构保留清晰迁移路径。
