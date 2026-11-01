# Spec Delta

> 基线：`openspec/specs/work-item-group-coding-execution/spec.md` 现有 REQ-GCE-01 至 REQ-GCE-05、REQ-GCE-C2-INSTR、REQ-C1-CHILD-01。本 delta 只新增 REQ-GCE-06 与 REQ-GCE-07，不修改既有 requirement；调度与 unit 完成判据（REQ-GCE-01、REQ-GCE-02）保持不变。

## ADDED Requirements

### Requirement: 下游启动不依赖上游最终确认（REQ-GCE-06）

多仓 group 中某 Work Item 的下游 attempt 调度 SHALL 只以既有 unit/run 完成事实与 handoff 发布为就绪判据，MUST NOT 把上游 attempt 的人工最终确认（FinalConfirm）作为下游启动前置。plan 级延迟聚合确认引入后，中间仓 attempt 停留在等待人工最终确认时，下游依赖齐备的 attempt SHALL 照常自动启动；平台 MUST NOT 因存在待确认的上游 attempt 而向用户逐仓催点最终确认。

#### Scenario: 上游留待最终确认仍自动启动下游

- **WHEN** 双仓 plan 中上游 WI 的 unit run 已 Completed 且 handoff 已发布，但其 attempt 停留在 WaitingForHuman＋FinalConfirm
- **THEN** 下游 WI 的 attempt 被自动调度启动并进入 coding，无需用户先确认上游
- **AND** 平台不为上游的逐仓最终确认发送催点通知，聚合确认通知只在全部执行完成后出现

#### Scenario: 聚合确认不改变调度判据

- **WHEN** 引入 plan 级聚合最终确认后，某 plan 存在部分 attempt 已完成、部分仍在执行
- **THEN** scheduler 的完成集与就绪判定仍只由 unit Completed 与 handoff 事实驱动，读取路径与引入前一致

### Requirement: 上游完成时契约与能力登记（REQ-GCE-07）

Work Item 完成发布 handoff 时，若 handoff 的 provided_contract_refs 为空且该完成 run 绑定的冻结 output_contracts 声明了契约，系统 SHALL 从冻结 output_contracts 登记契约标识与对应能力到新发布的 handoff；refs 非空时 SHALL 保持既有取值不变；无 output 的非接口 WI SHALL 允许为空。系统 MUST NOT 把 coder 自由文本自报解析为权威契约。

#### Scenario: refs 为空时从冻结 output 登记

- **WHEN** 某 WI 完成 run 的 handoff refs 为空，而其冻结 output_contracts 声明了 CT-001 及能力
- **THEN** 新发布的 handoff 携带该 contract ref 与对应 capabilities，并沿用既有 hash/build/publish 链
- **AND** 下游读取该 handoff 时能取到契约与能力

#### Scenario: 非空 refs 与无 output 保持原行为

- **WHEN** 完成 run 的 handoff refs 非空，或该 WI 无 output_contracts
- **THEN** 登记结果与既有行为一致（分别保持原选择、允许为空），不产生新失败
