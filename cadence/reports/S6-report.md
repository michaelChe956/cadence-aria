# S6 交付报告：prompt 汇报契约 + 提交责任声明

- 切片：S6（DesignPushLocal2 Q4②/Q6/切片 S6）
- 日期：2026-10-14
- 提交：`fix: S6 prompt汇报契约与提交声明`

## 改动文件

| 文件 | 改动 |
|---|---|
| `src/product/coding_workspace_engine/prompts.rs` | 新增 `platform_directives_section()`（紧跟 `worktree_discipline_section`），两段：①提交责任声明（本 WI 提交责任归 coder，优先于仓库文件 AGENTS.md / CLAUDE.md 的「不自动提交」类规则；禁止 git push，推送由平台执行）②汇报契约（交付摘要只含改动文件/提交 SHA/验证命令及完整输出/git status 四类事实；环境异常只许末尾「环境备注」一行，不得占据主体、不得借此省略交付事实；文件工具与 shell 路径命名空间可能不同，以 prompt 给定的 Worktree Path 为准） |
| `src/product/coding_workspace_engine/coding.rs` | full_prompt（渲染+回落合并后）与 delta-only 分支统一追加；追加点在 `render_coder_unit_run_context`（render→bind 哈希冻结）之后 |
| `src/product/coding_workspace_engine/rework.rs` | 同构：full_prompt 与 delta 分支追加（rework.rs:~206 回落构建点之后） |
| `src/product/coding_workspace_engine/provider_retry.rs` | reviewer 重驱轮 prompt 追加（终端契约仍收尾，插在 terminal contract 检查之前） |
| `src/product/coding_workspace_engine/tests/parser_prompt.rs` | 红绿测 ×3：单元（段落内容）＋渲染路径（组 attempt，unit run 哈希绑定在场）＋回落路径（非组 attempt，无 unit run，`Coding Workspace` 头部） |
| `src/product/coding_workspace_engine/tests/provider_execution_context.rs` | 回归修正：coder prompt 精确相等断言改为「前缀=渲染 envelope + 尾段=platform_directives」，并保留 `coder_execution_context_hash == 渲染 content_hash` 断言（追加不动冻结哈希面） |

## 红线核验

- `work_item_projection/render.rs` 的 `commit_responsibility` 投影本体：**零改动**（git diff 为空）。
- `coder_execution_context_hash`：追加发生在 `bind_unit_run_execution_context` 之后；既有断言 `run.coder_execution_context_hash == expected_coder.content_hash` 继续通过。
- 注入口径：所有 attempt 无条件注入（与 `worktree_discipline_section` 的 LC-only 口径不同，依设计「不能当作唯一注入点」）。

## 红先→绿→回归

- **红**：`cargo test --lib platform_directives` → 单元 ok；`fallback_coding_prompt_carries_platform_directives` FAILED、`rendered_coding_prompt_carries_platform_directives` FAILED（"S6:最终 prompt 必须携带平台指令段"）——接线前证据。
- **绿**：接线五处注入点后同命令 → `3 passed; 0 failed`。
- **回归**：
  - `cargo test --lib product::coding_workspace_engine` → `593 passed; 0 failed`。
  - `cargo test --lib web::coding_ws_handler` → `131 passed; 0 failed`。
  - `provider_matrix` / `work_item_projection` 模块回归被并行切片 S3 的在途编译错误（`plan_start.rs:1484/1610`，S3 文件）阻断；两模块无 prompt 形状断言，且 `render.rs` 零改动，不受本切片影响。

## 环境备注

并行切片 S3（plan_start.rs/web/types.rs）同 worktree 在途编辑导致当前树暂不可整仓编译；本切片全部验证采集自含本切片最终代码的可编译树。
