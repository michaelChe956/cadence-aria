# WP5 实施移交笔记（T5 交接 · 2026-09-19 · 第二版）

> 状态：**接近完成**——删除面/前端/preflight/负向测试全部落地；`cargo clippy --all-targets -D warnings`+`cargo fmt --check` **已绿**（c535000e）；lib 3299/0 绿、it_core 158/0 绿、web vitest 1473/1473+tsc 0 绿；**剩余=it_web 残余 legacy 驱动测试退役 + 全量终跑 + 残留断言定稿 + 报告完善**。Main 指示停止迭代，剩余移交。

## Commits（时序）

| commit | 内容 |
|---|---|
| d4c39752 | docs(wp5): 归属判定表+首版移交笔记 |
| cc073879 | refactor(ws)!: 后端 legacy 决策协议删除（52 文件 -8378 行） |

| 88e3dc20 | test(ws): SC revise gate 重钉 human-gate 路由 |
| ab3d5a0c | feat(lifecycle): preflight 新路径终态收敛+三测试重钉 |
| b50b09b7 | style(review): routing.rs 1200 行 guard 合规 |
| ae6aa769 | feat(web)!: 前端归零（28 文件 -1154 行） |
| c535000e | chore: clippy -D warnings 收口（45 文件 -2006 行：孤儿引擎面/测试夹具/导入卫生） |

## it_web 残余红名单（2026-09-19 实跑，31 failed / 328 passed / 12 ignored，511s）——接手人直接按此退役，无需重跑

- **web_work_item_plan_author（7）**：outline_review_revise_requires_decision_before_next_outline_provider_run / work_item_plan_author_completes_provider_node_before_author_confirm / work_item_plan_author_emits_provider_prompt_event / work_item_plan_author_persists_outline_without_draft_work_items_or_child_sessions / work_item_plan_author_streams_provider_output_before_outline_artifact / work_item_plan_outline_review_revision_reenters_review_after_revised_outline_confirm / work_item_plan_start_generation_returns_outline_artifact
- **web_work_item_plan_batch（4）**：batch_generation_invokes_one_provider_run_per_outline / batch_local_validation_failure_retries_once / batch_local_validation_second_failure_marks_validation_failed_and_continues / batch_mode_creates_batch_record_for_current_round
- **web_work_item_plan_mode（5）**：outline_human_confirm_request_change_starts_dedicated_revision_over_websocket / request_outline_revision_on_mode_node_sets_outline_revising / request_revision_on_outline_confirm_returns_to_outline_run_without_round / select_mode_rejected_outside_generation_mode_node / session_state_restores_generation_mode_node_with_outline_payload
- **web_work_item_plan_outline（7）**：context_blocker_confirm_is_rejected / context_blocker_human_resolution_appends_index_and_next_prompt / context_blockers_enter_context_blocker_node / outline_structured_json_parse_failure_auto_retries_then_accepts_valid_outline / outline_validation_failure_auto_retries_then_human_blocker / valid_outline_enters_outline_confirm / work_item_plan_start_generation_creates_outline_run_node
- **web_work_item_plan_serial（5）**：local_validation_success_enters_draft_confirm_with_accept / serial_draft_run_emits_provider_prompt_event / serial_item_run_writes_draft_record_not_real_work_item / serial_local_validation_failure_repairs_once_with_findings / serial_mode_starts_first_outline_by_topological_order
- **web_work_item_plan_staged_flow（1）**：session_state_restores_work_item_plan_staged_artifacts
- **web_workspace_recovery_consistency（2）**：reviewer_repair_failure_live_diagnostic_matches_reloaded_node_detail / story_design_work_item_plan_recovery_consistency

注意：author/serial 的 start_generation/draft_run 类测试名义锚 start_generation（保留面），但断言链落在 author_confirm→逐段决策（已删面）——整测退役即正确处置（T1 矩阵 legacy 回归留档在案）；处置脚本要点见文末附录。

## 已完成（全量）

1. **归属判定表**（wp5-attribution-table.md，已提交）：必删集 6 族+共享变体+ContextBlocker oracle (b)+SC 可达 review_decision 改道+flow_kind 落点。
2. **后端删除**：in_.rs 10 变体+7 DTO（WorkItemGenerationModeDto 迁 artifact.rs）；退役消息 parse 面拒收→`LEGACY_MESSAGE_RETIRED` stage-specific protocol error（protocol.rs helpers+socket.rs 接入+2 单测）；handler 3 决策函数删除+`handle_confirm_from_handler`（非 SC approve 帧直连）；inbound 9 臂删除+RequestRevision 非 plan 桥接→protocol error；引擎 7 决策函数+3 helper+draft/batch/outline/presentation/revert wire 面+孤儿面（34 函数）；`ReviewDecisionOutcome::HumanConfirm`/`AuthorDecisionOutcome`/`WorkItemPlanOutlineRevisionSource::HumanConfirm` 变体退役；routing/plan_repair_review SC 守卫改道 human gate（`is_single_candidate_plan` helper）。
3. **Step 4 preflight**：`LegacyFallback`→`Ineligible`；失败=SingleCandidate durable Failed+原因（`SINGLE_CANDIDATE_PREFLIGHT_FAILED`）；三测试重钉；inputs.rs Default 保持 Legacy（注释说明生产路径已显式恒 SC）。
4. **前端归零**：union 8 分支+presentation 出站 2 变体+`RevertWorkItemMessage`/`SaveHumanPresentationRevisionMessage`；7 发送器；ReviewDecisionActions/ActionBar+prop 链（ChatEntryList/Renderer/MessageGroupView/ReviewVerdictEntry）；cockpit handleAuthorDecision/staged props/ActionBar/送审定稿钮；ChatInputBar staged 分支链；StagedPanel 四分支；presentation 编辑器保存链（Editor/ArtifactPanel/Overview/ProjectionTabs）+store begin/fail+message-handler 两 case；mock 面收敛；测试退役/重钉（17 测退役+2 空壳/半段处置）。
5. **测试退役**：引擎 57+测、handler/ws_types、it_core part_01-06b（accept_author_output 夹具链 8 测连带）、it_web staged 族已删 51 测（staged_flow 2/serial 14/batch 7/compile 7+runtime_projection 1/mode 9/recovery_consistency 2）。
6. **门禁已过**：clippy -D warnings（lib+all-targets）✅；fmt ✅；lib 3299/0 ✅（1 例 codex_provider 写失败注入 flaky 单例单跑绿，RR-3 登记）；it_core 158/0 ✅；vitest 1473/1473 ✅；tsc -b 0 ✅。

## 剩余（按序，接手人执行）

1. **it_web 残余 legacy 驱动测试退役**（`cargo test --locked --test it_web` 实跑红名单，已观察到失败族）：
   - web_work_item_plan_author::（outline_review_revise_requires_decision…/completes_provider_node/emits_provider_prompt_event/persists_outline…/streams_provider_output… 等——文件 tests/it_web/web_work_item_plan_author/part_0*.rs）
   - web_work_item_plan_outline::context_blocker_*（confirm_is_rejected/human_resolution_appends…/enter_context_blocker_node——ContextBlocker 面已删）+outline_*_auto_retries_then_*（human_blocker 终态断言依赖旧面）
   - web_work_item_plan_batch::batch_generation_invokes_one_provider_run_per_outline
   - web_work_item_plan_mode::outline_human_confirm_request_change_starts_dedicated_revision_over_websocket
   - 处置=同款脚本退役（/tmp/retire_tests.py 尚存；DELETED 列表含 9 wire 名 raw 串）→ 复跑至 `--test it_web` 全绿。红名单获取：`cargo test --locked --test it_web 2>&1 | grep '^test .* FAILED'`。
2. **全量终跑**：`cargo fmt --check && cargo clippy --all-targets --all-features --locked -- -D warnings 2>&1 | tail -2 && cargo test --locked 2>&1 | tail -6 && cd web && npm test 2>&1 | tail -3`（预期：除 ①中 flaky 单例外全绿；it_core workspace_ws 集成含 part_07 负向测试应绿）。
3. **残留断言定稿**→`wp5-residue-scan.md`：9 符号全仓 grep（豁免=part_07 负向测试文件+cadence/reports 目录）。当前快照（已滤 part_07+退役注释）：`HumanConfirmDecision` 5 命中=conversational_gate.rs doc 注释+tests/conversational_gate.rs ×2（需查改）；`SelectWorkItemGenerationMode`/`WorkItemDraftDecision`/`WorkItemBatchDecision`(实为 Outcome 后缀误中)/`ReviewDecisionResponse` 命中=退役注释+kept 枚举后缀，需在注释中改写符号名或以 `\b` 边界定稿；`human_confirm` 349 命中=stage/node_type 只读串（豁免「历史只读」）——断言脚本用 `type: "human_confirm"`/`"type":"human_confirm"` 精确 wire 形态归零口径出具，豁免口径写入 scan 文档。
4. **报告**：`stage4-c3-t5-report.md` 骨架已建（见下），补门禁数字+residue 引用+提交清单后提交。
5. **openspec/tasks 勾选**：controller 统一处理（T6 范围）。

## 已知偏差（报告须如实登记，5 条）

1. `WorkItemGenerationModeDto` 迁 artifact.rs（历史 artifact 载荷字段+SC 内部诊断 `select_internal_generation_mode`），wire 面归零；残留断言符号表不含该名。
2. 非 SC `Confirm` 帧直连 `handle_confirm`（T4 §4「approve 帧 legacy 分支仍接受」承接）；错误码沿用 `INVALID_HUMAN_CONFIRM_ACTION`（前端 GATE_REJECTION_CODES 依赖）。
3. it_core choice/reconnect 13 测随 `accept_author_output` 夹具退役——choice/reconnect 面由 campaign_stage3 恢复矩阵与 disconnect e2e 承接。
4. staged 引擎内部函数（accept/rewrite/downgrade/metadata 族）compiler 判孤儿随删；`begin_work_item_batch_review_run` 以 `#[cfg(test)]` 保留（policy routing 测试构造 batch review 场景）。
5. lib 全量 1 例 `codex_provider_request_user_input_emits_protocol_error_on_write_failure` 首跑红/单跑绿=写失败注入 flaky（改动面零交集，RR-3：首败事实+定向复跑佐证）。

## 附：retire 脚本

`/tmp/retire_tests.py`（Rust #[test]/#[tokio::test] 函数级退役，DELETED 符号驱动）与 `/tmp/retire_js_tests.py`（vitest it/test 块级）在本机 /tmp；若丢失按 handoff 第一版描述重写（~40 行）。
