# main → feat-b-0808-add-monorepo 合并报告

- **合并源**：`9afcdeb7`（`origin/main`）
- **目标分支**：`feat-b-0808-add-monorepo`
- **合并原则**：冲突处取功能并集，保留多仓逻辑代码库（LC）链路与 main 的 spec 对话式修订循环、Canvas 审核面板及视觉升级。

## 冲突解决

1. `src/product/work_item_split_engine/tests.rs`（1 处）
   - 同时保留 `engine_gateway_guard.rs`、`outline_traceability_example.rs` 与 `design_context_canonical.rs` 三个测试 include。
   - 为 main 新增的 outline 测试补入 LC routing context 参数，使之匹配本分支已参数化的 prompt 签名。

2. `src/product/workspace_engine/controls.rs`（1 处）
   - 保留多仓 Story/Design 的 `validate_confirm_aggregate_spec` 确认门禁。
   - 将门禁提取为 `validate_confirm_aggregate_spec_gate`，同时由传统 `handle_confirm` 与 main 新增的 `finalize_current_artifact` 复用，确保 main 的 AuthorConfirm 定稿路径不会绕过 involved/change_order 校验。
   - 保留 main 的 WorkItemPlan 专用完成节点与 `finalize_current_artifact` 收口逻辑。

3. `src/product/workspace_engine/prompts/revision.rs`（1 处）
   - 保留 main 的 author-feedback 判定分支：`is_author_feedback_revision()` 调用 `build_author_revision_prompt(feedback)`。
   - reviewer 返修分支保留本分支 routing context 参数：`build_revision_delta_prompt(review, &context)`、`build_revision_full_prompt(&artifact, review, &context)`。
   - 补齐 main 新增 `WorkspaceSession` 字段在本文件测试 fixture 中的初始化。

4. `src/product/workspace_engine/review/drive.rs`（1 处）
   - 同时保留 `provider_allows_review_repair`（Pi 禁止 repair，Kimi 可复用既有一次 repair）及其测试。
   - 同时保留逻辑代码库 gateway 启动 helper：`start_review_session_via_gateway` 与 gateway→adapter 错误映射。
   - 将测试模块移至 helper 后，满足 clippy 的 `items_after_test_module` 规则。

5. `src/product/workspace_engine/tests.rs`（1 处）
   - 同时保留 `part_32.rs`，以及 main 的 `author_revision_loop`、`author_revision_review_routing` 模块。
   - 对 gateway review 测试更新断言，以匹配 main 的 Story/Design review report 返回 AuthorConfirm 的新路由。

6. `src/web/workspace_context/tests.rs`（1 处）
   - 同时保留 async Claude Code 结构化 AskUserQuestion 测试与 Kimi 结构化 permission/choice 测试。
   - Kimi 测试通过本分支的 `RoutingReferenceContext::Legacy` 参数调用工作流 prompt。

7. `src/web/state.rs`（3 处）
   - 保留本分支的 aggregate initialization dependencies、aggregate-index rebuild registry、注入 registry 后重建 logical gateway factory 的链路。
   - 保留 main 的 Pi/Kimi provider imports 与注册；生产 registry 现在注册 Claude Code、Codex、Pi、Kimi Code，fake registry 同样覆盖全部 provider 名称。
   - 更新旧有 registry 测试，以验证并集后的真实 provider 集合，而非错误地继续要求排除 Pi。

## 合并后集成修复

除 9 个文本冲突外，测试编译暴露了自动合并未能表达的跨提交依赖，已按“并集不丢功能”原则补齐：

- 恢复 `provider_drive.rs` 的聚合输出回写 helper，保留多仓 Story/Design 的 involved/change_order 回写与可见诊断。
- 在 `ProviderType::KimiCode` 上补齐 capability dialect 的穷尽匹配。
- 为被保留的 `part_32` gateway 测试保留仅测试构建的 `start_review_or_skip` 兼容入口。
- 为新旧测试 fixture 补齐 `provisional_reviewer_provider`、`reviewer_enabled_at_start` 字段。

## 验证

| 命令 | 结果 | 数字/摘要 |
| --- | --- | --- |
| `cargo check --locked` | 通过 | Rust 编译通过 |
| `cargo test --locked --lib workspace_engine` | 通过 | 903 passed, 0 failed |
| `cargo test --locked --lib work_item_split_engine` | 通过 | 91 passed, 0 failed |
| `cargo test --locked --lib workspace_context` | 通过 | 33 passed, 0 failed |
| `cargo test --locked --lib web::state` | 通过 | 27 passed, 0 failed |
| `cargo test --locked` | 通过 | 3,443 passed, 0 failed, 12 ignored（含 integration 与 doc tests） |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | 通过 | 0 warnings allowed |
| `cargo fmt --check` | 通过 | 无格式差异 |
| `cd web && pnpm test` | 通过 | 125 files / 942 tests passed |
| `cd web && pnpm tsc -b` | 通过 | TypeScript project build 通过 |
| 行数红线命令 | 通过 | `>1200` 的源文件输出为空 |
| 源码 merge marker / whitespace check | 通过 | 无 source marker、`git diff --check` 通过 |

> 全量 Rust 测试按要求未使用 `-j` 参数。测试输出中的 Git 默认分支提示来自临时 fixture 初始化，不是失败或 warning。
