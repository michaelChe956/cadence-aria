# PIB Task 3.1 修复报告（PibT31）

> 执行人：PibT31（实施）｜日期：2026-09-25
> 范围：k3 终审 `pib-review-k3.md` §2-F1（P1）修复——coding fork 三入口改读 `issue.base_branch`
> 基线：`cc916773`（worktree `feat-b-0808-add-monorepo`）
> 落盘：本报告与 `pib-review-k3.md` 同居 `openspec/changes/per-issue-base-branch/`（沿用本 change 既有报告惯例）

## 0. 结论

**F1 已闭环**。三处 fork 入口（单件 coding、组 coding、advance 引擎）由「`current_git_branch`→`HEAD`」改为「`IssueStore::get` → `resolve_effective_base_branch`」（三面同源唯一解析链）；不可解析一律 fail-closed（web 两入口 422 `issue_base_branch_not_found`，advance 引擎 `Err(diagnosis)`），**不再回退当前检出或 HEAD**。新增 6 例红绿用例（it_web 4 + lib 2），红态 6/6 失败、绿态 6/6 通过。全部门禁绿。`advance_split.rs` 按 Non-Goal 零改动；`tasks.md` 3.1 勾选自本次修复起为真（保持 `[x]`）。

## 1. 三入口 diff 摘要（前 → 后）

| 入口 | 位置（改后行） | 前 | 后 |
|---|---|---|---|
| 单件 coding | `src/web/handlers/coding.rs:158-161` | `current_git_branch(&repository.path).unwrap_or_else(|| "HEAD".to_string())` | `fork_base_branch_from_issue(&app_paths, &repository.path, &project_id, &issue_id)?` |
| 组 coding | `src/web/handlers/coding/group.rs:109-110` | 同上 | `super::fork_base_branch_from_issue(...)?` |
| advance 引擎 | `src/product/workspace_engine/advance.rs:511-520` | `.unwrap_or_else(|| Self::current_git_branch(&repository.path).unwrap_or_else(|| "HEAD".to_string()))` | `match`：journal 在场沿用冻结值；None → `Self::resolve_advance_base_branch(&advance_store.app_paths(), &repository.path, &input)?` |

配套（最小面）：

- `src/web/handlers/coding.rs:363-383` 新增共享解析点 `fork_base_branch_from_issue`（单件/组两入口共用，避免双份实现漂移）：
  1. `IssueStore::new(app_paths).get(project_id, issue_id)` → `map_err(product_store_api_error)`（issue 缺失 → 404 `issue_not_found`）；
  2. `crate::product::issue_baseline::resolve_effective_base_branch(repository_path, issue.base_branch.as_deref())` → `map_err(issue_baseline_api_error)`（422）。
- `src/web/handlers/product_resources.rs:233` 原私有 `fn issue_baseline_api_error` 提升为 `pub(crate)`（复用既有错误码映射：`issue_base_branch_not_found` / `issue_base_branch_required` / `repository_branch_list_failed` → 422，`error.rs` 已钉死映射）。
- `src/product/workspace_engine/advance.rs:857-876` 新增 `resolve_advance_base_branch`（先例 `resolve_advance_repository` 同款 `IssueStore::get` 读取 + String 诊断上浮；advance 在 product 层，无 ApiError，故诊断经 `initialize_advance` 既有失败通道落 advance record `Failed` + `error`）。
- `src/web/handlers/support.rs` 删除被本次 cutover 弃用的 `current_git_branch`（两处 web 调用点已迁走；引擎侧 `Self::current_git_branch` 仍在 `advance_split.rs:156` 使用，按 Non-Goal 保留）。

源码与先前行为差异的本质：旧码只把「当前检出分支名」写入 attempt/共享 worktree（detached/空仓退化为字面量 `HEAD`）；新码写「issue 锁定基线」，且解析失败即拒绝。

## 2. 红绿证据（TDD，命令与结果均为本会话亲跑）

修复前实现（仅暂存 5 个实现文件、保留测试）下的**红态**：

```text
--- advance (lib) RED ---
test ...advance_handler::advance_initialization_forks_from_issue_base_branch ... FAILED
test ...advance_handler::advance_initialization_fails_closed_when_issue_base_branch_deleted ... FAILED
test result: FAILED. 0 passed; 2 failed

--- it_web RED ---
test web_coding_attempt_api::coding_attempt_forks_from_issue_base_branch_not_current_checkout ... FAILED
test web_coding_attempt_api::group_coding_attempt_forks_from_issue_base_branch_not_current_checkout ... FAILED
test web_coding_attempt_api::coding_attempt_fails_closed_with_422_when_issue_base_branch_deleted ... FAILED
test web_coding_attempt_api::group_coding_attempt_fails_closed_with_422_when_issue_base_branch_deleted ... FAILED
test result: FAILED. 1 passed; 4 failed
```

红态失败原因（旧码语义）：`attempt.base_branch` 落仓库当前检出 `main`（≠ 断言的 `feature/x`）；被删分支用例旧码静默回落 `HEAD` 并以 200 建 attempt（≠ 断言的 422）。

修复后**绿态**：

```text
cargo test --locked --lib -- advance_initialization
  → 14 passed; 0 failed（含新 2 例）
cargo test --locked --test it_web -- web_coding_attempt_api::*forks_from_issue_base_branch web_coding_attempt_api::*fails_closed_with_422
  → 4 passed; 0 failed
```

## 3. 用例与断言（6 例）

| 用例 | 入口 | 断言 |
|---|---|---|
| `coding_attempt_forks_from_issue_base_branch_not_current_checkout` | 单件 | 夹具 `main`+`feature/x`（feature 有专属提交）、检出 `main`、issue 锁定 `feature/x` → 200；`attempt.base_branch=="feature/x"`；共享 worktree 记录 `base_branch=="feature/x"`；以生产 fork 原语 `GitWorkspaceService::create_branch(branch, attempt.base_branch)` 实分叉后 `rev-parse` 断言**分叉点 == feature/x tip 且 ≠ main tip** |
| `group_coding_attempt_forks_from_issue_base_branch_not_current_checkout` | 组 | 同构：`attempt.base_branch=="feature/x"` + 共享 worktree 同基线 |
| `coding_attempt_fails_closed_with_422_when_issue_base_branch_deleted` | 单件 | 锁定分支 `git branch -D` 后启动 → **422** `issue_base_branch_not_found`、诊断含分支名；无 attempt 落盘、无共享 worktree 登记 |
| `group_coding_attempt_fails_closed_with_422_when_issue_base_branch_deleted` | 组 | 同构 + 无 group journal 落盘 |
| `advance_initialization_forks_from_issue_base_branch` | advance 引擎 | `handle_advance` Completed；group journal `attempt.base_branch=="feature/x"` + 共享 worktree 同基线 |
| `advance_initialization_fails_closed_when_issue_base_branch_deleted` | advance 引擎 | 分支不可解析 → `Err` 含「基准分支不存在」；无 journal；advance record 落 `Failed` 且 `error` 携带诊断 |

测试落点：`tests/it_web/web_coding_attempt_api/part_24.rs`（新，并登记入 `web_coding_attempt_api.rs`）+ `src/product/workspace_engine/tests/advance_handler.rs` 追加两例。夹具策略：issue 经 REST 创建后由 `set_issue_base_branch` 将 `issue.json` 的 `base_branch` 锁定为 `feature/x`（等价于 REQ-PIB-01 场景 1 显式选择基线创建的记录；创建校验面已在 `web_product_api` 族钉死，本族聚焦 fork 消费面）。

## 4. 夹具处置论证（1 处，断言语义零改动）

**`tests/it_web/web_provider_health_api.rs::git_repo`**（唯一受影响夹具）：

- 原形态 = `git init --quiet`，**无任何提交**（unborn HEAD，无 `refs/heads/*`）。该形态下 REQ-PIB-01 的 issue 创建本就 fail-closed（`NoDefaultBranch` → 422），只是旧 coder 入口不读 issue，缺失被掩盖；T3.1 起 coder 入口读 issue → 先命中 404 `issue_not_found`，遮蔽该用例要钉的 500 `provider_unavailable`（实测 left=404 / right=500）。
- 处置 = 按 `pib-review-k3.md` §3 已核收的**同族先例**修夹具（`git init -b main` + config + 初始提交，注释标记 REQUIRED 不变量），使 issue 真正落地并携带 `base_branch=main`，该用例的 provider 门断言语义与断言字面量**零改动**。
- 佐证：该用例在「新夹具 + 旧实现」下同样通过（本会话红态批量运行中 1 passed 即它），说明夹具修复为语义中性、不构成放水；在「新夹具 + 新实现」下通过，证明 fork 入口读取 issue 后仍能到达 provider 门。

## 5. 门禁（worktree 当前树，本会话亲跑）

| 门禁 | 命令 | 结果 |
|---|---|---|
| 格式 | `cargo fmt --check` | ✅ 干净 |
| 静态检查 | `cargo clippy --locked --all-targets -- -D warnings` | ✅ 0 warning |
| lib 全量 | `cargo test --locked --lib` | ✅ 3635 passed / 0 failed / 3 ignored |
| it_core | `cargo test --locked --test it_core` | ✅ 187 passed / 0 failed / 0 ignored |
| it_web | `cargo test --locked --test it_web` | ✅ 351 passed / 0 failed / 1 ignored |
| openspec | `openspec validate per-issue-base-branch --strict` | ✅ valid |

定向面（fork/api）已在 §2 绿态单列。服务器进程未触碰。

## 6. Non-Goal 与勾选

- `src/product/workspace_engine/advance_split.rs:155-157` **保持现状**（分流路径沿用 `current_git_branch`→`HEAD`，按 pivot 定稿 Non-Goal 不变）；`Self::current_git_branch` 因该调用点保留，未删。
- `openspec/changes/per-issue-base-branch/tasks.md` 3.1 保持 `[x]`：此前为虚勾（k3 §2-F1），本修复落地后为真，不再改动勾选状态。

## 7. 残余风险与备注

- **advance 引擎错误面为字符串诊断**：advance 在 product 层无 ApiError，fail-closed 表现为 `Err("resolve advance base branch failed: 基准分支不存在：…")`，WS 侧经既有 `AdvanceRejected`/失败通道呈现（该面未新增 HTTP 422 语义，与 k3「advance.rs 同（先例 resolve_advance_repository）」一致）。
- **组入口 journal 重放**：基线仍每次重算（保持既有结构），由 `journal_matches_request` 全等校验兜底——分支在 advance 建 journal 后被删时，组端点由重算失败以 422 fail-closed（属 REQ-PIB-03 场景「基线消失 fail-closed」期望语义）。
- 未做（超出本修复面）：`it_web` 之外的其他测试目标（如 `it_product` 等）未在本次定向门禁内；如 Main 全量收口需要可顺跑。
