# 阶段 4 C3 · T5 后端 legacy 删除跨端原子（WP5 / REQ-RET-02 L2）报告

- **worktree**：`feat-b-0808-add-monorepo`；T5 提交区间 `d4c39752`（归属表）→`cc073879`（后端删除）→`88e3dc20`（SC gate 重钉）→`ab3d5a0c`（preflight 终态收敛）→`b50b09b7`（行数 guard）→`ae6aa769`（前端归零）→`c535000e`（clippy 收口）→`84b4cc02`（it_web staged 族 51 测退役+移交）→ 本报告收尾提交（it_web 残余退役+残留断言定稿+报告）。
- **状态**：**完成**——尾项（it_web 残余退役/全量终跑/残留断言定稿/报告）已收口，全部门禁绿（见 §2）。

## 1. 交付摘要（对照计划 Step 1-5）

| Step | 结果 |
|---|---|
| S1 归属判定 | `wp5-attribution-table.md`：必删集 6 族（契约钉死）+共享变体逐符号消费链复核；复核新发现并处置=SC 可达 review_decision 两处（review/routing policy 旁路臂+plan_repair_review）改道 human gate（否则删消息后成死局）；ContextBlocker oracle P2 二选一判定=**(b) 随 L2 删**（三入口 plan_outline/authoring.rs:213/248/318 全 legacy 流 SC 不触达+T4 后无入站消费） |
| S2 wire 层 | 红测先行（it_core part_07 through-socket 负向测试：10 退役 wire 名→`LEGACY_MESSAGE_RETIRED` stage-specific protocol error+零副作用断言，删除前实测红）→in_.rs 删 10 变体+7 DTO；socket parse 面退役识别（`RETIRED_INBOUND_MESSAGE_TYPES` 白名单活代码保留=parse 拒收功能本体）；protocol.rs 白名单收敛（ReviewDecision 阶段恒拒）；删 `single_candidate_generation_decision_error` 精确拒绝面（parse 拒收取代） |
| S3 引擎层 | 7 决策函数（handle_human_confirm/ContextBlocker 族/handle_author_decision/outline decision/handle_review_decision/skip_optional）+draft/batch wire handler+outline revision request+presentation save+revert mark+34 孤儿函数；变体退役=`AuthorDecisionOutcome` 整型+`WorkItemPlanOutlineRevisionSource::HumanConfirm`（fix round 1 措辞修正：`ReviewDecisionOutcome::HumanConfirm` 保留=读侧状态机兼容面，无新增构造点，inbound no-op 臂存续）；非 SC Confirm 帧直连 `handle_confirm`（T4 §4 承接）；flow_kind 单路径（生产唯一创建路径显式恒 SingleCandidate，serde 缺字段默认保持 Legacy=历史读兼容） |
| S4 preflight | `LegacyFallback`→`Ineligible`；失败=新路径 durable Failed 会话+原因+`SINGLE_CANDIDATE_PREFLIGHT_FAILED`；无 legacy 回落/无 flow_kind 切换；三测试重钉（:567 重钉为终态收敛、:514 rollout 不再切流、:545 改名单路径断言）；**fix round 1（k3 F1）**：preflight 去除 rollout flag 门控=恒评估（Legacy/Logical 两臂），flag off（CLI 默认）下多仓/零仓同样 prepare 期 durable Failed 收敛，并补 flag-off 多仓负向测试 |
| S5 跨端归零 | 前端 28 文件 -1154 行（union 8 分支+presentation 出站、7 发送器、ReviewDecisionActions/ActionBar+prop 链、cockpit/ChatInputBar 决策面、presentation 编辑器保存链、store 动作、mock 面）；legacy 回归测试族退役留档（退役依据=wp1-gate-retest/evidence-matrix.md:22「legacy 路径回归全绿」子项）；**收尾批**：it_web 残余红名单 31 测+残留 wire 字面量驱动的 `#[ignore]` legacy 测 11 测+共享夹具族随删（详见 `wp5-residue-scan.md` §退役）；9 符号残留断言定稿归零（见 §2 残留断言行） |

## 2. 验证证据（终态，2026-09-19 收尾后全量终跑）

| 门禁 | 命令 | 结果 |
|---|---|---|
| fmt | `cargo fmt --check` | ✅ |
| clippy | `cargo clippy --all-targets --all-features --locked -- -D warnings` | ✅ 0 error/0 warning |
| lib | `cargo test --locked --lib` | ✅ 3295/0（3 ignored 既有） |
| it_core | `cargo test --locked --test it_core` | ✅ 157/0（含 part_07 退役消息负向测试族） |
| it_web | `cargo test --locked --test it_web` | ✅ **328/0**（1 ignored 既有）——残余红名单 31 测+ignored legacy 11 测退役后收口（退役前 328 passed/31 failed/12 ignored；套件时长 511s→36s，超时等待已删 staged 流事件的测试全部移除） |
| 其余 Rust targets | `cargo test --locked`（main 0/aggregate 1/it_interactive 43/it_product 210/it_provider 54/it_task_run 31/web_logical 44/末尾 2） | ✅ 全绿，**全量合计 4165 passed / 0 failed**（ignored 4 既有） |
| 前端 | `cd web && npm test` | ✅ **1473/1473**（174 文件）；tsc -b 0 error（ae6aa769/d42edc03 已锚定） |
| 残留断言 | 9 符号+wire 精确口径 grep | ✅ **归零**——终版口径、脚本与执行输出留档见 `wp5-residue-scan.md`（豁免=part_07 负向测试+cadence/reports+protocol.rs 拒收白名单活代码；`node_type:"human_confirm"` 8 处=timeline 节点类型「历史只读」豁免面；宽口径 298=stage/node_type 只读串） |

> 注：全量链按 handoff 命令链**顺序执行**。开发过程中曾将 `cargo test` 与 `npm test` 并行触发，it_web 出现 1 例资源竞争失败（隔离复跑+顺序终跑均绿）——并行执行不属门禁口径，如实登记备查。

## 3. 已知偏差（如实登记，6 条+fix round 处置）

1. `WorkItemGenerationModeDto` 迁 artifact.rs（历史 outline candidate 载荷字段+SC 内部诊断 `select_internal_generation_mode`），wire 面归零；计划必删集字面「DTO 删除」按 wire 语义执行；残留断言符号表不含该名（其值类型本体=历史载荷兼容面，doc 注释已按描述性措辞定稿）。
2. 非 SC `Confirm` 帧直连 `handle_confirm`（approve 帧面保留=T4 §4 登记限制承接；错误码沿用 `INVALID_HUMAN_CONFIRM_ACTION`）。
3. it_core choice/reconnect 13 测随 `accept_author_output` 夹具退役——覆盖面由 campaign_stage3 恢复矩阵+disconnect e2e 承接。
4. staged 引擎内部函数（accept/rewrite/downgrade/metadata 族）compiler 判孤儿随删；`begin_work_item_batch_review_run` 以 `#[cfg(test)]` 保留（policy routing 夹具）；review routing legacy 臂仍调用的读侧状态机函数保留（服务在途会话）。
5. story/design 决策通道随协议整体终止（T4 §4「T5 后 legacy 决策通道整体终止」既定登记）；产物确认走 HTTP `POST /api/workspace-sessions/{id}/confirm`；在途 legacy 会话决策拒绝=REQ-RET-03 登记限制（part_07 负向测试锚定）。
6. （补充登记）lib 1 例 `codex_provider_request_user_input_emits_protocol_error_on_write_failure` 历史首跑红/单跑绿=写失败注入 flaky（RR-3 已登记；本收尾两次全量终跑均未发作）。
7. （fix round 1 处置，k3 F1/P2）preflight 曾以 rollout flag 门控：flag off（CLI 默认 opt-in，README 启动命令均未带 flag）时多仓/零仓 logical Issue 跳过 preflight，生成期才以 `TargetAmbiguous`/`TargetMissing` 失败——REQ-WSC-08 prepare 期终态收敛条款空洞+较 patch 前行为回退。**处置**：lifecycle.rs Legacy/Logical 两臂去 `if rollout_snapshot` 门控（preflight 恒评估，flag 其余用途/会话持久化不动），补 flag-off 多仓负向测试（prepare 期即 durable Failed+原因），并修正 lifecycle_tests.inc.rs「flag 仅历史记录」自相矛盾注释——flag 现真正仅随会话持久化作历史记录。

## 4. 提交清单（T5 区间，时序）

| commit | 内容 |
|---|---|
| d4c39752 | docs(wp5): 归属判定表+首版移交笔记 |
| cc073879 | refactor(ws)!: 后端 legacy 决策协议删除（52 文件 -8378 行） |
| 88e3dc20 | test(ws): SC revise gate 重钉 human-gate 路由 |
| ab3d5a0c | feat(lifecycle): preflight 新路径终态收敛+三测试重钉 |
| b50b09b7 | style(review): routing.rs 1200 行 guard 合规 |
| ae6aa769 | feat(web)!: 前端归零（28 文件 -1154 行） |
| c535000e | chore: clippy -D warnings 收口（45 文件 -2006 行） |
| 84b4cc02 | test(it_web)+docs(wp5): staged-decision it_web 51 测退役+T5 报告骨架+residue 快照+handoff v2 |
| c73a1269 | test(it_web)+docs(wp5): it_web 残余收尾——红名单 31+ignored legacy 11+共享夹具族退役、生产注释符号名描述性改写、residue 终版归零、T5 报告补完 |
| fix round 1 提交 | fix(lifecycle)+docs(wp5): k3 审修——preflight 去 rollout 门控恒评估+flag-off 多仓负向测试+注释矛盾修正；P3 三处（residue 数字 1 ignored/329、报告 S3 归属措辞+归属表 §5 回改、前端悬挂 break 删除）；报告补 fix round 记录 |

## 5. 移交收口说明

`wp5-handoff-notes.md` §剩余四项均已执行完毕（①it_web 残余退役→全绿 ②全量终跑→4165/0+1473/1473 ③残留断言定稿→`wp5-residue-scan.md` 终版归零 ④本报告补完）；⑤openspec/tasks 勾选=controller 统一处理（T6 范围，不在 T5 内）。

## 6. fix round 1 记录（k3 审 d4c39752..c73a1269，1×P2+3×P3，全部处置）

- **F1/P2**：preflight rollout gating 去除（controller 裁定=恒评估；REQ-WSC-08 单路径终态在 prepare 期收敛，gating 属 Legacy 过渡痕迹）。`src/web/handlers/lifecycle.rs` Legacy/Logical 两臂去门控；新增测试 `prepare_preflight_failure_converges_to_new_path_terminal_even_when_rollout_flag_is_disabled`（默认 flag off+2 仓→prepare 5xx+durable Failed+`single-candidate preflight requires exactly one logical repository` 原因）；`lifecycle_tests.inc.rs:517` 注释同步修正。flag 本身（CLI opt-in、会话 options 持久化、cli.rs 其他消费点）不动。
- **F2/P3**：`wp5-residue-scan.md` it_web 终态数字更正=1 ignored/总数 329（初版误记 10/338）。
- **F3/P3**：本报告 S3 变体退役措辞修正（`ReviewDecisionOutcome::HumanConfirm` 保留无构造点；实际退役=`AuthorDecisionOutcome` 整型+`WorkItemPlanOutlineRevisionSource::HumanConfirm`）；归属表 §5 inputs.rs Default 行回改为「保留 Legacy」与实现（保持 Legacy+注释）对齐。
- **F4/P3**：`web/src/hooks/workspace-ws-message-handler.ts` 退役注释后悬挂 `break;` 删除。
- 验证（fix round 1 后实测）：定向 lifecycle 8/0（含新增 flag-off 负向测试）；`cargo test --locked --lib` **3296/0**（3 ignored 既有；+1=新测试）；`cargo test --locked --test it_web` **328/0/1**；`cd web && npm test` **1473/1473**；`cargo fmt --check` ✅；`cargo clippy --all-targets --all-features --locked -- -D warnings` ✅ 0/0。
