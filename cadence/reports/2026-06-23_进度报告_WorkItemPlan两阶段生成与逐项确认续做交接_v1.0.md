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
- 当前 HEAD：`0262979 feat(work-item-plan): add serial draft generation flow`
- 本地分支与 `origin/feat-b-0616` 对齐。
- 记录前工作区干净。

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
