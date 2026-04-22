# 设计文档：Aria 终端 REPL 与多 Agent 编排 Runtime

**文档信息**
- **设计编号**：DES-2026-04-22-ARIA-REPL-RUNTIME
- **创建日期**：2026-04-22
- **版本**：v1.0
- **负责人**：Codex
- **关联需求**：构建一个类似 Claude Code 的终端 REPL，用于驱动 Claude Code 与 Codex 完成需求澄清、设计、评审、计划、任务分发、编码、测试、汇总与总结的端到端流程

## 1. 项目事实确认

### 1.1 项目类型

- [ ] 前端项目（Web/Client）
- [ ] 后端项目（API/Service）
- [x] 全栈项目
- [ ] 其他

说明：本次设计对象不是单一业务模块，而是一个本地优先的 Agent 编排产品，包含终端交互层、本地守护进程、Provider 适配层、任务运行隔离层与文档/状态存储层。

### 1.2 现有技术栈

| 维度 | 当前技术 | 版本/备注 |
|------|----------|-----------|
| 语言 | Rust core + TypeScript adapter/UI | 一期仍只通过 CLI 集成外部 Agent |
| 框架 | Rust workspace + Node.js/TypeScript provider adapter 层 | 当前仓库以规则与设计文档为主 |
| 数据访问 | 文件系统持久化 | 一期不引入数据库作为硬依赖 |
| 通信方式 | 本地 REPL 到本地 daemon 的进程间通信 | Unix 使用 Unix Domain Socket，Windows 兼容层使用 Named Pipe；消息协议为 line-delimited JSON |
| 测试框架 | Rust 测试框架 + TS 测试框架 + fake provider harness | 设计要求覆盖单元、组件、系统、冒烟四层以上验证 |

### 1.3 现有约定

- 请求入参处理方式：当前仓库未存在既有运行时代码，本次设计以 REPL 自然语言输入与命令式输入为主。
- 响应结构：当前仓库未存在既有接口契约，一期以终端摘要输出和结构化内部事件对象为主。
- 异常体系：当前仓库未存在既有异常体系，设计上区分 provider 调用失败、阶段产物失败、执行型失败、系统级失败、不可自动决策失败五类。
- 日志体系：当前仓库未存在既有日志实现，设计要求 daemon 维护 append-only event log 与 checkpoint。
- 分层/目录组织：当前仓库已有 `cadence/analysis-docs`、`cadence/designs`、`cadence/plans`、`cadence/prds` 文档结构，尚无运行时代码目录。

### 1.4 兼容性边界

- 必须保持兼容的对象：
  - `cadence/` 目录下的文档归档规则
  - 中文文档与中文交互要求
  - 未来对 Claude Code / Codex 的 CLI 接入方式可替换但不应破坏核心状态模型
- 本次允许变更的对象：
  - Aria 一期的内部运行时模型
  - 文档结构中新增设计文档
- 本次明确不改的对象：
  - `.claude/rules/` 框架规则
  - `cadence/project-rules/` 中的现有示例与规则说明

## 2. 设计概述

### 2.1 需求背景

目标是构建一个类似 Claude Code 的终端 REPL，但其本质不是单轮聊天工具，而是一个本地长期运行的多 Agent 编排系统。用户输入一句需求或一份较完整方案后，系统需要驱动 Claude Code 与 Codex 按角色分工完成需求澄清、规格文档、设计文档、设计评审、计划制定、任务分发、并行编码、并行测试、代码评审、串行集成与最终总结。

### 2.2 设计目标

- 提供一个本地优先、终端优先的 Aria REPL 入口。
- 让后台 daemon 能在终端关闭后继续运行任务，并在后续重连时恢复状态。
- 在单仓库范围内支持多任务并行执行，同时保证串行集成。
- 明确 Claude Code 与 Codex 的角色边界，并通过 provider adapter 隔离具体接入方式。
- 让系统既能产出用户可审阅的文档，又能维持 daemon 可恢复的内部状态。
- 一期只使用 `spawn + CLI` 接入外部 Agent，但为后续升级到 SDK / app-server 预留清晰边界。

### 2.3 设计范围

**包含**:
- 本地 REPL 与本地 daemon 的总体架构
- Claude Code / Codex 的角色分工与 provider adapter 边界
- 单仓库多任务并行的状态机与 worktree 隔离策略
- 文档产物与内部状态的双轨模型
- 人工闸门、失败恢复、重连恢复与串行集成机制
- 一期 MVP 与目标态蓝图的双层设计

**不包含**:
- 多用户共享 runtime
- 远程服务化部署与多租户治理
- 通用插件市场与通用工作流 DSL
- 一期内的全屏 TUI 或桌面 GUI
- 一期内对任意 Git 仓库的零配置完全适配

### 2.4 成功标准

- 用户可通过 REPL 输入自然语言需求，创建并推进一个 EpicTask。
- 系统可完成需求澄清、spec、design、design review、plan、dispatch、coding、testing、review、integration、summary 的闭环。
- 单仓库内多个 WorkTask 可并行执行，且每个任务有独立 worktree。
- daemon 可在 REPL 退出后继续执行，并在用户重连后恢复上下文。
- 人工审批与失败恢复路径明确，不出现静默挂死或不可解释的停滞。
- 一期架构允许后续将 CLI adapter 升级为 SDK / app-server adapter，而不推翻核心状态模型。

## 3. 方案摘要

### 3.1 一句话方案

Aria 一期采用“前台 REPL client + 本地 daemon runtime + provider adapters + task-scoped worktrees + 双轨状态/文档存储”的分层架构，以 Rust core 承担本地编排运行时，以 TypeScript 作为 provider adapter 与后续 UI 层承接点，默认让 Claude Code 负责 orchestrator 职责、Codex 负责 executor/reviewer 职责，并通过固定主状态机加并行 WorkTask 子闭环实现单仓库内多任务并行、串行集成的端到端流程。

### 3.2 方案类型

- [x] 新增功能
- [x] 架构调整
- [x] 数据模型变更
- [ ] 现有功能扩展
- [ ] 缺陷修复
- [ ] 性能优化
- [ ] 其他

### 3.3 兼容性影响

- [x] 无破坏性变更
- [ ] 有破坏性变更

说明：当前仓库尚无既有运行时代码，本次为新系统设计，不引入对现有代码路径的破坏性变更。

### 3.4 双层范围

**目标态蓝图**

- Aria 是一个本地 Agent Runtime，支持 REPL、TUI、桌面 GUI 等多前端壳。
- 核心 runtime 不绑定 CLI 文本格式，而绑定 `session/task/phase/artifact/approval/provider-run/worktree/event` 这些稳定对象。
- provider fabric 允许从 CLI adapter 升级到 Claude Agent SDK、Codex SDK 或 Codex app-server。
- 可观测性从摘要日志升级到结构化事件流与任务时间线。

**一期 MVP**

- 只提供 REPL client + 本地 daemon runtime。
- 只支持单用户、本机、单仓库、多任务并行。
- 默认优化 Cadence/OpenSpec/Superpowers 约定项目。
- 只用 `spawn + CLI` 对接 Claude Code 与 Codex。
- 只做受控并行与串行集成，不做远程服务化与多用户协作。

## 4. 架构与落点设计

### 4.1 当前工程调用链

当前仓库尚无运行时代码调用链，现阶段的事实结构为：

`规则文档 -> 调研文档 -> 设计文档 -> 后续计划与实现`

其中已存在的关键设计输入包括：

- `cadence/analysis-docs/2026-04-22_分析报告_Aria选型摘要_ClaudeCode_Codex_spawn_v1.1.md`
- `cadence/analysis-docs/2026-04-22_分析报告_ClaudeCode_Codex_spawn使用方式与限制调研_v1.1.md`

### 4.2 本次变更落点

| 层/目录 | 新增/修改 | 文件或模块 | 职责 | 放在这里的原因 |
|---------|-----------|------------|------|----------------|
| 文档设计层 | 新增 | `cadence/designs/2026-04-22_技术方案_Aria终端REPL与多Agent编排Runtime设计_v1.0.md` | 记录本次正式技术方案 | 符合项目文档归档规则 |
| REPL client | 新增 | `crates/aria-cli` | 承担用户交互、命令输入、审批响应、重连入口 | 前台进程保持轻量，避免持有关键运行状态 |
| daemon runtime | 新增 | `crates/aria-daemon` + `crates/aria-core` | 持有 session、task、phase、worktree、event log、checkpoint | daemon 是运行时真相源 |
| provider adapters | 新增 | `packages/provider-adapters` | 封装 Claude/Codex 的 `spawn + CLI` 调用 | 为后续 SDK/app-server 升级预留边界 |
| worktree 管理层 | 新增 | `crates/aria-worktree` | 为每个任务创建、维护、回收独立 worktree | 保证并行文件系统隔离 |
| 状态与产物存储层 | 新增 | `crates/aria-state-store` | 持久化事件、快照、文档产物、运行映射 | 同时满足用户审阅与 daemon 恢复 |

### 4.3 明确不落点

- 不把关键运行状态保存在 REPL 进程内，原因：终端关闭后无法续跑。
- 不把所有逻辑直接写入 provider adapter，原因：状态机、审批、worktree、恢复应属于 runtime，而不是接入层。
- 不把一期做成通用插件市场，原因：超出一期边界，会让范围失控。
- 不让多个并发任务共享同一工作目录，原因：与多任务并行目标冲突，且风险过高。

## 5. 契约设计

### 5.1 变更清单

| 类型 | 名称 | 变更说明 |
|------|------|----------|
| REPL 命令 | `status` | 查看当前 session、daemon、任务、审批摘要 |
| REPL 命令 | `tasks` | 查看任务树与阶段 |
| REPL 命令 | `focus <task-id>` | 设定默认焦点任务 |
| REPL 命令 | `attach <task-id>` | 进入某任务上下文 |
| REPL 命令 | `approvals` | 查看待处理闸门 |
| REPL 命令 | `approve/reject/reply <gate-id>` | 处理人工闸门 |
| REPL 命令 | `logs <task-id>` | 查看任务事件和日志摘要 |
| REPL 命令 | `runs <task-id>` | 查看 provider run 记录 |
| REPL 命令 | `retry/pause/resume <task-id>` | 控制任务执行 |
| REPL 命令 | `policy` / `policy set ...` | 查询和切换策略模式 |
| REPL 命令 | `worktrees` | 查看 worktree 健康状态 |
| REPL 命令 | `daemon status` / `daemon logs` | 查看守护进程诊断信息 |

### 5.2 请求与输入设计

| 字段 | 类型 | 必填 | 来源 | 说明 |
|------|------|------|------|------|
| `inputKind` | enum | 是 | REPL | `natural_language` 或 `command` |
| `rawText` | string | 是 | REPL | 用户原始输入 |
| `focusTaskId` | string | 否 | REPL/session | 当前焦点任务，便于补充上下文 |
| `policyMode` | enum | 否 | session/config | `conservative`、`balanced`、`aggressive` |
| `phaseOverrides` | map | 否 | session/config | 按阶段覆写人工介入策略 |
| `gateId` | string | 否 | command | 处理审批时必填 |
| `taskId` | string | 否 | command | 查询、控制、重试等命令使用 |

### 5.3 响应与输出设计

一期不定义 HTTP API，REPL 输出采用“摘要视图 + 可追溯结构化事件”模型。用户可见内容偏摘要，daemon 内部维护结构化对象。

```json
{
  "type": "task_summary",
  "sessionId": "sess_aria_001",
  "taskId": "task_work_003",
  "phase": "testing",
  "status": "blocked",
  "reason": "approval_required",
  "nextAction": "reply gate_014 with missing API contract details"
}
```

### 5.4 错误处理契约

| 场景 | 处理方式 | 错误类型 | 说明 |
|------|----------|----------|------|
| Provider CLI 调用失败 | 记录失败并有限重试，失败后转人工闸门 | `provider_run_failed` | 包含 exit code、stderr、调用元数据 |
| 产物结构不完整 | 阶段失败并触发修复回路 | `artifact_invalid` | spec/design/plan 缺关键信息 |
| 测试失败 | WorkTask 回流到 `rework` | `verification_failed` | 保留失败快照与测试摘要 |
| 集成冲突 | 当前任务退回重做或人工决策 | `integration_conflict` | 集成始终串行，避免批量污染 |
| daemon 崩溃或重启 | 从 checkpoint + event log 恢复 | `runtime_recovery_required` | 恢复后显式标注任务状态 |
| 需求矛盾/不可自动决策 | 进入人工介入终态 | `manual_intervention_required` | 不允许继续猜测推进 |

## 6. 数据与状态设计

### 6.1 领域对象

| 名称 | 用途 | 新增/修改 |
|------|------|-----------|
| `ProjectSession` | 绑定仓库范围内的一次 Aria 主会话 | 新增 |
| `Task` | 表示可推进的任务单元，分 EpicTask 与 WorkTask | 新增 |
| `Phase` | 限定任务所处主阶段 | 新增 |
| `Artifact` | 表示文档产物或内部快照 | 新增 |
| `ApprovalGate` | 表示待人工处理的闸门 | 新增 |
| `ProviderRun` | 表示一次 Claude/Codex 调用执行 | 新增 |
| `WorktreeLease` | 表示任务对独立 worktree 的持有关系 | 新增 |
| `EventRecord` | 记录 runtime 事件日志 | 新增 |

### 6.2 状态设计

| 状态域 | 存放位置 | 更新触发 | 说明 |
|--------|----------|----------|------|
| Session 状态 | daemon 持久化存储 | REPL 新建/恢复会话 | 记录 session 级配置与上下文 |
| Task/Phase 状态 | daemon 持久化存储 | 阶段推进、失败、审批、集成 | 作为运行时真相源 |
| Provider 映射 | session 配置 | session 创建、策略调整 | 默认绑定 Claude/Codex，支持未来升级 |
| Worktree 状态 | daemon 持久化存储 | 任务创建、同步、回收、集成 | 保障并行安全 |
| 文档产物索引 | daemon 持久化存储 + `cadence/` 文档路径 | 阶段完成、产物生成 | 文档是一等产物 |
| 事件日志 | append-only event log | 任何关键事件 | 用于回放、诊断、恢复 |

### 6.3 状态模型原则

- `ProjectSession` 可挂多个 `Task`，但一次 session 只绑定一个仓库。
- `Task` 在任意时刻只处于一个主 `Phase`。
- `Task` 的阶段推进会产生 `Artifact`、`ProviderRun` 和 `EventRecord`。
- 并行执行的最小隔离单元是 `WorkTask + WorktreeLease`。
- 人工闸门由 `ApprovalGate` 显式建模，而不是隐式停机。
- 文档产物与内部状态分离，但通过稳定 ID 关联。

## 7. 关键流程设计

### 7.1 主流程

```text
用户输入需求
-> 创建 ProjectSession / EpicTask
-> clarification
-> spec
-> design
-> design_review
-> design_revision（如需）
-> plan_readiness_check
-> plan
-> dispatch
-> 多个 WorkTask 并行执行 coding/testing/code_review/rework
-> ready_for_integration
-> integration queue 串行集成
-> final_review
-> summary
-> completed
```

### 7.2 异常流程

```text
阶段执行
-> provider/产物/测试/集成失败
-> 记录 EventRecord
-> 判断是否可自动重试
-> 可重试则有限重试
-> 不可重试则挂 ApprovalGate 或退回 rework
-> 用户确认/补充后继续
-> 若不可自动决策则进入 manual_intervention_required
```

### 7.3 时序说明

| 步骤 | 调用方 | 被调方 | 输入 | 输出 |
|------|--------|--------|------|------|
| 1 | REPL client | daemon runtime | 用户自然语言需求 | 新建 `ProjectSession` 与 `EpicTask` |
| 2 | daemon runtime | Claude adapter | 澄清/规格提示与上下文 | clarification/spec 文档与摘要 |
| 3 | daemon runtime | Claude adapter | 设计提示与规格文档 | design 文档 |
| 4 | daemon runtime | Codex adapter | design 文档与 review 任务 | design review 结果 |
| 5 | daemon runtime | Claude adapter | design review 修改点 | 修订后 design 或 readiness 判定 |
| 6 | daemon runtime | Claude adapter | 已确认 spec/design | plan 与 WorkTask 拆分结果 |
| 7 | daemon runtime | 多个 Codex runs | WorkTask + worktree | coding/testing/review 输出 |
| 8 | daemon runtime | 集成队列 | 已完成任务 | 串行集成结果 |
| 9 | daemon runtime | Claude adapter | 全局结果摘要 | final review 与 summary |

## 8. 非功能设计

### 8.1 安全

- 输入校验策略：REPL 命令和参数必须在 daemon 侧做结构校验。
- 权限/操作控制：一期为单用户本地工具，不做多用户鉴权，但需要显式区分自动动作和人工批准动作。
- 敏感信息处理：daemon 日志中不直接明文保存密钥；provider 调用配置应通过环境变量或本地安全配置注入。
- 文件系统隔离：每任务独立 worktree，避免并发任务互相污染。

### 8.2 性能

- 性能目标：
  - daemon 常驻内存与 CPU 占用保持稳定可控
  - 在多任务并发下，任务调度与日志聚合不成为主要瓶颈
  - REPL 重连后的状态恢复应在可感知的短时间内完成
- 优化策略：
  - Rust 负责核心 runtime，降低长时间运行时的内存与 GC 抖动风险
  - 事件日志采用 append-only 设计，checkpoint 周期性压缩恢复成本
  - 受控并发，避免无限制并发导致资源争用

### 8.3 可观测性

- 日志关键字段：`sessionId`、`taskId`、`phase`、`providerRunId`、`worktreeId`、`gateId`
- 指标/埋点：
  - 阶段耗时
  - provider 失败率
  - 自动重试次数
  - 审批等待时长
  - 集成队列等待时长
- 告警阈值：一期主要以 REPL/daemon 诊断输出为主，不做远程告警系统。

## 9. 风险、回滚与发布

### 9.1 风险评估

| 风险 | 影响 | 概率 | 缓解措施 |
|------|------|------|----------|
| CLI 输出与参数行为漂移 | 高 | 中 | provider adapter 隔离、版本探测、真实 smoke 测试 |
| conversation fork 不等于 filesystem fork | 高 | 高 | 以 worktree 作为代码隔离真相源 |
| daemon 长生命周期恢复复杂 | 高 | 中 | event log + checkpoint + 显式 recovering 状态 |
| 多任务并行后的集成冲突 | 高 | 高 | 并行执行、串行集成、失败回流 WorkTask |
| 人工闸门过多导致体验差 | 中 | 中 | 默认保守但支持全局与阶段策略覆写 |
| 范围膨胀为完整平台化系统 | 高 | 中 | 明确一期非目标，暂不做多用户、远程服务化、插件市场 |

### 9.2 回滚方案

由于一期属于新系统建设，回滚重点不在代码兼容而在运行控制：

- 任何自动动作都必须可暂停。
- 任何失败任务都可退回最近安全阶段重新执行。
- 任何 worktree 都可保留现场，供人工排查后决定是否回收。
- 遇到系统级不稳定时，可切回“保守模式 + 单任务串行执行”作为止损方案。

### 9.3 发布注意事项

- 一期先面向单用户本地使用，不承诺多用户或远程部署能力。
- 默认策略采用保守模式，并提供阶段级覆写。
- 必须准备 fake provider 与真实 smoke 两类验证环境，再进入真实编码实现。

## 10. 编码落地清单

### 10.1 文件变更清单

| 文件路径 | 动作 | 说明 |
|----------|------|------|
| `cadence/designs/2026-04-22_技术方案_Aria终端REPL与多Agent编排Runtime设计_v1.0.md` | 新增 | 本次正式技术方案文档 |

### 10.2 实施步骤

1. 先基于本设计文档写出一期实现计划，拆分 runtime、REPL、adapter、worktree、state store、test harness 等工作包。
2. 按 TDD 方式优先实现状态机、事件日志、checkpoint、policy 判定等核心纯逻辑。
3. 实现 daemon runtime 与本地持久化层。
4. 实现 REPL client 与命令集。
5. 实现 Claude/Codex 的 CLI adapter。
6. 实现 worktree 管理与集成队列。
7. 补齐 fake provider、系统级回归测试与真实 smoke 验证。

### 10.3 验证清单

- [x] 角色边界清晰：Claude 偏 orchestrator，Codex 偏 executor/reviewer
- [x] 会话、任务、阶段、审批、产物、worktree、provider run 模型闭环
- [x] 人工闸门与失败恢复路径明确
- [x] 并行执行与串行集成策略明确
- [x] 一期范围与非目标明确
- [x] 二期升级到 SDK / app-server 的边界明确

## 11. 附录

### 11.1 参考资料

- [Aria 选型摘要：Claude Code / Codex 的 spawn 方案](../analysis-docs/2026-04-22_分析报告_Aria选型摘要_ClaudeCode_Codex_spawn_v1.1.md)
- [Claude Code / Codex spawn 使用方式与限制调研](../analysis-docs/2026-04-22_分析报告_ClaudeCode_Codex_spawn使用方式与限制调研_v1.1.md)
- [Vibe Kanban GitHub 仓库](https://github.com/BloopAI/vibe-kanban)
- [oh-my-claudecode GitHub 仓库](https://github.com/Yeachan-Heo/oh-my-claudecode)
- [oh-my-codex GitHub 仓库](https://github.com/Yeachan-Heo/oh-my-codex)

### 11.2 术语表

| 术语 | 说明 |
|------|------|
| REPL | Aria 面向用户的终端交互入口 |
| daemon runtime | 本地后台守护进程，持有系统运行状态 |
| ProjectSession | 绑定单仓库的一次主会话 |
| EpicTask | 用户发起的主任务 |
| WorkTask | 从计划中拆出的可独立执行子任务 |
| Provider adapter | 封装 Claude/Codex 调用方式的接入层 |
| WorktreeLease | 任务对独立 git worktree 的占用关系 |
| ApprovalGate | 需要人工决策的显式闸门 |

### 11.3 变更记录

| 版本 | 日期 | 修改人 | 说明 |
|------|------|--------|------|
| v1.0 | 2026-04-22 | Codex | 初始版本 |
