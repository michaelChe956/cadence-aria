## Context

当前后端（Rust/axum）所有 AI 能力通过 CLI 子进程（Claude Code / Codex）驱动，**没有任何对外 HTTP/LLM 客户端**（`Cargo.toml` 无 `reqwest`）。prompt 迭代所需的「发 prompt → 拿流式输出 + 多轮续接」能力已由现有 `StreamingProvider`（`StreamingProviderInput { prompt, working_dir, resume_provider_session_id, structured_output_contract, ... }`）提供，且 prompt 是普通字符串、不限于编码。现有 `.aria` 状态目录（`AriaStatePaths`）是平台持久化用户态的既定位置。前端为 React + TanStack Router + Tailwind + Zustand，已有 `chat-workspace` 消息流组件可复用。

动机见 `proposal.md - Why`；可验收行为见 `specs/image-create-agent/spec.md`。本设计只记录已确认的架构边界与权衡。

## Goals / Non-Goals

**Goals:**

- 引入平台首个对外 HTTP 出站链路（`reqwest`），专用于图片生成，与现有 CLI 链路解耦。
- 复用 `StreamingProvider` 承载 prompt 迭代，复用 `AriaStatePaths` 承载配置持久化，最大化复用既有基建。
- 保持 image2 `api_key` 仅在后端、绝不进入 CLI 子进程的安全边界。
- 使图片创作与编码域（Project/Issue/Coding Workspace）完全解耦，作为独立顶层能力。

**Non-Goals:**

- 不引入新对话 LLM（prompt 迭代复用 CLI 执行器）。
- 不做多用户隔离与 `api_key` 加密落盘（与现有 `.aria` 明文状态文件同级）。
- 不支持执行器自主触发图片工具调用（生图始终由用户显式触发）。
- 不改造现有编码域任何 spec/行为。

## Decisions

### D1：prompt 迭代复用 CLI 执行器，而非新增 HTTP 对话 LLM

- **选择**：用现有 CLI 执行器（Claude Code / Codex 等）跑 prompt 迭代，通过 `resume_provider_session_id` 续接多轮，用 `structured_output_contract` 让其把「建议 prompt」结构化产出。
- **理由**：零新基建（不需引入对话 LLM 的 HTTP 客户端与第二套 key），复用平台已有执行器健康检测与会话续接；prompt 迭代是纯文本任务，CLI 执行器足够。
- **备选**：新增 OpenAI 兼容对话 LLM 客户端（更轻更快，但多一套 key/配置与 HTTP 链路）——已与用户讨论后否决。
- **代价**：CLI 执行器偏重、较慢、偏贵；prompt 迭代运行在临时 scratch 目录（无 git）。

### D2：两阶段解耦，用户显式触发生图（模型①）

- **选择**：执行器只做 prompt 迭代（文本）；图片生成由用户点按钮触发，Aria 后端用最终 prompt + 参数调用 image2。两者唯一耦合是「最终 prompt + 参数」从前端传给 `/generate`。
- **理由**：与「prompt 满意了再去生图」的既定流程吻合；可预测、不费钱；key 留在后端不进子进程；图片能干净回传浏览器。
- **备选**：给执行器挂 `generate_image` 工具让其自主生图（模型②）——已否决，因子进程难干净回传图片、key 泄露风险、可能未确认即生图。

### D3：独立顶层 + 临时 scratch 目录

- **选择**：新增 `/image-create` 顶层入口，与 `/workbench` 平级；每个会话用独立 scratch 目录承载执行器，不依赖 git/worktree。
- **理由**：图片创作与代码仓库无关，挂载到 Project/Issue 会硬塞进编码域；scratch 目录满足执行器对 `working_dir` 的要求即可。
- **代价**：需新建独立的「图片创作会话」session 存储与路由（存储实现可借鉴现有 `workspace_session`，但语义独立）。

### D4：参考图按有无自动选端点

- **选择**：有参考图 → `/v1/images/edits`（multipart `image[]` + prompt）；无参考图 → `/v1/images/generations`（json）。单张可选参考图，后端自动选端点，对用户透明。
- **理由**：`generations` 不接受参考图，要「以图为内容」必须走 `edits`；单张覆盖绝大多数「照着改」需求，避免多图上传 UI 过度复杂。
- **备选**：支持最多 16 张参考图——已否决（PPT/流程图场景用不到，UI 复杂度高）。

### D5：配置存 `.aria`，后端持有 key，前端脱敏

- **选择**：`base_url` / `api_key` / 默认参数存 `<workspace>/.aria/image_create.json`；`GET` 返回脱敏 key，`PUT` 写盘；key 仅后端 `reqwest` 使用。
- **理由**：与现有 `.aria` 状态惯例一致；浏览器直连 image2 网关存在 CORS 与 key 明文暴露风险；后端托管最安全可控。
- **代价**：key 明文落盘（与现有 provider_health 等状态文件同级），非加密。

### D6：参数枚举化、页面下拉框呈现

- **选择**：`size`/`quality`/`background`/`output_format`/`n` 用预定义枚举，前端下拉框；`n` 页面限制 1–4（API 上限 10，限制以控成本）；`input_fidelity`（仅 edits）下拉。
- **理由**：参数取值是有限枚举，下拉框优于自由输入，降低出错；`n` 收窄避免一次生图过多费钱。

## Risks / Trade-offs

- [CLI 执行器用于 prompt 迭代偏重/慢/贵] → 接受（用户已确认复用执行器）；后续可观察耗时再优化。
- [`api_key` 明文落盘 `.aria`] → 明确为非目标（与现有明文状态同级）；文档与设置界面提示风险；不在本期做加密。
- [新增 `reqwest` 首个对外 HTTP 客户端，引入供应链面] → 锁版本、限定 `json`/`multipart` feature；图片客户端单元测试覆盖端点选择与错误归一。
- [colorflowai 等网关参数枚举可能随官方变化] → 参数枚举以前端常量集中维护，便于后续增项；网关错误透传给用户。
- [执行器续接 session 在长会话/异常后可能失效] → 失败时以新 session 续跑，保留已沉淀的 prompt 区块供用户继续编辑。
- [scratch 目录生命周期未管理可能导致残留] → 会话删除时清理其 scratch 目录。

## Migration Plan

- 本变更新增独立能力，不触碰现有编码域，无数据迁移。
- 部署：新增 `/api/image-create/*` 路由与前端路由；`reqwest` 随后端发版引入。
- 回滚：移除 `/image-create` 前端路由与 `/api/image-create/*` 后端路由即可完全回退；`.aria/image_create.json` 为可选文件，删除不影响现有功能。
