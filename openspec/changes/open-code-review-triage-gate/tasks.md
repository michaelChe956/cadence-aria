## 1. 停机语义回归测试

- [ ] 1.1 为 `StopForHumanTriage` 编写失败测试，覆盖 reason code `code_review_output_human_triage`、attempt 由 `running` 转 `blocked`、动作集合为重试代码审查与终止。
- [ ] 1.2 为 `RetryVerification` 编写失败测试，覆盖 reason code `code_review_verification_incomplete` 与同一动作集合。
- [ ] 1.3 为 `OpenOperationalGate` 编写失败测试，覆盖 reason code `code_review_operational_blocker` 与同一动作集合。
- [ ] 1.4 为门禁互斥编写测试，断言 `verdict=blocked` 且无可执行 finding 时只落地单个 `code_review_blocked` 门禁。
- [ ] 1.5 为 Reviewer 契约编写失败测试，断言 projection 渲染同时包含 implementation defect 路由字段禁令与自然语言证据出口说明。

## 2. 生产实现

- [ ] 2.1 在 Code Review 执行路径中为三个人工路由决策落地 blocked gate 并置 attempt 为 `blocked`，与既有 `code_review_blocked` 门禁互斥。
- [ ] 2.2 在 Reviewer projection 结构化输出契约中增补 implementation defect 字段边界与证据出口说明。
- [ ] 2.3 确认未改动 plan defect 校验判定、未引入自动化验证补跑、未引入任何 Testing 或 tester 相关内容。

## 3. 验证与交付

- [ ] 3.1 运行本 change 相关定向测试与既有 Code Review、plan repair、internal PR review、completion gate 回归，并区分既有失败基线。
- [ ] 3.2 严格校验 OpenSpec change 并完成代码审查。
- [ ] 3.3 经用户确认后重启后端，由用户验证 Code Review 停机时具备可操作门禁且流程不再假死。
