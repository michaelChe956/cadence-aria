# Cadence-Aria Implementation Layout 配套设计

> **版本**：v1.0
> **日期**：2026-04-16
> **关联主文档**：`cadence/designs/2026-04-16_方案设计_Cadence-Aria_v1.3.md`

## 目标

本配套文档回答以下实现问题：

1. 代码应落在哪些目录
2. 每个模块负责什么
3. 模块之间如何调用
4. 哪些依赖方向是允许的，哪些不允许

本文件不替代主设计文档中的状态机、schema、错误码和 CLI 交互定义，而是为它们提供实现落位。

## 顶层目录建议

```text
cadence-aria/
  commands/
  runtime/
    orchestrator/
    contracts/
    states/
    adapters/
    reports/
    persistence/
    diagnostics/
  codex/
    prompts/
    workflows/
    templates/
  references/
  templates/
  docs/
```

## 顶层目录职责

| 目录 | 职责 | 不应承担的职责 |
|------|------|---------------|
| `commands/` | CLI 命令入口、参数解析、用户输出组装 | 状态机逻辑、contract 生成 |
| `runtime/orchestrator/` | 任务调度、状态推进、角色编排 | 直接做底层适配或模板渲染 |
| `runtime/contracts/` | `dispatch/patch` 等 contract 的构建、校验、序列化 | 调用外部工具执行任务 |
| `runtime/states/` | 状态机定义、守卫条件、状态转换 | 文件 IO、CLI 输出 |
| `runtime/adapters/` | Claude/Codex/OpenSpec/superpowers/VK 的 capability 适配 | 决定业务状态流转 |
| `runtime/reports/` | `review/test/verification/closure` 等报告构造 | 直接启动进程 |
| `runtime/persistence/` | `state.yaml` 与运行时工件读写 | 业务仲裁 |
| `runtime/diagnostics/` | capability report、错误码转写、日志辅助 | 改写核心业务状态 |
| `codex/prompts/` | Codex prompt 模板与边界约束模板 | 直接做状态推进 |
| `codex/workflows/` | Codex 执行/修补工作流模板 | 命令入口解析 |

## 推荐文件落位

### `commands/`

```text
commands/
  intake.ts
  start.ts
  run.ts
  fast.ts
  status.ts
  result.ts
  cancel.ts
  retry.ts
```

#### 责任

1. 解析命令参数
2. 调用 `runtime/orchestrator/` 的公开接口
3. 组装符合 CLI 交互设计的输出
4. 将错误码转为用户可读摘要

### `runtime/orchestrator/`

```text
runtime/orchestrator/
  task-orchestrator.ts
  formal-flow.ts
  fast-lane-flow.ts
  review-test-arbitration.ts
```

#### 责任

1. 驱动 `intake -> spec -> plan -> dispatch -> exec -> review/test -> patch`
2. 协调 `states/`、`contracts/`、`adapters/`、`reports/`、`persistence/`
3. 维护单任务调度上下文
4. 生成下一步动作与用户提示

### `runtime/states/`

```text
runtime/states/
  state-machine.ts
  guards.ts
  transitions.ts
  recovery-rules.ts
  extensions.md
```

#### 责任

1. 定义合法状态集合
2. 定义每个状态转换的 guard
3. 定义恢复与重试规则
4. 提供纯函数式状态判断

### `runtime/contracts/`

```text
runtime/contracts/
  dispatch-contract.ts
  patch-contract.ts
  contract-validator.ts
  scope-mapping.ts
  branch-placeholder.md
  pr-placeholder.md
```

#### 责任

1. 按 schema 生成 `dispatch contract`
2. 按仲裁结果生成 `patch contract`
3. 校验 `files_allowed/files_blocked`
4. 校验 contract 与 OpenSpec/plan 的映射关系

### `runtime/adapters/`

```text
runtime/adapters/
  host-adapter.ts
  codex-adapter.ts
  openspec-adapter.ts
  superpowers-adapter.ts
  vibe-kanban-adapter.ts
  capability-detector.ts
```

#### 责任

1. 把外部系统暴露为 capability
2. 屏蔽宿主差异
3. 返回结构化结果与错误码
4. 禁止在 adapter 内做状态机推进

### `runtime/reports/`

```text
runtime/reports/
  intake-card.ts
  plan-brief.ts
  review-report.ts
  test-report.ts
  verification-summary.ts
  closure-summary.ts
```

#### 责任

1. 构建 Markdown + front matter 结构
2. 统一报告字段名与展示格式
3. 供 `orchestrator` 写入任务目录

### `runtime/persistence/`

```text
runtime/persistence/
  state-repository.ts
  task-repository.ts
  file-lock.ts
  paths.ts
```

#### 责任

1. 读写 `state.yaml`
2. 维护任务目录路径
3. 防止多处写入同一任务状态
4. 隔离文件系统细节

### `runtime/diagnostics/`

```text
runtime/diagnostics/
  error-codes.ts
  capability-report.ts
  logger.ts
  sync-drift.ts
```

#### 责任

1. 统一错误码枚举
2. 生成 capability report
3. 记录结构化日志
4. 辅助同步漂移诊断

## 推荐调用链

### `aria:start`

```text
commands/start
  -> runtime/orchestrator/formal-flow
    -> runtime/adapters/openspec-adapter
    -> runtime/adapters/superpowers-adapter
    -> runtime/reports/intake-card
    -> runtime/reports/plan-brief
    -> runtime/persistence/state-repository
```

### `aria:run`

```text
commands/run
  -> runtime/orchestrator/task-orchestrator
    -> runtime/states/state-machine
    -> runtime/contracts/dispatch-contract
    -> runtime/adapters/codex-adapter
    -> runtime/reports/review-report
    -> runtime/reports/test-report
    -> runtime/contracts/patch-contract
    -> runtime/reports/verification-summary
    -> runtime/reports/closure-summary
    -> runtime/persistence/state-repository
```

### `aria:status`

```text
commands/status
  -> runtime/persistence/state-repository
  -> runtime/adapters/vibe-kanban-adapter (optional sync)
  -> runtime/diagnostics/sync-drift
```

## 依赖方向约束

### 允许的依赖

1. `commands -> runtime/orchestrator`
2. `runtime/orchestrator -> runtime/states`
3. `runtime/orchestrator -> runtime/contracts`
4. `runtime/orchestrator -> runtime/adapters`
5. `runtime/orchestrator -> runtime/reports`
6. `runtime/orchestrator -> runtime/persistence`
7. `runtime/orchestrator -> runtime/diagnostics`

### 禁止的依赖

1. `commands -> runtime/adapters` 直接调用外部系统
2. `runtime/adapters -> runtime/states` 直接推进状态机
3. `runtime/reports -> runtime/adapters` 在报告层访问外部系统
4. `runtime/contracts -> commands` 反向依赖命令层
5. `runtime/persistence -> runtime/orchestrator` 反向依赖业务层

## 模块边界规则

### `commands`

命令层应当是薄层。允许：

1. 参数解析
2. 输出格式化
3. 退出码映射

不允许：

1. 直接读写 `state.yaml`
2. 直接拼 `dispatch contract`
3. 直接调用 `codex`

### `adapters`

适配层只做“能力翻译”，不做业务决定。允许：

1. 探测 capability
2. 执行外部命令
3. 返回结构化结果

不允许：

1. 自行决定任务进入 `patching`
2. 自行修改 OpenSpec 边界
3. 自行写 closure summary

### `states`

状态层应保持纯逻辑。允许：

1. 输入当前状态和上下文
2. 输出是否允许转换、失败原因、下一状态

不允许：

1. 文件读写
2. CLI 输出
3. 进程管理

## 实现顺序建议

1. 先实现 `runtime/states` 与 `runtime/contracts`
2. 再实现 `runtime/persistence`
3. 然后实现 `runtime/adapters/capability-detector`
4. 再串 `runtime/orchestrator`
5. 最后接 `commands` 和 CLI 输出

## 与配套文档的关系

本文件与另外两份配套设计的关系如下：

1. `Runtime Schemas` 定义字段约束
2. `CLI Interactions` 定义命令输出快照
3. `Implementation Layout` 定义代码落位与依赖边界

三者合起来，才构成实现前的最小工程设计闭环。
