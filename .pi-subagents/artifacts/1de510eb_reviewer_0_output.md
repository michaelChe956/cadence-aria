## Verdict

本次 fix diff（`ccd7e111..98a4e32b`）对上一轮 6 项 finding 均已处理；未发现由该 fix wave 引入的新的 Critical / Important breakage。

1. **Critical：网关错误正文回显 API key 泄露 — ADDRESSED**
   - 非成功 HTTP 响应在构造 `ImageClientError::HttpStatus` 前统一调用脱敏函数，避免未处理的网关 body 沿错误链传播。`src/cross_cutting/image_client.rs:150-156`
   - 脱敏逻辑会替换 body 中**所有**完整 API key 字面量为 `[REDACTED]`，随后按 Unicode 字符截断至最多 500 字符；顺序正确，不会因先截断而保留尾部完整 key。`src/cross_cutting/image_client.rs:12-13,256-262`
   - 错误会经 `ImageCreateError::ImageClient` 返回 REST，并以同一已脱敏 message 写入会话事件；会话存储将事件持久化，因此 REST、事件和持久化路径共享已清理的错误文本。`src/product/image_create/engine.rs:329-377`、`src/web/handlers/image_create.rs:152-163`
   - 新回归测试实际模拟网关回显两处 key，断言 REST body、session event、session JSON 持久化均不含 secret 且含 `REDACTED`。`src/web/handlers/image_create.rs:856-936`
   - 客户端单测另验证回显 body 的 key 不存在、`Bearer [REDACTED]` 存在、字符数不超过 500。`src/cross_cutting/image_client.rs:452-475`

2. **Important：generate 路由仍受 Axum 默认 2 MiB 限制 — ADDRESSED**
   - `DefaultBodyLimit::max(11 * 1024 * 1024)` 仅挂载于 `POST /api/image-create/sessions/{id}/generate`，没有意外扩大其他路由的请求体上限。`src/web/app.rs:33-38`
   - 测试构造大于 2 MiB、且小于参考图 validator 的 10 MiB 上限的有效 PNG，验证成功；同时验证 `MAX_BYTES + 1` 返回 400 和 “10 MiB” 错误，证明路由限额没有替代业务侧严格校验。`src/web/handlers/image_create.rs:802-854`

3. **Important：`openSession` 的 A→B→A 乱序覆盖 — ADDRESSED**
   - 增加全局递增 `openSessionRequestSequence`；每次打开会话捕获 token，并在 API 成功、失败两个异步分支中均拒绝 stale completion。`web/src/state/image-create-store.ts:84,350-372`
   - A→B 和 A→B→A 的乱序返回测试均确认最终保留最新目标会话，且仅创建对应目标的 WebSocket。`web/src/state/image-create-store.test.ts:124-184`

4. **Important：resume 失效识别过宽 — ADDRESSED**
   - fallback 现仅在已有 provider session 的前提下触发，并要求错误具备 session 语境，且为显式 resume+session 失败，或明确 session “not found / missing / expired / invalid / unknown”。`src/product/image_create/prompt_iteration.rs:88-106,236-244`
   - 普通的 `invalid request: prompt is too long` 不再触发新 session fallback；测试断言仅执行一次 provider 调用。`src/product/image_create/prompt_iteration.rs:499-529`
   - 真实 session invalid 仍会走 fallback，且回灌历史上下文；回归未损失。`src/product/image_create/prompt_iteration.rs:531-580`

5. **Minor：OpenSpec `tasks.md` 未勾选 — ADDRESSED**
   - 25 个已完成任务均已改为 `[x]`；检索未发现残余 `[ ]`。`openspec/changes/add-image-create-agent/tasks.md:3-51`

6. **Recommendation：`ImageGenRequest` 请求 body 缺少固定 model — ADDRESSED**
   - 定义固定模型常量 `gpt-image-2`。`src/cross_cutting/image_client.rs:12`
   - 文生图 JSON body 包含 `model`，改图 multipart form 同样包含 `model`。`src/cross_cutting/image_client.rs:117-136,178-186`
   - generations 与 edits 的 mock 匹配测试均明确断言该字段。`src/cross_cutting/image_client.rs:296-330,343-387`

## Evidence

- 本次变更文件共 7 个：
  - `openspec/changes/add-image-create-agent/tasks.md`
  - `src/cross_cutting/image_client.rs`
  - `src/product/image_create/prompt_iteration.rs`
  - `src/web/app.rs`
  - `src/web/handlers/image_create.rs`
  - `web/src/state/image-create-store.ts`
  - `web/src/state/image-create-store.test.ts`
- 新增/更新回归覆盖：
  - API key 回显脱敏、REST / session event / 持久化验证：`src/web/handlers/image_create.rs:856-936`
  - 2–10 MiB 允许、>10 MiB 拒绝：`src/web/handlers/image_create.rs:802-854`
  - 客户端 key 脱敏与 500 字符上限：`src/cross_cutting/image_client.rs:452-475`
  - resume 普通错误负例：`src/product/image_create/prompt_iteration.rs:499-529`
  - A→B、A→B→A 前端竞态：`web/src/state/image-create-store.test.ts:124-184`
  - model 字段 generations / edits：`src/cross_cutting/image_client.rs:296-387`
- 已执行只读 diff 完整性检查：`git diff --check ccd7e111..98a4e32b` 无输出，未见 whitespace error。
- 已检查暂存区：`git diff --cached --name-only` 无输出，即无 staged files。

## Issues

### Critical
无。

### Important
无。

### Minor
无。

### Residual risks
- 实现者声称的 Rust fmt/clippy/311+doc、前端 TypeScript 和 770 测试，本次遵循只读审查限制未重新执行；结论基于 fix diff、实现调用链与新增回归测试的静态核验。
- 脱敏以“完整 API key 原始字面量”替换为边界；若上游将 key 进行非原样编码（例如 JSON 转义或 URL 编码）后回显，不属于当前替换函数覆盖范围。对通常 API key 的直接回显场景，修复完整有效。

**最终结论——分支 Ready to merge：是。** 无残留阻塞项。