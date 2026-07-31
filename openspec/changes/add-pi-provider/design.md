## Context

现有真实 Provider 分为两条架构：普通 Workspace 与 Coding Workspace 使用 `ProviderName`、Provider Registry 和 WebSocket 流式会话；Task Runner 使用独立的同步 `ProviderType`、静态节点契约和 CLI/API 路由。本变更只扩展前者。Pi 已提供 CLI、RPC 模式、会话恢复能力和扩展 UI 请求/响应机制，但没有内建的逐工具批准命令。

普通 Workspace 已有授权请求事件和页面弹窗，但没有持久化的角色权限模式，且多数运行点固定为 `Supervised`。Coding Workspace 已有每角色权限模式，默认值为 `Supervised`。Provider 健康检查、前端 Provider 目录和流式 Registry 目前只枚举 Claude Code 与 Codex。

## Goals / Non-Goals

**Goals:**

- 使 Pi 成为活跃 Workspace 与 Coding Workspace 中可选择、可检查和可执行的流式 Provider。
- 让 Pi 与既有运行协议统一支持输出流、取消、会话恢复和授权事件。
- 将可配置角色的默认权限模式统一为 `Auto`，同时保留真实的 `Supervised` 逐工具确认。
- 在无副作用的启动前失败边界提供可审计的 Provider 降级。

**Non-Goals:**

- 不改动 Task Runner 的 `ProviderType`、节点契约、CLI/API、Provider router 或同步适配器。
- 不改动添加代码库、仓库初始化或其 Claude Code 专用 Provider 选择。
- 不把 Pi 配置、扩展或用户认证信息写入全局 Pi 配置或版本库。
- 不在已经执行工具或输出部分结果后自动重放到其他 Provider。

## Decisions

### 1. Pi 只加入流式 Provider 域

在产品层将 `Pi` 增加到 `ProviderName` 及其健康、前端 wire value 和 Provider Registry。Task Runner 专用 `ProviderType` 保持不变，因此新增 Pi 不会扩张或修改 Task Runner 的任何运行路径。

这是把当前真实用户工作流与待清理批处理工作流隔离的最小边界。把 Pi 同时加入两种 Provider 类型会迫使 Task Runner router、兼容性矩阵和静态节点契约定义 Pi 的执行行为，违反本次已确认的范围。

替代方案是在共享 Provider 类型中加入 Pi 并让 Task Runner 明确拒绝它；该方案仍会改变冻结模块的公共契约和匹配分支，故不采用。

### 2. 使用 Pi RPC 会话和 Aria 临时扩展实现流式授权

新增独立的 Pi 流式适配器。每个运行启动 Pi RPC 会话，并临时加载仅属于该会话的 Aria 扩展；适配器把 Pi 文本、工具、完成、错误和会话事件映射为既有流式 Provider 事件，把取消、授权响应和恢复请求映射回 Pi RPC。

扩展是权限策略的执行点：

- `Auto` 在工具调用前直接放行并让适配器记录事件。
- `Supervised` 在工具调用前发出扩展 UI 请求，等待网页经现有授权桥接返回允许或拒绝，再把该结果交还 Pi。

Pi 的 `--approve` 只用于项目资源/配置的信任，不能替代工具授权；因此不把它作为 `Auto` 或 `Supervised` 的实现。

替代方案是仅调用 Pi 的非 RPC JSON 输出模式。它可较快产生文本，却不能在工具执行点等待网页授权或可靠地处理会话控制，不满足已确认的监督交互。

### 3. 按角色持久化权限模式并统一默认 Auto

普通 Workspace 在 session、创建输入、更新消息和前端快照中新增 Author 与 Reviewer 的权限模式；缺失字段以 `Auto` 反序列化，保证已有会话可继续读取。Coding Workspace 保留既有逐角色配置模型，只将各角色新建默认值改为 `Auto`。

两个页面都使用 `Auto` / `Supervised` 的一致文案和既有授权弹窗。权限模式变更只影响后续启动的 Provider 运行；活动会话保持启动时的模式，避免运行中改变安全语义。

### 4. 由运行编排层执行安全降级

自动降级放在 Workspace 与 Coding Workspace 的 Provider 启动编排层，而不是嵌入某个 CLI 适配器。候选顺序为：当前用户选择的 Provider 优先，其余候选按 Claude Code、Codex、Pi 的稳定顺序尝试并跳过已失败项。

仅当失败发生在工具调用、文件副作用或部分输出之前，编排层才启动下一个候选。运行事件记录请求 Provider、失败原因和实际 Provider。适配器一旦发出输出或工具活动，编排层将其视为已开始，不会静默接力，以避免重复写入或重复生成产物。

替代方案是任意错误后直接换 Provider 并重跑。它在编码与带工具的运行中可能重复修改代码，故不采用。

### 5. Pi 复用可用性与选择目录，不扩大初始化范围

Pi 健康检查通过其版本命令和既有 Provider 健康状态接口暴露。集中 Provider 目录负责展示名、安装提示、不可用原因和角色选择器选项；已配置但暂时不可用的 Pi 仍以禁用状态保留。

添加代码库及仓库初始化使用单独的 Claude Code 专用选项和执行依赖，不引用通用 Provider 目录中的 Pi。这样既能保持初始化行为不变，也避免未来通用目录扩展意外改变该页面。

## Risks / Trade-offs

- [Pi RPC 事件字段或扩展协议随 CLI 版本变化] → 将协议解析、扩展载荷和版本检测封装在 Pi 适配器中，并以录制 RPC fixture 覆盖关键事件。
- [会话临时扩展残留或并发运行串扰] → 每次运行使用唯一临时目录与会话标识，运行结束、取消或失败时清理。
- [默认 Auto 提升误操作风险] → 继续提供每角色 `Supervised`，并保留工具事件与授权决定的审计记录。
- [降级误判导致重复副作用] → 以首个输出或工具活动为硬边界；越过边界即失败而非自动重试。
- [已有持久化会话缺少权限模式字段] → 读取时默认 `Auto`，写入时持久化显式值，并用回归测试覆盖旧记录。

## Migration Plan

1. 发布后，既有 Workspace 与 Coding Workspace 配置保持原 Provider 选择；缺失的权限模式以 `Auto` 解释。
2. 健康状态开始报告 Pi；未安装 Pi 的用户看到禁用选项和安装提示，不影响 Claude Code 或 Codex 运行。
3. 如需回滚，停止注册 Pi 适配器与目录条目；已有 Pi 配置保留为不可用值，不破坏历史运行记录或其他 Provider 配置。
