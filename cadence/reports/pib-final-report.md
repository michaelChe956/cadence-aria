# PIB 实施终收报告（第三轮：全量绿收口）

- 日期：2026-09-25
- 工作树：`.worktrees/feat-b-0808-add-monorepo`（分支 `feat-b-0808-add-monorepo`）
- 范围：PibItc 报告（`pib-itcore-fix-report.md`）遗留的三类残余——40 个 src 内嵌
  夹具族失败、17 个 PibB 域回归（16 lib 失败 + 1 死锁）、门禁与提交。

## 1. 结果总览（红 → 绿）

| 门禁 | 修复前 | 修复后 |
| --- | --- | --- |
| `cargo test --lib` | 3575 passed; 57 failed; 3 ignored（+1 死锁 skip） | **3633 passed; 0 failed; 3 ignored**（3636 全量，含原死锁用例 0.65s 通过） |
| `cargo test --test it_core` | 187 全绿（PibItc 已修） | **187 全绿**（复验） |
| `cargo test --test it_web` | — | **347 passed; 1 ignored**（多次复验，见 §5） |
| `cargo clippy --all-targets -- -D warnings` | 6 errors | **通过** |
| `cargo fmt --check` | 多处缩进 diff | **通过** |

## 2. 根因判定：57 个 lib 失败全部同族——PibB fail-closed 新面 × 测试夹具未对齐生产不变量

PibB（REQ-PIB-02/03）为 issue 基线分支引入三面同源解析
（`src/product/issue_baseline.rs::resolve_effective_base_branch`）：

1. `resolve_author_baseline_tree`（`workspace_engine/prompts.rs`）：author 生成期
   基线树；
2. `plan_baseline_tree`（`workspace_engine/plan_preflight.rs`）：plan 运行期预校验
   与权威核对；
3. `build_work_item_plan_streaming_input` 由裸调用改为 `Result`，基线不可解析 →
   run 以可观测错误出站并终止（`followups.rs`/`provider_run.rs`/`single_candidate.rs`
   的 `Err(message)` 分支）。

**全部测试夹具的 repository 只是普通目录（无 `.git`）**，生产新面下
`git show-ref refs/heads/main` 得 128（not a git repository）→ fail-closed。
失败形态因错误出站路径不同而异（422 / error 事件 / Elapsed 超时 / 调用次数断言 /
挂死），但根因同一——**非 PibB 生产代码缺陷，全部以夹具对齐生产不变量修复**
（`git init --initial-branch main` + config + `commit --allow-empty`，与 PibB 在
it_web、PibItc 在 it_core 的修法同构，注释统一标记 REQ-PIB-02）。

### 2.1 A 组：41 个 src 内嵌夹具族（「基准分支解析所需 git 仓库不可用」直接 panic）

单一根：`src/product/workspace_engine/tests/part_09.rs::
make_work_item_plan_engine_with_draft_candidate` 的 `repository` 目录只
`create_dir` 不 git init；`conversational_gate_revision`（17）、
`single_candidate/{contract_autorepair,contract_prerevision,preflight_options_loop}`
（12）、`conversational_gate_amendment_real_chain`（2）、
`coding_workspace_engine::tests::campaign_stage3_amendment`（1，链上游同一夹具）、
`web::workspace_ws_handler::tests::campaign_stage3_interactive`（5）+
`campaign_stage3_recovery_matrix`（4）全部经此夹具族。

修复：part_09.rs 夹具 repository 补 `init_fixture_git_repo`（文件末尾新增
helper：`git init --initial-branch main` + `config user.email/user.name` +
`commit --allow-empty -m "fixture baseline"`）。**一处修复，40 例全绿**
（conversational_gate 83、single_candidate 65、campaign 三组全过）。

### 2.2 B 组：16 个 web provider run 域（Elapsed 超时 / provider 调用断言）

四处独立夹具，同一修法：

| 夹具 | 修复 | 覆盖用例 |
| --- | --- | --- |
| `src/web/workspace_ws_handler/tests/single_candidate_provider_run.rs::ProviderRunFixture::build`（repository_root tempdir） | 补 `super::init_ws_test_git_repo` | `single_candidate_provider_run::*` 10（含 legacy outline builder 用例——其失败形态为 outbound 收到基线 error 后 channel closed）+ `single_candidate_ir_reredrive::*` 3 |
| `src/web/workspace_ws_handler/tests.rs::start_generation_refreshes_stale_provider_guidance_before_prompting_author`（repo tempdir） | 补 `init_ws_test_git_repo` | 1 |
| `src/web/workspace_ws_handler/tests.rs::provider_select_then_user_message_forces_pi_to_auto_from_stale_supervised_mode`（repo tempdir） | 同上 | 1 |
| `src/web/workspace_session/tests/part_01.rs::provider_run_requested_without_attachments_spawns_throwaway_run`（repo=root 本身） | root 下 git init（内联同款四命令） | 1 |

`init_ws_test_git_repo` helper 定义于 `tests.rs`（`seed_legacy_project` 后）。
超时/断言形态的机理：`spawn_provider_run_from_handler` 在
`build_work_item_plan_streaming_input` 处 Err → error 出站 + run 终止
（伴随 PibB 诊断打点 `[aria-cancellation] handler_run_supersede` stderr），
测试等待的 expected stage / 第二次 provider start / 「无后续 provider 调用」
断言随之失败——日志里的 supersede 打点是**症状不是病因**。

### 2.3 死锁 1 个：`provider_run_events::single_candidate_revise_route_does_not_misfire_a_second_followup_review`

- 现象：单独运行 300s 挂死（PibItc bg_266 证据），确定性。
- 根因（三层叠加，夹具为主因）：
  1. `ProviderRunFixture` 的 repository_root 无 git 仓库（同 §2.2）；
  2. run-1 在 `run_single_candidate_author` 的基线解析处 fail-closed 提前终止，
     **不发射 `WorkItemPlanSingleCandidateAuthor` 接力事件**；
  3. 测试第一等待循环（`relay_observed`，3s timeout）用纯 `yield_now()` 永真自旋
     ——current_thread 运行时下饿死计时器，timeout 永不触发 → 挂死而非失败。
     （该测试第二处循环的注释早已记录同一教训并改用 sleep，第一处漏改。）
- 修复：夹具 git init（§2.2 第一行，修后本用例 0.88s→0.65s 通过）**加**
  第一处自旋改 `sleep(10ms)`（防未来任何回归再次把失败变挂死）。
- 定责：**PibB 生产改动无行为缺陷**；为 fail-closed 新面 × 夹具未对齐 × 测试
  自旋缺陷的叠加。

## 3. clippy/fmt 修复（PibB 在树改动遗留）

- `cargo fmt`：统一修复 PibB 引入的 `base_branch: None,` 错误缩进（约 29 处
  文件，含 HEAD 已提交态 `tests/web_logical_codebase_entrypoints/planning.rs`、
  `src/web/handlers/product_resources.rs` 等测试内嵌构造）。
- `cargo clippy -D warnings` 6 errors：
  - `workspace_engine/prompts.rs`：tests mod 删 unused `use super::*` / `PathBuf`；
  - `workspace_engine/draft_batch/runs.rs` 两处 `Ok(x?)` → `x`（needless_question_mark）；
  - `streaming_provider/mod.rs:410`、`web/handlers/product_resources.rs:210`：
    doc 列表续行缩进/分段（doc_lazy_continuation）。

## 4. 本次改动文件清单（在 PibB 45 + PibItc 3 之上）

| 文件 | 改动 |
| --- | --- |
| `src/product/workspace_engine/tests/part_09.rs` | 夹具 git init + `init_fixture_git_repo` helper（41 例） |
| `src/web/workspace_ws_handler/tests.rs` | `init_ws_test_git_repo` helper + 两测试接入（2 例） |
| `src/web/workspace_ws_handler/tests/single_candidate_provider_run.rs` | ProviderRunFixture 接入（13 例） |
| `src/web/workspace_session/tests/part_01.rs` | throwaway run 夹具 root git init（1 例） |
| `src/web/workspace_ws_handler/tests/provider_run_events.rs` | 死锁用例自旋改 sleep（防挂死复发） |
| `src/product/workspace_engine/draft_batch/runs.rs` | clippy×2 |
| `src/product/workspace_engine/prompts.rs` | clippy unused imports×2 |
| `src/cross_cutting/streaming_provider/mod.rs` | clippy doc 缩进 |
| `src/web/handlers/product_resources.rs` | clippy doc 缩进 |
| fmt 触达（纯缩进，PibB 遗留） | `tests/web_logical_codebase_entrypoints/planning.rs` 等 |
| `tests/it_web/web_coding_ws_handler/part_18.rs` | §5 flaky：execute 完成后排干 Ack 缓冲（select 双就绪观测竞态） |

## 5. it_web `coding_ws_new_connection_receives_and_answers_pending_choice` flaky 定位与修复

- 现象：并行全量/模块跑约 9 次挂 3 次（~33%），单独运行稳定通过；
  与 PIB 改动**无关**（`web_coding_ws_handler` 生产路径零改动）。
- 根因（bg_279 抓到失败详情 `engine must emit choice response ack for the
  resumed answer`）：**测试断言观测竞态**，非引擎缺陷——引擎确实消费了答案、
  关闭了 gate 并完成 run，Ack 事件已进入 channel 缓冲；但等待循环的
  `tokio::select!` 在「execute 完成」与「Ack 事件」双就绪时随机选分支，
  execute 先 break 即退出循环，已缓冲的 Ack 永不被读 → `saw_ack` 假阴性。
- 修复（`tests/it_web/web_coding_ws_handler/part_18.rs`）：execute 完成后以
  `try_recv()` 排空事件缓冲再断言（run 结束后引擎不再发事件，排空即终态）。
  修复后模块级 8 连跑全绿（bg_280）。

## 6. 提交

工作树全部本任务文件（PibB 45 + PibItc 3 + 本报告清单 + 两份报告 md）已按域
提交并 push：`1b1b37f9`（src 70 文件：生产接线+夹具+lint 门禁）、
`3fb8d54b`（tests 14 文件：集成夹具+flaky 竞态收口）、第三 commit（报告文件）。
`cadence/notes/**`（他人文件）与 `openspec/changes/human-gate-convergence/
c2-review-k3.md`（非本任务产物）不在提交集，留在工作树。
