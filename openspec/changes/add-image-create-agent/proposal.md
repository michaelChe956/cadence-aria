## Why

Cadence Aria 目前是「AI 辅助软件开发平台」，所有 AI 能力仅服务于编码域（通过 Claude Code / Codex 等 CLI 子进程驱动 Story/Design/Work-Item 与 Coding Workspace）。用户需要一个与编码无关的创作能力——**根据自然语言对话生成图片**（典型用途：PPT 配图、业务流程图）。

现有后端没有任何对外 HTTP/LLM 客户端（`Cargo.toml` 无 `reqwest`），也没有图片生成链路。本变更引入一个**独立顶层**的图片创作 Agent：用户通过多轮对话与执行器迭代「图片 prompt」，prompt 满意后由用户显式触发生图，Aria 后端统一调用 `gpt-image-2`（colorflowai 网关）返回 base64 图片。这填补了平台在「图文创作」维度的空白，且复用现有 CLI 执行器做多轮 prompt 迭代，工程增量集中在图片调用链路与配置能力。

## What Changes

- 新增独立顶层页面 `/image-create`，不依赖 Project/Issue/git 仓库，作为与 `/workbench` 平级的新入口。
- 新增多轮「图片 prompt 迭代」会话：复用现有 CLI 执行器（Claude Code / Codex 等所有支持执行器），通过 `resume_provider_session_id` 续接多轮；执行器按结构化约定产出「建议的最终 prompt」为可编辑区块；结构化解析失败时保留上一轮可编辑 prompt，续接失败时以新 session 回灌上下文。
- 新增「图片生成」阶段：由用户点按钮触发，Aria 后端使用新增的 `reqwest` 链路调用 image2 API；按用户是否提供参考图自动选择 `/v1/images/generations`（文生图）或 `/v1/images/edits`（参考图改图），返回 `b64_json` 按其媒体类型在前端直接展示；每次生成恰好一张。
- 新增图片生成的失败与计费边界：网关错误（连接/超时/4xx/5xx/空 data）归一为可读错误，**不自动重试**（防重复计费），失败不写入成功结果。
- 新增单会话操作并发约束：每会话任一时刻最多一个进行中后端操作，忙碌时拒绝新请求。
- 新增会话生命周期管理：删除会话时先取消进行中操作，再清理 scratch 目录与持久化记录。
- 新增「图片创作设置」能力：用户录入 image2 网关 `base_url` 与 `api_key` 及默认参数；配置存于 Aria 后端 `.aria` 目录，前端展示脱敏，**不传递给 CLI 子进程**；`base_url` 仅允许 HTTPS（或本地回环），跨域重定向不携带 Authorization。
- 新增「图片 prompt 模板」机制：预置两套模板（PPT 商务配图、业务流程图），支持用户选模板或填写一次性自定义引导词；引导词注入 prompt 迭代对话（不提供模板持久化 CRUD）。
- 新增图片生成参数的枚举化配置（size / quality / background / output_format / input_fidelity），在页面以下拉框呈现；参考图随生成请求直接 multipart 传输，不持久化。
- 新增后端依赖 `reqwest`（带 `json`、`multipart` feature）——本平台首个对外 HTTP 客户端。

## Capabilities

### New Capabilities

- `image-create-agent`: 独立顶层的图片创作 Agent，覆盖多轮 prompt 迭代会话（含结构化建议 prompt 与续接降级）、一次性模板引导词、单会话并发约束、参数化单张图片生成（文生图与参考图改图）、生成失败与计费边界、按媒体类型展示与历史留存、会话生命周期与资源清理，以及 image2 网关配置（base_url / api_key / 默认参数）的录入、脱敏存储与出站安全约束。

### Modified Capabilities

（无。本变更新增独立能力，不修改现有编码域 specs。）

## Impact

- **后端（Rust）**：
  - 新增对外 HTTP 客户端依赖 `reqwest`（首个对外 HTTP 调用链路，限定 `json`/`multipart` feature，含出站安全约束：HTTPS-only、不跨域携带 Authorization）。
  - 新增图片创作相关 handler（会话 CRUD、prompt 迭代 WS 流、图片生成、settings 读写），挂载在 `/api/image-create/*`。
  - 复用 `StreamingProvider`（Claude Code / Codex）承载 prompt 迭代，**仅传入不含 key 的 env_vars**；扩展 `AriaStatePaths` 提供 image-create 配置/会话/scratch 专用路径；图片创作会话使用独立的 session 仓库与运行注册表。
  - prompt 迭代执行器运行在临时 scratch 目录（无 git）；会话删除时清理 scratch 与记录。
- **前端（web/）**：
  - 新增顶层路由 `/image-create` 及导航入口。
  - 新增图片创作页面与组件（会话列表、聊天区复用 chat-workspace 消息流**展示组件**但使用独立的 image-create chat entry 模型与专用输入栏、可编辑 prompt 区块、参数面板、参考图上传、设置弹窗）。
  - 新增 API client 封装。
- **安全边界**：image2 `api_key` 仅后端持有、用于 `reqwest` 鉴权头；绝不传给 CLI 子进程；不在日志/诊断/测试夹具明文记录；前端不缓存明文。
- **API**：新增 `/api/image-create/sessions`（CRUD）、`/api/image-create/sessions/:id/chat`（WS）、`/api/image-create/generate`（POST，multipart 可选参考图）、`/api/image-create/settings`（GET/PUT，PUT 对 api_key 采用保留语义）等端点。
- **非目标**：不改造编码域；不引入新对话 LLM；不做多用户隔离与 key 加密落盘；不支持执行器自主触发生图；不支持多图批量生成；不提供模板持久化 CRUD。
