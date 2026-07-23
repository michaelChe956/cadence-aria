## Context

当前 `POST /api/projects/{project_id}/repositories` 在同一 HTTP 请求中依次完成 Cadence-skills 准备、Git 状态采集、四次 Claude Code 初始化及 Repository 持久化；前端仅在请求结束后得到最终成功或错误。现有全局 SSE 和 Workspace/Coding WebSocket 都服务于其他领域协议，尚未发布或消费代码库注册事件。

已确认的产品行为是：显示五个真实离散步骤而非百分比；初始化期间弹窗不可关闭；失败停在失败步骤并展示现有恢复信息；代码库只在全部初始化成功后出现于列表。四个 Claude Code 命令必须以 Cadence-skills README 规定的 `--no-interrupt` 模式执行。用户明确排除 KnowledgeBase 与 cadence-workflow 范围。

## Goals / Non-Goals

**Goals:**

- 将代码库初始化从同步 HTTP 等待改为有持久化状态的异步操作，并允许客户端轮询真实快照。
- 用唯一初始化操作关联请求、目标 Project、代码库输入、五个固定步骤、失败诊断和最终 Repository 结果。
- 让后端在每个步骤开始与结束时原子更新状态；任何失败阻止后续步骤与 Repository 持久化。
- 在不引入伪进度的前提下，为 React 弹窗提供可访问的、可靠的五步状态界面。
- 保持现有成功摘要、错误结构、Git 变更路径保护和同一路径初始化互斥的语义。

**Non-Goals:**

- 不流式传输 Claude 输出、工具调用细节或步骤内百分比。
- 不在本变更中提供用户取消、关闭后后台继续、重试同一 operation、SSE 或 WebSocket 订阅。
- 不改变 Cadence-skills 的实现、其下载源、软链策略或 Claude Code Provider 的权限策略。
- 不在失败后执行破坏性回滚，也不自动补偿目标仓库中的部分文件改动。

## Decisions

### 1. 采用持久化 operation 加 HTTP 轮询，而非 SSE 或专用 WebSocket

代码库注册入口会创建 `repository_initialization` operation，并立即返回 `202 Accepted`、操作标识和初始快照。前端在弹窗存续期间以短间隔请求 operation 快照，直至 `completed` 或 `failed`。

操作状态需要落在现有产品运行时存储路径中，而不是仅保留在进程内内存：这使刷新页面、短暂重连或轮询错过某次更新后仍可获取最终真实状态和诊断信息。operation 包含规范化的请求输入、`created/running/completed/failed` 生命周期、固定顺序的五个步骤、错误详情、最终成功响应及完成时间。

选择轮询是因为当前需求只需五项低频状态变更，不需日志流；它避免把 repository 注册事件耦合进现有通用 SSE，也避免新建并维护专用 WebSocket 的连接、重连与游标协议。轮询终态后停止，避免持续请求。

**替代方案：**
- 复用 `/api/events` SSE：已有 replay 基础设施，但必须新建全局事件类型、生产/过滤和前端消费者，当前范围过度设计。
- 专用 WebSocket：适合后续日志流与取消，但对离散步骤状态的连接生命周期成本过高。
- 保持同步 POST 并仅增加 spinner：无法真实区分五个步骤，不满足需求。

### 2. 固定步骤是后端唯一事实来源，并按严格顺序推进

operation 在创建时生成以下五项，顺序和值均固定：

1. `cadence_skills`：Cadence-skills 下载/更新与三层软链同步；
2. `rule_config`：`/rule-config --no-interrupt`；
3. `pre_check`：`/pre-check --no-interrupt`；
4. `mcp_configuration`：`/mcp-configuration --no-interrupt`；
5. `project_rules_examples`：`/project-rules-examples --no-interrupt`。

初始状态全为 `pending`。协调器在实际调用每一项前先写入该项 `running`，只在调用成功返回后写为 `completed`。任一错误将当前项写为 `failed`，operation 写为 `failed`，后续项保持 `pending`，并保存经既有脱敏/截断策略处理的诊断、可恢复标志与 Git changed paths。全部步骤完成、最后 Git 状态采集和 Repository 持久化成功后，operation 才写为 `completed` 并保存与当前成功响应等价的最终结果。

Cadence-skills 步骤作为 operation 的第一项，封装当前 `prepare_skills` 调用。Claude 初始化器接收一个进度报告接口或回调，以便每一个实际 Provider turn 发生状态转换；业务逻辑不依据前端时间或轮询次数推断进度。

**替代方案：**将第一项拆成下载、更新和每层软链子步骤。拒绝原因是产品已确认仅需一个 Cadence-skills 步骤，拆分会改变固定五步信息架构并增加不必要的 UI 噪声。

### 3. 用“启动 + 查询”API 取代同步创建 API，并保持最终结果形状

现有创建请求路径保留为启动 operation 的入口，但语义改为异步接受。响应包含 `operation_id` 和用于立即显示的 operation 快照；新增只读 operation 查询路径。快照应包含 operation 状态、五个步骤及其状态、当前/失败步骤标识、最终成功结果（仅 completed）、最终错误（仅 failed）、创建/更新时间。

最终成功 payload 保持现有 `repository` 与 `initialization` 形状，避免破坏成功面板和其它未来调用方。operation `failed` 时沿用现有 `RepositoryRegistrationError` 到 API error details 的结构，使 UI 能保留 stage、command、reason、retryable、action 与 changed paths。

启动后后台任务使用应用状态中可管理的 Tokio task 生命周期，operation 持久化失败或无法启动时立即在启动 API 返回结构化错误。创建请求接收后立即返回，不在 HTTP handler 内等待所有步骤。相同 Git 路径的既有初始化锁仍由后台注册协调器持有到终态；重复启动必须报告当前语义等价的冲突，不能创建两个会竞争的 operation。

### 4. 前端表单成功提交后替换为不可关闭的进度面板

用户提交表单并获得 operation 初始快照后，弹窗替换表单为进度面板：标题、`已完成 N / 5`、`aria-live="polite"` 的当前状态说明，以及五个顺序步骤。每步用一致的 Lucide SVG 图标、文字和非颜色独占状态：等待、执行中、已完成、失败。执行中项使用稳定的旋转 loading 图标，完成项用勾选，失败项用错误图标；尊重 `prefers-reduced-motion`。

operation 处于 `created` 或 `running` 时，“关闭”和“取消”保持禁用，面板展示“正在初始化，请保持此窗口打开”。完成后显示既有成功摘要和“完成”按钮；失败后显示当前结构化错误、失败步骤和“修复问题后可以重新提交”，恢复为可再次提交的表单，而非为旧 operation 增加重试协议。工作台只在 completed snapshot 包含 Repository 后刷新列表。

轮询逻辑独立于现有 Workspace/Coding WebSocket hooks；它处理初始查询、周期查询、卸载时清理、网络错误提示与终态停止。临时轮询错误不改变后端 operation 或虚构失败步骤；页面仍可继续轮询，直至后端给出终态或用户明确关闭终态弹窗。

### 5. 无中断提示词使用完整 token 形式

`ClaudeRepositoryInitializer` 的四个固定命令改为 README 明确支持的完整参数：`--no-interrupt`。它们作为单独的 Claude Provider turn 执行；命令字符串也必须原样出现在 operation 步骤和最终初始化摘要中，以便错误诊断和测试能确认无中断模式。

该模式禁止命令发起用户提问或等待输入；若 Provider 仍发出 permission/choice 请求，既有初始化器中止 session，将对应步骤标为失败，并返回现有“需要交互”的结构化恢复信息。

## Risks / Trade-offs

- [应用或进程在 operation 运行中退出] → operation 可能遗留 `running`；启动时将未完成 operation 归类为可诊断的中断失败，或在查询时提供明确的不可恢复终态，绝不无限显示执行中。
- [轮询带来额外请求] → 仅在可见的初始化弹窗和非终态期间按固定短间隔请求；终态、卸载与关闭时清理计时器。
- [目标库在失败前已有部分改动] → 保持现有 Git 前后状态采集、changed paths 与“不破坏性回滚”提示。
- [持久化 operation 与 Repository 写入的部分失败] → operation 明确保存失败状态和诊断；Repository 只有在所有步骤成功后创建，持久化 Repository 失败也使 operation 失败并保留 changed paths。
- [API 语义从 201 同步成功变为 202 异步接受] → 同一应用前端随本变更同步更新；集成测试覆盖新契约。此为内部 API 演进，不承诺旧客户端兼容。
- [用户在运行中刷新或离开页面] → 后端继续完成操作；重新打开或重新查询可获取持久化真实状态。当前 UI 在同一弹窗中禁止主动关闭，但不把浏览器关闭等同于取消。

## Migration Plan

1. 增加 operation 领域模型、持久化与启动/查询 HTTP 契约，同时保留协调器的核心初始化、锁和错误诊断责任。
2. 将注册执行改由 operation 后台任务驱动，确保五个状态更新在每项真实调用边界发生。
3. 更新前端类型、API client 和弹窗，使其消费 operation 快照并在终态刷新。
4. 通过后端和前端回归测试验证顺序、失败、终态恢复与轮询停止，再进行完整质量门禁。
5. 发布后新提交均使用异步 API；不存在数据迁移，因为 operation 是新运行时记录，现有 Repository 记录不需要转换。

回滚时可部署上一版本；已经创建的 operation 记录是附加数据，旧版本可忽略。对已经运行的 operation 不承诺跨版本继续执行，但其对目标仓库的任何部分变更仍须由用户按现有错误提示检查。

## Open Questions

- 无。已确认使用轮询、固定五步、运行中不可关闭、不展示步骤内进度，并使用 `--no-interrupt`。
