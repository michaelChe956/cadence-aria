## ADDED Requirements

### Requirement: 删除工作项组时自动化登记一并作废（REQ-WIGA-VOID-01）

删除工作项组时，该 issue 的自动化登记 SHALL 被标成【已作废】（`voided_at` + `void_reason`），SHALL NOT 物理删除该登记；作废 SHALL 以**账本追加一条**的方式留痕，绑定前代 SHALL 进历史。**已作废的登记 SHALL NOT 再约束该 issue 上的新工作项组**：新工作项组重新启用时，`options`（含 provider）、`source`、`target` SHALL NOT 再被逐字段相等拒绝。

**未作废的登记 SHALL 继续按既有语义处理**（REQ-WIGA-02 的 fail-closed / REQ-WIGA-SWAP-01 的换代 / REQ-DRG-18 的悬空只放行 source）。作废 SHALL 只由「删除工作项组」这一条路径产生，MUST NOT 由其它路径顺带产生。

#### Scenario: 删除工作项组使其自动化登记作废

- **WHEN** owner 删除一个工作项组，且该 issue 存在自动化登记
- **THEN** 该登记 SHALL 被标成已作废（带时间与原因），其终止态与作废态 SHALL 可区分
- **AND** 该登记 SHALL NOT 被物理删除

#### Scenario: 已作废的登记不再约束新工作项组

- **WHEN** owner 在同一 issue 上新建工作项组并以不同 provider 启用自动化，而该 issue 的旧登记已作废
- **THEN** 系统 SHALL 接受该次启用并完成换代，MUST NOT 因 provider / source / target 与旧记录不同而拒绝

#### Scenario: 作废以留痕方式表达，不抹掉历史

- **WHEN** 自动化登记被标成已作废
- **THEN** 命令账本 SHALL 追加一条作废条目；既有绑定与历史 SHALL 保留，MUST NOT 被清空或改写

#### Scenario: 未作废的登记语义不变

- **WHEN** 自动化登记未作废，且 owner 提交带不同 provider 的启用命令
- **THEN** 既有判定 SHALL 保持不变（REQ-WIGA-02 的 fail-closed、REQ-WIGA-SWAP-01 的换代放行、REQ-DRG-18 的悬空只放行 source）

#### Scenario: 界面如实显示已作废

- **WHEN** 某 issue 的自动化登记已作废
- **THEN** 界面 SHALL 显示「已随该工作项组删除作废」这一事实，MUST NOT 一律显示为「已于 X 终止」
