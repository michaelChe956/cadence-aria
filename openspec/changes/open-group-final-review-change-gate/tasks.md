## 1. 停机语义失败测试

- [ ] 1.1 为 group final review 要求修改结论编写测试，断言落地阻塞门禁且 attempt 状态为阻塞、不停留在运行中。
- [ ] 1.2 为重试验证决策编写测试，断言落地阻塞门禁且 attempt 状态为阻塞。
- [ ] 1.3 为人工分诊与运维阻塞决策编写测试，断言各自落地阻塞门禁且 attempt 状态为阻塞。
- [ ] 1.4 为通过结论编写回归测试，断言不落地门禁且完成路径不变。
- [ ] 1.5 为启动计划修订决策编写回归测试，断言不落地门禁且计划修订编排不变。
- [ ] 1.6 为四个决策编写测试，断言原因码互不相同。
- [ ] 1.7 为任意一次评审编写测试，断言落地的阻塞门禁数量不超过一个；并覆盖「阻塞结论 + actionable finding」这一既可达组合，断言只落一个门禁。
- [ ] 1.8 为分诊门禁编写测试，断言动作集合含重试评审、人工继续、终止。
- [ ] 1.9 为门禁原因码判定编写测试，断言 `RunCoderFix` + group 场景的分支仍生效（该分支可达，MUST NOT 被当作死代码移除）。
- [ ] 1.10 为分诊门禁动作编写测试，断言不唤起计划修订流程。
- [ ] 1.11 改写与新 spec 冲突的既有断言：`tests/plan_defect_entrypoints.rs:330-474`（verdict=blocked + `verification_incomplete` 时保持 `Running`、无 open gate）、`:310-327` 的 `RetryVerification → None`。**`:132-137`（human triage 门禁不含 `send_to_coder`）与 `:319-322`（`RunCoderFix(group) → Some("group_final_review_blocked")`）MUST 保持不变。**

## 2. 生产实现

- [ ] 2.1 去掉门禁落地的评审结论前置条件，改为由流程决策决定；四个需人工介入的决策各落地一个阻塞门禁。
- [ ] 2.2 为四个决策定义互不相同的原因码与门禁标题：`internal_review_change_requested`、`internal_review_verification_incomplete`（新增），`internal_review_human_triage`、`internal_review_operational_blocker`（已存在于 `internal_pr_review.rs:12-13`）。group final review 与 internal PR review 使用同一组原因码、标题前缀区分。不改既有命名家族的 `output_` 不齐。
- [ ] 2.3 实现互斥：落门禁判定收敛为「流程决策 → reason code」的单一映射，一个决策对应一个门禁。MUST NOT 保留独立的 verdict 落地路径与决策落地路径并存（那才是重复落地的来源）。`code_review.rs:202-203` 的 `lands_code_review_blocked` 形式可参考，但其语义依赖 internal review 不存在的独立分支，MUST NOT 照搬。
- [ ] 2.4 保留 `internal_pr_review.rs:8-10` 的 `RunCoderFix if is_group_final_review` 分支。原计划移除它是基于错误的不可达判断（`plan_defect.rs:215-217` 在 `Blocked` + actionable finding 下推出 `RunCoderFix`，前置成立）。
- [ ] 2.5 扩展阻塞门禁的动作集合，为四个新原因码提供重试评审、人工继续、终止三个动作。
- [ ] 2.6 移除既有 `internal_review_blocked` 门禁上必定失败的 `send_to_coder` 动作（`provider_failure.rs:65-70`）：`gates.rs:756-766` 要求 `stage == CodeReview && role == CodeReviewer`，不匹配则落到 `gates.rs:654` 的 `send_review_limit_feedback_to_coder`，后者在 `rework.rs:336-346` 同样要求 `stage == CodeReview`，必然返回 `send_to_coder_not_available`。同步调整 `tests/provider_failure_recovery.rs:590-609` 的动作断言。
- [ ] 2.7 确认未改动：流程决策判定逻辑、通过结论完成路径、计划修订唤起条件、Code Review 阶段分诊门禁行为、`valid_stage_transition`、`gates.rs` 的 `is_code_review_feedback_gate`、`rework.rs`、`web/coding_ws_handler/runner.rs`。
- [ ] 2.8 在 design 已知缺口中记录四项独立立项：「人工继续」空转、internal PR review 无送回 Coder 通道、`StartStoryAmendment` / `StartDesignAmendment` 无编排、reason code 命名家族不齐。

## 3. 验证与交付

- [ ] 3.1 运行本 change 相关定向测试与 internal PR review、code review 分诊、gate 动作、provider failure recovery 既有回归，并区分既有失败基线。
- [ ] 3.2 运行 `cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings` 与各测试目标（`--lib`、`it_core`、`it_web`、`it_provider`、`it_product`、`it_task_run`）。
- [ ] 3.3 严格校验 OpenSpec change 并完成代码审查。
- [ ] 3.4 经用户确认后重启后端，由用户验证 group final review 要求修改时出现带三个动作的阻塞门禁而非静默停机。同时说明当前无送回 Coder 通道，需返修时的替代路径是终止 attempt 后重新发起。
