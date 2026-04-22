# Claude Code / Codex spawn 使用方式与限制调研

> 日期：2026-04-22
> 文档类型：分析报告
> 版本：v1.1
> 调研目标：系统梳理 Claude Code 与 Codex 的 spawn 使用方式、适用边界、工程落地模式，并确认哪些场景仅靠 spawn 仍然无法优雅解决
> 结论状态：已结合官方文档、官方仓库、本地 CLI 实测、Vibe Kanban 公开资料交叉校验

---

## 一、相对 v1.0 的重点增补

本版相对 v1.0 主要补了 6 类信息：

1. 明确把 spawn 拆成 4 层，而不是把 CLI 拉起、子 agent、会话分叉混成一个概念。
2. 把 Claude Code 的 `subagents`、`agent teams`、Agent SDK 分开写，避免把稳定能力和实验能力混在一起。
3. 明确 Codex TypeScript SDK 的本质：**它是对 `codex` CLI 的封装，底层仍然会 spawn CLI 并通过 JSONL 通信**。
4. 用本地 `codex app-server generate-ts / generate-json-schema` 生成的版本匹配协议文件，确认 `thread/start`、`thread/resume`、`thread/fork`、`turn/steer`、`turn/interrupt`、`collabAgentToolCall` 等能力确实公开存在。
5. 把 “对话分叉” 和 “文件系统分叉” 明确拆开，避免误以为 `fork` 等于 worktree 或代码状态隔离。
6. 单独回答“spawn 无法解决的场景”，并给出为什么不能只靠 spawn 的具体原因。

---

## 二、先给最终结论

如果只看一句话，结论是：

1. **spawn 不是一个单一能力，而是一组能力面**：
   - 宿主进程 spawn CLI
   - 产品内部 spawn 子 agent
   - 在已有会话上 resume / fork
   - 用 SDK / server 把前面这些能力程序化
2. **Claude Code 更强在“主控与自动委派”**：
   - `claude -p` 很适合脚本、CI、一次性执行
   - `subagents` 更成熟，支持自动委派、显式委派、`--agent`、`--agents`
   - Agent SDK 的 `continue / resume / fork` 做得更完整
   - 但 `agent teams` 仍是实验能力，不能当成零风险稳定底座
3. **Codex 更强在“执行节点与事件化 runtime”**：
   - `codex exec`、`codex resume`、`codex fork`、`codex exec resume` 已可用
   - `subagents`、自定义 agent、`app-server`、`mcp-server` 已形成完整链路
   - `app-server` 的 thread/turn/item 粒度比裸 CLI spawn 更适合做正式集成
   - 但 `codex exec fork` 仍然缺失，这会直接影响无 UI 的自动化分支执行
4. **Vibe Kanban 的价值是真正的“宿主层编排经验”**，不是“产品能力上限证明”：
   - 它证明了 worktree、进程治理、review 回路、Agent 抽象这些宿主层设计很重要
   - 它不能证明某个产品只有 CLI spawn 一条路
5. **spawn 不能覆盖所有场景**：
   - 它能解决“拉起一个 agent 去干活”
   - 但解决不了“长期多会话、多 agent、可中途打断、可追踪、可审批、可隔离、可扩展”的完整 runtime 问题

---

## 三、证据来源与可信度分层

本报告按下面优先级判断事实：

1. **官方文档 / 官方仓库**
   - Anthropic：`code.claude.com`、`claude.com/blog`
   - OpenAI：`developers.openai.com`、`github.com/openai/codex`
2. **本地 CLI 实测**
   - `claude --help`、`claude agents --help`
   - `codex --help`、`codex exec --help`、`codex exec resume --help`
   - `codex resume --help`、`codex fork --help`
   - `codex app-server --help`、`codex mcp-server --help`
3. **本地生成协议文件**
   - `codex app-server generate-ts --out /tmp/codex-app-server-ts`
   - `codex app-server generate-json-schema --out /tmp/codex-app-server-schema`
4. **Vibe Kanban 官方文档 / 官方仓库**
   - 用来佐证宿主层最佳实践
5. **官方 issue**
   - 用来识别能力边界、已知缺口、工程风险
   - 不把 issue 当成最终规格书，但它对“哪里会踩坑”很有价值

### 3.1 本地版本基线

- Claude Code：`2.1.117`
- Codex CLI：`0.122.0`

### 3.2 一个重要原则

本报告里凡是涉及 “今天 / 当前 / 最新” 的结论，都以 **2026-04-22** 这一天的官方网页与本地 CLI 实测为准。

---

## 四、先统一术语：这里到底在研究哪种 spawn

如果不先分层，后面一定会把完全不同的问题混到一起。

### 4.1 宿主进程 spawn

指你的程序直接拉起 CLI：

```text
Node / Rust / Python
  -> spawn("claude", [...])
  -> spawn("codex", [...])
```

这是最传统、最 Unix 的集成方式。

优点：

- 通用
- 轻量
- 容易先跑通

缺点：

- 进程治理自己做
- stdout/stderr 协议自己解析
- 审批阻塞自己兜底
- 多会话、多 agent 生命周期自己维护

### 4.2 产品内部 spawn 子 agent

指不是你多开 CLI，而是产品自己在内部拉起代理：

- Claude Code：`subagents`、`agent teams`
- Codex：`subagents`、`spawn_agent` 相关协作能力

这类能力的关键不是多了个进程，而是：

- 上下文边界更清晰
- 结果回传路径更清楚
- 生命周期、权限、工具集更容易统一

### 4.3 会话 resume / fork

这不是“新开一个完全空白的 agent”，而是：

- 继续已有上下文：`resume`
- 在已有上下文上开新分支：`fork`

这类能力经常被误判成普通 spawn，但它本质上是：

- **对话历史分叉**
- 不是文件系统快照
- 不是 worktree
- 不是代码状态自动隔离

### 4.4 SDK / server 封装后的 spawn

当你使用 SDK 或 server 时，外表上不再是手写 `child_process.spawn()`，但底层常常仍然与 spawn 有关系。

这里要特别区分两种情况：

1. **高层封装但底层仍然依赖 CLI**
   - 典型例子：Codex TypeScript SDK
2. **直接暴露长期连接 / 事件协议**
   - 典型例子：Codex app-server

---

## 五、Vibe Kanban 应该如何参考

用户提到可以参考 Vibe Kanban，这个方向是对的，但要明确它能说明什么，不能说明什么。

### 5.1 它真正证明了什么

Vibe Kanban 官网和仓库都在强调三件事：

1. **每个任务都应该跑在隔离的 git worktree**
2. **宿主系统要负责 review、反馈回流、分支管理、dev server**
3. **宿主不应该把“AI coding agent”当成一个黑盒文本接口**

Vibe Kanban 文档把它自己定义成一个 orchestration platform，并强调：

- 计划与 review 是人类瓶颈
- Agent 运行要有工作区隔离
- 每个 workspace 应该有 branch、terminal、dev server

### 5.2 它不能证明什么

Vibe Kanban 不能用来证明：

1. Codex 只能被 CLI spawn
2. Claude Code 只有 CLI 入口
3. 产品内部没有 subagent / thread / server 能力

原因很简单：

- 它是一个宿主层产品
- 它优先选的是“最兼容多个 agent 的统一接入方式”
- “统一入口选 CLI” 不代表产品原生能力就只有 CLI

### 5.3 对 Aria 真正该继承的部分

Vibe Kanban 最值得 Aria 继承的是：

1. worktree 作为并行默认隔离单元
2. 统一 executor 抽象，而不是在业务层满地分支判断
3. review 与反馈闭环
4. 进程组治理
5. 宿主侧状态机，而不是只看一行最终输出

一句话总结：

**Vibe Kanban 是宿主层最佳实践参考，不是 Claude/Codex 产品能力边界的证明。**

---

## 六、Claude Code 的 spawn 使用方式

## 6.1 宿主侧最轻入口：`claude -p`

Claude Code 的 CLI 仍然是最轻入口，本地 `claude --help` 明确能看到：

- `-p / --print`
- `--output-format`
- `--resume`
- `--fork-session`
- `--agent`
- `--agents`
- `--worktree`
- `--permission-mode`

这意味着宿主层最基础的模式仍然是：

```bash
claude -p "任务描述"
```

比较关键的参数是：

| 能力 | 参数 |
|------|------|
| 非交互执行 | `-p` |
| 结构化输出 | `--output-format json` / `stream-json` |
| 恢复会话 | `--resume` |
| 从原会话分叉 | `--fork-session` |
| 指定主 agent | `--agent` |
| 动态注入 subagent | `--agents <json>` |
| 工作区隔离 | `--worktree` |
| 权限模式 | `--permission-mode` |

### 适用场景

- CI
- 脚本
- 单次分析
- “跑完就退出”的自动化任务

### 不足

- 需要宿主自己处理 stdout 解析
- 需要宿主自己处理进程回收
- 不适合长连接式 UI 控制

## 6.2 Claude Code 的会话控制：continue / resume / fork

Claude 的会话控制能力比很多旧资料里写得更完整。

官方会话文档和本地 CLI 一起可以确认：

1. CLI 支持 `--resume`
2. CLI 支持 `--fork-session`
3. SDK 支持 `continue / resume / fork`

Claude 官方会话文档强调了一个很重要的点：

- **session 持久化的是对话历史**
- **不是文件系统状态**

这意味着：

- 你可以分叉“对话上下文”
- 但你不能把它当成“自动生成一个隔离代码分支”
- 真正的代码隔离仍然要靠 worktree、checkpoint、容器或其它外部机制

## 6.3 Claude Code 的 subagents

Claude 的 subagents 是现在最成熟的内部 spawn 能力之一。

从官方文档可以确认 3 件核心事实：

1. Claude 会根据任务描述、subagent 的 `description`、当前上下文做自动委派
2. 也可以显式点名 subagent
3. 每个 subagent 都有自己的上下文窗口、工具限制、权限边界

### 6.3.1 触发方式

Claude 的 subagent 至少有 4 种用法：

1. 自然语言提及
2. `@` mention 明确指定某个 subagent
3. `--agent <name>` 让整个主线程直接运行在该 agent 身份下
4. `--agents <json>` 动态把自定义 subagent 注入当前会话

这点和 Codex 很不一样：

- Claude 更偏“主线程主动委派”
- Codex 更偏“你明确要求了它才会做”

### 6.3.2 前台与后台

Claude 文档明确区分了 foreground / background subagents：

- 前台 subagent：阻塞主线程，澄清问题和权限请求能直接传给用户
- 后台 subagent：并发执行，但需要更早做好授权；不适合高度交互

### 6.3.3 配置方式

Claude 的 subagent 主要有几种来源：

- `.claude/agents/`
- `~/.claude/agents/`
- `--agents` CLI flag
- 插件提供的 agents

文件格式是 Markdown + YAML frontmatter。

### 6.3.4 Claude subagents 的硬边界

这部分特别重要。

官方文档明确说：

- **Subagents cannot spawn other subagents**

这意味着：

1. Claude 的 subagent 体系更适合“一层委派”
2. 不适合自然生长成深层递归 agent tree
3. 如果你要做多层协作，要么主线程负责继续编排，要么改用 agent teams / 外部 orchestrator

## 6.4 Claude Code 的 agent teams

这是 v1.1 新增强调的部分。

官方 `agent teams` 文档明确写了：

- 这是 **experimental**
- 默认关闭
- 需要 `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`
- 存在已知的 **session resumption、task coordination、shutdown behavior** 限制

### 它和 subagents 的本质差别

| 能力 | Subagents | Agent teams |
|------|-----------|-------------|
| 关系 | 主线程派生 worker | lead + teammates |
| 上下文 | 各自独立 | 各自独立 |
| 互相通信 | 只回主线程 | 队友可直接通信 |
| 成熟度 | 更稳定 | 实验能力 |
| 适合 | focused worker | 复杂协作 |

### 什么时候才该用 agent teams

Claude 官方建议它更适合：

- research / review
- 多个相互独立的探索角度
- debugging with competing hypotheses
- 跨层协同但任务边界清楚的场景

不适合：

- 顺序依赖很强的任务
- 同文件并发写
- 你需要强稳定性、强恢复性、强可预测 shutdown 的生产底座

### 对 spawn 研究的意义

Claude 并不只有 subagent 这一种内部 spawn：

- 稳定的一层委派：subagents
- 实验性的多会话协作：agent teams

但后者还不应该被当成“无风险生产底座”。

## 6.5 Claude Agent SDK

Claude 官方现在把原 Claude Code SDK 统一表述成 Claude Agent SDK。

公开文档能确认：

1. 提供 Python / TypeScript 两套主 SDK
2. 可以 `query()` 一次性执行
3. 支持多轮会话
4. 支持 `continue / resume / fork`
5. 会自动加载 `.claude/` 目录里的 skills、memory、commands 等配置

官方 sessions 文档对使用方式给得很明确：

- 单次任务：一个 `query()` 就够
- 同进程多轮：TypeScript 用 `continue: true`，Python 用 `ClaudeSDKClient`
- 恢复指定会话：用 `resume`
- 试另一条路线：用 `fork`

### 对 spawn 的本质影响

这不是说 Claude “不再 spawn”。

更准确的说法是：

- 你不需要手写 `child_process.spawn("claude")`
- 但你拿到的是一个更高层的 agent runtime API
- 它把 session、tool、permissions、fork 这些问题一起封装了

### 什么时候该从 `claude -p` 升级到 Agent SDK

满足任一条件，就应该认真考虑 SDK：

1. 不是一次性任务，而是多轮会话
2. 需要在服务端长期保存 session ID
3. 需要 fork 会话而不是简单继续
4. 需要程序化控制 tool / permissions / settings sources
5. 需要把 `.claude` 配置一起纳入运行时

## 6.6 Claude Code 的定位总结

Claude 更像：

- **orchestrator**
- **主动委派者**
- **多轮上下文控制者**

它的强项不是无限深的递归 agent tree，而是：

- 一层 subagent 委派
- 明确的会话 resume / fork
- 对主线程规划和协作的支撑

---

## 七、Codex 的 spawn 使用方式

## 7.1 宿主侧最轻入口：`codex exec`

截至 2026-04-22，本地 `codex --help` 与 `codex exec --help` 可以确认：

- `codex exec`
- `codex review`
- `codex mcp-server`
- `codex app-server`
- `codex resume`
- `codex fork`

非交互最核心的是：

```bash
codex exec "任务描述"
```

关键参数包括：

| 能力 | 参数 |
|------|------|
| 非交互执行 | `codex exec` |
| JSONL 事件流 | `--json` |
| 临时会话 | `--ephemeral` |
| 结构化最终输出 | `--output-schema` |
| 工作目录 | `-C` / `--cd` |
| 附加可写目录 | `--add-dir` |
| 跳过 Git 检查 | `--skip-git-repo-check` |
| 沙箱模式 | `--sandbox` |
| 低摩擦自动执行 | `--full-auto` |

### `codex exec` 的角色定位

它非常适合：

- CI
- 一次性自动化任务
- 批处理
- 将结构化结果传回宿主系统

它不适合：

- 长连接式多轮控制
- 需要中途插话 / steer / interrupt
- 需要无 UI 的分叉执行

## 7.2 Codex 的 resume / fork：交互式与非交互式要分开看

这是本次调研里最容易被旧资料误导的地方。

### 7.2.1 交互式

本地 CLI 已明确支持：

```bash
codex resume
codex fork
```

并且：

- `codex resume --last`
- `codex fork --last`
- `--all`
- `SESSION_ID`

都是可用的。

### 7.2.2 非交互式

本地 CLI 已明确支持：

```bash
codex exec resume --last "继续处理"
codex exec resume <SESSION_ID> "继续处理"
```

这说明：

- Codex 已经不是“每次 exec 都必须从空白上下文开始”
- 至少 **resume** 在非交互入口已经成立

### 7.2.3 当前明确存在的缺口

截至 2026-04-22，本地 CLI 没有：

```bash
codex exec fork
```

这意味着：

- 交互式能 fork
- app-server 能 `thread/fork`
- 但最轻量的非交互入口 `codex exec` 还不能直接 fork

这不是语义小差别，而是会直接影响：

- 自动化分叉
- A/B 方案并行尝试
- 同一上下文下派生多条无人值守支线

## 7.3 Codex 的 subagents 与自定义 agents

Codex 官方 subagents 文档给了两个很关键的事实：

1. **Codex only spawns subagents when you explicitly ask it to**
2. 官方建议从 **read-heavy tasks** 开始，对 **parallel write-heavy workflows** 保持谨慎

### 7.3.1 这和 Claude 的最大差别

| 维度 | Claude Code | Codex |
|------|-------------|-------|
| 默认自动委派 | 较强 | 默认不主动 |
| 用户显式要求后委派 | 有 | 有 |
| 官方语气 | 偏鼓励主动 delegation | 偏要求显式触发 |

所以，如果你想做“总控 agent 自动识别何时该叫专家”：

- Claude 更自然
- Codex 更适合作为被明确调度的执行器

### 7.3.2 配置方式

Codex 的自定义 agent 主要走 TOML：

- `~/.codex/agents/*.toml`
- `.codex/agents/*.toml`
- `agents.<name>.config_file`

官方 config reference 里可以确认这些 agent 相关键：

- `agents.<name>.config_file`
- `agents.<name>.description`
- `agents.<name>.nickname_candidates`
- `agents.max_threads`
- `agents.max_depth`

这意味着 Codex 官方其实已经把“多 agent 深度”和“并发线程数”显式视为一等配置项。

### 7.3.3 当前运行时能看到的内建角色

在当前 Codex 工具面中，常见会暴露：

- `default`
- `worker`
- `explorer`

这里需要特别说明：

- 这是 **当前运行时 / 工具面** 的事实
- 不等价于公开文档一定完整列出了所有内部角色
- 所以在正式报告中更安全的说法是：**当前 Codex 运行时已经具备默认、执行、探索等角色化分工能力**

### 7.3.4 生命周期边界

Codex 的多 agent 不是 “spawn 完就结束”。

从官方 app-server 协议生成的 TS 类型可以确认，thread item 中已经有：

- `collabAgentToolCall`
- 工具名枚举：`spawnAgent | sendInput | resumeAgent | wait | closeAgent`

这说明 Codex 官方协议面已经明确建模了：

- 启动
- 继续输入
- 恢复
- 等待
- 关闭

也正因为如此，生命周期管理就不是可选项。

### 7.3.5 已知工程风险

OpenAI 官方仓库公开 issue `#18335`（2026-04-17）指出：

- 在 app-server / interactive CLI 这类持久会话里
- 如果 agent 到终态后没有显式 `close_agent`
- 可能出现 spawn slot 泄漏，后续 `spawn_agent` 被卡住

这不等于 Codex 不能做多 agent。

它真正说明的是：

- **持久会话里必须认真做 wait / close 收尾**
- “spawn 完就不管” 不是稳定工程模式

## 7.4 Codex SDK

这一块需要比 v1.0 说得更精确。

### 7.4.1 已确认的事实

OpenAI 官方仓库 `sdk/typescript/README.md` 可以直接确认：

1. 包名是 `@openai/codex-sdk`
2. `npm install @openai/codex-sdk`
3. TypeScript SDK **wraps the `codex` CLI**
4. SDK 与 CLI 之间通过 **stdin/stdout 的 JSONL 事件** 交换数据
5. 公开 README 中明确展示了：
   - `startThread()`
   - `thread.run()`
   - `thread.runStreamed()`
   - `resumeThread()`

### 7.4.2 这意味着什么

这意味着两件事要同时成立：

1. **Codex 确实已经有官方 SDK**
2. **但这个 SDK 在 TypeScript 侧并不是一个完全脱离 CLI 的新 runtime，而是对 CLI spawn 的高层封装**

所以，如果问题是“有没有官方程序化入口”：

- 有

如果问题是“用了 SDK 之后是不是就和 spawn 没关系了”：

- 不是

### 7.4.3 当前公开资料里能确认到什么边界

截至 2026-04-22，我能从公开 README 直接确认：

- `startThread`
- `run`
- `runStreamed`
- `resumeThread`

我**没有**在公开 README 中看到与 CLI `fork` 同级的 `forkThread` 文档。

公开 issue `#4972` 也曾明确指出 TS SDK 暴露的是：

- `startThread`
- `resumeThread`
- `run`
- `runStreamed`

因此更稳妥的工程判断是：

- **如果你要程序化 fork，当前更可靠的官方接口仍然是 app-server 的 `thread/fork`，而不是假定 SDK 已公开同级 API**

这条判断基于当前公开资料，不排除后续版本变化。

## 7.5 Codex app-server

这是 Codex 与 Claude 当前差异最大的地方之一。

官方 app-server 文档和官方仓库 README 已经把它定义得很清楚：

- 使用 JSON-RPC 2.0 语义
- 支持 `stdio://`、`ws://IP:PORT`、`off`
- websocket 仍是 experimental / unsupported

### 7.5.1 核心协议能力

官方文档和本地生成 schema 一起可以确认这些方法存在：

- `thread/start`
- `thread/resume`
- `thread/fork`
- `thread/read`
- `thread/list`
- `turn/start`
- `turn/steer`
- `turn/interrupt`
- `review/start`
- `mcpServer/tool/call`
- `fs/*`
- `command/exec`

### 7.5.2 对 spawn 研究最关键的价值

app-server 把很多在裸 CLI spawn 里很难优雅做的事情变成了一等公民：

1. thread 生命周期
2. turn 生命周期
3. item 级事件流
4. 显式 `fork`
5. 中途 steer / interrupt
6. 与子 agent 协作调用相关的事件观测

### 7.5.3 本地生成协议文件确认到的事实

2026-04-22 本地生成的 TS 协议中可以直接确认：

- `ClientRequest` 里有 `thread/resume`、`thread/fork`、`turn/interrupt`
- `ThreadItem` 里有 `collabAgentToolCall`
- `CollabAgentTool` 枚举值包括：
  - `spawnAgent`
  - `sendInput`
  - `resumeAgent`
  - `wait`
  - `closeAgent`

这非常重要，因为它说明：

- Codex 的多 agent 协作不是“不可见黑箱”
- 它已经在正式协议层被显式建模

### 7.5.4 什么时候必须上 app-server

满足任一条件，就不应继续把 `codex exec` 当成最终架构：

1. 需要长期持有多个 thread
2. 需要 thread 级 fork
3. 需要 turn 级 steer / interrupt
4. 需要 item 级事件流
5. 需要把 Codex 做成 UI 后端或正式 runtime

## 7.6 Codex mcp-server

本地 `codex mcp-server --help` 已明确表明：

```bash
codex mcp-server
```

它的意义不是再开一个 CLI，而是：

- 让 Codex 作为一个 stdio MCP server 被上层系统调用

适合：

1. 让 Claude / 其它 agent 把 Codex 当作一个工具节点
2. 把 Codex 纳入统一 MCP 工具生态
3. 做多模型、多代理系统里的“专用编码执行器”

## 7.7 Codex 的定位总结

Codex 更像：

- **executor**
- **thread runtime**
- **事件化后端**

它比 Claude 更适合做：

- 长驻 thread
- 协议化集成
- 明确的 turn / item 级观测

但它在默认自动委派体验上，不如 Claude 那么“主线程主动”。

---

## 八、Claude Code 与 Codex：spawn 维度逐项对比

| 维度 | Claude Code | Codex |
|------|-------------|-------|
| 宿主最轻入口 | `claude -p` | `codex exec` |
| 结构化流式输出 | `--output-format stream-json` | `--json` |
| 交互式 resume | 有 | 有 |
| 交互式 fork | 有 | 有 |
| 非交互式 resume | 有 | 有 |
| 非交互式 fork | 有，`--fork-session` | **无 `codex exec fork`** |
| 自动委派 | 强 | 默认不主动 |
| 显式 subagent | 有 | 有 |
| subagent 嵌套 | **不支持** | 可做更深协作，但受 `max_depth` 约束 |
| 团队式多会话协作 | `agent teams`，实验中 | 更偏 `app-server` / 协议编排 |
| 自定义 agent 配置 | Markdown + frontmatter | TOML + config |
| SDK 本质 | 高层 agent harness 接口 | TS SDK 公开确认会 wrap CLI |
| 更底层 server 面 | 主要是 SDK / CLI | `app-server` 很强 |
| conversation fork 与 filesystem fork 是否等价 | 否 | 否 |

一个很关键的结论是：

**Claude 与 Codex 并不是两个完全同构的执行器。**

- Claude 更适合做总控、规划、自动委派
- Codex 更适合做执行节点、thread backend、事件流 runtime

---

## 九、spawn 无法单独解决，或无法优雅解决的场景

这是用户最关心的问题之一，直接给结论：

**有，而且不少。**

下面按场景拆开。

## 9.1 想要“对话分叉”和“代码状态分叉”同时成立

仅靠 spawn 不够。

原因：

- Claude 的 `resume / fork` 分的是对话
- Codex 的 `resume / fork / thread/fork` 分的也是 thread 历史
- 它们都不是工作区快照

如果你要：

- 一边保留原路线
- 一边尝试方案 A / B
- 并且两个方案改动不能互相污染

你需要的不是只有 fork，而是：

1. worktree
2. checkpoint / snapshot
3. branch / merge 策略

## 9.2 需要长连接、可中途插话、可打断、可重定向

仅靠裸 CLI spawn 通常不优雅。

原因：

- 裸 spawn 更像一次性任务
- 你很难在“半路上”优雅地 steer 一个已经在跑的任务

更合适的能力面：

- Claude：Agent SDK
- Codex：app-server 的 `turn/steer`、`turn/interrupt`

## 9.3 需要稳定的多 agent 生命周期治理

spawn 本身只解决“拉起”。

它不天然解决：

- 何时等待
- 何时回收
- 何时继续给子 agent 发消息
- 何时 close

在 Codex 持久会话里，这个问题更直接，因为官方公开 issue 已经表明：

- 不显式 close，可能出现 slot 泄漏

在 Claude 里，这个问题会表现为：

- subagent 是一层式模型
- teams 又是实验能力

所以真正的难点不是 spawn，而是 **lifecycle**。

## 9.4 需要大量并发写代码且避免冲突

spawn 不能天然解决：

- 同文件并发写
- Git index 冲突
- worktree 合并冲突
- 互相覆盖修改

官方文档其实都在暗示这一点：

- Codex 明确提醒并行 write-heavy workflow 要谨慎
- Claude agent teams 明确说 sequential tasks / same-file edits 不适合

所以并发写的核心能力不在 spawn，而在：

1. 任务切片
2. worktree 隔离
3. merge 策略
4. 人工 review

## 9.5 需要可靠审批、审计和可解释的安全边界

spawn 只会把问题暴露出来，不会帮你解决审批体系。

典型失败模式：

1. agent 申请高权限操作
2. 宿主系统没有审批 UI 或审批回调
3. 进程卡死或报错退出

真正需要的是：

1. 权限策略
2. 审批流
3. 审计日志
4. 最小权限凭证注入

## 9.6 需要任务在本地终端关闭后继续跑

本地 spawn 解决不了 durable execution。

它天然依赖：

- 本机还开着
- 当前 runtime 还活着
- 当前进程还在

如果你要：

- 电脑关了还继续
- 和 GitHub / Slack / 定时任务联动
- 失败自动重试
- 多机调度

那么你需要的已经不是普通 spawn，而是：

- server
- queue
- scheduler
- cloud / background runtime

## 9.7 需要一等公民级 observability

spawn 最容易给你的只有两样东西：

- stdout
- stderr

但真正的 agent 系统会需要：

- thread 状态
- turn 状态
- item 状态
- 子 agent 状态
- 工具调用轨迹
- token usage
- diff 更新

这正是为什么 Codex app-server 比裸 `codex exec` 更适合正式集成。

## 9.8 需要多租户、多用户、长期会话服务

一旦你做的是服务端产品，而不是个人脚本，spawn 就会立刻碰到这些问题：

1. session ID 怎么持久化
2. 哪个用户对应哪个会话
3. 会话如何恢复
4. 用户 A 的凭证怎么不泄露给用户 B
5. 哪个 agent 可以访问哪个仓库

这时“能不能 spawn”已经不是主问题，真正的问题是：

- session store
- auth
- isolation
- governance

## 9.9 需要大规模并发和背压

spawn 在小规模很好用，在大规模时问题会迅速放大：

- 进程数爆炸
- 句柄数爆炸
- 缓冲区爆炸
- CPU 调度失控
- OOM 风险

此时更稳的方案通常是：

1. 固定 worker 池
2. 队列
3. 有状态 runtime
4. 资源配额

## 9.10 需要跨平台一致性

spawn 的平台差异会带来很多非产品问题：

- 信号处理差异
- 路径差异
- shell 差异
- worktree 行为差异
- 权限模型差异

所以如果 Aria 要跨 macOS / Linux / Windows 跑，必须把：

- process management
- path normalization
- sandbox / approval abstraction

放到宿主层统一处理。

---

## 十、对 Aria 的落地建议

## 10.1 一期怎么做最稳

一期建议：

1. 把 CLI spawn 当成**基础执行原语**
2. 不要把 CLI spawn 当成**最终 runtime 架构**
3. 默认使用 worktree 做隔离
4. 明确持久化 session / thread ID
5. 明确区分：
   - 对话分叉
   - 文件系统隔离

### 推荐组合

```text
Claude Code
  -> 负责规划、调研、拆任务、主动委派

Codex
  -> 负责执行、review、长驻 thread、事件化后端
```

也就是：

- Claude 更像 orchestrator
- Codex 更像 executor

## 10.2 什么时候该升级

### 从 Claude CLI 升级到 Claude Agent SDK

满足任一条件就应升级：

1. 多轮会话
2. 需要 fork / resume
3. 需要程序化控制工具和权限
4. 需要把 `.claude` 配置纳入正式运行时

### 从 `codex exec` 升级到 Codex SDK / app-server

满足任一条件就应升级：

1. 需要持久 thread
2. 需要 `thread/fork`
3. 需要 turn 级 steer / interrupt
4. 需要 item 级事件流
5. 需要把 Codex 嵌入正式 UI 或服务端

### 什么时候该上 `codex mcp-server`

当你希望：

- 让别的 agent 把 Codex 当成工具
- 统一走 MCP 工具栈
- 做多 agent 系统里的“专用编码执行器”

就该考虑它。

## 10.3 Aria 不该犯的 4 个错误

1. 把 Claude 和 Codex 当成两个完全同构的 CLI 工具
2. 把 conversation fork 误当成 code workspace fork
3. 把多 agent 并发误当成“多开几个进程”这么简单
4. 等做到后期才补 worktree、审批、审计、资源治理

---

## 十一、直接回答用户提出的三个要求

### 11.1 “要尽可能详细”

本版已经把 Claude 和 Codex 的 spawn 使用方式拆成：

1. 宿主 CLI spawn
2. 内部 subagent / team spawn
3. resume / fork
4. SDK / server 封装

这比只谈 “怎么 `spawn("claude")` / `spawn("codex")`” 更接近真实工程问题。

### 11.2 “要尽可能全面”

本版已经把下面几类面都纳入了：

- CLI
- subagents
- teams
- SDK
- app-server
- MCP server
- Vibe Kanban 宿主层经验
- 已知公开风险
- 安全与治理边界

### 11.3 “确认是否有 spawn 无法解决的场景”

结论是：

**有，而且这些场景不是边角问题，而是中大型系统迟早会碰到的主问题。**

最典型的 6 类是：

1. 对话分叉与代码状态分叉同时成立
2. 长连接、中途插话、打断与重定向
3. 稳定的多 agent 生命周期治理
4. 并发写冲突隔离
5. 审批、审计、最小权限治理
6. 大规模并发与持久执行

---

## 十二、主要依据

### 12.1 Claude Code

- CLI Reference
  - https://code.claude.com/docs/en/cli-reference
- Run Claude Code programmatically
  - https://code.claude.com/docs/en/headless
- Create custom subagents
  - https://code.claude.com/docs/en/sub-agents
- Orchestrate teams of Claude Code sessions
  - https://code.claude.com/docs/en/agent-teams
- Agent SDK overview
  - https://code.claude.com/docs/en/agent-sdk/overview
- Work with sessions
  - https://code.claude.com/docs/en/agent-sdk/sessions
- Session management blog
  - https://claude.com/blog/using-claude-code-session-management-and-1m-context

### 12.2 Codex

- Codex docs home
  - https://developers.openai.com/codex
- Subagents
  - https://developers.openai.com/codex/subagents
- Subagent concepts
  - https://developers.openai.com/codex/concepts/subagents
- App Server
  - https://developers.openai.com/codex/app-server
- Codex SDK page
  - https://developers.openai.com/codex/sdk
- Config reference
  - https://developers.openai.com/codex/config-reference
- OpenAI Codex 仓库 README
  - https://github.com/openai/codex
- Codex TypeScript SDK README
  - https://github.com/openai/codex/blob/main/sdk/typescript/README.md
- Codex app-server README
  - https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md

### 12.3 Vibe Kanban

- 官网文档
  - https://vibekanban.com/docs
- GitHub 仓库
  - https://github.com/BloopAI/vibe-kanban

### 12.4 本地实测与本地生成协议

- `claude --version`
- `claude --help`
- `claude agents --help`
- `codex --version`
- `codex --help`
- `codex exec --help`
- `codex exec resume --help`
- `codex resume --help`
- `codex fork --help`
- `codex app-server --help`
- `codex mcp-server --help`
- `codex app-server generate-ts --out /tmp/codex-app-server-ts`
- `codex app-server generate-json-schema --out /tmp/codex-app-server-schema`

### 12.5 公开 issue

- Codex SDK fork/backtrack 能力讨论
  - https://github.com/openai/codex/issues/4972
- Codex 持久会话里 spawn slot 泄漏
  - https://github.com/openai/codex/issues/18335

---

## 十三、最终一句话结论

如果 Aria 现在就要落地：

**一期用 CLI spawn + worktree 跑通，Claude 做总控，Codex 做执行；一旦进入多轮、分叉、事件流、长期协作，就分别升级到 Claude Agent SDK 与 Codex app-server / SDK。**

