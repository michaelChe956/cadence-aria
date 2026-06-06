# Workspace 页面内存占用治理验证报告 v1.0

**日期**：2026-06-07
**分支**：`fix_author_confirm_followup`
**计划文件**：`cadence/plans/2026-06-06_计划文档_性能优化_Workspace页面内存占用治理_v1.0.md`

## 验证结论

本轮已完成 Workspace 页面内存占用治理计划中的前端、后端和 E2E 验证闭环。

核心目标已验证：

- 初始 `session_state` 不再携带完整 `timeline_node_details`、完整 Provider Prompt、完整 Execution Output 或完整 Artifact markdown。
- Chat entry metadata 不再复制大 prompt/output/artifact markdown。
- 大型 workspace 默认通过虚拟列表渲染，DOM 节点数受控。
- Provider Prompt、Execution Output、Artifact Version 通过按需 API 加载完整内容。
- Artifact diff 仅加载 selected 和 previous 两个版本。
- 流式输出保留，并通过 buffer/节流降低每 chunk 的更新成本。

## 已运行命令

### 前端

| 命令 | 工作目录 | 结果 |
| --- | --- | --- |
| `pnpm test` | `web/` | PASS：`37` 个 test files，`297` 个 tests |
| `pnpm build` | `web/` | PASS：`tsc -b && vite build` 成功 |
| `pnpm test:e2e -- workspace-memory.spec.ts` | `web/` | PASS：`1 passed` |

备注：`pnpm build` 输出 Vite chunk size warning，`dist/assets/index-D2Hft-hc.js` minified 后约 `556.34 kB`，属于警告，不影响构建结果。

### Rust

| 命令 | 工作目录 | 结果 |
| --- | --- | --- |
| `cargo fmt --check` | repo root | PASS |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | repo root | PASS |
| `cargo check --locked` | repo root | PASS |
| `cargo test --locked` | repo root | PASS |

## E2E 覆盖

新增 `web/e2e/workspace-memory.spec.ts` 覆盖大型 workspace：

- `45` 个 timeline nodes。
- `12` 个 provider stream 节点。
- 多个 Provider Prompt，每个超过 `100KB`。
- 多个 Execution Output，每个超过 `100KB`。
- `5` 个 Artifact versions。

E2E 断言包括：

- Workspace 页面可打开，`chat-entry-list` 可见。
- Provider Prompt / Execution Output 入口可见。
- 通过页面 UI 点击展开后，完整大内容可见：
  - `完整提示词 large-prompt-0`
  - `完整输出 large-output-0`
- WebSocket `session_state` payload 不包含完整大 prompt/output/artifact markdown。
- 按需 API 返回完整大内容。
- DOM node count `< 3000`。

本轮 E2E 日志中 `session_state` payload 约 `65,763` bytes，低于 `< 500KB` 目标。

备注：E2E 期间 Vite 出现 `ws proxy socket error: write EPIPE` 日志；测试最终通过，判断为 WebSocket 关闭时的代理噪声。

## 关键行为验证

### SessionState 轻量化

- 后端新增 `NodeDetailSummary` 与 `ArtifactVersionSummary`。
- `SessionState` 输出 `timeline_node_summaries` 与 `artifact_version_summaries`。
- 兼容字段 `timeline_node_details` 与 `artifact_versions` 默认输出空集合。
- 按需 API 提供完整内容读取：
  - `GET /api/workspace-sessions/:session_id/timeline-node-details/:node_id`
  - `GET /api/workspace-sessions/:session_id/timeline-node-details/:node_id/prompt`
  - `GET /api/workspace-sessions/:session_id/timeline-node-details/:node_id/events/:event_id/output`
  - `GET /api/workspace-sessions/:session_id/artifact-versions/:version`

### 前端状态层

- `ChatEntry` 增加 `content_ref`、`content_size`、`has_full_content`。
- `contentCache` 与 `artifactContentCache` 按 session 切换清空，避免跨 session 污染。
- chat content cache 和 artifact cache 写入均增加 session guard。
- selector 拆分后，`ChatWorkspacePage` 不再订阅完整 store。

### 渲染层

- `ChatEntryList` 使用 TanStack Virtual 虚拟化。
- 虚拟 row 使用动态高度测量 `measureElement` 和稳定 item key。
- Timeline `scrollToEntry` 通过 entry id 到 virtual index 的映射定位。
- 长 Markdown 默认折叠，只渲染预览；展开后按需渲染全文。

### 按需内容

- `InlineEventRow` 支持 Provider Prompt / Execution Output 按需加载。
- `ArtifactPane` 支持 summary-only versions，按需加载 selected markdown。
- Diff 模式只加载 selected 与 previous。
- 加载错误提供明确错误提示和重试路径。
- session 切换、组件卸载、虚拟列表卸载场景已补回归测试，避免旧请求污染新 session cache。

## 残余风险

- `build_session_state()` 当前仍需要读取完整 `NodeDetail` 后生成 summary，因此 WebSocket payload 已显著降低，但构建快照瞬时内存峰值仍可能受 detail 总量影响。
- 前端生产构建存在 Vite chunk size warning，后续可通过 code splitting 或 manual chunks 优化。
- 真实 `workspace_session_0003` 浏览器内存未在本轮报告中给出人工采样值；本轮使用大型 E2E 的结构性断言和 payload 断言作为自动化守护。
- Story/Design/Work Item 三类 workspace 的共享链路通过通用 Workspace store / Timeline / Artifact 行为覆盖；大型 fixture 当前以 Story workspace 为主。

## 后续建议

1. 对真实 `workspace_session_0003` 做一次浏览器 DevTools 内存采样，记录开发模式和生产构建模式数据。
2. 评估 `build_session_state()` 是否需要进一步改为从持久化 detail 直接读取 summary metadata，降低构建快照瞬时内存峰值。
3. 对 Vite 大 chunk 做独立前端构建优化，不与本次内存治理合并。
