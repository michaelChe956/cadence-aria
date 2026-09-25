# F-58 覆盖补齐：reviewer 族与 author 修订面基线教学注入报告

- 日期：2026-09-25
- 工作树：`.worktrees/feat-b-0808-add-monorepo`（BASE=5a72a8b5）
- 范围：workspace_engine 的 story/design/plan 会话面（REQ-PIB-02 语义域）
- 结论：reviewer 族与 author 修订面补齐「基线解析 fail-closed + 教学注入 + 锚点透传」；
  lib 全量 3641 全绿，it_web 351 / it_provider 54 / it_core workspace_ws_integration 73 全绿，fmt/clippy 干净。

## 1. 现场与缺口

issue_0002 story 会话 `reviewer_run`：pi reviewer 以 bash 扫到 `.worktrees/aria-issues/*`（用户贴图实锤）。

根因：REQ-PIB-02 的基线限制教学块只接在 **author 两族 builder**
（`prompts.rs::build_streaming_input`、`build_work_item_plan_streaming_input_with_session`）；
**reviewer 族**（`prompts/review.rs` 各 review builder）与 **author 修订面**
（`prompts/revision.rs::build_revision_input_with_resume`）仍是 `baseline_tree: None` 且无教学注入——
reviewer 与修订产物对「仓库既有内容」没有任何基线来源约束，与 author 面同一禁令未覆盖。

## 2. 三面清单（同链同源，唯一解析点不变）

| 面 | 落点 | 教学块 | 锚点 |
|---|---|---|---|
| author 两族 | `prompts.rs`：`build_streaming_input`、`build_work_item_plan_streaming_input_with_session` | author 版（既有） | 既有 |
| reviewer 族 | `prompts/review.rs` 7 个叶子 builder：`build_review_input`（story/design）、`build_work_item_plan_review_input`（legacy plan）、`build_single_candidate_plan_review_input`、`build_projection_plan_review_input`、`build_work_item_plan_outline_review_input`、`build_work_item_batch_review_input`、`build_work_item_draft_review_input` | reviewer 版（新） | 新接线 |
| author 修订面 | `prompts/revision.rs`：`build_revision_input_with_resume`（reviewer 返修 delta/full 与 author 反馈增量两分支共用同一构造尾部） | author 版（与两族同构） | 新接线 |

顺带覆盖（零改动，自动继承）：design 复评、plan 各轮复评（每轮重新走同一 builder）；
Work Item **draft author** 面本就走 `build_work_item_plan_streaming_input`（已覆盖，非缺口）；
**门内修订**（`build_sc_manual_revision_prompt_for_turn` 产 prompt，最终由
`web/workspace_ws_handler/run/provider_run.rs` 经 `build_work_item_plan_streaming_input` 构造 input）
属 author plan 族，同样自动继承。

## 3. 修法

- `prompts.rs` 新增 `reviewer_baseline_teaching_block(branch)`：reviewer 版教学块
  （既有事实与 finding 证据只以基线树为来源；`.worktrees/`（含 `.worktrees/aria-issues/*`）、
  未提交改动、其他分支文件不得作既有事实或 finding 证据；不得访问工作区外路径；
  基线外路径属 `acceptance_path_not_in_baseline` 缺口）。沿用同一 header 标记
  `## 基准分支基线（issue 基线 = X）`，两面可被同一断言识别。仍为**软约束非安全边界**
  （语义同 `baseline_teaching_block` 注释；host-served 通道才具硬边界）。
- `resolve_author_baseline_tree` → `resolve_issue_baseline_tree`：四面共用一条解析链
  （issue.base_branch → `resolve_effective_base_branch`，唯一解析点不变）。
- 新增两个 helper：`append_author_baseline_teaching`（author 两族 + 修订面）、
  `append_reviewer_baseline_teaching`（reviewer 族）——解析 fail-closed + 教学注入 + 返回锚点；
  author 两族原两处重复块收敛进 helper。
- 单候选 review builder 的注入置于 `ensure_single_candidate_review_prompt_budget` **之前**，
  字节预算仍覆盖整段 prompt。
- **fail-closed 对齐**：基线不可解析（分支被删/存量皆无/仓库不可用）时 reviewer 与修订面
  同样终止该轮 provider 运行、给出同一诊断，不留「基线已消失但审核照跑」的缺口。

## 4. TDD 红→绿

- 新增 `prompts::reviewer_revision_baseline_tests`（2 例）：
  - `reviewer_and_revision_faces_inject_baseline_teaching`：story + design reviewer prompt 含教学块
    且 input 携带锚点（branch=main）；修订面同（branch=master）。
  - `reviewer_and_revision_faces_fail_closed_when_baseline_branch_missing`：reviewer 与修订面
    分支被删时 Err 含「基准分支不存在」。
- **红**（实现前）：上述 2 例失败（`reviewer baseline anchor` panic；reviewer 分支被删仍返回 `Ok`），
  既有 24 例（含 author 三例）通过。
- **绿**（实现后）：`cargo test --lib product::workspace_engine::prompts::` → 26 passed; 0 failed。
- 夹具：原 `author_baseline_tests` 内联夹具提取为 `baseline_teaching_fixture`（三面共用），
  author 三例零语义改动。

## 5. 验证证据

| 命令 | 结果 |
|---|---|
| `cargo test --lib product::workspace_engine::prompts::` | 26 passed; 0 failed |
| `cargo test --lib product::workspace_engine` | 765 passed; 0 failed |
| `cargo test --lib` | 3641 passed; 0 failed; 3 ignored |
| `cargo test --test it_core workspace_ws_integration` | 73 passed; 0 failed |
| `cargo test --test it_web` | 351 passed; 0 failed; 1 ignored |
| `cargo test --test it_provider` | 54 passed; 0 failed |
| `cargo fmt --all --check` | OK |
| `cargo clippy --lib --all-targets` | 无告警 |

改动文件：`src/product/workspace_engine/prompts.rs`、`prompts/review.rs`、`prompts/revision.rs`
（+ 本报告）。仓库其他未提交改动（cadence 笔记、openspec 评审批注）未触碰、未提交。

## 6. 残留与边界

- 教学块仍是软约束：provider 原生通道物理可达集不受限（沿用 D3 用户 2026-09-25 裁决）；
  host-served 通道硬边界仍在 kimi `client_services`（fs 树路由 / terminal 拒绝）。
- 覆盖范围限于 workspace_engine 的 story/design/plan 会话面；**coding 阶段 reviewer**
  （`coding_workspace_engine`：code review / internal PR review / group review）不在 REQ-PIB-02 语义内
  （其「既有内容」= attempt worktree 检出，非 issue 基线树）。
- `review_repair`（结构化输出格式修复轮）不注入：该轮明确「不得重新审核」，不产生仓库事实判断。
