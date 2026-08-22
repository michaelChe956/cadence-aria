# Story 链路弱模型加固与 Token 瘦身

## Why

Aria 的 Story Spec author/reviewer 链路在弱能力模型（claude code+glm-5.3、kimi+deepseek-v4-flash、pi+deepseek-v4-flash）下结构化输出失败率高、修订轮 token 膨胀严重；且 kimi provider 的 bash/grep 工具因 ACP 客户端能力未实现而完全不可用（实测复现：`kimi acp` 下工具调用固定失败，错误文本 `"ACP terminal capability is unavailable"`）。本 change 让 story 链路对弱模型全链路一次结构化成功率 ≥95%（以固定语料 campaign 实测验收）、降低修订轮 provider 实际 input token（目标 ≥40%，以 usage 为准），同时保持 story spec 产物语义内容与结构不变（REQ/AC/NFR/决策记录齐全、不发散）。

## What Changes

- **P0-1 sentinel 协议（nonce 单点校验，全局统一）**：所有设置了 `StructuredOutputContract` 的请求（workspace review、aggregate story/design author、coding review、image prompt iteration、work-item split 等）统一新协议：开始标签携带 nonce 属性；nonce 作为 envelope 字段冗余出现在 JSON 内；结束标签无属性。**不做旧格式兼容**（无需历史迁移：nonce 每次请求新生成、不落盘）。同时收敛 `workspace_engine/parsers.rs` 的重复 sentinel 实现到 `cross_cutting/structured_output.rs` 单一实现，所有消费方 prompt/fixture 在同一次提交原子切换到新格式。
- **P0-2 few-shot 示例**：author artifact 输出契约（初次/修订/retry 三个注入点）与 reviewer 裁决契约末尾注入最小同构示例。防照抄分两类：sentinel 消费方（reviewer 等）示例用不可通过校验的占位 nonce `EXAMPLE_NONCE`（真实 nonce 仅出现在输出模板）；author 示例为不含稳定 ID 与追踪 token 的结构骨架（照抄无法通过 artifact gate），不引入 sentinel nonce、不改动持久化 schema。
- **P0-3 JSON 受限恢复**：仅在已定位且 nonce 匹配的 sentinel 内容内部，用 string/escape-aware 状态机提取唯一完整顶层 JSON 对象并剥离一层包裹 code fence；存在多个候选对象或超限时失败，不做任意"首个/末个"选择。
- **P0-4 severity 三档**：live provider payload 只接受 `blocking`/`must_fix`/`suggestion`；持久化历史回放读取旧 6 档并归一化（`strong_recommend_fix`→`must_fix`，`minor`/`optional`→`suggestion`），归一化后不再写出旧值；`impact` 并入 `message`（规则：非空 impact 以 `"\n影响：" + impact` 追加）。覆盖后端 DTO、WebSocket 类型与 Web 前端类型/渲染。
- **P1 prompt 滑动窗口（含修订主入口）**：author 增量 prompt（`build_prompt`、`build_revision_full_prompt`）与 reviewer prompt（`build_review_input`）的历史重放改为滑动窗口；对 provider 原生 resume 的服务端历史累积，先实测 usage 再决定是否在窗口边界切换 fresh session + 确定性压缩上下文。
- **P2a kimi ACP 客户端服务**：kimi adapter 声明 `{"fs":{"readTextFile":true,"writeTextFile":true},"terminal":true}`（kimi dialect），实现反向调用的 `terminal/create|kill|release|wait_for_exit` 与 `fs/read_text_file|write_text_file`；以真实 transcript 冻结 wire fixture；终端/fs 请求接入 `ProviderPermissionMode` 权限门与路径沙箱（授权根前缀 + no-follow），设输出上限、命令超时、进程组 kill。
- **P2b kimi MCP 注入（policy-envelope 受控）**：`mcpServers` 不再硬编码 `[]`，改为经 session policy envelope 校验的受控 bundle（allowlist + digest + 凭据脱敏），覆盖 `session/new` 与 `session/load`。
- **P3 实测 campaign**：固定 5 需求语料（含待确认项、含返修轮形态）；开发迭代期每组合 5 样本快速反馈；最终验收 gate 每组合 20 样本。采集 author/reviewer/full-chain 三个一次成功率口径、retry 分布、失败分类、provider 返回的 input-token usage（fresh/resume 分列）、golden 规范化字段 diff。

## Capabilities

### New Capabilities

- `story-pipeline-weak-model-hardening`: story author（artifact fence 口径）与全链路结构化成功率、sentinel 协议、prompt token 预算、severity 归并与 campaign 验收要求。
- `kimi-acp-client-services`: kimi ACP adapter 的客户端服务能力（terminal/fs wire 契约、权限与沙箱、生命周期、MCP 受控注入）。

### Modified Capabilities

- `session-policy-envelope`: MCP bundle 注入扩展到 kimi ACP `mcpServers`（`session/new`/`session/load`），保持"Aria-owned、受控、digest 审计"的既有要求，新增 kimi dialect 的注入路径与 resume 配置一致性校验。

## Impact

- `src/cross_cutting/structured_output.rs`、`src/product/workspace_engine/parsers.rs`（协议收敛，全局影响 workspace/coding/image/work-item-split/fake 消费方）
- `src/product/workspace_engine/prompts.rs`、`prompts/review.rs`、`prompts/revision.rs`、`prompts/reviewer_context_filter.rs`（prompt 构建与压缩）
- `src/product/workspace_engine/review/structured_output.rs`、`src/product/workspace_engine/parsers.rs` findings 解析、`src/web/workspace_ws_types/review.rs`、`web/src/state/workspace-ws-store-types.ts`、`web/src/components/chat-workspace/entries/ReviewVerdictEntry.tsx`（severity 三档全栈）
- `src/cross_cutting/kimi_code_provider/session.rs`、`mod.rs`、`src/cross_cutting/json_rpc_peer.rs`（ACP 客户端服务与请求分发）
- 依赖：终端执行优先 pipe + 进程组方案；仅当 wire fixture 证明 kimi 依赖 TTY 时引入 pty crate。
- 安全：terminal/fs 具备宿主执行与写入能力，必须过权限门与路径沙箱（见 design Risks）。
