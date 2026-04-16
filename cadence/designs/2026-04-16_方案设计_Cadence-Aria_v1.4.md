# Cadence-Aria 方案设计

> **版本**：v1.4.2
> **更新日期**：2026-04-16
> **更新说明**：v1.4.1 基础上，根据更系统的 superpowers skills 调研结果修正假设1结论：superpowers  skills 可被自动调用，只是需区分"自动执行型"（writing-plans/executing-plans/requesting-code-review 等）与"多轮交互型"（brainstorming 等）。将 orchestrator 恢复为"全自动编排器"定位；capability_mapping 恢复为"自动调用映射"，并增加 `execution_mode` 字段区分调用类型；MVP 阶段在关键状态节点保留用户确认点，但最终目标是全自动编排。

## 概述

`Cadence-Aria` 定位为当前项目的第一个插件，对外以 `Claude Code plugin` 形式发布，对内提供一层正式的 `Aria runtime` 编排层，并通过一个可替换的 `Codex adapter` 驱动 `Codex CLI`，用于编排 `Claude Code -> Codex` 的完整任务流。

一期目标不是替代现有全部开发基础设施，而是建立一个以 `issue` 为最小闭环单位的多角色编排系统：

- `Claude Code` 负责 `intake / spec / plan / dispatch / review / test`（其中自动执行型 superpowers skills 由 Aria 在对应状态自动调用；多轮交互型 skills 如 `brainstorming` 由用户主动调用）
- `Codex` 负责 `exec / patch`
- `OpenSpec` 负责正式任务的边界与工件主线
- `superpowers` 负责规划、并行、验证、调试等方法层能力
- `Vibe Kanban` 只负责 `issue / workspace / worktree / 执行入口 / 状态展示`

一期必须做到：

1. 能从 `Vibe Kanban` 或 `Aria` 原生命令入口统一接收任务
2. 所有正式任务强制进入 `OpenSpec` 主线
3. `Claude Code` 能通过 `Aria runtime` 驱动 `Codex` 并行执行 `exec`
4. `Claude Code` 能在 Aria 的全自动编排下完成 `review` 和 `test`（Aria 自动调用对应 superpowers skills，收集产出并推进状态；MVP 阶段关键节点保留用户确认点）
5. `Codex` 能在 `Aria runtime` 注入的修补上下文中执行 `patch`
6. `Aria` 能输出结构化状态与闭环摘要

## 背景与目标

`Cadence-Aria` 是一个独立的新插件，目标是形成一个长期可扩展的编排方案，方向参考 `oh-my-claudecode` 与 `oh-my-codex` 的团队编排经验，但不把 `Vibe Kanban` 扩展成重型编排器，也不假设 `Codex` 原生具备结构化 contract runtime。

### 设计目标

1. 建立清晰的多角色协作模型
2. 让 `Claude Code` 成为唯一主编排入口
3. 让 `Codex` 成为正式执行端与修补端，但由 `Aria runtime` 承担 contract 解释、上下文注入和结果回写
4. 让 `OpenSpec` 成为正式任务强制主线
5. 让 `superpowers` 成为方法层依赖，而不是内嵌副本
6. 让 `Aria` 能接入 `Vibe Kanban`，也能脱离 `Vibe Kanban` 独立运行
7. 一期完成 `issue` 级闭环，同时预留 `branch / PR` 扩展接口

## 技术栈定义

> **v1.4 新增**：根据设计评审建议，参考 `oh-my-claudecode` 技术选型，明确 Aria 的技术栈。

### 选型原则

1. 与 Claude Code plugin 生态一致（TypeScript 为主）
2. 与参考项目（oh-my-claudecode、codex-plugin-cc）技术栈兼容
3. 满足项目既有约束（pnpm 为前端包管理器）

### 核心技术栈

| 维度 | 选型 | 版本要求 | 选型理由 |
|------|------|---------|---------|
| **语言** | TypeScript（严格模式，ESM） | >= 5.x | Claude Code plugin 生态原生语言；oh-my-claudecode、codex-plugin-cc 均使用 TypeScript |
| **运行时** | Node.js | >= 20.0.0 | Claude Code plugin 系统要求；ESM 模块支持 |
| **模块系统** | ESM（`"type": "module"`） | - | 现代 Node.js 标准；oh-my-claudecode 同样采用 ESM |
| **包管理** | pnpm | >= 9.x | 项目 CLAUDE.md 已规定前端项目必须使用 pnpm |
| **构建** | esbuild | latest | 极快的构建速度；oh-my-claudecode 同样采用 |
| **测试** | vitest + V8 覆盖率 | latest | 与 pnpm 生态兼容；oh-my-claudecode 同样采用 |
| **类型校验** | zod | latest | 用于 state.yaml、contract 的运行时类型校验；schema 即文档 |
| **YAML 解析** | yaml | latest | 比 js-yaml 更好的 ESM 支持和错误信息 |
| **Git 操作** | simple-git | latest | 用于 diff、worktree、commit 操作的成熟封装 |
| **文件锁** | proper-lockfile | latest | 并发状态文件写入安全 |
| **日志** | pino | latest | 结构化日志；性能优异；JSON 输出便于后续分析 |

### 不采用的方案

| 维度 | 不采用 | 理由 |
|------|--------|------|
| 状态持久化 | better-sqlite3（oh-my-claudecode 使用） | 一期任务量不需要 SQLite 的查询能力；YAML 人类可读、调试方便、与工件理念一致 |
| Agent SDK | @anthropic-ai/claude-agent-sdk | Aria 不创建子 Agent，只编排现有工具和外部进程 |
| MCP SDK | @modelcontextprotocol/sdk | 一期 Aria 不暴露 MCP 工具服务器 |
| AST 分析 | @ast-grep/napi | 一期文件范围校验用 Git diff 即可，不需要 AST 级分析 |
| CLI 框架 | commander | Aria 通过 Claude Code 的 skill/command 系统注册命令，不需要独立 CLI 框架 |

### tsconfig 建议

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "outDir": "dist",
    "rootDir": "src",
    "declaration": true,
    "sourceMap": true
  }
}
```

### 项目结构（更新版）

````text
cadence-aria/
├── src/                          # TypeScript 源代码
│   ├── commands/                 # CLI 命令入口（薄层，注册到 Claude Code skill/command 系统）
│   ├── runtime/
│   │   ├── orchestrator/         # 流程编排（精简版，只做调度协调）
│   │   ├── state-machine/        # 纯状态机 + 守卫条件
│   │   ├── scheduler/            # 执行单元调度 + 并行管理 + 依赖解析
│   │   ├── arbitrator/           # review/test 仲裁 + patch contract 生成
│   │   ├── contracts/            # contract 生成与解析
│   │   ├── reports/              # 报告生成
│   │   └── persistence/          # 状态持久化 + 文件锁
│   ├── adapters/
│   │   ├── codex/                # Codex CLI 适配器
│   │   ├── openspec/             # OpenSpec 适配器
│   │   ├── superpowers/          # superpowers 适配器（能力抽象层）
│   │   ├── vk/                   # Vibe Kanban 适配器
│   │   └── host/                 # 宿主能力适配器（Bash、文件系统、Git）
│   ├── schemas/                  # Zod schema 定义（state、contract、report 等）
│   ├── config/                   # 配置加载与三级优先级合并
│   ├── diagnostics/              # 能力探测 + 错误码 + aria:doctor
│   └── utils/                    # 工具函数（路径、时间、ID 生成等）
├── skills/                       # Aria 自有 skill 定义
├── templates/                    # 工件模板（contract、report 等）
├── codex/                        # Codex 侧资源
│   ├── prompts/                  # prompt 模板（exec、patch）
│   └── templates/                # Codex 输出模板
├── tests/                        # 测试文件
│   ├── unit/                     # 单元测试
│   ├── integration/              # 集成测试
│   └── e2e/                      # 端到端测试
├── docs/                         # 开发文档
├── package.json
├── tsconfig.json
├── vitest.config.ts
└── esbuild.config.mjs
````

### 依赖方向规则（补充）

在 Implementation-Layout 已定义的基础上，补充以下规则：

1. `schemas/` 是最底层模块，不依赖任何运行时模块
2. `adapters/` 各子模块互相独立，不互相引用
3. `runtime/state-machine/` 不依赖 `adapters/`（纯逻辑）
4. `runtime/scheduler/` 依赖 `adapters/host/` 和 `adapters/codex/`
5. `runtime/orchestrator/` 是唯一可以引用所有 `runtime/` 子模块的上层编排器

## 外部参考

截至 `2026-04-16`，本方案优先参考以下现成项目的公开仓库与 README：

| 参考项目 | 参考点 | 使用方式 |
|--------|--------|---------|
| `oh-my-claudecode` | teams-first 编排、插件化工作流入口、多 agent 组织方式 | 作为 `Aria` 编排风格与能力边界参考 |
| `oh-my-codex` | Codex 侧 hooks、agent teams、HUD、工作流增强层 | 作为 `Aria` 的 `Codex adapter` 与资源组织参考 |
| `codex-plugin-cc` | `Claude Code -> Codex` 桥接入口、委派/审查命令集 | 作为 Claude 侧触发 Codex 的桥接参考 |

参考链接：

- <https://github.com/Yeachan-Heo/oh-my-claudecode>
- <https://github.com/Yeachan-Heo/oh-my-codex>
- <https://github.com/openai/codex-plugin-cc>

### 差异化定位

`Aria` 不是对上述项目的重复封装，而是面向当前仓库的正式任务闭环编排层。

| 能力 | `oh-my-claudecode` | `oh-my-codex` | `codex-plugin-cc` | `Cadence-Aria` |
|------|--------------------|---------------|-------------------|----------------|
| Claude 侧多 agent 编排 | 强 | 弱 | 否 | 是 |
| Codex 侧工作流增强 | 否 | 强 | 弱 | 通过 `Codex adapter` 吸收必要能力 |
| `OpenSpec` 强制主线 | 否 | 否 | 否 | 是 |
| issue 级正式闭环 | 部分 | 否 | 否 | 是 |
| review/test/patch 仲裁 | 弱 | 否 | 否 | 是 |
| 面向当前仓库的运行时工件 | 否 | 否 | 否 | 是 |

因此，一期的定位不是“再造一个通用多 agent 框架”，而是：

1. 借鉴成熟生态的交互与组织方式
2. 在当前仓库内补齐 `OpenSpec + superpowers + Claude/Codex` 的正式闭环
3. 把 `Codex` 当作执行引擎，而不是假定其天然就是可消费 YAML contract 的原生 runtime

### 不采纳的方向

以下方向不作为一期方案：

1. 把 `Vibe Kanban` 做成主编排器
2. 把 `OpenSpec` 或 `superpowers` 内嵌进 `Aria`
3. 做成 `Claude` 与 `Codex` 双发行物或双主入口
4. 在一期内引入 `merge / release / archive` 等交付后角色

## Plugin 安装与分发方式

> **v1.4 新增**：根据设计评审建议，明确 Aria 的安装与分发方式。

### 发布形态

`Cadence-Aria` 以 **Claude Code Plugin** 形式发布，通过 Claude Code Plugin Marketplace 安装。

### 安装方式

```text
claude /plugin marketplace add cadence-aria
```

也支持本地开发安装：

```text
# 从源码安装（开发模式）
cd cadence-aria && pnpm install && pnpm build
claude /plugin add ./cadence-aria
```

### `.claude-plugin` 元数据

```yaml
# .claude-plugin
name: cadence-aria
version: 0.1.0
description: "多角色任务编排插件，驱动 Claude Code -> Codex 的完整闭环工作流"
entry: dist/index.js
commands:
  - aria:intake
  - aria:start
  - aria:run
  - aria:fast
  - aria:status
  - aria:result
  - aria:cancel
  - aria:retry
  - aria:doctor
skills:
  - aria-orchestration
hooks:
  pre_tool_use: dist/hooks/pre-tool.js
  post_tool_use: dist/hooks/post-tool.js
dependencies:
  openspec: ">=0.1.0"
  superpowers: ">=0.1.0"
```

### 发布产物结构

````text
dist/                    # 编译后的 TypeScript
skills/                  # Skill 定义文件
templates/               # 工件模板
codex/                   # Codex 侧资源（prompts、templates）
docs/                    # 文档
.claude-plugin           # Plugin 元数据
package.json             # 包信息
````

### 版本策略

1. 遵循语义化版本（SemVer）
2. `0.x.y`：一期开发阶段，API 可能不稳定
3. `1.0.0`：正式流完整闭环通过 E2E 测试后发布
4. 兼容性声明在 `.claude-plugin` 的 `dependencies` 中定义推荐版本范围

### 与其他 Plugin 的共存

| 共存 Plugin | 是否冲突 | 说明 |
|------------|:---:|------|
| `oh-my-claudecode` | 否 | Aria 不注册魔法关键词，不冲突 hook 系统 |
| `codex-plugin-cc` | 否 | Aria 的 Codex adapter 独立实现，不使用 codex-plugin-cc 的命令 |
| `OpenSpec` | 否 | Aria 是 OpenSpec 的消费者，不注册同名命令 |
| `superpowers` | 否 | Aria 通过能力抽象层调用，不注册同名 skill |

### 与 codex-plugin-cc 的关系

`codex-plugin-cc` 是 OpenAI 官方提供的 `Claude Code ↔ Codex` 桥接插件，已实现 `/codex:review`、`/codex:delegate` 等命令。Aria 与 `codex-plugin-cc` 的关系需要明确定义。

#### 定位：互补而非替代

`Aria` 与 `codex-plugin-cc` 是**互补关系**，不是替代关系：

| 维度 | `codex-plugin-cc` | `Cadence-Aria` |
|------|-------------------|----------------|
| 定位 | 轻量级桥接工具 | 完整编排系统 |
| 能力 | 单次委派/审查命令 | 多角色编排、状态机、contract、仲裁 |
| 状态管理 | 无持久化 | 文件级状态持久化、跨会话恢复 |
| OpenSpec 集成 | 无 | 强制主线 |
| 并行支持 | 无 | 支持并行执行（验证通过后） |
| 闭环能力 | 单次执行 | issue 级完整闭环 |

#### 共存策略

1. **一期不依赖 `codex-plugin-cc`**：Aria 的 `Codex adapter` 独立实现，不复用 `codex-plugin-cc` 的代码
2. **两者可共存**：用户可同时安装两者，`codex-plugin-cc` 提供快速委派/审查能力，`Aria` 提供正式任务编排能力
3. **不冲突**：`codex-plugin-cc` 的 `/codex:delegate` 不经过 Aria 的状态机，不影响 Aria 管理的任务
4. **后续评估复用**：如果 `codex-plugin-cc` 的桥接能力足够成熟，二期可考虑基于它构建 Aria 的 `Codex adapter`，减少重复实现

#### 职责边界

```text
codex-plugin-cc:
  - 单次代码审查（/codex:review）
  - 单次任务委派（/codex:delegate）
  - 适用于不进入 Aria 编排流程的快速操作

Cadence-Aria:
  - issue 级多角色编排
  - 状态机驱动的正式闭环
  - contract + 仲裁 + patch 循环
  - 适用于需要完整质量保障的正式任务
```

## 总体架构

`Cadence-Aria` 采用四层结构：

1. `Vibe Kanban`：任务来源层
2. `OpenSpec`：正式任务 contract 层
3. `superpowers`：方法层
4. `Cadence-Aria`：编排控制层

### 分层职责

#### 1. Vibe Kanban

负责：

- `issue`
- `workspace`
- `worktree`
- 执行入口
- 状态展示

不负责：

- 子任务代理系统
- 多角色编排
- 审查与测试闭环
- 重型状态机

#### 2. OpenSpec

负责：

- 正式任务 change 身份
- `proposal / design / tasks`
- 范围边界
- 非目标
- 正式任务升级目标

不负责：

- 具体执行调度
- review/test/patch 运行时协议
- Claude/Codex 角色编排

#### 3. superpowers

负责：

- brainstorming
- writing-plans
- 并行调度方法
- 执行计划方法
- 代码审查方法
- 验证与调试方法

不负责：

- 正式任务立项
- change 身份
- 运行时状态机
- 工件归档事实

#### 4. Cadence-Aria

负责：

- 统一接收任务
- 驱动 `OpenSpec`
- 在合适状态调用 `superpowers`
- 驱动 `Claude -> Codex` 工作流
- 汇总 `review + test`
- 生成 `patch contract`
- 输出闭环结果

## 耦合原则

`Cadence-Aria` 必须与 `OpenSpec` 和 `superpowers` 保持协议级松耦合，不做代码级内嵌。

### 约束

1. `OpenSpec` 和 `superpowers` 是前置 plugin
2. 两者**必须**安装在 `Claude Code` 侧
3. `Codex` 侧**最小依赖**：`Codex CLI` + `Codex adapter` 所需最小执行能力
4. 一期**默认不要求** `OpenSpec` 与 `superpowers` 在 `Codex` 侧也必须可用
5. `Aria` 只依赖：
   - 能力存在
   - 入口可探测
   - 角色到能力映射可配置
6. `Aria` 不复制：
   - OpenSpec 工件体系
   - superpowers skill 内容

### 依赖分层

| 层级 | 所在端 | 强依赖 | 说明 |
|------|--------|--------|------|
| 编排层 | `Claude Code` | `OpenSpec`、`superpowers`、`Aria runtime` | 负责 contract 解释、状态机、仲裁、方法层编排 |
| 执行层 | `Codex` | `Codex CLI`、`Codex adapter` | 负责在受控 prompt 下执行代码修改，不直接解析 YAML contract |

### 兼容策略

1. 采用最小依赖面
2. 优先做能力检测
3. 提供推荐版本范围，但不内嵌固定版本
4. 不兼容时必须明确报错能力缺失点

## 前置依赖与检查机制

`Cadence-Aria` 启动时必须先完成前置依赖检查，再允许进入正式流或 `fast-lane`。

### 前置依赖

**Claude Code 侧（编排层）**

1. `OpenSpec plugin`
2. `superpowers plugin`
3. `Aria runtime` 自身

**Codex 侧（执行层）**

4. `Codex` 可执行入口
5. `Codex adapter` 最小可用能力
6. Git worktree 能力

### 检查项

| 检查项 | 检查方式 | 失败处理 |
|------|---------|---------|
| `OpenSpec` 可用 | 探测命令入口与核心产物能力 | 阻断正式流，提示安装或修复 |
| `superpowers` 可用 | 探测关键 skill 是否存在 | 阻断对应角色流转 |
| `Codex` 可用 | 探测 `codex` 执行入口与最小调用能力 | 阻断 `aria:run` 与 `aria:fast` |
| `Codex adapter` 可用 | 探测 prompt 注入、工作目录切换、结果回写、超时控制能力 | 阻断正式 `exec/patch`，允许仅做只读预检 |
| Git worktree 可用 | 探测 Git 仓库状态与 worktree 能力 | 阻断并行执行，允许退化为串行 |

### 能力契约定义

为避免把“插件存在”误判为“能力可用”，一期必须把外部依赖的可用性收敛到**能力契约**，而不是名字契约。

#### 宿主能力矩阵

`Aria runtime` 所在宿主至少要提供以下能力：

| 能力 ID | 是否必须 | 用途 | 一期默认来源 | 缺失时行为 |
|--------|---------|------|-------------|-----------|
| `host.exec.foreground` | 是 | 启动前台命令并获得退出码 | Claude Code shell / Bash 工具 | 阻断运行 |
| `host.exec.background` | 否 | 启动后台 `Codex` 进程并记录 `pid` | Claude Code Bash 后台模式 | 降级为串行前台执行 |
| `host.process.signal` | 否 | 向运行中进程发送终止信号 | Claude Code shell / Bash 工具 | 禁用强终止，只允许软取消 |
| `host.fs.read` | 是 | 读取 contract、报告、日志、状态文件 | Claude Code 文件读取能力 | 阻断运行 |
| `host.fs.write` | 是 | 写入状态与运行时工件 | Claude Code 文件写入能力 | 阻断运行 |
| `host.git.worktree` | 否 | 创建独立 worktree | 本地 Git 能力 | 降级为串行单工作区 |
| `host.git.diff` | 是 | 做文件越界校验与摘要生成 | 本地 Git 能力 | 阻断正式 `exec/patch` |

#### 外部依赖 capability contract

一期至少定义以下 capability：

| 依赖 | capability | 判定标准 | 失败后行为 |
|------|------------|---------|-----------|
| `OpenSpec` | `openspec.change.create` | 能创建或更新正式任务的 `proposal/design/tasks` 最小集合 | 阻断 formal flow |
| `OpenSpec` | `openspec.artifact.read` | 能读取既有 `proposal/design/tasks` 并稳定解析目标字段 | 阻断 `spec -> plan` |
| `superpowers` | `superpowers.plan` | 存在 `brainstorming`、`writing-plans` 等方法能力中被一期使用的最小集合 | 阻断对应角色 |
| `superpowers` | `superpowers.review` | 存在 `requesting-code-review`、`verification-before-completion` 等验证能力 | 阻断 `review/test` |
| `Codex adapter` | `codex.exec.single` | 能在指定工作目录运行一轮 `Codex` 并收集退出状态与结果文件 | 阻断 `exec/patch` |
| `Vibe Kanban` | `vk.task.pull` | 能拉取外部任务基础字段 | 禁用 `vk intake`，保留 `native intake` |
| `Vibe Kanban` | `vk.status.push` | 能按 task 映射写入状态摘要 | 禁用同步，不阻断主流程 |

#### 能力探测输出

能力探测结果必须写成结构化报告，例如：

```yaml
capabilities:
  host.exec.foreground: available
  host.exec.background: degraded
  host.process.signal: unavailable
  openspec.change.create: available
  openspec.artifact.read: available
  superpowers.plan: available
  superpowers.review: available
  codex.exec.single: available
  vk.task.pull: unavailable
  vk.status.push: unavailable
```

状态值统一为：

- `available`：可直接使用
- `degraded`：存在能力，但只能以降级方式使用
- `unavailable`：不可用，必须阻断或关闭对应路径

#### 能力探测规则

1. 一期不以“插件名存在”作为通过条件，必须以 capability 实测为准
2. 一期不硬编码具体 skill 名称到状态机守卫，守卫依赖的是方法能力集合
3. 探测失败必须返回能力 ID、失败原因、建议修复动作
4. capability report 应落盘到任务目录或全局诊断目录，供恢复与排障使用

#### capability report 最小结构

一期建议固定为如下结构，避免不同适配器写出不同格式：

```yaml
report_version: "1.0"
generated_at: "2026-04-16T10:00:00+08:00"
host:
  runtime: "claude-code"
  version: "detected-version"
summary:
  available_count: 5
  degraded_count: 2
  unavailable_count: 3
  formal_flow_allowed: false
  fast_lane_allowed: true
capabilities:
  - id: host.exec.foreground
    status: available
    source: "bash"
    evidence: "foreground command returned exit code 0"
    remediation: ""
  - id: vk.task.pull
    status: unavailable
    source: "vk-adapter"
    evidence: "adapter config missing external endpoint"
    remediation: "disable vk intake or configure adapter"
```

#### capability report 字段约束

1. `report_version` 必须存在，用于未来兼容
2. `summary` 必须提供 formal/fast-lane 是否允许的布尔结论
3. 每个 capability 项必须包含 `id`、`status`、`source`、`evidence`
4. `remediation` 在 `degraded` 或 `unavailable` 时必须非空
5. `evidence` 必须是可审计事实，不允许只写“检测失败”

#### 能力探测错误码

能力探测与适配阶段统一使用以下错误码前缀：

| 错误码 | 含义 | 推荐处理 |
|-------|------|---------|
| `ARIA-CAP-001` | 宿主前台执行能力缺失 | 阻断全部运行 |
| `ARIA-CAP-002` | 宿主后台执行能力缺失 | 降级为串行 |
| `ARIA-CAP-003` | 进程信号能力缺失 | 禁用强取消 |
| `ARIA-CAP-101` | `OpenSpec` 创建能力缺失 | 阻断 formal flow |
| `ARIA-CAP-102` | `OpenSpec` 读取能力缺失 | 阻断 `spec -> plan` |
| `ARIA-CAP-201` | `superpowers` 计划能力缺失 | 阻断 `plan` |
| `ARIA-CAP-202` | `superpowers` 验证能力缺失 | 阻断 `review/test` |
| `ARIA-CAP-301` | `Codex adapter` 单轮执行能力缺失 | 阻断 `exec/patch` |
| `ARIA-CAP-401` | `VK` 拉取能力缺失 | 禁用 `vk intake` |
| `ARIA-CAP-402` | `VK` 推送能力缺失 | 禁用状态同步 |

### 兼容声明

`Aria` 只声明"推荐版本范围"，不锁死具体版本。兼容性文档中至少要包含：

1. 推荐的 `OpenSpec` 版本范围
2. 推荐的 `superpowers` 版本范围
3. 当前测试通过的 `Codex` 版本范围
4. 当前支持的 `Codex adapter` 模式与限制
5. 当能力探测失败时的可解释错误信息

### 降级策略

1. `OpenSpec` 缺失：正式流阻断，`fast-lane` 仅允许低风险原生任务
2. `superpowers` 缺失：阻断对应角色；不允许伪造"兼容实现"
3. `Codex` 缺失：阻断 `exec / patch`
4. `Codex adapter` 能力不足：降级为"prompt 注入 + 单轮执行"模式；若连结果回写都无法保证，则阻断正式流
5. Git worktree 缺失：允许单任务串行模式，不允许并行模式

## 角色模型

一期采用 `8+1` 角色模型：

- 8 个正式业务角色
- 1 个特殊入口
- 1 个系统控制层

### 正式业务角色

1. `intake`
2. `spec`
3. `plan`
4. `dispatch`
5. `exec`
6. `review`
7. `test`
8. `patch`

### 特殊入口

9. `fast-lane`

### 系统控制层

- `orchestrator`

`orchestrator` 不是普通业务角色，而是 `Aria` 的控制面，负责状态流转、角色编排、结果汇总与外部同步。

### orchestrator 细化定义

> **v1.4 修订**：将原 orchestrator 的 7 项职责拆分为 4 个子组件 + 1 个精简版 orchestrator，降低单模块复杂度。

`orchestrator` 是 `Aria runtime` 的上层编排器，不直接暴露给用户的命令。为避免单一模块职责过重，拆分为以下子组件：

#### 子组件职责划分

| 子组件 | 对应源码路径 | 职责 |
|--------|------------|------|
| `StateMachine` | `runtime/state-machine/` | 纯状态流转、守卫条件校验、合法转换判定。不依赖任何外部 adapter |
| `Scheduler` | `runtime/scheduler/` | 执行单元调度、并行管理、依赖解析、worktree 分配、超时监控。依赖 `adapters/host/` 和 `adapters/codex/` |
| `Arbitrator` | `runtime/arbitrator/` | review/test 仲裁、patch contract 生成、冲突判定。纯逻辑，不直接调用外部系统 |
| `SyncService` | `adapters/vk/` | VK 状态同步、漂移检测、单向投影。同步失败不阻断主流程 |
| `PromptService` | `runtime/orchestrator/prompt-service/` | 在需要用户调用 superpowers skill 的状态节点，生成明确的下一步提示语（含推荐的 skill 名称、预期产出工件、状态流转条件） |
| `Orchestrator`（精简版） | `runtime/orchestrator/` | 流程编排（调用上述子组件）、**状态衔接**、用户交互响应、任务上下文维护 |

#### orchestrator 调用关系

```text
user command
  -> Orchestrator（精简版）
    -> StateMachine（状态流转）
    -> PromptService（在用户交互状态生成下一步提示）
    -> Scheduler（exec/patch 调度）
    -> Arbitrator（review/test 仲裁）
    -> SyncService（VK 同步）
```

#### 子组件间依赖规则

```text
Orchestrator
  ├── StateMachine（必须）
  ├── PromptService（必须，plan/review/test 等用户交互状态）
  ├── Scheduler（必须，exec/patch 阶段）
  ├── Arbitrator（必须，review/test 阶段）
  └── SyncService（可选，VK 未配置时跳过）

StateMachine:   无外部依赖（纯逻辑）
PromptService:  无外部依赖（纯文本生成，依赖 capability_mapping 配置）
Scheduler:      → adapters/host/、adapters/codex/
Arbitrator:     无外部依赖（纯逻辑，输入为 report 文件）
SyncService:    → adapters/vk/
```

#### Orchestrator 的推进机制

> **v1.4.2 修正**：orchestrator 是统一自动推进的。任务创建后，状态机按守卫条件自然流转；只有在需求边界不清、需要创意探索时，才会在 `intake` / `spec-required` 阶段调用 `brainstorming` 这类多轮交互型 skill。一旦进入 `spec-approved` 及之后的阶段，`plan`、`review`、`test` 等状态转换均由 orchestrator 自动调用对应 superpowers skills 完成。MVP 阶段可在关键状态节点增加用户确认提示，但最终版本应移除确认实现全自动。

**统一自动推进流程**

orchestrator 按状态机守卫条件自动完成所有状态转换：
- `intake -> spec-required`（基于 intake card 判定）
- `spec-required -> spec-approved`（基于工件存在性判定 + 用户确认）
- `spec-approved -> planned`（orchestrator 自动调用 `writing-plans` skill，生成 plan brief 后流转）
- `planned -> dispatched`（用户显式执行 `aria:run`）
- `dispatched -> executing`（基于 contract 和 worktree 准备状态）
- `executing -> reviewing/testing`（基于所有 exec_unit 完成）
- `reviewing/testing -> patching`（基于仲裁结果）
- `patching -> reviewing/testing`（基于 patch_unit 完成）
- `reviewing/testing -> verified`（orchestrator 自动调用 `requesting-code-review` 和 `verification-before-completion`，读取 report 后判定）
- `verified -> done`（基于 summary 落盘）

**多轮交互型 skills 的介入点**

在以下阶段，若需求边界不清或需要创意探索，orchestrator 可提示用户调用多轮交互型 skill：
- `intake` / `spec-required` 阶段（可选）：orchestrator 提示用户调用 `/brainstorming` 澄清需求
- 用户完成多轮对话后，产出工件（如澄清后的 proposal）落盘，orchestrator 继续推进后续状态

**MVP 安全网（用户确认点）**：
- `spec-approved -> planned`：Aria 自动调用 `writing-plans` 前，MVP 可先提示"即将自动制定执行计划，是否继续？"
- `executing -> reviewing/testing`：Aria 自动调用 `requesting-code-review` / `verification-before-completion` 前，MVP 可先提示"即将自动执行代码审查与验证，是否继续？"

**规则**：
1. orchestrator 是**统一自动推进**的，状态转换由守卫条件驱动，不因 skill 类型而切换推进模式
2. **自动执行型 superpowers skills**（`writing-plans`、`requesting-code-review`、`verification-before-completion` 等）：orchestrator 在对应状态节点**自动调用**，通过检测产出工件推进状态
3. **多轮交互型 superpowers skills**（如 `brainstorming`）：仅在需求澄清阶段由 orchestrator 生成调用指引，等待用户完成 skill 调用后检测工件并继续自动推进
4. **MVP 阶段**：关键自动调用节点可增加用户确认提示，但最终版本应移除确认点实现全自动
5. 用户未响应交互型 skill 提示前，状态机保持在当前状态，不悬空

#### PromptService 输出示例

**自动执行型 skill 的确认提示**（MVP 阶段，最终版可跳过确认）：

当任务处于 `spec-approved` 状态时，PromptService 可能输出：

```text
[Aria]
当前状态：spec-approved
下一步：自动制定执行计划
即将调用：/writing-plans
要求产出：cadence/cache/aria/tasks/<task-id>/plan-brief.md
流转条件：plan brief 包含 plan_id、quality_gates（至少1条）、exec_unit_count（>=1）
[MVP 确认] 是否继续？(y/n)
```

**自动执行型 skill 的执行中提示**（最终版）：

```text
[Aria]
当前状态：spec-approved
正在调用：/writing-plans
预期产出：cadence/cache/aria/tasks/<task-id>/plan-brief.md
```

**多轮交互型 skill 的提示**：

当任务处于 `spec-required` 且需要需求澄清时：

```text
[Aria]
当前状态：spec-required
下一步：需求澄清与创意探索
推荐调用：/brainstorming
说明：该 skill 需要多轮问答交互，请手动调用
```

当任务处于 `reviewing/testing` 且 review 未完成时（最终版自动调用）：

```text
[Aria]
当前状态：reviewing/testing
review_status：pending
test_status：passed
正在调用：/requesting-code-review
预期产出：cadence/cache/aria/tasks/<task-id>/review-report.md
```

`orchestrator`（精简版）自身是有状态的。状态以任务为单位持久化，而不是全局单例内存态。

## 角色职责矩阵

> **v1.4 修订**：应使用的能力列改为引用 `capability_id` 而非具体 skill 名称。

| 角色 | 所属端 | 主要输入 | 主要输出 | 应使用的 capability_id | 不允许做的事 |
|---|---|---|---|---|---|
| `intake` | Claude Code | issue、命令入口、任务描述、Vibe Kanban 上下文 | 标准化任务卡、任务分类、风险初判、流转目标 | `capability.brainstorm`（推荐） | 直接规划、直接执行、跳过正式流判定 |
| `spec` | Claude Code | intake 任务卡、背景上下文、用户目标 | OpenSpec change、proposal、design、tasks 的最小完整集合 | OpenSpec；`capability.brainstorm`（推荐） | 直接派发 exec；绕过 OpenSpec 把正式任务送去执行 |
| `plan` | Claude Code | OpenSpec 工件、约束、非目标、验收目标 | 执行计划、依赖图、验收策略、质量门、并行候选 | `capability.plan`（必须）、`capability.brainstorm`（推荐） | 直接调度 worker；擅自修改 OpenSpec 边界 |
| `dispatch` | Claude Code | 执行计划、OpenSpec tasks、依赖关系 | 执行队列、并行批次、ownership 切分、回收/重派策略 | `capability.dispatch`（必须）、`capability.subagent`（推荐） | 重写 plan；自行放宽质量门；直接修代码 |
| `exec` | Codex | dispatch contract、任务边界、实现目标、局部验收标准 | 实现结果、变更说明、执行记录、自检结果 | `capability.execute`（必须）、`capability.tdd`（可选） | 修改 spec/plan；自行扩 scope；跳过回报直接宣告完成 |
| `review` | Claude Code | exec 结果、OpenSpec 边界、plan 验收标准 | 结构化 review 报告、问题级别、是否退回 patch | `capability.review`（必须） | 直接改代码；替代 test；改动正式边界 |
| `test` | Claude Code | exec 结果、plan 验证策略、任务类型分级规则 | 验证报告、失败证据、通过/退回结论 | `capability.verify`（必须）、`capability.debug`（推荐） | 直接修复问题；替代 review；重写验收口径 |
| `patch` | Codex | review/test 报告、失败证据、原任务边界 | 修补结果、修补说明、重新提交验证 | `capability.debug`（必须）、`capability.receive-review`（必须）、`capability.tdd`（可选） | 擅自改 spec/plan；借修补扩 scope；跳过复检 |
| `fast-lane` | 特殊入口 | 小修小补请求、低风险判定 | 轻量执行记录、完成摘要，或升级到正式流 | `capability.plan`（推荐）、`capability.verify`（推荐） | 处理高风险任务；长期绕过正式流 |

## 状态机与任务流转

### 状态持久化方案

一期建议采用“文件级运行时状态”方案，而不是仅依赖内存态。

状态存储位置建议为：

```text
cadence/cache/aria/
  tasks/
    <task-id>/
      state.yaml
      intake-card.md
      plan-brief.md
      dispatch-contract.yaml
      review-report.md
      test-report.md
      patch-contract.yaml
      verification-summary.md
      closure-summary.md
```

### 持久化原则

1. 一个任务一个目录
2. 一个状态文件 `state.yaml`
3. 运行时工件按轮次写入任务目录
4. 多任务之间完全隔离
5. 关闭 Claude Code 后可根据 `task-id` 恢复

### 文件锁机制

> **v1.4 新增**：根据设计评审建议，补充并发状态文件写入的安全保障。

#### 为什么需要文件锁

虽然一期每个任务有独立目录，但以下场景存在并发写入风险：

1. 同一任务的 `Scheduler` 和 `Orchestrator` 可能同时读写 `state.yaml`
2. `aria:cancel` 可能在 `Scheduler` 写入状态的瞬间被触发
3. `aria:status` 读取状态时，`Orchestrator` 可能正在更新状态

#### 实现方案

采用 `proper-lockfile` 库实现文件级互斥锁：

````text
写入流程：
1. 获取 state.yaml 的文件锁
2. 读取当前 state.yaml
3. 修改内存中的状态
4. 原子写入（先写临时文件，再 rename）
5. 释放文件锁

读取流程：
1. 获取共享读锁（或无锁读取，容忍短暂不一致）
2. 读取 state.yaml
3. 释放锁
````

#### 锁超时与异常

1. 默认锁超时：5 秒
2. 锁超时后抛出 `ARIA-STATE-003` 错误
3. 若进程异常退出，锁文件可能残留，恢复时自动检测并清理过期锁
4. 锁文件存储在 `cadence/cache/aria/tasks/<task-id>/.state.yaml.lock`

#### 错误码补充

| 错误码 | 含义 | 处理方式 |
|-------|------|---------|
| `ARIA-STATE-003` | 文件锁获取超时 | 等待并重试，超过 3 次则提示用户检查是否有其他 Aria 进程 |
| `ARIA-STATE-004` | 残留锁文件检测到 | 检查对应进程是否存活，不存活则清理锁文件并重试 |

### `state.yaml` 最小字段

```yaml
task_id: aria-20260415-001
source: vk | native
flow_type: formal | fast-lane
status: planned
current_round: 1
review_status: pending
test_status: pending
patch_required_by: none
patch_round: 0
exec_units:
  exec-01:
    status: succeeded
    contract_path: "cadence/cache/aria/tasks/aria-20260415-001/dispatch-contract-exec-01.yaml"
    worktree_ref: "wt-exec-01"
    attempt: 1
    exit_code: 0
    result_path: "cadence/cache/aria/tasks/aria-20260415-001/exec-01-result.md"
    started_at: "2026-04-15T10:00:00+08:00"
    finished_at: "2026-04-15T10:15:00+08:00"
    blocked_by: []
  exec-02:
    status: pending
    contract_path: ""
    worktree_ref: ""
    attempt: 0
    exit_code: null
    result_path: ""
    started_at: ""
    finished_at: ""
    blocked_by: [exec-01]
patch_units:
  patch-01:
    status: pending
    based_on_exec_unit: exec-01
    contract_path: ""
    attempt: 0
    started_at: ""
    finished_at: ""
created_at: "2026-04-15T10:00:00+08:00"
updated_at: "2026-04-15T10:30:00+08:00"
```

> **v1.4 修订**：移除原 `workspace_ref` 和 `worktree_ref` 顶层字段。任务级工作区信息通过 `source` 字段（`vk` 来源时记录 VK workspace）承载；执行单元级 worktree 信息通过 `exec_units.<id>.worktree_ref` 承载。避免顶层与单元级字段含义重叠。
```

#### 字段说明

| 字段 | 说明 |
|------|------|
| `exec_units.<id>.status` | 执行单元状态：`pending \| running \| succeeded \| failed \| timeout \| cancelled \| blocked` |
| `exec_units.<id>.contract_path` | 该单元对应的 `dispatch contract` 路径 |
| `exec_units.<id>.worktree_ref` | 该单元独占的 worktree 标识 |
| `exec_units.<id>.attempt` | 已尝试次数，从 `0` 开始 |
| `exec_units.<id>.exit_code` | Codex 进程退出码，`null` 表示未结束 |
| `exec_units.<id>.result_path` | 执行结果落盘路径 |
| `exec_units.<id>.blocked_by` | 阻塞该单元的上游执行单元 ID 列表 |
| `patch_units.<id>.status` | Patch 单元状态：`pending \| running \| succeeded \| failed \| cancelled` |
| `patch_units.<id>.based_on_exec_unit` | 该 patch 基于哪个执行单元 |
| `patch_units.<id>.contract_path` | 该 patch 对应的 `patch contract` 路径 |

### 恢复机制

1. `aria:status` 可列出活跃任务与最后状态
2. `aria:run` 支持基于 `task-id` 继续执行
3. 当存在未完成任务时，`orchestrator` 优先提示恢复而不是重建
4. 若运行时工件与状态不一致，以状态文件为主，并报告损坏

#### 人工恢复边界

当系统判定为“状态损坏”时，一期不尝试自动修复所有异常，只允许以下人工恢复动作：

1. 修正缺失但可重建的运行时工件路径
2. 将明显错误的 `running` 状态改为 `failed` 或 `timeout`
3. 补写 capability report 或日志索引等非业务事实工件
4. 对未开始的执行单元执行重新调度

以下情况禁止人工直接修改后继续执行，必须退回 `plan` 或 `spec`：

1. `dispatch contract` 与 OpenSpec 边界不一致
2. `patch contract` 已要求越界修补
3. 多个执行单元实际改动与 ownership 显著冲突
4. 无法确认 `base_revision` 与当前工作区关系

#### 恢复判定规则

1. 若损坏仅影响运行时索引文件，不影响 OpenSpec、plan、contract 主事实，可修复后恢复
2. 若损坏影响 contract 与状态的一致性，应停止恢复并给出错误码
3. 若已无法判断某执行单元是否真正完成，应按未完成处理，禁止假定为成功
4. 恢复动作必须写入日志与 closure summary 附录

### 正式任务通道

正式任务状态机如下：

`intake -> spec-required -> spec-approved -> planned -> dispatched -> executing -> reviewing/testing -> patching -> verified -> done`

### 状态定义

| 状态 | 含义 |
|------|------|
| `intake` | 任务进入 Aria，由 `intake` 标准化与分类 |
| `spec-required` | 任务被判定为正式任务，必须进入 OpenSpec |
| `spec-approved` | `OpenSpec` 主线工件达到最小完整集合 |

#### `spec-approved` 最小门槛表

一期为 `spec-approved` 定义以下可操作的最小判定标准：

| 门槛项 | 是否必须 | 判定标准 | 负责角色 |
|--------|---------|---------|---------|
| `proposal` 工件存在 | 是 | 文件存在且包含 `goal`、`scope`、`non-goals` 字段 | `spec` |
| `design` 工件存在 | 是 | 文件存在且包含 `architecture`、`decisions`、`risks` 字段 | `spec` |
| `tasks` 工件存在 | 是 | 文件存在且至少包含 1 个任务条目，每个条目有 `id`、`description`、`acceptance` | `spec` |
| 工件字段机器可读 | 否 | YAML front matter 或 YAML 文件可被解析 | `spec`（辅助） |
| 人工最终确认 | 是 | `orchestrator` 提示用户确认 OpenSpec 集合完整 | 用户 |

**允许精简的例外**：

1. 纯文档/规则类低风险任务，允许 `design` 合并进 `proposal` 的一个独立章节，但必须有明确标注
2. 单点脚本修复类任务，允许 `tasks` 只含 1 个条目
3. 任何情况下 `proposal` 的 `goal` 和 `scope` 字段不可缺失

#### 状态流转与守卫条件

> **v1.4.2 修正**：恢复自动调用定位，仅 `brainstorming` 等多轮交互型 skills 需要用户手动调用。MVP 阶段关键自动调用节点可带用户确认点。

正式任务状态机的每个转换必须满足以下守卫条件：

| 当前状态 | 目标状态 | 进入条件 | 负责角色 | 触发方式 | 退出条件（目标状态完成标志） |
|---------|---------|---------|---------|---------|---------------------------|
| `intake` | `spec-required` | intake card 已落盘，flow_type 判定为 `formal` | `orchestrator` | 自动 | intake card 包含 task_id、source、risk_level |
| `spec-required` | `spec-approved` | 门槛表全部通过（见上方门槛表） | `spec` → 用户确认 | 自动（工件检测）+ 用户确认 | proposal/design/tasks 工件存在且字段完整，用户已确认 |
| `spec-approved` | `planned` | plan brief 已落盘，包含 plan_id、quality_gates、exec_unit_count、parallel_candidates、acceptance_strategy | `plan` | **自动调用** `writing-plans`（MVP 可带确认点） | plan brief 中所有字段非空，quality_gates 至少 1 条，exec_unit_count >= 1 |
| `planned` | `dispatched` | 用户显式执行 `aria:run --task-id <id>` | 用户触发 → `orchestrator` | 用户命令触发 | dispatch contract 已为每个 exec_unit 生成，state.yaml 中 exec_units 非空 |
| `dispatched` | `executing` | 所有 exec_unit 的 dispatch contract 已落盘，worktree 已创建 | `orchestrator` | 自动 | 至少 1 个 exec_unit 状态为 `running` |
| `executing` | `reviewing/testing` | 所有 exec_unit 状态均为 `succeeded`/`failed`/`timeout`，无 `running` 或 `pending` 状态 | `orchestrator` | 自动 | review_status 和 test_status 不全为 `pending` |
| `reviewing/testing` | `patching` | review_status 或 test_status 为 `failed` | `orchestrator` | 自动 | patch contract 已生成且落盘，至少 1 个 patch_unit 状态为 `running` |
| `reviewing/testing` | `verified` | review_status 和 test_status 均为 `passed`，无需 patch | `orchestrator` | 自动 | verification summary 已落盘 |
| `patching` | `reviewing/testing` | 所有 patch_unit 状态不为 `running`，patch 结果已落盘 | `orchestrator` | 自动 | review_status 和 test_status 被重置为 `pending` |
| `verified` | `done` | verification summary 与 closure summary 均已落盘 | `orchestrator` | 自动 | 任务标记为 `done`，向 VK 推送完成摘要 |
| 任意状态 | `cancelled` | 用户执行 `aria:cancel` | 用户触发 | 用户命令触发 | 所有 running 进程已停止，工件保留 |

#### 自动执行型 skills 的流转机制

> **v1.4.2 修正**：`writing-plans`、`requesting-code-review`、`verification-before-completion` 等自动执行型 skills 由 orchestrator 自动调用，不再需要用户显式触发。

**`spec-approved` → `planned` 的自动流转流程**：

```text
1. Orchestrator 检测到状态为 spec-approved
2. [MVP 可选] PromptService 输出确认提示："即将自动调用 /writing-plans 制定执行计划，是否继续？"
3. Orchestrator 自动调用 /writing-plans skill
4. Skill 执行完成后，plan brief 工件落盘
5. Orchestrator 检测到 plan-brief.md 存在且字段完整
6. StateMachine 执行 spec-approved -> planned 转换
```

**`executing` → `reviewing/testing` 的自动 review 流程**：

```text
1. Orchestrator 检测到所有 exec_unit 已完成
2. 状态自动进入 reviewing/testing
3. [MVP 可选] PromptService 输出确认提示
4. Orchestrator 自动调用 /requesting-code-review skill
5. Skill 执行完成后，review report 工件落盘
6. Orchestrator 读取 review report 并更新 review_status
```

**`executing` → `reviewing/testing` 的自动 test 流程**：

```text
1. 与 review 并行（或按序）进行
2. [MVP 可选] PromptService 输出确认提示
3. Orchestrator 自动调用 /verification-before-completion skill
4. Skill 执行完成后，test report 工件落盘
5. Orchestrator 读取 test report 并更新 test_status
```

#### 多轮交互型 skills 的流转机制

**`brainstorming` 的用户交互流程**（仅在 intake/spec-required 阶段可选使用）：

```text
1. Orchestrator 判断当前需求边界不够清晰
2. PromptService 输出："建议调用 /brainstorming 进行需求澄清，该 skill 需要多轮问答交互"
3. 用户调用 /brainstorming 并完成多轮交互
4. 产出工件（如澄清后的 proposal）落盘
5. Orchestrator 读取工件后继续推进状态
```

**规则**：
1. **自动执行型 skills**：orchestrator 自动调用，skill 执行结果通过工件落盘被 Aria 感知
2. **多轮交互型 skills**：用户未在合理时间内调用时，状态保持不变，orchestrator 可在后续 `aria:status` 查询时再次提示
3. 所有 skill 调用结果必须通过**工件落盘**被 Aria 感知，而不是通过内存事件
4. 若产出工件不满足守卫条件，orchestrator 应明确指出缺失项，状态不推进
5. **MVP 阶段**：关键自动调用节点可增加用户确认提示，最终目标是全自动编排

**异常路径守卫**：

| 异常场景 | 守卫条件 | 处理方式 |
|---------|---------|---------|
| 部分成功 | 存在 `failed`/`timeout` 的 exec_unit 且无 `running` | 进入 `reviewing/testing`，由 review/test 判定是否需要 patch |
| patch 超限 | `patch_round` 达到上限（默认 2） | 退回 `planned`（边界仍有效）或 `spec-required`（边界已失效） |
| 状态恢复 | state.yaml 存在且工件目录完整 | 从当前状态继续；若工件损坏，停止执行并要求人工恢复 |
| 重试 | 执行单元状态为 `failed`/`timeout`/`cancelled` 且 `attempt` 未超上限 | 允许 `aria:retry`；超限则提示退回 `plan` |

### fast-lane 轻量通道

轻量状态机如下：

`intake -> fast-triage -> execute -> review/testing-lite -> done`

### 升级规则

当出现以下情况之一时，必须从 `fast-lane` 升级回正式任务通道：

1. 影响范围超过单模块
2. 需要新增设计决策
3. 需要并行拆分
4. 出现多轮 `patch`
5. 需要长期保留正式边界

### 升级失败处理

当 `fast-lane` 触发升级，但发现 `OpenSpec` 或其他正式流依赖仍然不可用时，系统进入明确的阻塞状态，而不是悬空：

#### 阻塞状态

| 状态 | 含义 |
|------|------|
| `upgrade-blocked` | fast-lane 已超界需要升级，但正式流依赖缺失 |
| `awaiting-dependency-fix` | 等待用户修复缺失的前置依赖 |

#### 升级失败时的行为

1. **用户提示**：`orchestrator` 明确告知用户"fast-lane 已超界，但正式流所需的 `<缺失项>` 不可用，任务已挂起"
2. **保留工件**：已产生的 `fast-lane` 运行时工件（执行记录、轻量报告）全部保留，不清理
3. **恢复入口**：用户修复依赖后，可通过 `aria:run --task-id <id> --resume` 自动检测依赖并尝试升级为正式流
4. **降级选择**：用户也可以选择取消升级，将任务标记为 `cancelled` 并手动接管

#### 状态机补充

```text
fast-lane execute -> review/testing-lite -> done
                |
                -> 超界需升级
                      |
                      -> 依赖可用 -> 进入 formal flow (intake -> spec-required)
                      -> 依赖不可用 -> upgrade-blocked -> awaiting-dependency-fix
```

## Codex 触发与执行机制

`Claude Code 驱动 Codex` 在一期中采用“`orchestrator` 生成执行协议，`Aria runtime` 解释 contract，Claude 侧调用 Codex CLI 执行”的模式。

### 设计修订原则

本方案不再假设 `Codex` 原生具备“读取 YAML contract 并严格按字段执行”的 runtime 能力。一期采用薄适配器路线：

1. `Aria runtime` 负责读取和校验 `dispatch contract` / `patch contract`
2. `Aria runtime` 负责把结构化约束转换成 `Codex` 可消费的 prompt 和启动参数
3. `Codex` 负责在给定上下文中执行代码修改、输出说明、返回会话结果
4. 结果落盘、状态流转、重试与仲裁全部由 `Aria runtime` 负责

### 触发机制

1. `dispatch` 生成 `dispatch contract`
2. `orchestrator` 为每个执行单元创建独立工作目录上下文
3. `Aria runtime` 将 contract 渲染为 `Codex` 初始提示、执行约束和结果输出目标
4. `orchestrator` 调用 `Codex CLI`
5. `Codex` 在指定 worktree 中执行 `exec` 或 `patch`
6. `Aria runtime` 将执行结果、摘要和状态写回任务目录，再由 `orchestrator` 汇总

### 一期执行模型

1. 一个执行单元对应一个 `Codex` 运行实例
2. 并行执行通过多个独立 `Codex` 实例实现
3. `review` 与 `test` 在 Claude 侧默认并行，但可因上下文成本或冲突风险退化为分阶段
4. `Codex` 只承担“执行引擎”职责，不承担运行时状态机和 contract 解析职责

### Codex Runtime 详细设计

一期的 `Codex runtime` 应明确拆成两层：

1. `Aria runtime`：运行在 Claude 侧，负责 contract 解释、生命周期管理、状态持久化、结果回写
2. `Codex adapter`：面向 `Codex CLI` 的薄桥接层，负责把结构化约束转换成 `Codex` 可消费的 prompt 和启动参数

### `Codex adapter` 责任边界

`Codex adapter` 必须实现以下最小能力：

1. 接收 `dispatch contract` / `patch contract`
2. 切换到指定 `worktree`
3. 将允许/禁止修改范围、目标、验收条件写入 `Codex` 初始上下文
4. 收集 `Codex` 的退出状态、摘要、输出文件引用
5. 将结果交回 `Aria runtime`

以下能力不放到 `Codex adapter`：

1. 生成或修改 `OpenSpec` 工件
2. 决定是否进入 patch
3. 仲裁 `review` 与 `test` 冲突
4. 跨任务调度其他 `Codex` 实例

### Codex 进程管理模型

`Codex adapter` 负责管理 `Codex` 进程的完整生命周期。一期的技术方案如下：

#### 进程启动方式

若宿主提供 `host.exec.background`，则采用后台子进程模式启动 `Codex CLI`；否则降级为串行前台执行。

1. `Aria runtime` 将 contract 转写为受控 prompt 和启动参数
2. 通过宿主执行能力启动 `codex` 命令
3. 每个执行单元对应一个独立的后台进程
4. 进程启动后立即记录 `pid`、`started_at` 到 state.yaml

```text
Aria runtime
  -> 写入 contract 到任务目录
  -> 生成 codex CLI 命令行（含 prompt 注入参数）
  -> host.exec.background 启动子进程
  -> 记录 pid 和 started_at
```

#### 超时实现方式

采用双层超时控制：

1. **进程级超时**：宿主执行能力提供的超时参数，映射 contract 中的 `timeout_minutes`
2. **调度级超时**：`orchestrator` 定期检查 `started_at + timeout_minutes`，超时则主动终止

```yaml
# 超时配置示例
exec:
  default_timeout_minutes: 30
  max_timeout_minutes: 120
patch:
  default_timeout_minutes: 20
  max_timeout_minutes: 60
```

#### 取消信号传递

一期采用以下取消策略：

1. `aria:cancel` 触发时，若宿主提供 `host.process.signal`，`orchestrator` 发送 SIGTERM
2. 等待 5 秒后检查进程是否已退出
3. 若未退出，再发送强制终止信号
4. 更新 state.yaml 中对应单元状态为 `cancelled`

```text
aria:cancel
  -> signal TERM <pid>
  -> wait 5s
  -> check process status
    -> exited -> mark cancelled, done
    -> still running -> signal KILL <pid> -> mark cancelled
```

#### 结果收集方式

采用"文件系统落盘 + stdout/stderr 捕获"混合方式：

1. **stdout/stderr**：宿主执行能力的输出写入输出文件或缓冲区，再由 `orchestrator` 读取
2. **执行结果文件**：`Codex adapter` 在 prompt 中要求 Codex 将结果写入指定路径（如 `result_path`）
3. **Git diff**：exec 完成后，`orchestrator` 执行 `git diff` 获取实际变更摘要
4. **结构化转写**：`Aria runtime` 将 Codex 的自然语言输出转写为结构化执行记录

#### 异常清理方式

1. **僵尸进程检测**：`orchestrator` 在每次状态恢复时，检查 state.yaml 中所有 `running` 状态的进程是否仍然存活
2. **孤儿清理**：若 `Claude Code` 会话异常退出，下次恢复时扫描任务目录中所有 `running` 状态的单元，检查对应 pid 是否存活
3. **Worktree 清理**：任务进入 `done`/`cancelled` 后，按配置决定是否保留或清理 worktree
4. **临时文件清理**：超时的进程可能留下不完整的输出文件，恢复时检测并标记为 `timeout`

```text
恢复时的清理流程：
1. 读取 state.yaml
2. 遍历所有 running 状态的 exec/patch 单元
3. 对每个单元检查 pid 是否存活（kill -0 <pid>）
4. 不存活 -> 标记为 timeout/failed
5. 存活但已超时 -> 发送 SIGTERM 终止
```

#### 宿主绑定边界

一期可以**优先**针对 `Claude Code` 宿主完成适配，但文档层必须明确以下边界：

1. `Aria runtime` 依赖的是 capability，而不是具体工具名
2. “Claude Code Bash 工具”只是一期默认实现，不是抽象层接口名
3. 若后续宿主不支持后台子进程，则自动退化为串行前台执行模型
4. 只有 capability matrix 中被声明的能力，才能写入状态机守卫或错误处理逻辑

### Claude Code ↔ Codex 通信协议

一期采用“文件系统为主，prompt 注入为辅”的本地协议：

1. `Aria runtime` 将 contract 落盘到 `cadence/cache/aria/tasks/<task-id>/`
2. 启动 `Codex CLI` 时，通过工作目录、任务摘要和 contract 摘要进行 prompt 注入
3. `Codex` 的自然语言结果由 `Aria runtime` 转写为结构化执行记录
4. 最终以任务目录中的工件作为唯一源事实

该协议刻意不依赖远程 API，也不要求 `Codex` 直接解析 YAML 文件。

### Codex Prompt 注入模板

> **v1.4 新增**：根据设计评审建议，定义 Codex adapter 的 prompt 模板最小格式。

#### 模板变量

prompt 模板支持以下变量插值，格式为 `{{variable_name}}`：

| 变量名 | 来源 | 说明 |
|--------|------|------|
| `{{task_id}}` | state.yaml | 任务唯一标识 |
| `{{exec_unit_id}}` | dispatch contract | 执行单元标识 |
| `{{goal}}` | dispatch contract / patch contract | 执行目标描述 |
| `{{scope_allowed}}` | contract.scope.files_allowed | 允许修改的文件列表 |
| `{{scope_blocked}}` | contract.scope.files_blocked | 禁止修改的文件列表 |
| `{{acceptance}}` | contract.acceptance | 验收条件列表 |
| `{{base_revision}}` | contract.base_revision | 基线 Git revision |
| `{{result_path}}` | contract.result_path | 结果输出目标路径 |
| `{{timeout_minutes}}` | contract.timeout_minutes | 超时时间 |
| `{{mode}}` | contract.mode | 执行模式（exec / patch） |
| `{{must_fix}}` | patch contract.must_fix | 必须修复的问题列表（仅 patch 模式） |
| `{{must_not_change}}` | patch contract.must_not_change | 不可变更的范围（仅 patch 模式） |

#### Exec 模板

```markdown
# Task: {{task_id}} / {{exec_unit_id}}

## Objective
{{goal}}

## Scope
### Files you MAY modify
{{scope_allowed}}

### Files you MUST NOT modify
{{scope_blocked}}

## Acceptance Criteria
{{acceptance}}

## Constraints
1. Base revision: {{base_revision}}
2. Do NOT modify any file outside the allowed scope
3. Do NOT change the task boundary or acceptance criteria
4. Write a summary of changes to: {{result_path}}

## Output Format
When complete, write a markdown file to {{result_path}} with:
- Summary of changes made
- Files modified (list)
- Any issues encountered
- Self-check result (pass/fail)
```

#### Patch 模板

```markdown
# Patch Task: {{task_id}} / {{exec_unit_id}}

## Objective
Fix the following issues identified during review/testing.

## Must Fix
{{must_fix}}

## Must NOT Change
{{must_not_change}}

## Scope
### Files you MAY modify
{{scope_allowed}}

### Files you MUST NOT modify
{{scope_blocked}}

## Acceptance Criteria
{{acceptance}}

## Constraints
1. Only fix the issues listed in "Must Fix" — do NOT expand scope
2. Do NOT modify files in "Must NOT Change"
3. Base revision: {{base_revision}}
4. Write a summary of changes to: {{result_path}}

## Output Format
When complete, write a markdown file to {{result_path}} with:
- Summary of fixes applied
- Which must_fix items were addressed
- Files modified (list)
- Any remaining issues
- Self-check result (pass/fail)
```

#### 模板渲染规则

1. 变量插值在 `Codex adapter` 中完成，不依赖外部模板引擎
2. 列表变量（`scope_allowed`、`acceptance`、`must_fix` 等）渲染为 Markdown 列表（`- item`）
3. 空列表渲染为 `（无）`
4. 模板存储在 `codex/prompts/exec.md` 和 `codex/prompts/patch.md`
5. 用户可在 `cadence/cache/aria/config.yaml` 中覆盖默认模板路径

#### Prompt 注入方式

1. **方式 A（推荐）**：通过 Codex CLI 的 `--prompt` 参数或 stdin 注入渲染后的 prompt
2. **方式 B（备选）**：将渲染后的 prompt 写入临时文件，通过 Codex CLI 的 `--prompt-file` 参数引用
3. 具体使用哪种方式取决于 Codex CLI 支持的参数，由 `Codex adapter` 在初始化时探测

### 生命周期管理

`orchestrator` 负责：

1. 创建执行单元
2. 启动 Codex 进程
3. 监控退出状态
4. 记录 stdout/stderr 摘要
5. 执行结果转写与落盘
6. 处理超时与取消
7. 标记成功、失败、超时、取消状态

### 超时与失败

| 场景 | 处理方式 |
|------|---------|
| Codex 启动失败 | 标记执行单元失败，阻断依赖单元 |
| Codex 超时 | 标记超时，允许用户重试或取消 |
| Codex 异常退出 | 记录错误摘要，生成失败报告 |
| 执行结果缺失 | 视为失败，不进入 review/test |

### 不采纳的机制

一期不设计：

1. Codex 内部多任务并行
2. 基于远程 API 的执行编排
3. 由 Codex 自主管理其他 Codex 实例

## 并行模型与资源隔离

### 并行粒度

一期采用混合粒度：

1. 默认按 `OpenSpec tasks` 并行
2. 单个 task 过大时，再拆为模块 ownership 并行

### 隔离策略

1. 每个执行单元独占一个 worktree
2. 每个执行单元只写自己的 ownership 范围
3. 禁止多个执行单元同时修改同一文件
4. review/test 总是在汇总结果后执行，不与同一单元的 patch 并发

### ownership 冲突检测

一期采用保守检测策略，而不是做重型静态分析：

1. `dispatch` 在切分执行单元时必须显式声明 `files_allowed`
2. 若两个执行单元的 `files_allowed` 有交集，则直接判定冲突并强制串行
3. 若执行单元只声明目录级 ownership，则按路径前缀做冲突判定
4. 无法可靠判定时，默认串行而不是乐观并行

### 并行上限

一期建议默认上限：

1. `exec` 并行数默认 `2`
2. 可配置上限 `4`
3. 超出上限的单元进入等待队列

### 背压策略

1. 当 worktree 资源不足时，自动退回串行
2. 当同层任务存在文件 ownership 冲突时，强制串行
3. 当上游依赖失败时，阻断其下游执行

### 并行执行假设与 MVP 验证

方案的核心假设是"多个 Codex 实例可通过宿主后台执行能力并行启动并独立收集结果"。此假设需要在 MVP 阶段**优先验证**。

#### 待验证假设

1. `Claude Code` 能通过宿主后台执行能力同时启动 2 个 `Codex CLI` 子进程
2. 子进程能在独立 worktree 中并行执行，互不干扰
3. `orchestrator` 能在等待期间被用户中断（`aria:cancel`）
4. 结果文件能被独立读取和转写

#### MVP 验证步骤

```text
1. 准备 2 个独立 worktree
2. 通过宿主后台执行能力同时启动 2 个 codex 进程
3. 每个进程执行一个最小修改任务（如修改 README 中的日期）
4. 等待两个进程完成
5. 分别读取输出文件和 git diff
6. 验证无交叉污染
```

#### 验证失败时的降级方案

如果并行假设验证失败，一期退化为**串行调度 + 逻辑并行**模式：

1. `dispatch` 仍然切分多个执行单元
2. 但执行时按序逐个启动 `Codex` 进程
3. 前一个完成后才启动下一个
4. 并行上限参数在串行模式下无效
5. 方案中的并行模型、背压策略等设计保留，但标记为"一期串行模式下暂不启用"

#### 验证时机

并行验证应作为 **MVP 前置条件**，在正式开发 Aria runtime 之前完成。若验证通过，则按并行模型实现；若验证失败，则立即切换到串行降级方案。

## review / test 仲裁与 patch 生成

### 执行策略

`review` 与 `test` 的默认策略是"**以任务汇总态为主，必要时补充单元级附录**"：

1. 默认以**任务汇总态**做 `review/test`，即汇总所有 `exec` 结果后统一评审和验证
2. 仅在单元级失败定位明显时，补充单元级附录报告
3. 若任务上下文小、验证命令明确，则 `review` 与 `test` 并行执行
4. 若测试需要大量构建输出或失败日志，允许先执行 `test`，再让 `review` 消费测试结果
5. 若 token 或日志体积过高，`orchestrator` 可以退化为串行，以减少重复上下文装载

### partial 状态

`review` 与 `test` 的结果必须分别落盘并单独建状态字段，不再只用一个合并态表达：

1. `review_status`: `pending | passed | failed`
2. `test_status`: `pending | passed | failed`
3. `patch_required_by`: `review | test | both`

### 仲裁规则

当 `review` 与 `test` 存在不同结论时，按以下顺序处理：

1. **任一角色提出且不突破原边界的 blocker，都可进入 `must_fix`**
2. 只有建议项才需要在仲裁后降级为 `advisory_only`
3. `test` 失败且涉及功能正确性、接口兼容性时，优先级高于纯可读性重构建议
4. `review` 建议若会扩大 scope、改变接口或与测试修复冲突，必须降级为"建议项"，不得直接进入 `must_fix`
5. 若两类必须修问题互相冲突，则退回 `plan`
6. 无法在原边界内同时满足时，退回 `plan`；若边界已失效，则退回 `spec`

## fast-lane 准入标准

只有满足以下条件的任务，才允许进入 `fast-lane`：

1. 单文件或单一明确 ownership 范围
2. 不引入新设计决策
3. 不需要 OpenSpec 正式变更边界
4. 不需要并行拆分
5. 预计一轮 `exec + review/testing-lite` 可闭环
6. 不涉及跨模块接口修改
7. 不涉及状态迁移、数据迁移或发布策略

### 典型适用场景

1. 文档小改
2. 规则文案修正
3. 单点配置修补
4. 小范围脚本参数修复

### 禁止场景

1. 新增功能
2. 跨模块重构
3. 多文件联动改动
4. 需要多轮 patch 的复杂 bug
5. 需要正式设计评审的任务

### 执行主体

`fast-lane` 仍由 `orchestrator` 驱动，执行主体默认仍是 `Codex exec`，不是 Claude 直接代替执行。

## 运行时工件格式

一期建议采用“Markdown 承载说明 + YAML 承载结构字段”的混合格式。

### 格式选择

| 工件 | 格式 | 理由 |
|------|------|------|
| `task intake card` | Markdown + YAML front matter | 兼顾可读性与结构化 |
| `plan brief` | Markdown + YAML front matter | 需要人读，也需要解析 |
| `dispatch contract` | YAML | 面向执行契约，结构优先 |
| `review report` | Markdown + YAML front matter | 便于人读审查问题 |
| `test report` | Markdown + YAML front matter | 需要记录证据与结论 |
| `patch contract` | YAML | 面向修补契约，结构优先 |
| `verification summary` | Markdown | 面向汇总展示 |
| `closure summary` | Markdown | 面向最终闭环展示 |

### 工件引用关系

```text
task intake card
  -> plan brief
    -> dispatch contract
      -> exec result
        -> review report
        -> test report
          -> patch contract
            -> verification summary
              -> closure summary
```

### `task intake card` YAML front matter

```yaml
---
task_id: aria-20260415-001
source: vk | native | aria-native
flow_type_suggestion: formal | fast-lane
risk_level: low | medium | high
scope_summary: "简述任务范围"
boundary_check:
  single_file: true | false
  single_module: true | false
  cross_module: true | false
  needs_design_decision: true | false
created_at: "2026-04-15T10:00:00+08:00"
---
```

#### 字段说明

| 字段 | 说明 |
|------|------|
| `task_id` | 唯一任务标识，格式 `aria-YYYYMMDD-NNN` |
| `source` | 任务来源：`vk`（Vibe Kanban）、`native`（用户直接输入）、`aria-native`（Aria 内部生成） |
| `flow_type_suggestion` | 建议的流转类型：`formal` 或 `fast-lane` |
| `risk_level` | 风险初判：`low`/`medium`/`high` |
| `scope_summary` | 任务范围简述 |
| `boundary_check` | 边界检查结果，用于辅助 flow_type 判定 |

### `plan brief` YAML front matter

```yaml
---
plan_id: plan-aria-20260415-001
task_id: aria-20260415-001
quality_gates:
  - type: test_coverage
    threshold: 80
  - type: format_check
    enabled: true
exec_unit_count: 2
parallel_candidates:
  - [exec-01, exec-02]
acceptance_strategy: "all_units_pass"
generated_at: "2026-04-15T10:05:00+08:00"
---
```

#### 字段说明

| 字段 | 说明 |
|------|------|
| `plan_id` | 计划唯一标识 |
| `task_id` | 关联的任务 ID |
| `quality_gates` | 质量门列表，每项包含 `type` 和具体参数 |
| `exec_unit_count` | 执行单元总数 |
| `parallel_candidates` | 可并行执行的单元组列表 |
| `acceptance_strategy` | 验收策略：`all_units_pass`（全部通过）、`majority_pass`（多数通过）等 |

### `review report` YAML front matter

```yaml
---
task_id: aria-20260415-001
exec_units_reviewed:
  - exec-01
  - exec-02
blockers:
  - issue_id: review-001
    severity: blocker
    exec_unit: exec-01
    description: "描述问题"
    file_path: "path/to/file"
    line_range: "10-20"
suggestions:
  - issue_id: review-002
    severity: advisory
    exec_unit: exec-01
    description: "描述建议"
verdict: passed | failed | needs_patch
reviewed_at: "2026-04-15T10:30:00+08:00"
---
```

#### 字段说明

| 字段 | 说明 |
|------|------|
| `task_id` | 关联的任务 ID |
| `exec_units_reviewed` | 已审查的执行单元列表 |
| `blockers` | 阻断性问题列表，每项包含 `issue_id`、`severity`、`exec_unit`、`description`、`file_path`、`line_range` |
| `suggestions` | 建议性改进列表，结构同 blockers |
| `verdict` | 审查结论：`passed`/`failed`/`needs_patch` |
| `reviewed_at` | 审查完成时间 |

### `test report` YAML front matter

```yaml
---
task_id: aria-20260415-001
exec_units_tested:
  - exec-01
  - exec-02
failures:
  - test_id: test-001
    exec_unit: exec-01
    evidence: "错误输出或日志摘要"
    severity: blocker | warning
    file_path: "path/to/file"
    test_command: "pnpm test -- path/to/test"
passed_count: 5
failed_count: 1
verdict: passed | failed
tested_at: "2026-04-15T10:35:00+08:00"
---
```

#### 字段说明

| 字段 | 说明 |
|------|------|
| `task_id` | 关联的任务 ID |
| `exec_units_tested` | 已测试的执行单元列表 |
| `failures` | 失败项列表，每项包含 `test_id`、`evidence`、`severity`、`file_path`、`test_command` |
| `passed_count` | 通过数 |
| `failed_count` | 失败数 |
| `verdict` | 测试结论：`passed`/`failed` |
| `tested_at` | 测试完成时间 |

### `dispatch contract` 最小字段

```yaml
contract_version: "1.0"
generated_at: "2026-04-15T10:00:00+08:00"
base_revision: "abc1234"
input_artifacts:
  plan_brief: "cadence/cache/aria/tasks/aria-20260415-001/plan-brief.md"
  openspec_tasks: "cadence/openspec/tasks.yaml"
generated_from_plan: "plan-aria-20260415-001"
source_task_refs:
  - "aria-20260415-001"
task_id: aria-20260415-001
exec_unit_id: exec-01
parent_task: task-a
mode: exec
scope:
  files_allowed:
    - path/to/file.md
  files_blocked:
    - path/to/other.md
goal: ""
acceptance:
  - ""
dependencies: []
worktree_ref: ""
result_path: "cadence/cache/aria/tasks/aria-20260415-001/exec-01-result.md"
timeout_minutes: 30
retry_allowed: true
```

#### 快照字段说明

| 字段 | 说明 |
|------|------|
| `contract_version` | 本 contract 格式版本 |
| `generated_at` | 生成时间戳 |
| `base_revision` | 生成时的 Git revision |
| `input_artifacts` | 生成该 contract 所依赖的输入工件路径 |
| `generated_from_plan` | 来源 plan 标识 |
| `source_task_refs` | 关联的任务 ID 列表 |

#### `dispatch contract` 生成规则

`dispatch contract` 不能只靠自由 prompt 拼装，一期至少要遵循以下机械生成规则：

1. `task_id` 来自当前 Aria 任务主键
2. `parent_task` 必须映射到 OpenSpec `tasks` 中的一个明确任务 `id`
3. `goal` 必须来自该 OpenSpec task 的 `description` 与 `proposal/design` 的最小摘要，不允许手写发散
4. `acceptance` 必须优先继承该 OpenSpec task 的 `acceptance`，再附加 plan 中的质量门结果
5. `dependencies` 必须来自 plan 中声明的依赖图，不能由 dispatch 临时猜测
6. `base_revision` 必须取生成时实际 Git revision，用于重试与 patch 回溯
7. `result_path` 必须唯一，且和 `exec_unit_id` 一一对应

#### ownership 与文件范围映射规则

`files_allowed` 的生成必须有明确来源，一期采用保守映射：

1. 若 OpenSpec task 已显式声明文件或目录范围，则直接继承
2. 若 OpenSpec task 未声明文件范围，但 plan 已定义 ownership，则以 plan 为准
3. 若两者都未声明，dispatch 不得生成并行单元，必须退回 `plan`
4. 目录级 ownership 必须标准化为仓库相对路径前缀
5. 文件级 ownership 优先级高于目录级 ownership
6. 任一 `files_blocked` 规则不得与 `files_allowed` 重叠；若重叠，视为 contract 生成错误

#### dispatch 生成错误码

| 错误码 | 含义 | 处理方式 |
|-------|------|---------|
| `ARIA-DISPATCH-001` | OpenSpec task 无法映射到 `parent_task` | 退回 `spec` |
| `ARIA-DISPATCH-002` | ownership 缺失，无法生成 `files_allowed` | 退回 `plan` |
| `ARIA-DISPATCH-003` | `files_allowed` 与 `files_blocked` 冲突 | 阻断 dispatch |
| `ARIA-DISPATCH-004` | 依赖图有环或引用缺失 | 退回 `plan` |
| `ARIA-DISPATCH-005` | `result_path` 或 `exec_unit_id` 非唯一 | 阻断 dispatch |

### `patch contract` 最小字段

```yaml
contract_version: "1.0"
generated_at: "2026-04-15T10:30:00+08:00"
base_revision: "abc1234"
input_artifacts:
  review_report: "cadence/cache/aria/tasks/aria-20260415-001/review-report.md"
  test_report: "cadence/cache/aria/tasks/aria-20260415-001/test-report.md"
  based_on_dispatch_contract: "cadence/cache/aria/tasks/aria-20260415-001/dispatch-contract-exec-01.yaml"
generated_from_plan: "plan-aria-20260415-001"
source_task_refs:
  - "aria-20260415-001"
task_id: aria-20260415-001
patch_unit_id: patch-01
source_exec_unit: exec-01
based_on_dispatch_contract: dispatch-contract-exec-01.yaml
must_fix:
  - review-issue-1
  - test-failure-1
advisory_only:
  - readability-suggestion-1
must_not_change:
  - scope boundary
  - unrelated files
acceptance:
  - review issues resolved
  - tests pass
patch_required_by: both
timeout_minutes: 20
```

### `dispatch contract` 与 `patch contract` 的关系

1. `patch contract` 继承原 `dispatch contract` 的边界
2. `patch contract` 只追加修补要求，不重写原任务目标
3. 若 patch 需要扩大边界，必须退回 `plan` 或升级正式流
4. `patch contract` 必须区分 `must_fix` 与 `advisory_only`

#### `patch contract` 生成规则

`patch contract` 的生成必须是仲裁结果的机械转写，而不是重新发明一个新任务：

1. `source_exec_unit` 必须指向一个已完成且进入复检的执行单元
2. `must_fix` 只允许来自 review/test 的 blocker 级问题
3. `advisory_only` 只允许来自不会改变边界的建议项
4. `must_not_change` 必须继承原 `dispatch contract` 的边界声明
5. `acceptance` 必须显式引用“哪些 blocker 被关闭”与“哪些验证重新通过”
6. 若同一问题无法归属到具体 `exec_unit`，则不得生成单元级 patch，必须退回任务级 `plan`

#### `patch contract` 生成错误码

| 错误码 | 含义 | 处理方式 |
|-------|------|---------|
| `ARIA-PATCH-001` | 无法定位 `source_exec_unit` | 退回任务级仲裁 |
| `ARIA-PATCH-002` | `must_fix` 混入建议项或越界项 | 重新仲裁 |
| `ARIA-PATCH-003` | patch 需要扩大原边界 | 退回 `plan` 或 `spec` |
| `ARIA-PATCH-004` | 修补目标无法映射到已存在问题 ID | 阻断 patch 生成 |

## OpenSpec 与 superpowers 的结合方式

`Aria` 不是二选一使用 `OpenSpec` 与 `superpowers`，而是按职责层分工结合使用。

### 总原则

`OpenSpec` 定义正式任务是否合法、边界在哪里。  
`superpowers` 定义任务如何被高质量地思考、拆解、执行、验证。  
`Cadence-Aria` 负责在正确状态下编排两者。

### 角色到能力映射

> **v1.4.2 修正**：恢复 `capability_id` 的"自动调用映射"语义，同时增加 `execution_mode` 字段区分两类 skills：> - `auto`：自动执行型 skills，由 Aria 在对应状态节点**自动调用**> - `interactive`：多轮交互型 skills，由 Aria 提示用户调用

#### 能力抽象层定义

| `capability_id` | 用途 | 一期默认映射的 superpowers skill | 必须性 | execution_mode |
|-----------------|------|--------------------------------|:---:|:--------------:|
| `capability.brainstorm` | 需求澄清、创意探索 | `brainstorming` | 推荐 | `interactive` |
| `capability.plan` | 制定执行计划 | `writing-plans` | 必须 | `auto` |
| `capability.dispatch` | 并行任务调度 | `dispatching-parallel-agents` | 推荐 | `auto` |
| `capability.subagent` | 子代理驱动开发 | `subagent-driven-development` | 推荐 | `auto` |
| `capability.execute` | 执行计划实现 | `executing-plans` | 必须 | `auto` |
| `capability.tdd` | 测试驱动开发 | `test-driven-development` | 可选 | `auto` |
| `capability.review` | 代码审查 | `requesting-code-review` | 必须 | `auto` |
| `capability.verify` | 完成前验证 | `verification-before-completion` | 必须 | `auto` |
| `capability.debug` | 系统化调试 | `systematic-debugging` | 推荐 | `auto` |
| `capability.receive-review` | 接收审查反馈 | `receiving-code-review` | 推荐 | `auto` |

#### 能力映射配置

能力 ID 到 skill 的映射存储在 `cadence/cache/aria/config.yaml` 中，供 orchestrator 调用和 PromptService 生成提示时使用：

```yaml
# 能力映射配置（自动调用映射，带 execution_mode 区分）
capability_mapping:
  capability.brainstorm:
    skills: ["brainstorming"]
    execution_mode: "interactive"
  capability.plan:
    skills: ["writing-plans"]
    execution_mode: "auto"
  capability.dispatch:
    skills: ["dispatching-parallel-agents"]
    execution_mode: "auto"
  capability.subagent:
    skills: ["subagent-driven-development"]
    execution_mode: "auto"
  capability.execute:
    skills: ["executing-plans"]
    execution_mode: "auto"
  capability.tdd:
    skills: ["test-driven-development"]
    execution_mode: "auto"
  capability.review:
    skills: ["requesting-code-review"]
    execution_mode: "auto"
  capability.verify:
    skills: ["verification-before-completion"]
    execution_mode: "auto"
  capability.debug:
    skills: ["systematic-debugging"]
    execution_mode: "auto"
  capability.receive-review:
    skills: ["receiving-code-review"]
    execution_mode: "auto"
```

#### 角色到能力 ID 映射

状态机守卫和 PromptService 只引用 `capability_id`，不硬编码具体 skill 名称：

| 角色 | OpenSpec 使用方式 | 需要的 capability_id | 调用方式 |
|------|-------------------|---------------------|---------|
| `intake` | 不直接产出 OpenSpec 工件 | `capability.brainstorm`（推荐） | 用户可自愿调用（interactive） |
| `spec` | 强制进入正式 change 主线 | `capability.brainstorm`（推荐） | 用户可自愿调用（interactive） |
| `plan` | 读取 proposal/design/tasks | `capability.plan`（必须）、`capability.brainstorm`（推荐） | **自动调用**（auto）；MVP 可带确认点 |
| `dispatch` | 读取 tasks 和 plan 输出 | `capability.dispatch`（必须）、`capability.subagent`（推荐） | 由 orchestrator 自动完成 |
| `exec` | 遵守既定边界，不改写工件 | `capability.execute`（必须）、`capability.tdd`（可选） | 由 Codex 自动执行 |
| `review` | 以 OpenSpec 边界为审查基线 | `capability.review`（必须） | **自动调用**（auto）；MVP 可带确认点 |
| `test` | 以 plan 验证策略为验证基线 | `capability.verify`（必须）、`capability.debug`（推荐） | **自动调用**（auto）；MVP 可带确认点 |
| `patch` | 服从原边界，不改写工件 | `capability.debug`（必须）、`capability.receive-review`（必须）、`capability.tdd`（可选） | 由 Codex 自动执行 |

#### 能力探测与守卫

1. 状态机守卫只检查 `capability_id` 是否 `available`，不检查具体 skill 名称
2. 能力探测时，通过配置文件查找 `capability_id` 对应的 skill 名称列表，逐个探测可用性
3. 若映射配置中某个 `capability_id` 对应的 skill 全部不可用，则该 capability 标记为 `unavailable`
4. **自动调用型 skills**（`execution_mode: auto`）：orchestrator 在对应状态节点直接通过 `Skill` 工具加载并执行，通过检测产出工件推进状态
5. **交互型 skills**（`execution_mode: interactive`）：`PromptService` 生成调用指引，等待用户手动触发
6. MVP 阶段：自动调用型 skills 可在调用前增加用户确认提示，但最终版本应直接自动调用
7. 用户可通过修改配置文件将 `capability_id` 映射到不同的 skill 实现，无需修改 Aria 代码

## Runtime Schemas

为避免实现时在正文中来回查找字段，一期补充统一的 schema 视图。这里的 schema 用于：

1. 约束字段名、类型、必填性
2. 明确哪些字段可为空、哪些字段只能由系统生成
3. 为后续拆分独立 schema 文档提供母版

详细配套文档见：

- `cadence/designs/2026-04-16_配套设计_Runtime-Schemas_Cadence-Aria_v1.0.md`

### `state.yaml` schema 摘要

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `task_id` | string | 是 | `aria-YYYYMMDD-NNN` |
| `source` | enum | 是 | `vk \| native \| aria-native` |
| `flow_type` | enum | 是 | `formal \| fast-lane` |
| `status` | enum | 是 | 状态机中定义的合法状态 |
| `current_round` | integer | 是 | `>= 1` |
| `review_status` | enum | 是 | `pending \| passed \| failed` |
| `test_status` | enum | 是 | `pending \| passed \| failed` |
| `patch_required_by` | enum | 是 | `none \| review \| test \| both` |
| `patch_round` | integer | 是 | `>= 0` |
| `exec_units` | map | 是 | key 为 `exec-xx` |
| `patch_units` | map | 否 | key 为 `patch-xx` |
| `created_at` | datetime string | 是 | ISO 8601 |
| `updated_at` | datetime string | 是 | ISO 8601 |

#### `exec_units.<id>` schema

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `status` | enum | 是 | `pending \| running \| succeeded \| failed \| timeout \| cancelled \| blocked` |
| `contract_path` | string | 是 | 指向 `dispatch contract` |
| `worktree_ref` | string | 否 | 并行/隔离模式下建议非空 |
| `attempt` | integer | 是 | `>= 0` |
| `exit_code` | integer/null | 是 | 未结束时可为 `null` |
| `result_path` | string | 是 | 指向 exec result |
| `started_at` | datetime string | 否 | 未开始可空 |
| `finished_at` | datetime string | 否 | 未结束可空 |
| `blocked_by` | string[] | 是 | 依赖的上游 exec unit 列表 |

#### `patch_units.<id>` schema

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `status` | enum | 是 | `pending \| running \| succeeded \| failed \| cancelled` |
| `based_on_exec_unit` | string | 是 | 必须引用已存在的 `exec-xx` |
| `contract_path` | string | 是 | 指向 `patch contract` |
| `attempt` | integer | 是 | `>= 0` |
| `started_at` | datetime string | 否 | 未开始可空 |
| `finished_at` | datetime string | 否 | 未结束可空 |

### `task intake card` schema 摘要

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `task_id` | string | 是 | 与 `state.yaml.task_id` 一致 |
| `source` | enum | 是 | `vk \| native \| aria-native` |
| `flow_type_suggestion` | enum | 是 | `formal \| fast-lane` |
| `risk_level` | enum | 是 | `low \| medium \| high` |
| `scope_summary` | string | 是 | 非空 |
| `boundary_check` | object | 是 | 包含布尔边界判定字段 |
| `created_at` | datetime string | 是 | ISO 8601 |

### `plan brief` schema 摘要

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `plan_id` | string | 是 | `plan-<task-id>` 或其变体 |
| `task_id` | string | 是 | 必须映射现有任务 |
| `quality_gates` | object[] | 是 | 至少 1 项 |
| `exec_unit_count` | integer | 是 | `>= 1` |
| `parallel_candidates` | array | 否 | 每项为 exec unit 组 |
| `acceptance_strategy` | enum/string | 是 | 一期至少支持 `all_units_pass` |
| `generated_at` | datetime string | 是 | ISO 8601 |

### `review report` / `test report` schema 摘要

#### `review report`

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `task_id` | string | 是 | 必须映射现有任务 |
| `exec_units_reviewed` | string[] | 是 | 不得为空 |
| `blockers` | object[] | 否 | 每项必须有 `issue_id` 与 `severity` |
| `suggestions` | object[] | 否 | 建议项不得进入 `must_fix` |
| `verdict` | enum | 是 | `passed \| failed \| needs_patch` |
| `reviewed_at` | datetime string | 是 | ISO 8601 |

#### `test report`

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `task_id` | string | 是 | 必须映射现有任务 |
| `exec_units_tested` | string[] | 是 | 不得为空 |
| `failures` | object[] | 否 | 失败时必须包含 `test_command` 与 `evidence` |
| `passed_count` | integer | 是 | `>= 0` |
| `failed_count` | integer | 是 | `>= 0` |
| `verdict` | enum | 是 | `passed \| failed` |
| `tested_at` | datetime string | 是 | ISO 8601 |

### `dispatch contract` / `patch contract` schema 摘要

#### 共享字段

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `contract_version` | string | 是 | 一期固定 `1.0` |
| `generated_at` | datetime string | 是 | ISO 8601 |
| `base_revision` | string | 是 | Git revision |
| `input_artifacts` | object | 是 | 路径必须存在于当前任务工件集中 |
| `generated_from_plan` | string | 是 | 指向 `plan_id` |
| `source_task_refs` | string[] | 是 | 至少 1 项 |
| `task_id` | string | 是 | 必须映射现有任务 |
| `timeout_minutes` | integer | 是 | `> 0` 且受配置上限约束 |

#### `dispatch contract`

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `exec_unit_id` | string | 是 | `exec-xx` |
| `parent_task` | string | 是 | 映射 OpenSpec task |
| `mode` | enum | 是 | 一期固定 `exec` |
| `scope` | object | 是 | 至少包含 `files_allowed` |
| `goal` | string | 是 | 非空 |
| `acceptance` | string[] | 是 | 至少 1 项 |
| `dependencies` | string[] | 是 | 可为空数组 |
| `worktree_ref` | string | 否 | 并行模式建议非空 |
| `result_path` | string | 是 | 当前任务内唯一 |
| `retry_allowed` | boolean | 是 | 显式指定 |

#### `patch contract`

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `patch_unit_id` | string | 是 | `patch-xx` |
| `source_exec_unit` | string | 是 | 必须引用已存在 `exec-xx` |
| `based_on_dispatch_contract` | string | 是 | 指向原始 dispatch contract |
| `must_fix` | string[] | 是 | 至少 1 项，且均为 blocker 问题 ID |
| `advisory_only` | string[] | 否 | 可为空 |
| `must_not_change` | string[] | 是 | 至少 1 项 |
| `acceptance` | string[] | 是 | 至少 1 项 |
| `patch_required_by` | enum | 是 | `review \| test \| both` |

### `verification summary` / `closure summary` schema 建议

一期这两个工件仍以 Markdown 为主，但建议在 front matter 中补最小结构字段，避免后续无法做自动聚合。

#### 建议最小字段

| 工件 | 字段 | 类型 | 必填 |
|------|------|------|------|
| `verification summary` | `task_id` | string | 是 |
| `verification summary` | `review_verdict` | enum | 是 |
| `verification summary` | `test_verdict` | enum | 是 |
| `verification summary` | `final_patch_round` | integer | 是 |
| `closure summary` | `task_id` | string | 是 |
| `closure summary` | `final_status` | enum | 是 |
| `closure summary` | `completed_at` | datetime string | 是 |
| `closure summary` | `recovery_actions` | string[] | 否 |

### schema 约束原则

1. 同名字段跨工件应保持同一语义，不允许在不同工件中复用为不同含义
2. 所有时间字段统一使用 ISO 8601 字符串
3. 所有路径字段统一使用仓库相对路径
4. 所有 ID 字段必须可回溯到单一任务、单一执行单元或单一问题项
5. schema 是实现约束，不是展示文案；展示性内容应放 Markdown 正文

## 质量门定义

质量门由 `plan` 角色定义，`orchestrator` 负责执行与校验。

### 质量门分层

1. 全局默认质量门
2. 任务类型质量门
3. 单任务覆盖性质量门

### 默认质量门

| 任务类型 | 质量门 |
|--------|-------|
| 文档/规则/配置 | 格式正确、引用有效、结构一致、必要 dry-run 通过 |
| 脚本/自动化 | 命令可运行、关键路径验证通过 |
| 代码类 | 变更符合边界、验证命令通过、必要测试通过 |

### 质量门归属

1. `plan` 定义任务级质量门
2. `review` 校验实现质量
3. `test` 校验验证质量
4. `orchestrator` 汇总是否满足“通过”条件

### 验证命令来源规则

为避免 `test` 阶段临时猜命令，一期要求验证命令必须有明确来源：

1. 优先使用 OpenSpec task 或 plan 中已显式声明的验证命令
2. 若 OpenSpec 未声明，则使用仓库既有标准命令，并在 `plan brief` 中固化
3. `test report.failures[].test_command` 必须来自实际执行的命令，不允许事后推测
4. 对文档/规则类任务，允许将 `test_command` 记为 `dry-run`、`lint` 或 `manual-checklist`，但必须在 report 中写明证据
5. 若验证命令来源不明确，`plan` 不得通过，必须补齐后再进入 `dispatch`

### 验证命令错误码

| 错误码 | 含义 | 处理方式 |
|-------|------|---------|
| `ARIA-VERIFY-001` | 验证命令来源缺失 | 退回 `plan` |
| `ARIA-VERIFY-002` | 实际执行命令与 plan 不一致 | 标记验证无效，要求重测 |
| `ARIA-VERIFY-003` | 文档类任务无证据支撑 `manual-checklist` | 标记测试报告不完整 |

## 错误处理策略

### 错误分类

1. 前置依赖错误
2. OpenSpec 工件错误
3. Codex 执行错误
4. review/test 失败
5. 状态损坏错误
6. 用户取消

### 处理规则

| 错误类型 | 处理方式 |
|---------|---------|
| 前置依赖错误 | 阻断流转，给出明确缺失项 |
| OpenSpec 创建失败 | 回退到 `spec-required`，等待修复 |
| Codex 执行失败 | 标记执行单元失败，可重试 |
| review/test 失败 | 进入 `patching` |
| 状态损坏 | 停止继续执行，要求人工恢复或重建 |
| 用户取消 | 标记为 `cancelled`，保留当前工件 |

### 错误码分层

为避免日志、状态机和报告使用不同表述，一期建议统一错误码分层：

| 前缀 | 层级 | 示例场景 |
|------|------|---------|
| `ARIA-CAP-*` | 能力探测层 | 外部依赖缺失、宿主能力不足 |
| `ARIA-DISPATCH-*` | contract 生成层 | ownership 缺失、依赖图错误 |
| `ARIA-PATCH-*` | patch 生成层 | 修补目标定位失败、越界修补 |
| `ARIA-VERIFY-*` | 验证层 | 验证命令来源缺失、验证证据不足 |
| `ARIA-EXEC-*` | 执行层 | Codex 启动失败、结果文件缺失、超时 |
| `ARIA-STATE-*` | 状态机层 | 非法状态转换、恢复不一致、工件损坏 |
| `ARIA-SYNC-*` | 外部同步层 | VK 推送失败、映射缺失、幂等冲突 |

### 执行与同步错误码示例

| 错误码 | 含义 | 处理方式 |
|-------|------|---------|
| `ARIA-EXEC-001` | Codex 进程启动失败 | 标记执行单元失败 |
| `ARIA-EXEC-002` | 结果文件缺失 | 视为失败，不进入 review/test |
| `ARIA-EXEC-003` | 执行超时 | 标记 `timeout`，允许重试 |
| `ARIA-STATE-001` | 非法状态转换 | 阻断流转并写错误日志 |
| `ARIA-STATE-002` | state.yaml 与工件不一致 | 停止恢复，要求人工处理 |
| `ARIA-SYNC-001` | 外部任务映射缺失 | 禁用该任务的 VK 同步 |
| `ARIA-SYNC-002` | VK 推送失败 | 记录 `warn`，主流程继续 |

### patch 循环上限

为避免无限修补循环，一期建议：

1. 默认最多 `2` 轮 patch，统计口径以 `task-id` 为单位
2. `patch_round` 的计数语义为**任务中任意执行单元的最大 patch 轮次**（取最大值，非累计值）
   - 例如：exec-01 需要 2 轮 patch，exec-02 需要 1 轮 patch → task 的 `patch_round` = 2
3. 超过上限后必须退回 `plan`
4. 若边界已失效，则退回 `spec`

## 配置管理

### 配置文件位置

一期采用项目级配置文件，存储在 `cadence/cache/aria/config.yaml`：

```text
cadence/cache/aria/
  config.yaml          # 项目级配置（不入版本控制）
```

### 配置优先级

配置值按以下优先级覆盖（高 → 低）：

1. **任务级 contract 参数**：dispatch contract 或 patch contract 中的显式设定
2. **项目级配置**：`cadence/cache/aria/config.yaml`
3. **全局默认值**：Aria runtime 内置的默认值

### 配置项清单

```yaml
# Cadence-Aria 配置文件示例
# 存储位置：cadence/cache/aria/config.yaml

# 执行配置
exec:
  max_parallel: 2           # exec 并行数默认值
  max_parallel_limit: 4     # exec 并行数上限
  default_timeout_minutes: 30
  max_timeout_minutes: 120

# 修补配置
patch:
  max_rounds: 2             # patch 循环上限
  default_timeout_minutes: 20
  max_timeout_minutes: 60

# 重试配置
retry:
  max_attempts: 3           # 单个执行单元最大重试次数

# 质量门配置
quality_gates:
  code:
    test_coverage_threshold: 80
    format_check: true
  doc:
    dry_run: true
    link_check: false
  script:
    critical_path_only: true

# 日志配置
logging:
  level: info               # debug | info | warn | error
  max_file_size_mb: 10      # 单个日志文件最大体积
  max_files: 5              # 日志文件轮转上限

# Worktree 配置
worktree:
  cleanup_on_done: false    # 任务完成后是否自动清理 worktree
  cleanup_on_cancel: true   # 取消后是否自动清理 worktree

# Vibe Kanban 配置
vibe_kanban:
  sync_enabled: true        # 是否启用 VK 状态同步
  sync_timeout_seconds: 10  # VK 同步超时时间
```

### 配置文件生命周期

1. **首次运行**：`Aria runtime` 启动时检测配置文件，不存在则使用默认值并写入默认配置
2. **读取时机**：每次任务启动、恢复、重试时读取
3. **修改方式**：用户直接编辑 `config.yaml`，Aria 不提供命令行修改入口
4. **不入版本控制**：`config.yaml` 位于 `cadence/cache/` 下，受 `.gitignore` 保护

### 任务标识

每个任务必须有唯一 `task-id`。所有 `status`、`result`、恢复、取消操作都以 `task-id` 为主键。

### `aria:start`

`aria:start` 的结束状态固定为 `planned`。用户确认计划后，由 `aria:run --task-id <id>` 进入 `dispatched`。

### 计划修改语义

为避免把“终止执行”和“重开计划”混用，一期增加显式的计划重开语义：

1. `planned` 状态下如需修改计划，不使用 `aria:cancel`
2. 用户可重新执行 `aria:start --task-id <id> --replan`
3. `--replan` 只允许在 `planned`、`spec-approved`、`upgrade-blocked` 状态使用
4. `--replan` 会保留原 `task-id`，生成新的 `plan_id`
5. 旧 `plan brief` 与旧 contract 不删除，转入 `superseded` 状态，供审计与回溯
6. 若任务已进入 `executing` 及之后状态，想改边界必须先显式取消或退回

#### `--replan` 的状态语义

```text
planned
  -> aria:start --task-id <id> --replan
  -> spec-approved 或 planned（重算计划）
```

该示例要求：

1. `task_id` 不变
2. 原计划不删除
3. 只有 `plan_id` 更新

#### `aria:run` 参数精确定义

| 参数 | 作用 |
|------|------|
| `--task-id <id>` | 指定要运行或恢复的任务 ID，必须参数 |
| `--resume` | 从当前状态继续执行，不重新生成 dispatch contract |
| `--retry-failed` | 仅重试状态为 `failed`、`timeout`、`cancelled` 的执行单元 |

**使用示例**：

```text
# 首次运行
aria:run --task-id aria-20260415-001

# 中断后恢复
aria:run --task-id aria-20260415-001 --resume

# 失败后仅重试失败单元
aria:run --task-id aria-20260415-001 --retry-failed
```

#### `aria:status` 参数补充

| 参数 | 作用 |
|------|------|
| `--task-id <id>` | 精确查询指定任务 |
| `--sync` | 对支持外部映射的任务执行一次显式状态重投影 |

#### `aria:cancel` 行为定义

1. 向所有 `running` 状态的 `exec` 和 `patch` 进程发送取消信号
2. 将任务状态标记为 `cancelled`
3. 保留已产生的运行时工件
4. 被取消的执行单元状态改为 `cancelled`，下游依赖单元标记为 `blocked`

#### `aria:retry` 行为定义

1. 仅对 `failed`、`timeout`、`cancelled` 状态的执行单元或 patch 单元生效
2. 重试时 `attempt` 计数 +1
3. 不改变原有 contract 内容，仅重新调度执行
4. 若重试次数超过配置上限，提示用户退回 `plan` 或 `spec`

### 交互示例约束

1. 命令输出应优先展示 task_id、status、next
2. 未完成任务的 `aria:result` 只能返回当前摘要，不能伪造最终结果
3. 所有示例中的状态变化必须符合状态机守卫条件
4. 命令输出中的字段名应尽量复用 runtime schema 与 state.yaml 字段名

## 验证与验收策略

### 一期最简验收场景

至少要验证以下 3 类场景：

1. `native intake -> formal flow -> done`
2. `vk intake -> formal flow -> patch -> verified`
3. `fast-lane -> done` 与 `fast-lane -> upgrade`

### 端到端验证目标

1. 状态能跨会话恢复
2. `dispatch contract` 与 `patch contract` 能被 `Aria runtime` 正确解释并转写给 `Codex`
3. review/test 报告能驱动 patch
4. 多任务状态不会串扰
5. 失败与取消可正确落盘
6. capability report 能正确驱动阻断与降级
7. 并行假设失败时能自动退化到串行模式
8. 外部同步失败不会污染主状态机

### 核心假设验证清单

> **v1.4 评审补充**：在启动任何实现代码前，必须先验证以下三个核心假设。若任一假设不成立，v1.4 的底层架构可能需要推翻或大幅收缩。
> **v1.4.2 修正**：假设 1 的验证结论已更新，详见 `cadence/cache/aria/verification/2026-04-16_核心假设验证报告_v1.1.md`。

#### 假设 1：superpowers 可被程序代码自动调用

**验证目的**：确认 superpowers skills（如 `brainstorming`、`writing-plans`）能否在 Aria 运行时被自动触发，而不仅依赖用户手动输入 `/skill-name`。

**验证步骤**：
1. 使用 `Skill` 工具调用 `brainstorming` skill，观察其行为模式
2. 使用 `Skill` 工具调用 `writing-plans`、`requesting-code-review`、`executing-plans`、`subagent-driven-development` 等 workflow skills
3. 观察各 skill 加载后的行为：是自动执行完成，还是需要用户多轮交互

**分类结果**：
- **自动执行型 skills**：`writing-plans`、`executing-plans`、`requesting-code-review`、`verification-before-completion`、`subagent-driven-development` 等。加载后由 agent 自动执行，无需用户逐轮确认，可被 orchestrator 自动调用。
- **多轮交互型 skills**：`brainstorming` 等。需要用户一问一答交互，不适合自动调用。

**结论**：
- 自动执行型 skills **可以被自动调用**，orchestrator 可在对应状态节点自动加载并执行
- 多轮交互型 skills 保留用户交互，在需求澄清等阶段由用户主动调用
- MVP 阶段可在关键自动调用节点增加用户确认提示，最终目标是全自动编排

**失败处理**：
- 若自动执行型 skills 调用后无法正确产出工件：增加更多的人工确认点，或在特定状态退化为提示用户手动调用
- 多轮交互型 skills 始终由用户手动调用

---

#### 假设 2：Claude Code Bash 工具能稳定启动后台进程

**验证目的**：确认通过 Claude Code Bash 工具启动的后台进程是否能在当前对话/会话关闭后继续存活，且能被外部探测和管理。

**验证步骤**：
1. 在当前仓库创建一个临时后台脚本（如 `sleep 60`）
2. 通过 Bash 工具的 `run_in_background` 启动该脚本，记录 `pid`
3. 等待 5 秒后，通过 `kill -0 <pid>` 确认进程存活
4. 关闭当前 Claude Code 会话（或等待会话因超时而结束）
5. 重新进入 Claude Code，再次执行 `kill -0 <pid>` 确认进程是否仍在运行
6. 向该进程发送 `SIGTERM` 并观察是否能在 5 秒内退出

**通过标准**：
- 步骤 3 中进程存活
- 步骤 5 中进程仍然存活（证明会话关闭不终止后台进程）
- 步骤 6 中进程能被信号正常终止

**失败标准**：
- 步骤 3 中进程未启动或已退出
- 步骤 5 中进程已消失（证明后台进程随会话结束而终止）
- 无法向进程发送信号或进程不响应信号

**失败处理**：
- 将并行执行模型标记为**远期目标**
- 一期默认采用**串行前台执行**：`dispatch` 仍切分多个执行单元，但按序逐个启动 Codex 进程
- `exec.max_parallel` 固定为 `1`

---

#### 假设 3：Codex CLI 可用且支持 prompt 注入

**验证目的**：确认 Codex CLI 在当前环境中已安装、可执行，且支持通过命令行参数或文件方式注入 prompt。

**验证步骤**：
1. 在终端执行 `codex --version` 或 `codex --help`，确认 CLI 已安装
2. 执行 `codex --prompt "Write a one-line greeting to /tmp/codex-test.txt"`（或 Codex CLI 实际支持的等效参数）
3. 若 `--prompt` 不支持，尝试将 prompt 写入临时文件后使用 `--prompt-file <path>`
4. 观察 Codex 是否正常执行、是否生成了预期文件

**通过标准**：
- `codex` 命令可执行且不报错
- `--prompt` 或 `--prompt-file` 至少一种方式可用
- Codex 能按注入的 prompt 完成最小任务并产出结果

**失败标准**：
- `codex` 命令不存在或未安装
- `codex` 不支持 `--prompt` 也不支持 `--prompt-file`
- Codex 执行后无输出或无法理解注入的指令

**失败处理**：
- 把 prompt 注入方式从"命令行参数"改为纯"文件系统契约"：Aria 将 prompt 写入任务目录文件，Codex adapter 通过 `--prompt-file` 或工作目录约定来读取
- 若 Codex CLI 完全不可用，则需要**重新评估是否继续该项目**或寻找替代执行引擎

---

#### 验证执行顺序与时间安排

| 顺序 | 假设 | 预计耗时 | 阻塞性 |
|------|------|----------|--------|
| 1 | superpowers 自动调用 | 1-2 小时 | **高**（决定 orchestrator 模式） |
| 2 | Bash 后台进程稳定性 | 1 小时 | **高**（决定并行/串行模型） |
| 3 | Codex CLI 可用性 | 30 分钟 | **高**（决定执行引擎选择） |

#### 实际验证结果（2026-04-16）

> **v1.4.2 修正**：核心假设验证已在本日完成，详细报告见 `cadence/cache/aria/verification/2026-04-16_核心假设验证报告_v1.1.md`。

| 假设 | 结果 | 关键发现 |
|------|------|----------|
| 假设 1：superpowers 自动调用 | **成立（需分类处理）** | 自动执行型 skills 可被自动调用；多轮交互型 skills（如 brainstorming）需要用户介入 |
| 假设 2：Bash 后台进程稳定性 | **大概率成立** | `nohup` 和 `run_in_background` 都能启动独立存活的后台进程 |
| 假设 3：Codex CLI prompt 注入 | **成立** | Codex CLI v0.121.0 已安装，`codex exec "prompt"` 可直接注入并执行 |

**基于验证结果的方案调整**：
1. 假设 1 的结论支持 orchestrator 恢复"全自动编排"定位，自动执行型 skills 由 Aria 自动调用，仅 brainstorming 等交互型 skills 保留用户介入
2. 假设 2 的通过意味着并行模型可继续推进，但需在 P0 中增加真实场景二次确认
3. 假设 3 的通过意味着 Codex adapter 设计无需调整

**规则**：
- 三个假设全部验证通过，方可进入 P0 的代码实现
- 任一假设失败，必须先根据"失败处理"调整方案，再进入实现
- 验证过程本身必须记录验证报告，落盘到 `cadence/cache/aria/verification/` 目录

### MVP 前置验证

在全面开发前，必须先跑通一个最小闭环原型：

```text
Claude Code 生成 dispatch contract
  -> Aria runtime 创建 worktree
  -> Aria runtime 调用 Codex CLI
  -> Codex 在受控 prompt 下完成一次最小修改
  -> Aria runtime 回写执行结果
  -> Claude Code 生成 review/test 报告
```

若该 MVP 不能在"薄适配器"前提下稳定跑通，则一期必须降级，而不是继续放大 `Codex runtime` 假设。

#### MVP 通过准则

只有同时满足以下条件，MVP 才算通过：

1. 单轮 `exec` 能稳定完成，且结果、diff、状态三者一致
2. PromptService 能在正确状态输出准确的自动调用提示或确认提示
3. Aria 能自动调用 superpowers skills 并正确读取工件、推进状态（MVP 允许带确认点）
4. 至少 1 个失败场景能稳定进入 `patch`
5. capability report 中的降级项能触发预期降级，而不是直接崩溃
6. 状态恢复后不会重复调度已完成单元
7. 文件越界校验能拦截至少 1 个越界样例

#### MVP 失败退出准则

出现以下任一情况，一期应立即收缩范围，而不是继续扩大实现：

1. 无法稳定完成单轮 Codex 调用与结果回写
2. 状态恢复后经常出现重复执行或状态漂移
3. 后台进程模型在当前宿主不稳定
4. 外部依赖探测无法形成稳定 capability report

#### 验测异常路径

除主路径外，一期 MVP 必须显式验证以下异常路径：

1. `OpenSpec` 缺失，formal flow 被阻断
2. `superpowers` 验证能力缺失，`review/test` 被阻断
3. `Codex adapter` 仅支持单轮执行时，系统正确降级
4. `Vibe Kanban` 推送失败，主流程继续
5. state.yaml 与工件不一致时，系统停止并报告损坏

## 与现有目录规则的关系

`cadence-aria/` 作为插件源码目录，不属于 `cadence/` 文档产物目录。

### 边界

1. `cadence-aria/`：插件源码、命令、skill、runtime 模板
2. `cadence/`：设计文档、计划文档、评审文档、运行阶段输出文档

### 运行时状态目录

一期建议将运行时状态落在：

`cadence/cache/aria/`

原因：

1. 符合当前项目对 Cadence 产物的集中存储约束
2. 避免将运行时状态散落在插件源码目录
3. 便于后续清理、恢复与调试

### `.gitignore` 规则

`cadence/cache/aria/` 目录下的运行时状态文件**不应提交到版本控制**。必须在项目根目录 `.gitignore` 中添加以下规则：

```gitignore
# Aria 运行时状态（不入版本控制）
cadence/cache/aria/tasks/
cadence/cache/aria/logs/
cadence/cache/aria/config.yaml
```

**例外处理**：

1. **closure summary**：任务完成后的 `closure summary` 如果需要长期保留，可以在 `done` 状态后自动复制到 `cadence/reports/` 目录（该目录入版本控制）
2. **config.yaml 模板**：Aria 提供一个 `config.example.yaml` 模板文件（入版本控制），供用户参考
3. **调试场景**：开发者如需临时提交运行时状态用于问题排查，应手动 `git add -f` 而非修改 `.gitignore`

### 文件范围安全校验

`files_allowed` 和 `files_blocked` 是通过 prompt 注入传达给 Codex 的**软约束**。为增强安全保障，exec 完成后增加**硬校验**：

1. `orchestrator` 在 exec 完成后执行 `git diff --name-only` 获取实际变更文件列表
2. 将变更文件列表与 dispatch contract 中的 `files_allowed` 和 `files_blocked` 比对
3. **越界变更**（修改了 `files_blocked` 中的文件，或创建了 `files_allowed` 之外的新文件）自动标记为 review 的 `blocker`
4. 校验结果写入 review report 的 `scope_violations` 字段

```text
exec 完成
  -> git diff --name-only
  -> 与 files_allowed 比对
  -> 与 files_blocked 比对
  -> 无越界 -> 正常进入 review/test
  -> 有越界 -> 标记为 blocker，写入 review report
```

## 目录结构建议

建议为 `Cadence-Aria` 建立独立目录：

详细配套文档见：

- `cadence/designs/2026-04-16_配套设计_Implementation-Layout_Cadence-Aria_v1.0.md`

```text
cadence-aria/
  commands/
  skills/
  references/
  templates/
  runtime/
    contracts/
    states/
    reports/
    adapters/
  codex/
    roles/
    prompts/
    workflows/
    templates/
  docs/
```

### 目录职责

| 目录 | 职责 |
|------|------|
| `commands/` | 用户可见的工作流入口 |
| `skills/` | Aria 自己的编排 skill |
| `references/` | 角色矩阵、兼容说明、依赖说明 |
| `templates/` | Aria 自有运行时工件模板 |
| `runtime/contracts/` | 角色输入输出协议 |
| `runtime/states/` | 状态机与升级规则 |
| `runtime/reports/` | 汇总与闭环摘要格式 |
| `runtime/adapters/` | Claude 与 Codex 的本地桥接逻辑 |
| `codex/roles/` | Codex 侧角色定义 |
| `codex/prompts/` | Codex 侧角色提示与边界约束 |
| `codex/workflows/` | Codex 执行与修补流程 |
| `codex/templates/` | Codex 输出模板 |


## 工件边界

### OpenSpec 工件

`OpenSpec` 继续负责：

- `proposal`
- `design`
- `tasks`

这些工件是正式任务的源事实。

### Aria 运行时工件

`Aria` 应定义自己的运行时工件：

1. `task intake card`
2. `plan brief`
3. `dispatch contract`
4. `review report`
5. `test report`
6. `patch contract`
7. `verification summary`
8. `closure summary`

这些工件回答的是运行时问题，而不是正式边界问题。


## 命令面与最小可用工作流

一期命令面遵循以下原则：

1. 命令少而稳
2. 命令表达阶段，不直接暴露内部角色
3. 角色由 `Aria` 内部编排

### 一期命令集

| 命令 | 作用 |
|------|------|
| `aria:intake` | 统一任务入口，决定 formal 或 fast-lane 建议 |
| `aria:start` | 启动正式流，驱动 `intake -> spec -> plan` |
| `aria:run` | 启动或继续正式执行流，驱动 `dispatch -> exec -> review/test -> patch` |
| `aria:fast` | 启动小修小补轻量流 |
| `aria:status` | 查看当前任务状态、质量门状态、来源与阻塞点 |
| `aria:result` | 输出当前任务闭环摘要 |
| `aria:cancel` | 取消指定任务或当前活跃任务，保留已产生工件 |
| `aria:retry` | 对指定任务的失败执行单元或 patch 单元进行重试 |
| `aria:doctor` | 诊断环境依赖，检测并修复能力缺失问题 |

### `aria:doctor` 行为定义

> **v1.4 新增**：根据设计评审建议，增加环境诊断与修复引导命令。

`aria:doctor` 是一个只读诊断命令，不修改任何状态，只输出结构化的诊断报告和修复建议。

#### 输出内容

````text
$ aria:doctor

[Aria Doctor] 环境诊断报告

✅ host.exec.foreground    — 可用（bash foreground command）
⚠️ host.exec.background    — 降级（后台执行受限，将使用串行模式）
✅ host.fs.read            — 可用
✅ host.fs.write           — 可用
✅ host.git.worktree       — 可用
✅ host.git.diff           — 可用
✅ openspec.change.create  — 可用（OpenSpec v0.3.1）
✅ openspec.artifact.read  — 可用
✅ capability.plan         — 可用（brainstorming: interactive, writing-plans: auto）
✅ capability.review       — 可用（requesting-code-review: auto）
✅ capability.verify       — 可用（verification-before-completion: auto）
✅ codex.exec.single       — 可用（codex CLI v1.2.0）
❌ vk.task.pull            — 不可用（VK 适配器未配置）

汇总：
- 正式流（formal flow）：允许
- 轻量流（fast-lane）：允许
- 并行执行：降级为串行
- VK 同步：禁用

修复建议：
1. [host.exec.background] 后台执行受限。可通过配置 exec.max_parallel=1 显式使用串行模式
2. [vk.task.pull] VK 适配器未配置。若不需要 VK 入口可忽略；否则请在 config.yaml 中配置 VK 连接信息
````

#### 参数

| 参数 | 作用 |
|------|------|
| `--fix` | 自动修复可自动修复的问题（如清理残留锁文件、重置错误状态） |
| `--verbose` | 输出详细诊断过程（含每个检测的 evidence 字段） |

#### `--fix` 可修复项

| 可修复项 | 行为 |
|---------|------|
| 残留锁文件 | 检查对应进程是否存活，不存活则清理 |
| 错误的 `running` 状态 | 将无活跃进程的 `running` 状态重置为 `failed` |
| 缺失的配置文件 | 生成默认 `config.yaml` |
| 缺失的目录结构 | 创建 `cadence/cache/aria/` 及子目录 |

#### `--fix` 禁止修复项

1. OpenSpec 工件格式不兼容
2. Codex CLI 版本不匹配
3. contract 与 OpenSpec 边界不一致
4. 任何涉及业务逻辑的状态修复

### `aria:run` 参数精确定义

| 参数 | 作用 |
|------|------|
| `--task-id <id>` | 指定要运行或恢复的任务 ID，必须参数 |
| `--resume` | 从当前状态继续执行，不重新生成 dispatch contract |
| `--retry-failed` | 仅重试状态为 `failed`、`timeout`、`cancelled` 的执行单元 |

**使用示例**：

```text
# 首次运行
aria:run --task-id aria-20260415-001

# 中断后恢复
aria:run --task-id aria-20260415-001 --resume

# 失败后仅重试失败单元
aria:run --task-id aria-20260415-001 --retry-failed
```

### `aria:status` 参数补充

| 参数 | 作用 |
|------|------|
| `--task-id <id>` | 精确查询指定任务 |
| `--sync` | 对支持外部映射的任务执行一次显式状态重投影 |

### `aria:cancel` 行为定义

1. 向所有 `running` 状态的 `exec` 和 `patch` 进程发送取消信号
2. 将任务状态标记为 `cancelled`
3. 保留已产生的运行时工件
4. 被取消的执行单元状态改为 `cancelled`，下游依赖单元标记为 `blocked`

### `aria:retry` 行为定义

1. 仅对 `failed`、`timeout`、`cancelled` 状态的执行单元或 patch 单元生效
2. 重试时 `attempt` 计数 +1
3. 不改变原有 contract 内容，仅重新调度执行
4. 若重试次数超过配置上限，提示用户退回 `plan` 或 `spec`

### 正式流

1. 用户进入 `aria:intake`
2. `Aria` 判定任务类型
3. 正式任务进入 `aria:start`
4. `Aria` 驱动 `spec` 与 `plan`
5. 用户确认计划后执行 `aria:run`
6. `Aria` 驱动 `dispatch`
7. `Codex exec` 执行
8. `Claude review` 与 `Claude test` 并行优先执行，必要时降级为分阶段
9. 失败则生成 `patch contract`
10. `Codex patch` 修补
11. 回到 `review/test`
12. 通过后生成 `verification summary` 与 `closure summary`

### fast-lane 流

1. 用户执行 `aria:fast`
2. `Aria` 做低风险判定
3. 通过后直接生成轻量执行协议
4. `Codex exec` 执行
5. `Claude review/testing-lite`
6. 成功后输出轻量闭环摘要
7. 超界则升级到正式流

### 命令交互示例

以下示例用于定义一期 CLI 的最小交互预期，重点是：

1. 命令输入
2. 关键输出摘要
3. 状态变化
4. 用户下一步动作

详细配套文档见：

- `cadence/designs/2026-04-16_配套设计_CLI-Interactions_Cadence-Aria_v1.0.md`

#### 示例 1：native formal flow

```text
$ aria:intake "为 Aria 增加 capability report 结构化输出"

[Aria]
- task_id: aria-20260416-001
- source: native
- flow_type_suggestion: formal
- risk_level: medium
- next: aria:start --task-id aria-20260416-001
```

```text
$ aria:start --task-id aria-20260416-001

[Aria]
- status: planned
- plan_id: plan-aria-20260416-001
- exec_unit_count: 2
- parallel_candidates: [exec-01, exec-02]
- quality_gates: [format_check, contract_validation]
- next: aria:run --task-id aria-20260416-001
```

```text
$ aria:run --task-id aria-20260416-001

[Aria]
- status: executing
- started_exec_units: [exec-01, exec-02]
- review_status: pending
- test_status: pending
```

```text
$ aria:result --task-id aria-20260416-001

[Aria]
- current_status: executing
- summary: "2 个执行单元运行中，暂无最终结果"
```

#### 示例 2：计划重开

```text
$ aria:start --task-id aria-20260416-001 --replan

[Aria]
- task_id: aria-20260416-001
- previous_plan_id: plan-aria-20260416-001
- new_plan_id: plan-aria-20260416-002
- superseded: [plan-aria-20260416-001]
- status: planned
```

该示例要求：

1. `task_id` 不变
2. 原计划不删除
3. 只有 `plan_id` 更新

#### 示例 3：patch 闭环

```text
$ aria:run --task-id aria-20260416-002

[Aria]
- status: reviewing/testing
- review_status: failed
- test_status: passed
- patch_required_by: review
- next: generating patch contract
```

```text
$ aria:status --task-id aria-20260416-002

[Aria]
- status: patching
- patch_unit: patch-01
- source_exec_unit: exec-02
- must_fix_count: 2
- advisory_only_count: 1
```

```text
$ aria:result --task-id aria-20260416-002

[Aria]
- final_status: done
- patch_round: 1
- review_status: passed
- test_status: passed
```

#### 示例 4：取消与恢复

```text
$ aria:cancel --task-id aria-20260416-003

[Aria]
- status: cancelled
- cancelled_exec_units: [exec-01]
- blocked_exec_units: [exec-02]
- artifacts_preserved: true
```

```text
$ aria:run --task-id aria-20260416-003 --resume

[Aria]
- resumed_from: cancelled
- retryable_units: [exec-01]
- blocked_units_recomputed: true
- next: user may run aria:retry --task-id aria-20260416-003
```

```text
$ aria:retry --task-id aria-20260416-003

[Aria]
- retried_units: [exec-01]
- unblocked_units: [exec-02]
- status: executing
```

#### 示例 5：VK 同步重投影

```text
$ aria:status --task-id aria-20260416-004 --sync

[Aria]
- task_id: aria-20260416-004
- internal_status: verified
- vk_sync: attempted
- vk_sync_result: success
```

若同步失败：

```text
[Aria]
- task_id: aria-20260416-004
- internal_status: verified
- vk_sync: failed
- error_code: ARIA-SYNC-002
- note: "主流程状态不受影响"
```

#### 示例 6：fast-lane 升级

```text
$ aria:fast "修复跨模块配置读取错误"

[Aria]
- task_id: aria-20260416-005
- flow_type_suggestion: fast-lane
- execution_result: upgrade-required
- reason: "cross_module=true"
- next: aria:start --task-id aria-20260416-005
```

### 交互示例约束

1. 命令输出应优先展示 task_id、status、next
2. 未完成任务的 `aria:result` 只能返回当前摘要，不能伪造最终结果
3. 所有示例中的状态变化必须符合状态机守卫条件
4. 命令输出中的字段名应尽量复用 runtime schema 与 state.yaml 字段名



## 与 Vibe Kanban 的关系

`Vibe Kanban` 在一期中只承担三类角色：

1. 任务来源
2. 工作区容器
3. 状态映射目标

为满足“既能接入，也能脱离”的目标，`Aria` 需要支持两类入口：

1. `vk intake`
2. `native intake`

两类入口统一汇入 `aria:intake`。

### 状态同步机制

一期采用**单向投影**模型：

1. **`Aria` 内部状态为主事实源**：任务的生命周期、状态流转、仲裁结论以 `Aria` 的 `state.yaml` 为准
2. **`Vibe Kanban` 仅做投影展示**：`Aria` 在关键状态节点主动向 `Vibe Kanban` 推送状态摘要（如 `spec-approved`、`executing`、`done`、`upgrade-blocked`）
3. **按需拉取作为补充**：`aria:intake` 从 `Vibe Kanban` 拉取任务列表和初始描述，但拉取后任务即在 `Aria` 内部独立流转
4. **同步失败不影响 Aria 内部闭环**：若向 `Vibe Kanban` 推送状态失败，`orchestrator` 记录告警日志并继续执行，不阻断主流程
5. **冲突处理**：若 `Vibe Kanban` 中的状态与 `Aria` 内部状态不一致，以 `Aria` 内部状态为准，`aria:status` 会提示存在外部状态漂移

### VK 适配契约

为降低对 `Vibe Kanban` 具体实现的耦合，一期只依赖以下适配字段：

| 字段 | 用途 | 是否必须 |
|------|------|---------|
| `external_task_id` | 外部任务主键映射 | 是 |
| `title` | intake 摘要 | 是 |
| `description` | 初始上下文 | 否 |
| `source_workspace` | 来源工作区 | 否 |
| `status_projection` | 外部展示状态 | 否 |
| `last_synced_at` | 幂等同步时间戳 | 否 |

#### 同步幂等规则

1. `Aria` 内部使用 `task-id + target-status` 作为幂等键
2. 重复推送同一状态时，适配器应覆盖同一投影，而不是追加新记录
3. 外部状态回写失败不重置内部状态
4. 若外部 `external_task_id` 缺失，则禁止走 `vk intake`

#### 漺移修复规则

1. 发现外部状态漂移时，只提示，不自动回写纠正
2. 用户执行 `aria:status --sync` 时，可触发一次显式重投影
3. 重投影只同步摘要与状态，不回写内部运行时细节

#### VK 适配错误码

| 错误码 | 含义 | 处理方式 |
|-------|------|---------|
| `ARIA-SYNC-001` | `external_task_id` 缺失 | 禁止 `vk intake` 或状态同步 |
| `ARIA-SYNC-002` | 状态推送失败 | 记录告警，主流程继续 |
| `ARIA-SYNC-003` | 幂等键冲突 | 保留内部状态，要求人工排查适配器 |
| `ARIA-SYNC-004` | 外部任务不存在或已删除 | 标记外部状态漂移，不阻断内部任务 |

### 同步触发点

| Aria 状态变化 | 同步动作 |
|--------------|---------|
| `intake` 完成 | 向 VK 写入任务已接收确认 |
| `spec-approved` | 向 VK 写入 OpenSpec 链接 |
| `executing` | 向 VK 写入执行开始状态 |
| `done` / `verified` | 向 VK 写入完成摘要 |
| `upgrade-blocked` | 向 VK 写入阻塞原因和待修复项 |
| 用户取消 | 向 VK 写入取消标记 |

## 扩展接口预留

一期只完成 `issue` 级闭环，但必须预留：

1. `branch lifecycle`
2. `PR lifecycle`

建议通过以下位置预留扩展位：

- `runtime/contracts/branch-*.md`
- `runtime/contracts/pr-*.md`
- `runtime/states/extensions.md`

这样后续可向 `oh-my-claudecode` 风格的更完整 orchestrator 演进，而不推翻一期目录与状态机。

## 日志与可观测性

一期采用最小日志方案，确保运行时问题可追溯。

### 日志存储

日志文件存储在 `cadence/cache/aria/logs/` 目录下：

```text
cadence/cache/aria/logs/
  aria-YYYY-MM-DD.log      # 按日期轮转
```

### 日志级别

| 级别 | 用途 | 示例 |
|------|------|------|
| `debug` | 详细调试信息 | contract 字段解析结果、prompt 生成详情 |
| `info` | 正常流程关键节点 | 状态转换、角色调度、配置加载 |
| `warn` | 非阻断性异常 | VK 同步失败、配置项缺失使用默认值 |
| `error` | 阻断性错误 | Codex 启动失败、状态损坏、文件读写失败 |

### 关键日志记录点

| 事件 | 级别 | 记录内容 |
|------|------|---------|
| 状态转换 | `info` | task_id、from_status、to_status、触发角色 |
| Codex 进程启动 | `info` | task_id、exec_unit_id、pid、worktree |
| Codex 进程退出 | `info` | task_id、exec_unit_id、exit_code、耗时 |
| 仲裁结论 | `info` | task_id、review_verdict、test_verdict、must_fix 数量 |
| VK 同步失败 | `warn` | task_id、目标状态、失败原因 |
| 文件范围越界 | `warn` | task_id、越界文件列表 |
| Codex 超时 | `error` | task_id、exec_unit_id、超时时间 |
| 状态损坏检测 | `error` | task_id、损坏位置、期望状态 |

### 日志轮转策略

1. 按日期创建新文件
2. 单文件最大体积由配置 `logging.max_file_size_mb` 控制（默认 10MB）
3. 保留文件数由 `logging.max_files` 控制（默认 5 个）
4. 超出上限时删除最旧文件

### 调试模式

当 `logging.level` 设为 `debug` 时，额外记录：

1. contract 完整内容
2. Codex prompt 注入内容
3. Codex stdout/stderr 完整输出
4. 状态机守卫条件检查详情

## Aria 自身测试策略

Aria 作为一个多角色编排系统，自身需要独立的测试策略。

### 单元测试

以下模块必须有单元测试覆盖：

| 测试对象 | 覆盖要求 | 关键测试场景 |
|---------|---------|-------------|
| 状态机转换 | 守卫条件全覆盖 | 正向转换、非法转换、异常路径、恢复路径 |
| Contract 生成与解析 | 字段完整性校验 | dispatch contract 生成、patch contract 生成、字段缺失检测 |
| 仲裁逻辑 | 所有可能组合 | review fail + test pass、review pass + test fail、双方 fail、冲突退回 |
| 冲突检测 | files_allowed 交集 | 有交集→强制串行、无交集→允许并行、目录级前缀匹配 |
| 配置加载与优先级 | 三级覆盖链 | 任务级覆盖项目级、项目级覆盖默认值、缺失使用默认值 |

### 集成测试

| 测试对象 | 覆盖要求 |
|---------|---------|
| Claude → Codex 桥接 | contract 生成 → prompt 注入 → Codex 启动 → 结果收集 的完整链路 |
| 状态持久化与恢复 | 写入 state.yaml → 关闭会话 → 重新加载 → 状态一致 |
| 文件范围校验 | exec 产出越界文件 → 检测 → 写入 review blocker |

### E2E 测试

MVP 必须覆盖的最小闭环场景：

1. **正式流完整闭环**：`aria:intake` → `aria:start` → Aria 自动调用 `writing-plans` 完成 plan（MVP 可带确认点）→ `aria:run` → exec 完成 → Aria 自动调用 `requesting-code-review` / `verification-before-completion` → `done`
2. **正式流带 patch**：`aria:run` → review fail → patch → Aria 自动重新执行 review → `done`
3. **fast-lane 闭环**：`aria:fast` → exec → review-lite pass → `done`
4. **fast-lane 升级**：`aria:fast` → scope 超界 → 升级到正式流
5. **取消与恢复**：`aria:run` → 中途 `aria:cancel` → `aria:run --resume`

## 用户确认计划的交互方式

`aria:start` 的结束状态固定为 `planned`，采用**方案 B**交互模式：

1. `aria:start` 执行完毕时，自动展示 plan brief 摘要（exec_unit_count、parallel_candidates、quality_gates）
2. 提示用户："计划已生成。确认后执行 `aria:run --task-id <id>`，或先使用 `aria:status --task-id <id>` 查看完整计划"
3. `aria:run` 启动时不做额外确认，直接从 `planned → dispatched` 转换
4. 如果用户需要修改计划，应使用 `aria:start --task-id <id> --replan`

**设计理由**：

1. 符合"命令少而稳"原则
2. 用户确认动作就是"执行 aria:run"，不需要额外的确认步骤
3. 计划修改通过显式 `--replan` 完成，避免把业务重开与执行取消混为一谈

## 分阶段开发路线图

> **v1.4 新增**：根据设计评审建议，将原"一期"拆分为 4 个交付批次，明确每阶段的交付物和通过准则。

### P0 - MVP 验证

> **v1.4.2 修正**：P0 目标恢复为"验证全自动编排（MVP 带关键节点用户确认点）+ Codex 执行"。

**目标**：验证核心假设，确认"Aria 自动调用 superpowers skills（自动执行型）+ Codex 执行"的技术路线可行。

**交付物**：

1. `adapters/host/`：宿主能力适配器（Bash 执行、文件读写、Git 操作）
2. `adapters/codex/`：Codex CLI 最小适配器（启动、prompt 注入、结果收集）
3. `runtime/state-machine/`：最小状态机（`intake → executing → done`）
4. `runtime/persistence/`：state.yaml 读写
5. `runtime/orchestrator/prompt-service/`：最小 PromptService（能在 spec-approved / reviewing/testing 等状态输出下一步提示）
6. `commands/aria:fast`：唯一用户入口（仅支持串行 fast-lane）
7. `schemas/`：最小 Zod schema（state.yaml）

**MVP 验证步骤**：

````text
1. 执行 aria:fast 进入 fast-lane
2. Aria 自动完成 intake，状态到达 spec-required
3. 用户手动完成 spec（生成 OpenSpec 最小工件集合）
4. Aria 检测到 spec-approved
5. [MVP] PromptService 输出确认提示，用户确认后 Aria 自动调用 /writing-plans
6. [最终版] Aria 直接自动调用 /writing-plans，生成 plan-brief.md
7. Aria 读取 plan brief，自动流转到 planned
8. 用户执行 aria:run，Aria 调度 Codex exec 完成最小修改
9. exec 完成后，Aria 自动进入 reviewing/testing
10. [MVP] PromptService 输出确认提示，用户确认后 Aria 自动调用 /requesting-code-review 和 /verification-before-completion
11. [最终版] Aria 直接自动调用 review/test skills，生成 review-report.md / test-report.md
12. Aria 读取报告，自动流转到 verified → done
13. 验证 state.yaml、diff、报告三者一致
````

**通过准则**：

1. 单轮 `exec` 能稳定完成，且结果、diff、状态三者一致
2. PromptService 能在正确状态输出准确的自动调用提示或确认提示
3. Aria 能自动调用 superpowers skills 并正确读取工件、推进状态（MVP 允许带确认点）
4. 至少 1 个失败场景能稳定进入 `patch`
5. capability report 中的降级项能触发预期降级，而不是直接崩溃
6. 状态恢复后不会重复调度已完成单元
7. 文件越界校验能拦截至少 1 个越界样例

**失败退出准则**：

1. 无法稳定完成单轮 Codex 调用与结果回写
2. 状态恢复后经常出现重复执行或状态漂移
3. 后台进程模型在当前宿主不稳定
4. 外部依赖探测无法形成稳定 capability report

**验证失败时**：
- 若 Codex 桥接失败：退化为"纯人工执行 + Aria 记录状态"模式
- 若自动编排失败（如 skill 自动调用后无法正确产出工件）：增加更多的人工确认点，或在特定状态退化为"提示用户手动调用 skill"
- 若后台进程不稳定：采用串行调度模式，并行设计标记为"暂不启用"

### P1 - 正式流闭环

**前置条件**：P0 MVP 验证通过。

**交付物**：

1. `runtime/state-machine/`：完整状态机 + 守卫条件
2. `runtime/contracts/`：dispatch contract 生成与解析、patch contract 生成与解析
3. `adapters/openspec/`：OpenSpec 适配器（spec-approved 门槛检查）
4. `runtime/arbitrator/`：review/test 仲裁逻辑
5. `runtime/reports/`：review report、test report、verification summary 生成
6. `commands/`：`aria:intake`、`aria:start`、`aria:run`、`aria:status`
7. `schemas/`：完整 Zod schema（所有工件）
8. `diagnostics/`：capability 探测 + 错误码

**通过准则**：

1. `aria:intake` → `aria:start` → Aria 自动调用 `writing-plans` 完成 plan → `aria:run` → exec 完成 → Aria 自动调用 `requesting-code-review` / `verification-before-completion` 完成 review/test → `done` 的完整自动闭环
2. `aria:run` → review fail → patch → Aria 自动重新执行 review → `done` 的自动闭环
3. PromptService 在 `spec-approved`、`reviewing/testing` 等状态能输出准确的自动调用提示或确认提示
4. spec-approved 门槛检查能正确阻断不完整的 OpenSpec 工件
5. 文件范围校验能检测越界变更

### P2 - 增强能力

**前置条件**：P1 正式流闭环通过。

**交付物**：

1. `runtime/scheduler/`：并行执行管理 + 依赖解析 + worktree 隔离
2. Patch 循环（多轮 patch + 退回 plan/spec）
3. fast-lane 通道 + 升级规则
4. `adapters/vk/`：Vibe Kanban 适配器（单向投影）
5. `adapters/superpowers/`：能力抽象层 + skill 名称映射
6. 完整错误处理（所有错误码实现）
7. 日志系统（pino + 轮转）
8. `commands/aria:fast`、`aria:doctor`

**通过准则**：

1. 2 个 Codex 实例并行执行，无交叉污染
2. fast-lane 超界时正确升级到正式流
3. VK 同步失败不阻断主流程
4. 日志能覆盖关键状态转换和错误事件

### P3 - 收尾打磨

**前置条件**：P2 增强能力通过。

**交付物**：

1. `commands/aria:result`、`aria:cancel`、`aria:retry`
2. 配置管理完善（三级优先级合并）
3. 状态恢复 + 异常清理（僵尸进程检测、孤儿 worktree 清理）
4. `aria:start --replan` 计划重开
5. E2E 测试完善（5 个必须场景）
6. Plugin 安装与分发（`.claude-plugin` 元数据）
7. 开发文档与用户引导

**通过准则**：

1. 5 个 E2E 场景全部通过
2. 中断后 `aria:run --resume` 能正确恢复
3. `aria:doctor` 能诊断并给出修复建议
4. Plugin 可通过 `claude /plugin add` 安装

### 各阶段时间估算

| 阶段 | 建议周期 | 核心风险 |
|------|---------|---------|
| P0 | 1-2 周 | Codex 桥接稳定性、后台进程管理 |
| P1 | 2-3 周 | OpenSpec 工件格式兼容、仲裁逻辑正确性 |
| P2 | 2-3 周 | 并行执行正确性、VK 适配稳定性 |
| P3 | 1-2 周 | E2E 场景覆盖、边界情况处理 |

## 一期边界

### 一期纳入范围

1. `issue` 级任务闭环
2. `Claude -> Codex` 角色编排
3. `OpenSpec` 正式任务强制主线
4. `superpowers` 方法层编排（通过能力抽象层）
5. `Vibe Kanban` 接入与原生入口双支持
6. review/test/patch 闭环
7. `Codex adapter` 薄桥接实现
8. 闭环结果摘要输出
9. Plugin 安装与分发（`.claude-plugin` 元数据）
10. 环境诊断（`aria:doctor`）
11. 安全审查（路径穿越防护、contract 注入防护、产出安全检查）
12. 文件锁机制（并发状态安全）
13. 分阶段开发路线图（P0-P3）

### 一期不纳入范围

1. `merge`
2. `release`
3. `archive`
4. PR 自动化
5. 分支治理自动化
6. 把 `Vibe Kanban` 变成总控编排器
7. 内嵌或 fork `OpenSpec` 与 `superpowers`
8. 构建厚重的 Codex 原生 runtime 或远程编排服务

## 成功标准

一期成功标准定义如下：

1. 能统一接收来自 `Vibe Kanban` 与原生命令的任务
2. 正式任务不能绕过 `OpenSpec`
3. `Claude Code` 能通过 `Aria runtime` 驱动 `Codex` 执行，并在具备条件时并行
4. `review` 与 `test` 能分别出报告，并被 `orchestrator` 明确仲裁
5. `patch` 能形成闭环修补循环
6. MVP 最小闭环能够稳定跑通
7. `Aria` 能输出结构化状态与结果摘要
8. `Aria` 与 `OpenSpec/superpowers` 保持松耦合升级关系

## 安全审查要求

> **v1.4 新增**：根据设计评审建议，补充安全性审查要求。

### 文件范围安全

方案已定义 `files_allowed` / `files_blocked` 的 prompt 软约束和 `git diff --name-only` 的硬校验。补充以下安全要求：

#### 路径穿越防护

1. `files_allowed` 和 `files_blocked` 中的路径必须是**仓库相对路径**，禁止包含 `..`
2. contract 生成时验证路径规范化（`path.resolve` 后仍在仓库根目录下）
3. Codex 执行的工作目录必须是仓库根目录或其子目录
4. `result_path` 同样受路径穿越检查

```text
contract 生成时的路径校验：
1. 规范化路径（path.normalize）
2. 检查是否包含 .. 父目录引用
3. 检查是否以 / 开头（绝对路径）
4. 检查解析后是否仍在仓库根目录下
5. 任一检查失败 → ARIA-DISPATCH-006 错误
```

| 错误码 | 含义 | 处理方式 |
|-------|------|---------|
| `ARIA-DISPATCH-006` | 路径穿越风险检测 | 阻断 dispatch，提示路径不合法 |

### Codex 产出安全审查

1. `review` 角色在审查 exec 结果时，应检查以下安全项：
   - 新增依赖是否引入已知漏洞
   - 是否存在硬编码密钥或 token
   - 是否存在命令注入、XSS、SQL 注入等 OWASP Top 10 风险
2. 安全审查结果写入 review report 的 `security_findings` 字段

```yaml
# review report 补充字段
security_findings:
  - category: "hardcoded-secret"
    severity: blocker
    file_path: "src/config.ts"
    line_range: "15-15"
    description: "疑似硬编码 API Key"
```

### Contract 注入防护

1. 用户输入在写入 YAML contract 前必须转义特殊字符
2. 使用 Zod schema 校验所有 contract 字段的类型和格式
3. prompt 模板变量插值时过滤 Markdown 注入（防止通过 task description 注入恶意指令）

### Codex 进程隔离

1. Codex 子进程的环境变量应最小化，不继承宿主的敏感环境变量（如 API Key）
2. Codex 子进程的工作目录限制在指定 worktree 内
3. Codex 子进程的网络访问策略（如可配置，应限制为只读）

## 结论

`Cadence-Aria` 一期应被定义为：

- 一个新的 `Claude Code plugin`（TypeScript + pnpm + vitest + zod）
- 带有正式 `Aria runtime` 与薄 `Codex adapter`
- orchestrator 拆分为 `StateMachine` + `PromptService` + `Scheduler` + `Arbitrator` + `SyncService`
- **全自动编排 + 关键节点确认**模式：Aria 负责状态维护、执行调度、工件衔接；自动执行型 superpowers skills（`writing-plans`、`requesting-code-review`、`verification-before-completion` 等）由 Aria 在对应状态节点**自动调用**；多轮交互型 skills（如 `brainstorming`）保留用户交互
- MVP 阶段在关键自动调用节点（`spec-approved → planned`、`executing → reviewing/testing`）可保留用户确认点，最终版本移除确认实现全自动
- 以 `issue` 为最小闭环单位
- 强制正式任务进入 `OpenSpec`
- 通过能力抽象层（`capability_id`）映射 superpowers 调用，不硬编码 skill 名称；`capability_mapping` 增加 `execution_mode` 字段区分 `auto` 与 `interactive`
- 接入但不吞并 `Vibe Kanban`
- 按分阶段路线图（P0-P3）交付
- 预留 `branch / PR` 扩展接口

**v1.4.2 关键修正**：

1. 恢复 `Aria` 的"全自动多角色编排器"定位（MVP 阶段关键节点带用户确认点）
2. `superpowers` 的调用方式修正为：自动执行型 skills 由 Aria **自动调用**，不需要用户显式输入 `/skill-name`；只有 `brainstorming` 等多轮交互型 skills 需要用户介入
3. `PromptService` 负责两类提示：自动执行型 skills 的"自动调用提示/MVP 确认提示"，以及多轮交互型 skills 的"用户调用指引"
4. 需要 superpowers 能力的状态转换（`spec-approved → planned`、review、test）主要由 Aria **自动调用 skill + 检测工件落盘**推进，不再需要用户手动触发

这一定位能够同时满足：

1. 当前希望快速建立 Claude/Codex 协作工作流
2. 中期希望把 `Vibe Kanban` 的基础能力收拢进 `Aria`
3. 长期希望向 `oh-my-claudecode` 风格的 orchestration 方向演进
