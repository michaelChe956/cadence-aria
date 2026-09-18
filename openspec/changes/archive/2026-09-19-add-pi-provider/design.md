## Context

现有真实 Provider 分为两条架构：普通 Workspace 与 Coding Workspace 使用 `ProviderName`、Provider Registry 和 WebSocket 流式会话；Task Runner 使用独立的同步 `ProviderType`、静态节点契约和 CLI/API 路由。本变更只扩展前者的 Pi 执行能力；由于 `ProviderType` 同时被流式域的 `StreamingProviderInput` 使用，会增加 `ProviderType::Pi` 类型变体，但后者的调度、路由、兼容性和节点契约仍显式拒绝 Pi。Pi 已提供 CLI、RPC 模式、会话恢复能力和扩展 UI 请求/响应机制，但没有内建的逐工具批准命令。

普通 Workspace 已有授权请求事件和页面弹窗，但没有持久化的角色权限模式，且多数运行点固定为 `Supervised`。Coding Workspace 已有每角色权限模式，默认值为 `Supervised`。Provider 健康检查、前端 Provider 目录和流式 Registry 目前只枚举 Claude Code 与 Codex。

## Goals / Non-Goals

**Goals:**

- 使 Pi 成为活跃 Workspace 与 Coding Workspace 中可选择、可检查和可执行的流式 Provider。
- 让 Pi 与既有运行协议统一支持输出流、取消、会话恢复（Auto 模式）。
- 让 Pi 支持结构化提问（`aria-ask.ts` 扩展），使 Pi 需要用户决策时弹出与 Claude/Codex 一致的选择卡片，答案在同一进程内接续。
- 将可配置角色的默认权限模式统一为 `Auto`；Claude Code 与 Codex 保留既有 `Supervised`，Pi 仅支持 `Auto`。
- Provider 启动或运行失败时直接报错，不在运行期自动切换 Provider（与现有 Claude Code、Codex 行为一致）。

**Non-Goals:**

- 不扩展 Task Runner 的节点契约、CLI/API、Provider router 或同步适配器的可调度 Provider 范围和运行行为；为支持共享 `ProviderType::Pi` 所需的显式拒绝分支除外。Pi 不被 Task Runner 调度、路由或执行。
- 不为 Pi 实现逐工具授权拦截（不做 `tool_call` 钩子式 Supervised）；提问扩展仅注册 `ask_user` 自定义工具，不拦截任何工具调用。
- 不改动添加代码库、仓库初始化或其 Claude Code 专用 Provider 选择。
- 不把 Pi 配置或用户认证信息写入全局 Pi 配置或版本库（提问扩展内容经 `include_str!` 嵌入 Aria 二进制，运行时落缓存目录，不写全局 Pi 配置）。
- 不在 Provider 启动或运行失败后自动重放或切换到其他 Provider。

## Decisions

### 1. Pi 加入流式 Provider 域，`ProviderType` 加变体但 Task Runner 拒绝调度

在产品层将 `Pi` 增加到 `ProviderName` 及其健康、前端 wire value 和 Provider Registry。同时**给共享的 `ProviderType` 加 `Pi` 变体**，原因见下。

背景：`ProviderType` 名义上属 Task Runner 域，但代码中它同时是流式域 `StreamingProviderInput.provider_type` 的类型，且普通 Workspace 与 Coding Workspace 运行链路通过 `provider_type_for_name(ProviderName) -> ProviderType` 的全匹配 match 构造它。`ProviderName` 加 `Pi` 后该 match 必须覆盖 `Pi` 分支才能编译，因此 `ProviderType` 也必须有 `Pi` 变体。不加变体的方案在本仓库结构下不可行。

为守住「Task Runner 运行路径不变」的边界，`ProviderType::Pi` 只作为类型变体存在：Task Runner 的调度入口、Provider router、兼容性矩阵与静态节点契约**显式拒绝** `Pi`（不匹配、不路由、返回既有“不支持/不可用”错误），使 Task Runner 的实际行为与加变体前完全一致。`adapter_compatibility` 兼容性矩阵不为 Pi 增加条目。

这是保持流式域可扩展而冻结 Task Runner 行为的最小代价。被否决的替代方案是把 `StreamingProviderInput.provider_type` 改为 `ProviderName` 以解耦流式域——该改动会波及流式域 20+ 处对 `provider_type` 的读取，影响面过大，不采用。

### 2. 使用 Pi RPC 会话实现流式执行（Auto-only，结构化提问扩展）

新增独立的 Pi 流式适配器。每个运行启动一个独立的 `pi --mode rpc` 子进程（JSONL over stdin/stdout）并加载 Aria 自带的 `aria-ask.ts` 提问扩展；适配器把 Pi 文本、工具、完成、错误和会话事件映射为既有流式 Provider 事件，把取消、提问响应和恢复请求映射回 Pi RPC。

**Pi 仅以 `Auto` 模式运行，不做逐工具授权扩展**——Pi 刻意不包含内建逐工具批准（其文档明确「intentionally does not include built-in permission popups」，需以扩展自建），`--approve` 只是项目资源信任而非工具授权；为实现 Supervised 而自建逐工具拦截扩展会引入每次工具调用的授权 UI 往返与机器可读 payload 的复杂度，超出本变更价值。产品决策：Pi 不提供 Supervised。

**但 Pi 引入结构化提问扩展（`aria-ask.ts`），与 Claude Code 的 `AskUserQuestion` / Codex 的 `requestUserInput` 对齐**：Pi 自身的 LLM 能力覆盖了「发现需求歧义→需要用户决策→提问→等待→带着答案继续」这个完整语义，唯一缺的是结构化提问的通道——Pi 扩展的 `ctx.ui.select()` 在 RPC 模式下经 `extension_ui_request/response` 子协议往返（已由 Phase 0 spike 验证可用）。Aria 在适配器内把 `extension_ui_request(method=select)` 映射为既有 `ProviderEvent::ChoiceRequest`（source 为 `ProviderChoice`），把用户的 `ChoiceResponse` 映射回 `extension_ui_response(value)`，使前端弹出与 Claude/Codex 完全一致的选择卡片，且答案在同一 Pi 进程内接续，上下文完整。

提问扩展与被否决的授权扩展的关键区别：提问扩展不拦截工具调用（不挂 `tool_call` 钩子），只注册一个 `ask_user` 自定义工具让 LLM 在需要澄清时主动调用；提问在整个会话中最多发生少数几次，不是每次工具调用都触发。

**扩展交付方式**：`aria-ask.ts` 的内容经 Rust `include_str!` 在编译期嵌入二进制，运行时按内容哈希写入 Aria 缓存目录（内容不变则复用），`pi -e` 指向该路径。扩展不依赖 npm 安装布局、不写入用户全局 Pi 配置或项目版本库。用户零安装步骤。

Pi 的 `Auto` 运行与其他 Provider 的 `Auto` 一致：工具调用直接执行，运行事件与工具活动照常记录（审计），无需逐项确认。Claude Code 与 Codex 的 `Supervised` 是其内建能力，不在本变更范围，不受影响。

实施前提与 spike：Pi RPC 的会话粒度、流式事件映射、取消与恢复已由首个 spike 验证可用；`ctx.ui.select()` 经 `extension_ui_request/response` 往返并由同进程续跑已由第二个 spike（A 方案）验证可用（含 LLM 主动调用 `ask_user` 工具、回传 `value` 后同进程继续）。两项均通过。

### 3. 按角色持久化权限模式并统一默认 Auto

普通 Workspace 在 session、创建输入、更新消息和前端快照中新增 Author 与 Reviewer 的权限模式；缺失字段以 `Auto` 反序列化，保证已有会话可继续读取。Coding Workspace 保留既有逐角色配置模型，只将各角色新建默认值改为 `Auto`。

两个页面都使用 `Auto` / `Supervised` 的一致文案和既有授权弹窗；选择 Pi 的角色只提供 `Auto`。权限模式变更只影响后续启动的 Provider 运行；活动会话保持启动时的模式，避免运行中改变安全语义。

类型边界：保留普通 Workspace 使用的 `streaming_provider::ProviderPermissionMode` 与 Coding Workspace 使用的 `CodingProviderPermissionMode` 两套独立类型，各自将默认值改为 `Auto`，不合并为单一 enum，以维持最小影响面并避免跨域序列化回归。

### 4. Pi 复用可用性与选择目录，不扩大初始化范围

Pi 健康检查通过其版本命令和既有 Provider 健康状态接口暴露。集中 Provider 目录负责展示名、安装提示、不可用原因和角色选择器选项；已配置但暂时不可用的 Pi 仍以禁用状态保留。

添加代码库及仓库初始化使用单独的 Claude Code 专用选项和执行依赖，不引用通用 Provider 目录中的 Pi。这样既能保持初始化行为不变，也避免未来通用目录扩展意外改变该页面。

## Risks / Trade-offs

- [Pi RPC 事件字段、CLI 协议或 Extension API（自定义工具注册、`ctx.ui.select`、`extension_ui_request/response`）随版本变化] → 将协议解析与扩展交互封装在 Pi 适配器中；健康检查检测并报告不受支持的 Pi 版本，并以录制 RPC fixture 覆盖普通流式事件和结构化提问完整往返。
- [默认 Auto 提升误操作风险] → Claude Code 与 Codex 继续提供每角色 `Supervised`；Pi 仅 Auto，但保留工具事件与运行的审计记录。
- [已有持久化会话缺少权限模式字段] → 读取时默认 `Auto`，写入时持久化显式值，并用回归测试覆盖旧记录。
- [两套权限模式类型并存] → 不合并 `ProviderPermissionMode` 与 `CodingProviderPermissionMode`，避免跨域序列化回归风险。

**fail-fast 边界澄清：** “不切换、重放或重试”指禁止跨到其他 Provider，不禁止同一 Provider 的内部重试（如 Claude/Codex 现有的 artifact 补漏重试、resume-stall 新会话重试）。Pi 不实现同 Provider 内部重试：启动或运行失败即进入终态失败。

## Migration Plan

1. 发布后，既有 Workspace 与 Coding Workspace 配置保持原 Provider 选择；缺失的权限模式以 `Auto` 解释。
2. 健康状态开始报告 Pi；未安装 Pi 的用户看到禁用选项和安装提示，不影响 Claude Code 或 Codex 运行。
3. 如需回滚，停止注册 Pi 适配器与目录条目；已有 Pi 配置保留为不可用值，不破坏历史运行记录或其他 Provider 配置。
