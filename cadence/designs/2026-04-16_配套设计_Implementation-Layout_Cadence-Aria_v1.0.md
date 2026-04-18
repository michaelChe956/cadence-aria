# Cadence-Aria Implementation Layout 配套设计

> **版本**：v1.0.3
> **日期**：2026-04-18
> **关联主文档**：`cadence/designs/2026-04-16_方案设计_Cadence-Aria_v1.4.md`（当前修订：v1.4.5）

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
  src/
    commands/
    runtime/
      orchestrator/
      state-machine/
      scheduler/
      arbitrator/
      contracts/
      reports/
      persistence/
    adapters/
      codex/
      openspec/
      superpowers/
      vk/
      host/
    schemas/
    config/
    diagnostics/
    utils/
  skills/
  codex/
    prompts/
    templates/
  templates/
  tests/
  docs/
```

## 顶层目录职责

| 目录 | 职责 | 不应承担的职责 |
|------|------|---------------|
| `src/commands/` | CLI 命令入口、参数解析、用户输出组装 | 状态机逻辑、contract 生成 |
| `src/runtime/orchestrator/` | 任务调度、状态推进、角色编排 | 直接做底层适配或模板渲染 |
| `src/runtime/contracts/` | `dispatch/patch` contract 与 `execution context bundle` 的构建、校验、序列化 | 调用外部工具执行任务 |
| `src/runtime/state-machine/` | 状态机定义、守卫条件、状态转换 | 文件 IO、CLI 输出 |
| `src/runtime/scheduler/` | 执行单元调度与最小串行执行编排；多单元并行属于 Layer 3 | 直接产出用户文案 |
| `src/runtime/arbitrator/` | 基于 `result_set_id` 的 review/test 仲裁与 patch contract 生成 | 直接启动外部进程 |
| `src/adapters/` | Claude/Codex/OpenSpec/superpowers/VK 的 capability 适配 | 决定业务状态流转 |
| `src/runtime/reports/` | `review/test/verification/closure` 等报告构造 | 直接启动进程 |
| `src/runtime/persistence/` | `state.yaml` 与运行时工件读写 | 业务仲裁 |
| `src/diagnostics/` | capability report、错误码转写、日志辅助 | 改写核心业务状态 |
| `src/schemas/` | Zod schema 与运行时类型约束 | 访问外部系统 |
| `src/config/` | 配置加载、优先级合并、默认值装配 | 决定状态流转 |
| `src/utils/` | 路径、时间、ID 等通用工具 | 持有业务状态 |
| `codex/prompts/` | Codex prompt 模板与边界约束模板 | 直接做状态推进 |
| `codex/templates/` | Codex 输出模板 | 直接参与编排决策 |
| `tests/` | 单元、集成、E2E 验证 | 存放运行时逻辑 |

## 推荐文件落位

### `src/commands/`

```text
src/commands/
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
2. 调用 `src/runtime/orchestrator/` 的公开接口
3. 组装符合 CLI 交互设计的输出
4. 将错误码转为用户可读摘要

### `src/runtime/orchestrator/`

```text
src/runtime/orchestrator/
  task-orchestrator.ts
  formal-flow.ts
  fast-lane-flow.ts
  prompt-service.ts
```

#### 责任

1. 驱动 `intake -> clarification/spec-drafting/spec-review -> planning/plan-review -> dispatch -> exec -> review/test -> patch`
2. 协调 `state-machine/`、`scheduler/`、`arbitrator/`、`contracts/`、`adapters/`、`reports/`、`persistence/`
3. 维护单任务调度上下文
4. 生成下一步动作与用户提示

### `src/runtime/state-machine/`

```text
src/runtime/state-machine/
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

### `src/runtime/scheduler/`

```text
src/runtime/scheduler/
  exec-scheduler.ts
  dependency-graph.ts
  worktree-manager.ts
  timeout-monitor.ts
```

#### 责任

1. Layer 1 / Layer 2 中负责单任务、单执行单元的串行调度
2. 处理超时、取消与重试前置判断
3. Layer 3 再引入并行上限、依赖关系、等待队列与 worktree 分配
4. 不把多执行单元调度作为一期最小闭环前置条件

### `src/runtime/arbitrator/`

```text
src/runtime/arbitrator/
  review-test-arbitrator.ts
  patch-decision.ts
  conflict-detector.ts
```

#### 责任

1. 汇总绑定同一 `result_set_id` 的 review/test 结论
2. 生成 `patch contract`
3. 判定是否进入 `verified`、`patching` 或 `blocked`
4. 保持纯逻辑，不直接调用外部系统

### `src/runtime/contracts/`

```text
src/runtime/contracts/
  dispatch-contract.ts
  patch-contract.ts
  execution-context-bundle.ts
  contract-validator.ts
  scope-mapping.ts
  branch-placeholder.md
  pr-placeholder.md
```

#### 责任

1. 按 schema 生成 `dispatch contract`
2. 按仲裁结果生成 `patch contract`
3. 构建并校验 `execution context bundle`
4. 校验 contract 与 OpenSpec/plan 的映射关系
5. 校验 `files_allowed/files_blocked`

### `src/adapters/`

```text
src/adapters/
  host/
    host-adapter.ts
  codex/
    codex-adapter.ts
  openspec/
    openspec-adapter.ts
  superpowers/
    superpowers-adapter.ts
  vk/
    vibe-kanban-adapter.ts
  capability-detector.ts
```

#### 责任

1. 把外部系统暴露为 capability
2. 屏蔽宿主差异
3. 返回结构化结果与错误码
4. 禁止在 adapter 内做状态机推进

### `src/runtime/reports/`

```text
src/runtime/reports/
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

### `src/runtime/persistence/`

```text
src/runtime/persistence/
  state-repository.ts
  task-repository.ts
  result-set-repository.ts
  confirmation-event-repository.ts
  file-lock.ts
  paths.ts
```

#### 责任

1. 读写 `state.yaml`
2. 维护冻结引用、`result_set`、`confirmation event` 与阻塞恢复元数据
3. 维护任务目录路径
4. 防止多处写入同一任务状态
5. 隔离文件系统细节

## 一期分层落位补充

### Layer 1

- `src/runtime/state-machine/`
- `src/runtime/contracts/`
- `src/runtime/persistence/`
- `src/runtime/orchestrator/`
- `src/runtime/arbitrator/`
- `src/runtime/reports/`

### Layer 2

- `src/runtime/contracts/patch-contract.ts`
- `src/runtime/persistence/result-set-repository.ts`
- `src/runtime/persistence/confirmation-event-repository.ts`
- `src/diagnostics/`

### Layer 3

- `src/runtime/scheduler/dependency-graph.ts`
- `src/runtime/scheduler/worktree-manager.ts`
- `src/adapters/vk/`

## 文档联动修订要求

1. 状态语义变更同步更新一期收敛方案与 Runtime Schemas
2. 角色职责变更同步更新主方案与一期收敛方案
3. contract、bundle、report 字段变更同步更新 Runtime Schemas 与 Implementation Layout
4. 模块边界变更同步更新 Implementation Layout 与主方案

### `src/diagnostics/`

```text
src/diagnostics/
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

### `src/schemas/`

```text
src/schemas/
  state-schema.ts
  contract-schema.ts
  report-schema.ts
```

#### 责任

1. 定义所有运行时工件的 schema
2. 提供运行时校验器与类型导出
3. 保持为最底层模块，不依赖运行时实现

### `src/config/`

```text
src/config/
  load-config.ts
  merge-config.ts
  default-config.ts
```

#### 责任

1. 加载项目级配置
2. 处理默认值与优先级合并
3. 向 orchestrator、scheduler、adapters 暴露稳定配置接口

## 推荐调用链

### `aria:start`

```text
src/commands/start
  -> src/runtime/orchestrator/formal-flow
    -> src/adapters/openspec/openspec-adapter
    -> src/adapters/superpowers/superpowers-adapter
    -> src/runtime/reports/intake-card
    -> src/runtime/reports/plan-brief
    -> src/runtime/persistence/state-repository
```

### `aria:run`

```text
src/commands/run
  -> src/runtime/orchestrator/task-orchestrator
    -> src/runtime/state-machine/state-machine
    -> src/runtime/contracts/dispatch-contract
    -> src/runtime/scheduler/exec-scheduler
    -> src/adapters/codex/codex-adapter
    -> src/runtime/arbitrator/review-test-arbitrator
    -> src/runtime/reports/review-report
    -> src/runtime/reports/test-report
    -> src/runtime/contracts/patch-contract
    -> src/runtime/reports/verification-summary
    -> src/runtime/reports/closure-summary
    -> src/runtime/persistence/state-repository
```

### `aria:status`

```text
src/commands/status
  -> src/runtime/persistence/state-repository
  -> src/adapters/vk/vibe-kanban-adapter (optional sync)
  -> src/diagnostics/sync-drift
```

## 依赖方向约束

### 允许的依赖

1. `src/commands -> src/runtime/orchestrator`
2. `src/runtime/orchestrator -> src/runtime/state-machine`
3. `src/runtime/orchestrator -> src/runtime/contracts`
4. `src/runtime/orchestrator -> src/runtime/scheduler`
5. `src/runtime/orchestrator -> src/runtime/arbitrator`
6. `src/runtime/orchestrator -> src/adapters`
7. `src/runtime/orchestrator -> src/runtime/reports`
8. `src/runtime/orchestrator -> src/runtime/persistence`
9. `src/runtime/orchestrator -> src/diagnostics`
10. `src/runtime/* -> src/schemas`
11. `src/* -> src/config`（只读配置消费）

### 禁止的依赖

1. `src/commands -> src/adapters` 直接调用外部系统
2. `src/adapters -> src/runtime/state-machine` 直接推进状态机
3. `src/runtime/reports -> src/adapters` 在报告层访问外部系统
4. `src/runtime/contracts -> src/commands` 反向依赖命令层
5. `src/runtime/persistence -> src/runtime/orchestrator` 反向依赖业务层
6. `src/schemas -> src/runtime/*` 反向依赖业务层

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

### `state-machine`

状态层应保持纯逻辑。允许：

1. 输入当前状态和上下文
2. 输出是否允许转换、失败原因、下一状态

不允许：

1. 文件读写
2. CLI 输出
3. 进程管理

## 实现顺序建议

1. 先实现 `src/schemas`、`src/runtime/state-machine` 与 `src/runtime/contracts`
2. 再实现 `src/runtime/persistence`
3. 然后实现 `src/adapters/capability-detector` 与 `src/config`
4. 再串 `src/runtime/orchestrator`、`src/runtime/scheduler`、`src/runtime/arbitrator`
5. 最后接 `src/commands` 和 CLI 输出

## 与配套文档的关系

本文件与另外两份配套设计的关系如下：

1. `Runtime Schemas` 定义字段约束
2. `CLI Interactions` 定义命令输出快照
3. `Implementation Layout` 定义代码落位与依赖边界

三者合起来，才构成实现前的最小工程设计闭环。
