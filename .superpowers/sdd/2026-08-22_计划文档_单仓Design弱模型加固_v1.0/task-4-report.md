# Task 4：Design 弱模型 campaign WS driver 实施报告

## 范围

本任务只新增 Design campaign 的单样本 WS driver 与批量循环封装；未修改生产代码，未启动后端，也未发起任何真实 Provider 生成。静态验证仅覆盖 Node 语法、参数解析和冻结语料读取。

## Story fixture 种入方式调研结论

### API 路径：不可行

已检查 `src/web/app.rs`、`src/web/handlers/lifecycle.rs` 与 `src/web/types.rs`。

- 已有 `POST /api/projects/{project_id}/issues/{issue_id}/story-specs:generate`，它只能新建 Draft Story Spec 和 Story workspace，随后必须由 Provider 走生成与确认流程。
- 已有 Story/Design delete 路由，但没有直接 create Story Spec、append fixture version 或把 Story Spec 置为 `confirmed` 的 HTTP 端点。
- `POST /api/issues/{issue_id}/gates/{gate_id}/confirm` 是通用 Gate resolve，不是 Story Spec create/confirm API，不能构造前置 Confirmed Story Spec。

因此，API 方式不能满足“避免跑 Story Provider、直接种入冻结 Confirmed Story Spec”的两阶段设计。

### 文件系统路径：选用并已实现

driver 在通过 HTTP 创建新的单仓 Issue 后，直接向该 Issue 的 `.aria` 生命周期存储写入 fixture：

- `projects/project_0001/issues/<issueId>/story-specs/story_spec_0001.json`：与 `StorySpecRecord` 对齐，`current_version: 1`、`confirmation_status: "confirmed"`、单仓 `repository_id: "repository_0001"`。
- `projects/project_0001/issues/<issueId>/versions/story_spec_0001/version_0001.json`：与 `SpecVersionRecord` 对齐，包含 fixture markdown、版本号、空 provider/review refs 和 `confirmed_by: "campaign_fixture"`。

该布局已以 `.aria/projects/project_0001/issues/issue_0001/` 的真实样例复核；字段结构依据 `src/product/lifecycle_store/spec.rs` 的 `create_story_spec`/`append_version` 和 `src/product/models/outline.rs` 的 `SpecVersionRecord`。新 Issue 的首个 Story Spec 由 `LifecycleStore::create_story_spec` 按每 Issue 目录内文件数产生 `story_spec_0001`，所以固定 ID 在本 driver 的 fresh Issue 前提下与存储规则一致。所有写入使用排他创建，避免意外覆盖已有 fixture。

## `design-specs:generate` 参数确认

路由为：

```text
POST /api/projects/{project_id}/issues/{issue_id}/design-specs:generate
```

`GenerateDesignSpecsRequest`（`src/web/types.rs`）要求：

- `title: string`
- `story_spec_ids: string[]`

并接受 provider 配置：

- `author_provider`
- `reviewer_provider`
- `review_rounds`
- `superpowers_enabled`
- `openspec_enabled`

逻辑代码库专用的可选 `involved_repository_ids`、`change_order` 在本单仓 campaign 中不发送。driver 以种入的 `story_spec_0001` 作为唯一的 `story_spec_ids` 元素，随后从返回体读取 `workspace_session.id` 和 `design_specs[0].design_spec_id`（另兼容同义 ID 字段）。

## 相比 Story driver 的改动

1. **两阶段输入**：不把 design corpus 直接当作 Story generation 的 Issue；先创建含 Design 语料的 Issue，种入对应的冻结 `NN-story-fixture.md` 为 Confirmed Story Spec，再调用 Design generate API。
2. **冻结防护**：在每次运行（包括 `--dry-run`）中读取 `corpus/digests.txt`，校验 Design 语料与 Story fixture 的 SHA-256，不匹配即失败且不联网。
3. **Design 请求**：改为 `design-specs:generate`，显式传 `story_spec_ids`，并记录 `storySpecId`、`designSpecId`。
4. **manifest 对齐**：单样本 `result.json` 按 Task 3 schema 包含顶层 provider/model/model_version、双角色 `role_provider`、`strategy: "fresh"`、`resume_available: false`、`case_id` 和 `boundary_kind`。model/model_version 暂用明确的 CLI 占位符。
5. **使用量披露**：从 WebSocket 事件递归提取可见 `input_tokens`/`prompt_tokens` 与 `cache_read_tokens`/`cache_read_input_tokens`；没有可用数值时写 `usage_unavailable: true`，满足二选一 schema。
6. **反馈返修**：首次 `review_complete` 为 `revise` 或 `needs_human` 时，在返回 `author_confirm` 后发送 `author_decision: { revise: { feedback: "请根据评审意见修订设计" } }`，修订完成后直接 `accept_finalize`。D05 另记录 `first_review_verdict`。
7. **自动化交互与失败归类**：保留选择题首选项回传、permission 自动同意、timeline/WS JSONL 记录；600 秒硬上限以及 author/reviewer/finalize 阶段超时均归为 `driver-timeout`，与 Provider/protocol 错误区分。
8. **批量封装**：`gate-loop.sh` 默认顺序跑 3 provider × 6 shape × 2 repetition，并在已有 `result.json` 时跳过，支持环境变量缩小集合和 `DRY_RUN=1`。

## 未执行项与后续注意

本任务按约束没有进行真实机采集；实施后需要在运行中的后端上先执行有限 sanity 样本。文件系统种入依赖当前 `.aria` JSON store 的单仓路径与 ID 规则；如生产代码改变 `StorySpecRecord`、`SpecVersionRecord` 或存储布局，应先重新审查 driver，再采集 baseline。
