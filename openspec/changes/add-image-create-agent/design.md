## Context

当前后端（Rust/axum）所有 AI 能力通过 CLI 子进程（Claude Code / Codex）驱动，**没有任何对外 HTTP/LLM 客户端**（`Cargo.toml` 无 `reqwest`）。prompt 迭代所需的「发 prompt → 拿流式输出 + 多轮续接」能力已由现有 `StreamingProvider`（`StreamingProviderInput { prompt, working_dir, resume_provider_session_id, structured_output_contract, env_vars, timeout_secs, ... }`）提供，且 prompt 是普通字符串、不限于编码；其 `env_vars` 由调用方显式提供，因此可保证不向子进程注入 key。现有 `.aria` 状态目录（`AriaStatePaths`，当前仅提供 `.aria` 根与 provider-health 路径）是平台持久化用户态的既定位置，需为本能力新增专用路径方法。前端为 React + TanStack Router + Tailwind + Zustand，`chat-workspace` 的消息流展示组件可复用，但其 `ChatEntry` 类型与输入栏绑定 Coding/Workspace 领域语义，图片域需独立的 entry 模型与专用输入栏。

动机见 `proposal.md - Why`；可验收行为见 `specs/image-create-agent/spec.md`。本设计只记录已确认的架构边界与权衡。

## Goals / Non-Goals

**Goals:**

- 引入平台首个对外 HTTP 出站链路（`reqwest`），专用于图片生成，与现有 CLI 链路解耦。
- 复用 `StreamingProviderAdapter` 承载 prompt 迭代，复用 `AriaStatePaths` 模式承载配置与会话持久化，最大化复用既有基建。
- 保持 image2 `api_key` 仅在后端、绝不进入 CLI 子进程，并对出站目标施加安全约束。
- 使图片创作与编码域完全解耦，作为独立顶层能力。

**Non-Goals:**

- 不引入新对话 LLM（prompt 迭代复用 CLI 执行器）。
- 不做多用户隔离与 `api_key` 加密落盘（与现有 `.aria` 明文状态文件同级）。
- 不支持执行器自主触发图片工具调用（生图始终由用户显式触发）。
- 不支持多图批量生成（每次生成恰好一张）。
- 不提供模板的持久化 CRUD（自定义引导词为一次性会话引导词）。
- 不改造现有编码域任何 spec/行为。

## Decisions

### D1：prompt 迭代复用 CLI 执行器，而非新增 HTTP 对话 LLM

- **选择**：用现有 CLI 执行器跑 prompt 迭代，通过 `resume_provider_session_id` 续接多轮，用 `structured_output_contract` 约束其结构化产出「建议 prompt」。
- **理由**：零新对话基建，复用平台已有执行器健康检测与会话续接；prompt 迭代是纯文本任务，CLI 执行器足够。
- **备选**：新增 OpenAI 兼容对话 LLM 客户端（更轻快，但多一套 key/配置与 HTTP 链路）——已与用户讨论后否决。
- **代价**：CLI 执行器偏重/慢/贵；运行在临时 scratch 目录（无 git）。

### D2：两阶段解耦，用户显式触发生图（模型①）

- **选择**：执行器只做 prompt 迭代（文本）；图片生成由用户点按钮触发，Aria 后端用最终 prompt + 参数调用 image2。两者唯一耦合是「最终 prompt + 参数」从前端传给生成端点。
- **理由**：与「prompt 满意了再去生图」流程吻合；可预测、不费钱；key 留后端；图片能干净回传浏览器。
- **备选**：给执行器挂 `generate_image` 工具让其自主生图——已否决（子进程难干净回传图片、key 泄露、可能未确认即生图）。

### D3：独立顶层 + 临时 scratch 目录 + 独立会话存储

- **选择**：新增 `/image-create` 顶层入口，与 `/workbench` 平级；每个会话用独立 scratch 目录承载执行器；图片创作会话使用独立的 session 仓库与运行注册表（借鉴现有 `workspace_session`/`coding_socket_registry` 模式，但语义独立）。
- **理由**：图片创作与代码仓库无关；scratch 目录满足执行器对 `working_dir` 的要求。
- **代价**：需新建独立的会话存储、运行注册表与取消机制。

### D4：参考图按有无自动选端点，参考图随生成请求直接 multipart 传输

- **选择**：有参考图 → `/v1/images/edits`（multipart `image[]` + prompt）；无参考图 → `/v1/images/generations`（json）。单张可选参考图，后端自动选端点，对用户透明。参考图**随生成请求直接以 multipart 传输**，不在 scratch 或磁盘持久化。
- **理由**：`generations` 不接受参考图；单张覆盖绝大多数「照着改」需求；直接 multipart 避免参考图的临时文件生命周期管理。
- **备选 1**：参考图先存 scratch 再引用——已否决（引入额外文件生命周期与清理负担）。
- **备选 2**：支持最多 16 张参考图——已否决（场景用不到，UI/校验复杂度高）。

### D5：配置存 `.aria`，后端持有 key，前端脱敏，设置采用保留语义

- **选择**：`base_url` / `api_key` / 默认参数存 `<workspace>/.aria/image_create.json`；`GET` 返回脱敏 key，`PUT` 写盘；key 仅后端 `reqwest` 使用。设置 `PUT` 采用保留语义：`api_key` 为空或为脱敏占位时保留原值，清除需显式动作。
- **理由**：与现有 `.aria` 状态惯例一致；浏览器直连有 CORS 与 key 明文暴露风险；脱敏占位回写会误清空 key，故采用保留语义。
- **代价**：key 明文落盘（与现有状态同级），非加密。

### D6：参数枚举化、单图、页面下拉框呈现

- **选择**：`size`/`quality`/`background`/`output_format`/`input_fidelity` 用预定义枚举，前端下拉框；每次生成恰好一张（不暴露 `n` 多图参数）；`input_fidelity` 仅在参考图改图时发送，文生图时忽略。
- **理由**：参数为有限枚举，下拉优于自由输入；多图批量会引入「丢图/多图结果集/计费」复杂度，MVP 不做；`input_fidelity` 对改图质量有用故保留。
- **备选**：暴露 `n=1..4`——已否决（流程只取 `data[0]` 会丢图，且增加计费复杂度）。

### D7：出站安全约束（base_url 与 Authorization）

- **选择**：`base_url` 仅允许 HTTPS（或本地回环）；image2 请求**默认禁用自动重定向**，如允许则仅限同 origin（scheme+host+port 全同）且拒绝 HTTPS→HTTP 降级、不在跨 origin/降级重定向中携带 Authorization；错误日志/诊断/测试夹具不明文记录 `api_key`。
- **理由**：用户可任意录入 `base_url`，若不约束存在 SSRF 与经重定向泄露 key 的风险；仅“跨 host 不带鉴权”不足（同 host 跨端口或 HTTPS→HTTP 降级仍会明文泄露 key），故采用默认禁用重定向 + 同 origin 才跟随的最严策略，兼容用户切换不同 OpenAI 兼容网关。
- **备选**：host 白名单（仅 colorflowai）——已否决（过度限制用户切换兼容网关的能力）。

### D8：单会话操作互斥，生图不自动重试

- **选择**：每个会话任一时刻最多一个进行中后端操作（prompt 迭代或生成），新请求被拒绝并提示忙碌（不自动排队/取消）；图片生成请求**不自动重试**（防重复计费），失败归一为可读错误，用户可显式重发。
- **理由**：生图是付费且可能耗时的出站请求，自动重试在「未知是否已生成」时会重复计费；单会话互斥避免并发竞态与重复请求。
- **代价**：用户需在失败后手动重发（可接受，因需明确知情计费）。

## Risks / Trade-offs

- [CLI 执行器用于 prompt 迭代偏重/慢/贵] → 接受（用户已确认复用执行器）。
- [`api_key` 明文落盘 `.aria`] → 明确为非目标（与现有明文状态同级）；设置界面提示风险；本期不做加密。
- [新增 `reqwest` 首个对外 HTTP 客户端，引入供应链面] → 锁版本、限定 `json`/`multipart` feature；单元测试覆盖端点选择、错误归一与出站约束。
- [SSRF / 经重定向泄露 key] → D7 约束：HTTPS-only + 默认禁用重定向、仅同 origin 且不降级才跟随。
- [生图自动重试导致重复计费] → D8：不自动重试，用户显式重发。
- [结构化建议 prompt 解析失败] → spec 规定保留上一轮可编辑 prompt 并提示，不展示空 prompt。
- [执行器续接 session 在长会话/异常后失效] → spec 规定识别失效后以新 session 回灌上下文续跑。
- [会话删除与进行中操作并发竞态] → spec 规定删除为线性化操作：先 tombstone 拒新请求、取消并等待在途任务终止确认（或用 generation token 条件写入）、确认无在途写入才清理，删除后异步完成不回写。
- [colorflowai 等网关参数枚举可能随官方变化] → 参数枚举以前端常量集中维护；网关错误透传给用户。
- [结果历史 base64 可能膨胀 `.aria`] → 本期明确接受「无配额」风险：结果纳入会话历史供回看，参考图不持久化；写盘失败时记一条失败事件并提示用户，不静默丢图；后续可评估结果存储配额与淘汰（本期非目标）。

## Migration Plan

- 本变更新增独立能力，不触碰现有编码域，无数据迁移。
- 部署：新增 `/api/image-create/*` 路由与前端路由；`reqwest` 随后端发版引入。
- 回滚：移除 `/image-create` 前端路由与 `/api/image-create/*` 后端路由即可使功能不可用并回退行为；已写入的 `.aria/image_create.json`、会话历史与 scratch 残留**不会自动删除**，需手动清理，但不影响现有编码域能力。
