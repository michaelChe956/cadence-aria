# S6 交付报告：prompt 汇报契约 + 提交责任声明

- 切片：S6（DesignPushLocal2 Q4②/Q6/切片 S6）
- 日期：2026-10-14
- 提交：`fix: S6 prompt汇报契约与提交声明`（9e6b0ffa）；审查处置 `fix: S6 认领摘要部署边界登记与断言加强`

## 改动文件

| 文件 | 改动 |
|---|---|
| `src/product/coding_workspace_engine/prompts.rs` | 新增 `platform_directives_section()`（紧跟 `worktree_discipline_section`），两段：①提交责任声明（本 WI 提交责任归 coder，优先于仓库文件 AGENTS.md / CLAUDE.md 的「不自动提交」类规则；禁止 git push，推送由平台执行）②汇报契约（交付摘要只含改动文件/提交 SHA/验证命令及完整输出/git status 四类事实；环境异常只许末尾「环境备注」一行，不得占据主体、不得借此省略交付事实；文件工具与 shell 路径命名空间可能不同，以 prompt 给定的 Worktree Path 为准） |
| `src/product/coding_workspace_engine/coding.rs` | full_prompt（渲染+回落合并后）与 delta-only 分支统一追加；追加点在 `render_coder_unit_run_context`（render→bind 哈希冻结）之后 |
| `src/product/coding_workspace_engine/rework.rs` | 同构：full_prompt 与 delta 分支追加（rework.rs:~206 回落构建点之后） |
| `src/product/coding_workspace_engine/provider_retry.rs` | reviewer 重驱轮 prompt 追加（终端契约仍收尾，插在 terminal contract 检查之前） |
| `src/product/coding_workspace_engine/tests/parser_prompt.rs` | 红绿测 ×3：单元（段落内容）＋渲染路径（组 attempt，unit run 哈希绑定在场）＋回落路径（非组 attempt，无 unit run，`Coding Workspace` 头部） |
| `src/product/coding_workspace_engine/tests/provider_execution_context.rs` | 回归修正＋审查加强：coder prompt 断言升级为 `format!("{}{}", expected_coder.text, platform_directives_section())` 精确拼接相等（前缀零改写、中段零插入），并保留 `coder_execution_context_hash == 渲染 content_hash` 断言（追加不动冻结哈希面） |

## 红线核验

- `work_item_projection/render.rs` 的 `commit_responsibility` 投影本体：**零改动**（git diff 为空）。
- `coder_execution_context_hash`：追加发生在 `bind_unit_run_execution_context` 之后；既有断言 `run.coder_execution_context_hash == expected_coder.content_hash` 继续通过。
- 注入口径：所有 attempt 无条件注入（与 `worktree_discipline_section` 的 LC-only 口径不同，依设计「不能当作唯一注入点」）。

## 红先→绿→回归

- **红**：`cargo test --lib platform_directives` → 单元 ok；`fallback_coding_prompt_carries_platform_directives` FAILED、`rendered_coding_prompt_carries_platform_directives` FAILED（"S6:最终 prompt 必须携带平台指令段"）——接线前证据。
- **绿**：接线五处注入点后同命令 → `3 passed; 0 failed`。
- **回归**（审查处置轮，断言加强后干净树复跑）：
  - `cargo test --lib product::coding_workspace_engine` → `601 passed; 0 failed`。
  - `cargo test --lib web::coding_ws_handler` → `132 passed; 0 failed`。
  - `cargo test --lib platform_directives` → `3 passed; 0 failed`。
  - 首轮回归（593/131）同理全绿；`provider_matrix` / `work_item_projection` 无 prompt 形状断言且 `render.rs` 零改动，交由 P3 整仓验证覆盖。

## 部署边界（P2 处置，审查 2026-10-14）

注入段推高 `rework-claims.json` 的 `rendered_prompt_digest` 冻结绑定：Window A'（认领 journal 已落盘、消费标记未落盘）的 in-flight attempt 跨本构建重放会恒 IdentityMismatch → AMR 死循环。现场暴露为零（两 AMR attempt `rework_count=0`、无 claims），但真实链路会写该 journal。处置：

1. **部署前清点（本报告登记）**：逐 attempt 取 `rework-claims.json` 认领记录 × 未消费 rework instruction（`consumed_at` 为空）交集；命中者显式清认领（删除该 claim 记录或整 journal 重写，保持 instruction 可读可重试），再部署新构建。
2. **恢复预案执行**：controller（Main）在恢复演练前执行同款清点。
3. **耐久修法（后续包，不在本轮）**：claim 增加 scheme 标识／旧 digest 规范化（追加段不进 digest 基面或按 scheme 迁移），消除跨构建重放失配类。

## 环境备注

首轮验证时并行切片 S3 在途编辑曾致树暂不可整仓编译；S3 落地后已在干净树复跑（见回归记录）。
