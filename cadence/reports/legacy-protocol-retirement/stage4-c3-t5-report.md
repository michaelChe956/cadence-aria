# 阶段 4 C3 · T5 后端 legacy 删除跨端原子（WP5 / REQ-RET-02 L2）报告

- **worktree**：`feat-b-0808-add-monorepo`；T5 提交区间 `d4c39752`（归属表）→`cc073879`（后端删除）→`88e3dc20`（SC gate 重钉）→`ab3d5a0c`（preflight 终态收敛）→`b50b09b7`（行数 guard）→`ae6aa769`（前端归零）→`c535000e`（clippy 收口）→ 本报告提交。
- **状态**：**接近完成**——删除面全落地、主要门禁绿；`it_web` 残余 legacy 驱动测试退役+全量终跑+残留断言定稿为移交尾项（见 `wp5-handoff-notes.md` §剩余）。

## 1. 交付摘要（对照计划 Step 1-5）

| Step | 结果 |
|---|---|
| S1 归属判定 | `wp5-attribution-table.md`：必删集 6 族（契约钉死）+共享变体逐符号消费链复核；复核新发现并处置=SC 可达 review_decision 两处（review/routing policy 旁路臂+plan_repair_review）改道 human gate（否则删消息后成死局）；ContextBlocker oracle P2 二选一判定=**(b) 随 L2 删**（三入口 plan_outline/authoring.rs:213/248/318 全 legacy 流 SC 不触达+T4 后无入站消费） |
| S2 wire 层 | 红测先行（it_core part_07 through-socket 负向测试：10 退役 wire 名→`LEGACY_MESSAGE_RETIRED` stage-specific protocol error+零副作用断言，删除前实测红）→in_.rs 删 10 变体+7 DTO；socket parse 面退役识别；protocol.rs 白名单收敛（ReviewDecision 阶段恒拒）；删 `single_candidate_generation_decision_error` 精确拒绝面（parse 拒收取代） |
| S3 引擎层 | 7 决策函数（handle_human_confirm/ContextBlocker 族/handle_author_decision/outline decision/handle_review_decision/skip_optional）+draft/batch wire handler+outline revision request+presentation save+revert mark+34 孤儿函数；`ReviewDecisionOutcome::HumanConfirm` 等变体退役；非 SC Confirm 帧直连 `handle_confirm`（T4 §4 承接）；flow_kind 单路径（生产唯一创建路径显式恒 SingleCandidate，serde 缺字段默认保持 Legacy=历史读兼容） |
| S4 preflight | `LegacyFallback`→`Ineligible`；失败=新路径 durable Failed 会话+原因+`SINGLE_CANDIDATE_PREFLIGHT_FAILED`；无 legacy 回落/无 flow_kind 切换；三测试重钉（:567 重钉为终态收敛、:514 rollout 不再切流、:545 改名单路径断言） |
| S5 跨端归零 | 前端 28 文件 -1154 行（union 8 分支+presentation 出站、7 发送器、ReviewDecisionActions/ActionBar+prop 链、cockpit/ChatInputBar 决策面、presentation 编辑器保存链、store 动作、mock 面）；legacy 回归测试族退役留档（退役依据=wp1-gate-retest/evidence-matrix.md:22「legacy 路径回归全绿」子项）；clippy -D warnings+fmt 全绿 |

## 2. 验证证据（截至本报告）

| 门禁 | 命令 | 结果 |
|---|---|---|
| clippy | `cargo clippy --all-targets --all-features --locked -- -D warnings` | ✅ 0 error/0 warning |
| fmt | `cargo fmt --check` | ✅ |
| lib | `cargo test --locked --lib` | ✅ 3299/0（3 ignored 既有；1 例 codex_provider 写失败注入首跑红/单跑绿=flaky RR-3 登记） |
| it_core | `cargo test --locked --test it_core` | ✅ 158/0（含 part_07 退役消息负向测试族） |
| it_web | `cargo test --locked --test it_web` | ⏳ **待收口**——staged 决策驱动 51 测已退役；残余红名单（work_item_plan_author/outline context_blocker 族/batch_generation/mode outline_human_confirm 等）待同款退役后复跑 |
| 前端 | `npx tsc -b --force`+`npx vitest --run` | ✅ 0 error；1473/1473 |
| 残留断言 | 9 符号 grep | ⏳ 待定稿 `wp5-residue-scan.md`（快照与口径见 handoff §剩余-3） |

## 3. 已知偏差（如实登记）

1. `WorkItemGenerationModeDto` 迁 artifact.rs（历史 outline candidate 载荷字段+SC 内部诊断消费），wire 面归零；计划必删集字面「DTO 删除」按 wire 语义执行。
2. 非 SC `Confirm` 帧直连 `handle_confirm`（approve 帧面保留=T4 §4 登记限制承接；错误码沿用 `INVALID_HUMAN_CONFIRM_ACTION`）。
3. it_core choice/reconnect 13 测随 `accept_author_output` 夹具退役——覆盖面由 campaign_stage3 恢复矩阵+disconnect e2e 承接。
4. staged 引擎内部函数 compiler 判孤儿随删；`begin_work_item_batch_review_run` `#[cfg(test)]` 保留（policy routing 夹具）。
5. story/design 决策通道随协议整体终止（T4 §4「T5 后 legacy 决策通道整体终止」既定登记）；产物确认走 HTTP `POST /api/workspace-sessions/{id}/confirm`；在途 legacy 会话决策拒绝=REQ-RET-03 登记限制（part_07 负向测试锚定）。

## 4. 移交

见 `wp5-handoff-notes.md`（第二版）：①it_web 残余红名单退役（脚本与处置规则在册）②全量终跑 ③残留断言定稿口径（`type: "human_confirm"` wire 形态归零+stage 串只读豁免）④本报告补数字⑤openspec/tasks=controller（T6）。
