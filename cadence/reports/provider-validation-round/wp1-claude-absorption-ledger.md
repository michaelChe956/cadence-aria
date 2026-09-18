# WP1 claude headless 吸收核账表（close-provider-validation Task 1 / REQ-CCI-06）

> 实施日期：2026-09-18；worktree `feat-b-0808-add-monorepo`，HEAD=`c8101323`（=计划 BASE `4cc834e2` + 本计划文档提交，`git diff --stat 4cc834e2..c8101323` 仅含计划 .md 一个文件，src/ 零变更——计划全部代码锚点仍有效，逐行实读复核见 §1）。
> claude 面零代码改动（D1 补验性质）；本文件为吸收核账+门禁补验留档，供 T3 smoke 与 T6 关闸报告引用。

## §0 吸收标记核验（标记已前置，登记在案）

| 核验项 | 命令 | 结果 |
|---|---|---|
| `fix-claude-code-ask-user-question-headless/proposal.md` 首行 | `head -1` | `> **[已被吸收]** 本 change 已被 \`close-provider-validation\` 吸收（契约唯一权威版本=该 change 的 claude-code-structured-interaction delta），禁止单独 archive；归档动作由 close-provider-validation WP4.3 统一执行。` ✓ |
| `fix-claude-code-ask-user-question-headless/tasks.md` 首行 | `head -1` | 同上全文 ✓ |
| 提交 C `3164f6a7` 是 HEAD 祖先 | `git merge-base --is-ancestor` | `C-ancestor-ok` ✓ |
| 提交 B `e367a84e` 是 HEAD 祖先 | 同上 | `B-ancestor-ok` ✓ |
| 提交 A `20493b65` 是 HEAD 祖先 | 同上 | `A-ancestor-ok` ✓ |
| 三提交日期 | `git show -s --format='%h %ad %s' --date=short` | `3164f6a7 2026-08-21 fix(claude): let native control flow own AskUserQuestion tool results` / `e367a84e 2026-08-21 fix(claude): map aria permission policy to valid callback mode` / `20493b65 2026-08-21 fix(claude): register stdio permission callback in auto mode` ✓（均=2026-08-21） |

本 Task 不动旧文件夹、不走 archive——归档动作并入 T5 owner 确认后统一执行。

## §1 吸收核账表（旧提案 task → 提交 / 行号证据，无缺口）

「现行代码锚」列为 2026-09-18 HEAD=`c8101323` 逐行实读复核值（与计划锚点一致，无漂移；HEAD 前移仅计划文档提交，见文件头注）。

| 旧提案 task | 提交 | 现行代码锚（实读复核） | 测试锚 | 状态 |
|---|---|---|---|---|
| 3.1/3.2/3.3（提交 A：所有模式注册 stdio 回调） | `20493b65` | `mod.rs:201`（无条件 push `--permission-prompt-tool=stdio`，`build_args` :171-204）✓ 实读一致 | `tests/args.rs:58`（`claude_args_always_include_stdio_permission_prompt`，Auto+Supervised 双模式循环断言）✓ | 已落地 |
| 2.1/2.2/2.3（提交 B：权限模式映射合法 wire 值） | `e367a84e` | `mod.rs:147-152`（`permission_mode_for_claude`：Auto\|Supervised→`"default"`，含 wire 兼容名注释）✓ 实读一致 | `tests/permissions.rs:89`（`claude_supervised_permission_mode_maps_to_default`；同文件 :97 Auto→default、:105 初始消息仅发合法模式）✓ | 已落地 |
| 2.3 回归（Auto 经 ApprovalBridge 自动批准） | `e367a84e` | stream/mod 控制流：`mod.rs:317`（`parse_control_request` 定义）+ `stream.rs:111`（分发消费）✓ 实读在场 | `tests/ask_user_question.rs`（Auto 系用例族：:312 control_response 所有权、:403 重复 control_request 缓存复用等）✓ | 已落地 |
| 1.1/1.2/1.3（提交 C：结果所有权归 control_request） | `3164f6a7` | `stream.rs:49`（`resolved_ask_user_questions: HashMap` 缓存）+ `:234-275`（is_error 协议错误 :241-258 → 缓存消费 :259-260 → 无 control_request 协议错误 :261-275）；`grep -rn write_tool_result src/` = 0 处（无合成回写）✓ 实读一致 | `tests/ask_user_question.rs:312`（`claude_provider_answers_ask_user_question_only_via_control_response`）/`:403`（`claude_provider_reuses_choice_for_duplicate_control_request_until_native_tool_result`，fixture 内置 exit 44 断言 aria 不得注入 tool_result）✓ | 已落地 |
| 1.4/1.5（实现面收口+fixture 更新） | `3164f6a7` | fixture 三脚本在场：`tests/fixtures/provider/claude_ask_user_question_fixture.sh`、`claude_ask_user_question_tool_error_fixture.sh`、`claude_ask_user_question_tool_use_bridge_failure_fixture.sh` ✓ | 同上用例族（tool_error/tool_use_bridge_failure 专项 fixture）✓ | 已落地 |
| 4.1（全量门禁） | — | 本 Task §2 补验留档 | — | 本 Task 补验 |
| 4.2（真实 CLI smoke 2.1.237+） | — | T3 Step 3（live 门控测试，provider 级 `live_claude.rs`） | — | T3 补验 |

**结论：实做=提案 tasks 1.x-3.x 全部；4.1/4.2 由本 change 补验（吸收语义）。**

## §2 全量门禁补验留档（吸收提案 4.1 / REQ-CCI-06 场景 2）

实施日期 2026-09-18（终版数字为 T2/T4 并行 Task commit 合流后工作树：HEAD=`d7525220`，src 面仅含已提交状态）。全量日志留 `/tmp/wg-{fmt,clippy,test,claude,itweb-rerun}.log`，本节记命令+exit+摘要。

| 命令 | exit | 摘要 |
|---|---|---|
| `cargo fmt --check` | 0 | 无输出，干净 |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | 0 | `Finished dev profile … in 16.85s`，零告警 |
| `cargo test --locked` | 101 | lib **3368 passed / 0 failed / 3 ignored**（41.12s）；doc-test 1 passed；集成面 it_web **404 passed / 4 failed / 12 ignored**（143.90s），其余集成族（175/43/210/54/31）全绿——唯一红面=`tests/it_web/web_work_item_plan_mode/` 4 例（见下 RR-3 定性） |
| `cargo test --locked claude_`（族过滤复核） | 0 | lib 56 passed（args/permissions/ask_user_question 全族）+ it_web 4 + it_core 4 = **64 passed / 0 failed** |

### it_web 4 失败的 RR-3 定性（既有基线红，非本 change 面——如实登记呈报 controller）

首败事实（4 panic 断言，`tests/it_web/web_work_item_plan_mode/part_04.rs`）：
- `:444` `corrupt_outline_revision_journal_fails_closed_without_starting_provider`——「恢复错误不得使 manager 创建失败；连接仍应收到 session_state」
- `:352` `dedicated_request_outline_revision_resumes_incremental_prompt_on_new_app_websocket`——resumed_messages 增量 prompt 断言
- `:487` `missing_legacy_outline_resume_detail_falls_back_to_initial_author_prompt`
- `:312` `persisted_outline_revision_intent_resumes_incremental_prompt_on_new_app_websocket`

定向复跑佐证：`cargo test --locked --test it_web web_work_item_plan_mode::` → **17 passed / 4 failed**（53.24s），同 4 例复现——**确定性失败，非 flaky**。

diff 无交集：该目录最近改动=`edf6f4c2`（2026-09-17「manager 回收可达性/摘除原子性/router 生命周期/恢复错误降级」，`git merge-base --is-ancestor edf6f4c2 4cc834e2` = 在计划 BASE 内）；本 change 已提交面（`054560bf`=kimi live 测试+docs、`d7525220`=kimi terminal 测试区、本 Task=纯 docs）与其零交集。

处置：**不修**（范围铁律——非本 change 引入；修复属 controller 另行处置面），如实登记呈报。claude 面（本 Task 补验对象）全绿：lib 56+集成 8 全数通过，REQ-CCI-06 场景 2 门禁证据成立。

过程记录（RR-3 首败-复跑链完整性）：第一次全量跑（23:12 前后）撞上并行 T4 terminal.rs 中间态编译错（E0277 `byte == Ok(b'a')` @ terminal.rs:963，T4 编写中笔误），非测试失败；经 C1T4 知会编辑落定（commit `d7525220`）后取终版数字，上表即终版。
