# Spec Delta

## ADDED Requirements

### Requirement: 分诊门人工继续后回到原编排链

在任一 Code Review 分诊门（`code_review_output_human_triage`、`code_review_verification_incomplete`、`code_review_operational_blocker`）执行 `manual_continue` 后，系统 MUST 关闭该门、把 attempt 从 `blocked` 恢复为可执行，并唤回 runner 从 Code Review 之后的阶段继续推进。系统 MUST 复用该次已持久化的 Code Review 结论，MUST NOT 重新启动已完成的 Code Reviewer 运行。该续跑 MUST NOT 依赖提交动作的连接保持打开。

#### Scenario: manual_continue 后自动续跑

- **WHEN** 用户在 Code Review 分诊门执行 `manual_continue`，随后关闭页面与 coding socket
- **THEN** attempt 离开 `blocked` 并从 Code Review 之后的阶段继续推进，Code Reviewer 运行次数不增加

#### Scenario: 续跑中断后可再次继续

- **WHEN** `manual_continue` 已持久化但 runner 唤回失败
- **THEN** 系统落地可操作等待项并通知用户，用户点击继续后从同一阶段续跑，不重复已完成 review

### Requirement: 验证处理入口独立于分诊门四动作

三个 Code Review 分诊门的可执行动作集合 MUST 保持恰好为 `send_to_coder`、`retry_review`、`manual_continue` 与 `abort`。当门的 finding 涉及计划验证命令不可满足时，系统 MAY 在门旁提供转入独立“验证处理”面的入口；该入口 MUST NOT 作为门动作出现在门的动作集合中，转入本身 MUST NOT 唤起 plan repair，MUST NOT 关闭或改写该门及其 finding。验证处理中用户批准“计划修订”时，系统 MUST 经既有 plan amendment 链由验证处理记录发起，而不是作为该门的动作。验证处理得出用户批准的等价证据或限域例外结论后，原链继续的效果 MUST 与在该门执行 `manual_continue` 一致并留下验证处理审计。

#### Scenario: 转入验证处理不改变门动作

- **WHEN** Code Review 验证不完整门打开，用户点击“进入验证处理”
- **THEN** 门的可执行动作集合仍恰好为四动作，门与 finding 保持不变，验证处理记录被创建

#### Scenario: 验证处理批准后续跑

- **WHEN** 用户在验证处理面批准可信等价证据
- **THEN** 原门关闭并按 `manual_continue` 语义续跑，finding 保留且关联该验证处理审计，不唤起 plan repair
