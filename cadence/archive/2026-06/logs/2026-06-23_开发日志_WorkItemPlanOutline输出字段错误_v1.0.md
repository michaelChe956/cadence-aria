# WorkItemPlan Outline 输出字段错误定位记录

## 现象

- 用户在端到端手工验证中看到错误：
  - ``outline generate failed: WorkItemPlan Outline output must not contain `work_item_id` ``
- 当前服务仍在运行，未修改业务代码。

## 运行数据

- Worktree：`.worktrees/feat-b-0616`
- Session：`.aria/projects/project_0001/issues/issue_0001/workspace-sessions/workspace_session_0003.json`
- Timeline：
  - `.aria/projects/project_0001/issues/issue_0001/workspace-timelines/workspace_session_0003/timeline_nodes.json`
  - `.aria/projects/project_0001/issues/issue_0001/workspace-timelines/workspace_session_0003/timeline_node_details/timeline_node_002.json`
- 节点信息：
  - `node_type = work_item_plan_outline_run`
  - `agent_role = author`
  - provider：`claude_code`
  - `timeline_nodes.json` 中该节点仍为 `status = active`

## 定位结论

报错来自后端 WorkItemPlan Outline 阶段的 structured output 校验，不是前端展示问题。

相关代码：

- `src/product/work_item_split_engine.rs:1107` 的 `parse_work_item_plan_outline_output()`
- `src/product/work_item_split_engine.rs:1300` 的 `forbidden_outline_field()`
- `src/product/work_item_split_engine.rs:1318` 的 `is_forbidden_outline_key()`
- `src/web/workspace_ws_handler.rs:2066` 捕获解析错误后向前端发送 `outline generate failed: ...`

后端当前明确禁止 Outline 输出以下字段：

- `work_items`
- `work_item_id`
- `work_item_ids`
- `verification_plan`
- `verification_plans`
- `repository_profile`
- `parallel_groups`

本次 provider 最终 `<ARIA_STRUCTURED_OUTPUT nonce="972f86da">` 中的 JSON 不是当前后端期望的 `WorkItemPlanOutline` 结构，而是偏旧的 Work Item 拆分计划结构。

具体表现：

- `outline.work_item_outlines[]` 中使用了旧字段：
  - `id`
  - `layer`
  - `summary`
  - `key_paths`
  - `reuse_modules`
  - `test_strategy`
  - `acceptance_refs`
- 当前后端模型期望的字段是：
  - `outline_id`
  - `title`
  - `kind`
  - `goal`
  - `scope`
  - `non_goals`
  - `source_story_spec_ids`
  - `source_design_spec_ids`
  - `exclusive_write_scopes`
  - `forbidden_write_scopes`
  - `depends_on`
  - `verification_intent`
  - `handoff_notes`
- `outline.dependency_graph[]` 中使用了错误格式：
  - `{"work_item_id": "...", "depends_on": [...]}`
- 当前后端期望的 dependency edge 是：
  - `{"from_outline_id": "...", "to_outline_id": "..."}`

因此，`work_item_id` 只是第一个被 forbidden-field 拦截到的非法字段。即使仅把 `work_item_id` 改名，后续仍会因为 `work_item_outlines` item schema 和 `dependency_graph` edge schema 不匹配继续失败。

## 初步根因

根因更像是 provider 输出 schema 漂移：

- Prompt 文字已经写明“只能输出 WorkItemPlan Outline”和“不得输出 work_item_id/work_item_ids”。
- 但 `WORK_ITEM_PLAN_OUTLINE_OUTPUT_SCHEMA` 目前只约束了顶层 `outline` 的 required 字段，没有展开 `work_item_outlines[]` 和 `dependency_graph[]` 的子结构，也没有对嵌套对象设置 `additionalProperties: false`。
- Claude Code provider 仍按旧实现计划/Work Item 拆分格式生成了字段，说明当前 schema/prompt 对嵌套结构约束不够强。

## 附带发现

本次 `timeline_node_002.json` 的 provider execution event 显示：

- provider `cwd` 为 `/Users/michaelche/Documents/git-folder/github-folder/cadence-aria`
- 读取文件路径也指向主仓库，而不是 `.worktrees/feat-b-0616`
- `.aria/projects/project_0001/repos.json` 中 `repository_0001.path` 也绑定到主仓库路径

这不直接导致 `work_item_id` 报错，但会影响当前“在 `.worktrees/feat-b-0616` 中做端到端验证”的隔离性。后续需要确认：测试数据里的 repository path 是否要切换到当前 worktree，还是允许 provider 读取主仓库。

另一个状态问题：

- 解析失败后 `src/web/workspace_ws_handler.rs:2068` 只调用 `engine.mark_active_run_finished()`。
- `mark_active_run_finished()` 只清理 active run id，不会把当前 timeline node 标记为 failed。
- 因此 `timeline_nodes.json` 中 `timeline_node_002` 仍停留在 `active`，这可能造成 UI 状态看起来还在运行。

## 建议修复方向

如确认可以修改代码，建议按 TDD 处理：

1. 先补单元测试，覆盖 Outline prompt/schema 必须包含并要求：
   - `outline_id`
   - `from_outline_id`
   - `to_outline_id`
   - 禁止 implementation-plan 风格字段，如 `id/layer/key_paths/test_strategy/acceptance_refs/work_item_id`
2. 收紧 `WORK_ITEM_PLAN_OUTLINE_OUTPUT_SCHEMA`：
   - 展开 `outline.work_item_outlines[]` item schema
   - 展开 `outline.dependency_graph[]` edge schema
   - 对嵌套对象设置 `additionalProperties: false`
3. 在 prompt 中加入一个最小正确 JSON 示例，明确：
   - outline item id 字段叫 `outline_id`
   - dependency graph 边字段叫 `from_outline_id/to_outline_id`
   - 不要输出完整 Work Item/implementation plan 字段
4. 失败路径补状态收敛：
   - structured output 解析失败或 Outline 解析失败时，把 active timeline node 标记为 failed，避免 UI 一直显示 active。
5. 独立确认 repository path/worktree 隔离策略。

## 修复记录

- 已收紧 `WORK_ITEM_PLAN_OUTLINE_OUTPUT_SCHEMA`：
  - 展开 `outline.work_item_outlines[]` 的字段结构。
  - 展开 `outline.dependency_graph[]` 的 `from_outline_id/to_outline_id` 边结构。
  - 对嵌套对象设置 `additionalProperties: false`。
- 已强化 Outline prompt：
  - 明确 `work_item_outlines[]` 的条目标识字段必须是 `outline_id`。
  - 明确 `dependency_graph[]` 必须使用 `from_outline_id/to_outline_id`。
  - 明确不要输出旧版 implementation plan / Work Item 拆分计划字段。
  - 增加最小正确 JSON 示例。
- 已新增 `WorkspaceEngine::finish_active_run_with_failed_node()`，并在 WorkItemPlan Outline 初次生成与 revision 的 structured output/Outline 解析失败路径中调用，避免 timeline node 继续停留在 `active`。

## 验证状态

- 已按 TDD 添加回归测试。
- 验证命令：
  - `cargo test --locked --lib outline_author_prompt_forbids_full_work_items_and_repository_profile`
  - `cargo test --locked --lib finish_active_run_with_failed_node_marks_outline_node_failed`
  - `cargo test --locked --lib outline`
  - `cargo fmt --check`
  - `cargo test --locked --lib`
  - `cargo check --locked`
  - `cargo clippy --all-targets --all-features --locked -- -D warnings`
  - `cargo test --locked`
- 以上验证均通过。
