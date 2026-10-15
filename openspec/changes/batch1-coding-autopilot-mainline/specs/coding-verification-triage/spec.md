# Spec Delta

> 基线：`openspec/specs/coding-verification-triage/spec.md` 现有 REQ-CVT-01 至 REQ-CVT-05。本 delta 只新增 REQ-CVT-06，不修改既有 requirement；REQ-CVT-03 独立验证处理入口与 code_review_* 三门的 evidence_refs 非空前置保持不变。

## ADDED Requirements

### Requirement: coder 输出门的验证处理入口可用（REQ-CVT-06）

对 reason_code 属于 `{coding_output_human_triage, coding_operational_blocker}` 的门，点击「转入验证处理」时前端 SHALL 在 gate 的 `evidence_refs` 无非空项的情况下回退使用 `gate_id` 作为 finding 绑定（`finding_id="<gate_id>#0"`）并发起既有验证处理 REST；`coding_operational_blocker` SHALL 同步加入前后端验证处理资格白名单。code_review_* 三门的既有行为（evidence_refs 非空才可进）MUST 保持不变；后端对 coder 门分支的判定仍只看 reason_code。回退值 MUST NOT 使用含 `/` 的 `raw_provider_output_ref`（会被 store 的相对 id 校验拒绝）。

#### Scenario: 空证据门可进验证处理

- **WHEN** 夹具为 `coding_output_human_triage` 门且 `evidence_refs: []`，用户点击「转入验证处理」
- **THEN** 前端发起验证处理请求且 `finding_id` 为 `<gate_id>#0`，不再提前报「门缺少 finding 绑定证据」而无请求

#### Scenario: 新环境门具备验证处理资格且 CR 门零回归

- **WHEN** `coding_operational_blocker` 门进入验证处理，或既有 code_review_* 三门进入验证处理
- **THEN** 前者被前后端白名单放行；后者行为与断言（含 evidence_refs 非空前置）与改前一致
