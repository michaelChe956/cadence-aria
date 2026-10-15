# Spec Delta

> 基线：`openspec/specs/per-repo-coding-execution/spec.md` 现有 REQ-COD-01 至 REQ-COD-06（REQ-COD-06 由未归档 change `fix-local-delivery-and-chain-truth` 修订，与本 delta 无重叠）。本 delta 只新增 REQ-COD-07 与 REQ-COD-08，不修改既有 requirement；REQ-COD-05 跨仓只读证据检索语义保持不变。

## ADDED Requirements

### Requirement: 上游交付注入 coder prompt（REQ-COD-07）

对 work_item_group、存在冻结 target 快照且存在跨仓依赖的 attempt，系统 SHALL 在 coder 的 fresh、delta、rework、teaching 各实际 prompt 组装中追加运行期上游交付清单段（`ResolvedUpstreamDelivery`：上游逻辑 WI/revision、owner attempt、unit run、handoff revision 与 SHA、owner worktree 绝对路径、provided_contracts/capabilities），该段 SHALL 在实际 prompt 记录/claim 前追加。单仓与无依赖 attempt 的 prompt MUST NOT 出现该段。注入信息为运行期 DTO，MUST NOT 写入 immutable handoff、envelope、projection 或 renderer 哈希，MUST NOT 新增 REST/持久字段。

#### Scenario: 下游 prompt 含真实上游交付

- **WHEN** 上游 unit/run 已完成并发布 handoff，而其 attempt 尚待人工确认，下游 coder 构建 fresh/delta/rework/teaching 任一 prompt
- **THEN** 该 prompt 含上游路径、commit SHA 与契约标识（如 CT-001），且不要求上游已最终确认

#### Scenario: 单仓无注入段

- **WHEN** legacy 单仓或无依赖的 group target 构建 coder prompt
- **THEN** prompt 不含上游交付清单段，渲染路径、执行上下文哈希与既有行为一致

### Requirement: 上游不可读走环境阻塞门（REQ-COD-08）

coder 无法读取注入的上游 source_path 时，SHALL 产出 `defect_class=operational_blocker`、`recommended_route=operational_gate` 的 finding（建议 reason_code=`upstream_source_unavailable`，repair_target=null，保留读取命令与路径 evidence），并按环境阻塞门（REQ-CRO-09 的 `coding_operational_blocker`）落门与通知；MUST NOT 落人工输出分诊门，MUST NOT 以契约替身通过替代真实集成验收。

#### Scenario: 上游目录不可读落环境阻塞门

- **WHEN** 注入的上游 worktree 路径不存在或不可读，coder 报告运行环境类 finding
- **THEN** 该 finding 不因 plan 未预声明 blocker_rules 被判无效，attempt 落 `coding_operational_blocker` 门并进入平台通知，用户可再试/说明情况/终止
- **AND** 该 finding 不落 `coding_output_human_triage`
