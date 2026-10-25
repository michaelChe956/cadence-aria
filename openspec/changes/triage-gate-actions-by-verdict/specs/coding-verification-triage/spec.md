# 规范增量：验证处理入口与按 verdict 派生的门动作集合

<!-- 来源：owner 2026-10-25 裁定（方案 A）。本 delta 只 MODIFIED REQ-CVT-03：
     把「三个 Code Review 分诊门 SHALL 仍恰好提供四动作」改为「SHALL 保持转入前由最新审查
     结论 verdict 派生的动作集合」。REQ-CVT-01/02/04/05 与 `coding_output_human_triage`
     两动作约束保持不变（coder 侧门与 verdict 派生无关）。
     在途 change batch1-coding-autopilot-mainline 在本 capability 只 ADDED REQ-CVT-06，
     与本次 MODIFIED 的 header 无重叠。 -->

## MODIFIED Requirements

### Requirement: 独立验证处理入口（REQ-CVT-03）

当计划字面验证命令在当前环境不可满足时，系统 SHALL 从原门提供受控转入独立“验证处理”面的入口；该入口 MUST NOT 改变原门动作集合：`coding_output_human_triage` 门 SHALL 仍恰好提供 `retry_coding` 与 `abort`，三个 Code Review 分诊门 SHALL 保持转入前由最新一次 Code Review 结论 verdict 派生的动作集合（`retry_review`、`manual_continue`、`abort` 恒在，`send_to_coder` 是否在集合内 MUST 与转入前一致，派生规则见 `coding-code-review-triage`）。验证处理记录 SHALL 绑定 finding、check、plan revision、原命令、替代命令（如有）、cwd、执行结果、测试执行数量、环境与适用 scope。同一 finding／check／plan revision 在已有未决验证处理时 SHALL 拒绝重复转入。

#### Scenario: 从 coder 输出门转入

- **WHEN** coder 输出门因计划字面命令不可满足而打开，用户点击“进入验证处理”
- **THEN** 系统创建唯一的验证处理记录并绑定上述字段，原门动作集合保持 `retry_coding`／`abort` 不变

#### Scenario: 从 Code Review 门转入

- **WHEN** Code Review 验证不完整门打开，用户点击“进入验证处理”
- **THEN** 系统创建或返回同一验证处理记录，原门动作集合与转入前逐项相同（含是否包含 `send_to_coder`），门仍保持 open 且 finding 不变

#### Scenario: 重复转入被拒

- **WHEN** 同一 finding／check／plan revision 已存在未决验证处理，用户再次转入
- **THEN** 系统返回既有记录，不创建第二条
