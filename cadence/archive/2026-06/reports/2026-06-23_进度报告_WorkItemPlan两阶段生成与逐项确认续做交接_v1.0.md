# WorkItemPlan 两阶段生成与逐项确认续做交接

## 文档信息

- 文档类型：进度报告
- 日期：2026-06-23
- 版本：v1.0
- worktree：`.worktrees/feat-b-0616`
- 分支：`feat-b-0616`
- 记录时间：2026-06-23 00:37 CST

## 当前 Git 状态

- `git status --short --branch`：`## feat-b-0616...origin/feat-b-0616`
- 本节原始记录时 HEAD：`0262979 feat(work-item-plan): add serial draft generation flow`
- 本地分支与 `origin/feat-b-0616` 对齐。
- 记录前工作区干净。

## 续做暂停记录

- 记录时间：2026-06-23 00:41 CST
- 追加记录前 HEAD：`c4b129a docs(work-item-plan): record staged flow handoff`
- `git status --short --branch`：`## feat-b-0616...origin/feat-b-0616`
- 工作区干净，本地分支与 `origin/feat-b-0616` 对齐。
- 当前恢复点：已确认 WP4 已完成并推送，正在复核 WP5 计划与现有 Batch/Draft 代码结构，尚未开始 WP5 代码修改。
- 已确认现有代码具备部分 WP5 基础：
  - `WorkItemDraftRecord.batch_id`
  - `WorkItemGenerationMode::{Serial, Batch}`
  - `WorkItemBatchRecord`
  - `WorkItemBatchStatus::{Generating, Completed, ReviewPending, ReviewDone}`
  - `TimelineNodeType::WorkItemBatchRun`
- 明天继续入口：
  - 先确认 `git status --short --branch`。
  - 重新读取 worktree 内规则文件。
  - 继续实施 WP5，优先按 TDD 新增 `tests/it_web/web_work_item_plan_batch.rs`。
  - 先覆盖 Batch record、拓扑序队列、自动串行生成全部 draft，再继续 validation retry、batch decision 与整组 review。

## Goal 模式退出前暂停记录

- 记录时间：2026-06-23 00:48 CST
- 当前 HEAD：`d608e5c docs(work-item-plan): update handoff pause point`
- `git status --short --branch`：`## feat-b-0616...origin/feat-b-0616`
- 工作区干净，本地分支与 `origin/feat-b-0616` 对齐。
- 按用户指令已停止继续实施 WP5；本轮没有新增测试、没有修改业务代码、没有更新 WP5 plan checklist。
- 已完成的恢复动作：
  - 重新读取 worktree 内 `AGENTS.md`、`CLAUDE.md`。
  - 重新读取 `.claude/rules/` 相关规则。
  - 重新读取 `cadence/project-rules/README.md` 及已启用规则：
    - `cadence/project-rules/build-test-commands.md`
    - `cadence/project-rules/workspace-artifact-bug-triage.md`
  - 执行 `git fetch origin feat-b-0616`，未发现需要合并的新远端提交。
  - 复核 WP5 plan 与当前 Batch/Draft 相关代码结构。
- 本轮确认的 WP5 当前代码状态：
  - `select_work_item_generation_mode(Batch)` 仍只创建 `work_item_batch_run` 占位节点。
  - `workspace_ws_handler.rs` 目前只在 Serial 模式选择后启动 `ProviderRunKind::WorkItemPlanDraft`，Batch 模式尚未启动 provider flow。
  - `WorkItemBatchRecord`、`WorkItemBatchStatus`、`WorkItemDraftRecord.batch_id`、`next_batch_id`、batch/serial draft 语义校验等基础模型与 store helper 已存在。
  - WS 协议已有 `TimelineNodeType::WorkItemBatchRun`，但仍缺 `work_item_batch_confirm`、`work_item_batch_review`、Batch payload 与 Batch decision message。
  - Review parser 已支持 `WorkItemPlanReviewScope::Batch` 的结构化解析基础，但 active node 路由尚未识别 Batch review。
- 明天继续的推荐入口：
  1. 先确认 `git status --short --branch` 为干净。
  2. 重新读取 worktree 内规则。
  3. 从 WP5 TDD RED 开始，新增 `tests/it_web/web_work_item_plan_batch.rs` 并注册到 `tests/it_web.rs`。
  4. 第一批失败测试建议覆盖：
     - `batch_mode_creates_batch_record_for_current_round`
     - `batch_queue_uses_outline_topological_order`
     - `batch_generation_invokes_one_provider_run_per_outline`
     - `batch_generation_does_not_enter_item_confirm`
  5. 实现顺序建议：
     - 新增 Batch payload / confirm / review node type。
     - 选择 Batch 时创建 `WorkItemBatchRecord(status=generating)` 与 batch queue。
     - handler 中接入 Batch provider loop，按拓扑序逐个调用单 item draft prompt。
     - 再补 validation retry、batch decision、整组 review。
  6. Task 6 `downgrade_to_serial` 与 WP6 strict validator 失败入口耦合，实施时先落基础 helper/协议入口；真实触发若依赖 WP6，应在 WP5 plan 中说明保留到 WP6 串联。

## 已完成内容

已完成并推送 WP4：

- Commit：`0262979b522828f1e55a9774a1c0c94743178462`
- Commit message：`feat(work-item-plan): add serial draft generation flow`
- 主要内容：
  - 串行模式单 item Draft 生成。
  - Draft confirm / accept / rewrite / pause。
  - 逐项 reviewer。
  - Draft local validator。
  - downstream invalidation 基础 helper。
  - rewrite / reviewer revise feedback 进入下一次 draft prompt。
  - WP4 plan checklist 与验证命令已同步。

WP4 完成时已验证：

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked --test it_web work_item_plan_serial
cargo test --locked --lib work_item_split_engine
cargo test --locked --lib work_item_split_validator
cargo test --locked --test it_product work_item_plan_store
cargo test --locked
```

## 本次续做前已确认的规则

已在 worktree 内重新读取：

- `AGENTS.md`
- `CLAUDE.md`
- `.claude/rules/language.md`
- `.claude/rules/code-usage.md`
- `.claude/rules/document-storage.md`
- `.claude/rules/markdown-format.md`
- `.claude/rules/mcp-servers.md`
- `cadence/project-rules/README.md`
- `cadence/project-rules/build-test-commands.md`
- `cadence/project-rules/workspace-artifact-bug-triage.md`

关键约束：

- 全程中文回复。
- 继续只在 `.worktrees/feat-b-0616` 工作。
- Rust/Cargo 使用宿主机环境，不使用 Docker。
- 禁止 `cargo test` 携带 `-j 1`。
- 定向单测必须按项目规则限制目标，例如 `cargo test --locked --lib <filter>`。
- 修改功能必须遵循 TDD。
- 手工编辑文件使用 `apply_patch`。
- 完成后必须汇报验证结果。

## 已读取的剩余计划

已读取：

- `cadence/designs/2026-06-22_技术方案_WorkItemPlan两阶段生成与逐项WorkItem确认流程_v1.5.0.md`
- `cadence/plans/2026-06-22_计划文档_实施计划_WorkItemPlan两阶段生成与逐项确认_拆分总览_v1.0.md`
- `cadence/plans/2026-06-22_计划文档_实施计划_WorkItemPlan两阶段生成与逐项确认_WP5_自动Batch生成确认与整组Review_v1.0.md`
- `cadence/plans/2026-06-22_计划文档_实施计划_WorkItemPlan两阶段生成与逐项确认_WP6_FinalCompile事务与恢复_v1.0.md`
- `cadence/plans/2026-06-22_计划文档_实施计划_WorkItemPlan两阶段生成与逐项确认_WP7_前端WorkItemPlan两阶段Workspace_v1.0.md`
- `cadence/plans/2026-06-22_计划文档_实施计划_WorkItemPlan两阶段生成与逐项确认_WP8_贯通验收与回归_v1.0.md`

## 下一步建议

明天从 WP5 开始：

1. 实施 `cadence/plans/2026-06-22_计划文档_实施计划_WorkItemPlan两阶段生成与逐项确认_WP5_自动Batch生成确认与整组Review_v1.0.md`。
2. 先按 TDD 写 `tests/it_web/web_work_item_plan_batch.rs` 的失败测试。
3. 优先完成 Task 1 到 Task 3：
   - Batch record 与队列状态。
   - 自动按拓扑序串行生成全部 draft。
   - local validation 失败自动重试一次，二次失败后记录 `validation_failed_ids` 并继续。
4. 然后实现 Task 4 到 Task 5：
   - `work_item_batch_decision`。
   - 整组 accept/rewrite/pause。
   - 整组 reviewer。
5. Task 6 的 downgrade to serial 与 WP6 strict validator 失败入口耦合较强，实施时需要核对是否应先做基础 helper，还是等 WP6 补全触发路径。

## WP5 重点文件

- `src/product/workspace_engine.rs`
- `src/product/work_item_plan_store.rs`
- `src/web/workspace_ws_types.rs`
- `src/web/workspace_ws_handler.rs`
- `tests/it_web.rs`
- `tests/it_web/web_work_item_plan_batch.rs`
- WP5 plan 文档本身。

## WP5 验证命令

计划内定向命令：

```bash
cargo test --locked --test it_web work_item_plan_batch
cargo test --locked --lib workspace_engine
cargo test --locked --test it_product work_item_plan_store
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
```

若 WP5 完成并准备提交，建议再跑：

```bash
cargo test --locked
```

## 注意事项

- WP0 / WP1 计划文档 checklist 仍可能未完全同步实际提交状态；继续前可根据现有代码和提交记录决定是否先修文档勾选，避免总览显示误导。
- WP5 会继续修改 `workspace_engine.rs` 和 `workspace_ws_handler.rs`，需要特别注意不要影响 Story / Design / 普通 WorkItem 的共享 review / human confirm 流程。
- 若引入 `WorkItemBatchStatePayload`，后续 WP7 前端会依赖其字段命名，建议保持 snake_case serde 输出稳定。
