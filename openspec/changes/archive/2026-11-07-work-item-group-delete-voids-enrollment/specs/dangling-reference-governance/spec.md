## MODIFIED Requirements

### Requirement: 承接在途 enrollment 语义（REQ-DRG-18）

自动化 enrollment 记录在 story/design 被删后 source 指向已删实体，属于在途变更覆盖的已知状态。巡检 MUST NOT 将其判为未归类；销账 MUST NOT 删除 enrollment 记录。系统 MUST NOT 重新裁定 enrollment 的重新授权语义。story/design 的悬空判定 MUST 只判存在性：实体仍在但版本或确认态漂移 MUST NOT 判为悬空。

**工作项组被删除时，其自动化登记 SHALL 被标成【已作废】**（`voided_at`/`void_reason`），SHALL NOT 物理删除该登记（留痕与历史保留见 REQ-WIGA-VOID-01）。**已作废的登记视为无约束力的历史记录** ⇒ 其重新开启 SHALL NOT 再受 options/target 逐字段相等约束。**未作废的登记，其悬空态 SHALL 仍只放行 source 漂移。**

「读取失败按未悬空处理」这条口径 MUST **只约束 enrollment 重新授权判定**，MUST NOT 被当作通用规则用于巡检或销账的证据判定。

#### Scenario: enrollment 不判未归类

- **WHEN** 巡检遇到一个 source 指向已删 story_spec 的自动化 enrollment 记录
- **THEN** 该记录 MUST 被识别为已知的、在途覆盖的状态，MUST NOT 归入未归类节

#### Scenario: enrollment 不被销账

- **WHEN** 销账在状态根下执行
- **THEN** 自动化 enrollment 记录 MUST 保持不变

#### Scenario: 不重裁重新授权语义

- **WHEN** 系统判定 enrollment 的 source 已悬空
- **THEN** 系统 MUST 只记录该事实，MUST NOT 决定如何重新授权，MUST NOT 改变既有精确校验的三条判定

#### Scenario: 悬空只判存在性

- **WHEN** enrollment 的 source 引用的 story 与 design 实体都仍存在，但版本或确认态已漂移
- **THEN** 系统 MUST NOT 判为悬空，MUST 维持既有的逐字段相等拒绝语义

#### Scenario: 重新授权判定的读取失败按未悬空处理

- **WHEN** 判定 enrollment 的 source 是否悬空时读取目标记录失败
- **THEN** 系统 MUST 按未悬空处理，MUST NOT 据此放宽任何拒绝语义

#### Scenario: 读取失败口径不得外推到巡检

- **WHEN** 巡检判定某条旁支记录的目标是否存在时读取失败
- **THEN** 系统 MUST NOT 把该条按未悬空处理，MUST 归入不可判定

#### Scenario: 只有 source 可被放行

- **WHEN** 重新开启时终止记录的 source 已悬空，但 options 或 target 与本次提交不同
- **THEN** 系统 MUST 拒绝该次重新开启，MUST NOT 因 source 悬空而放行 options 或 target 的漂移
- **AND** ★例外：该登记已【作废】时，按 REQ-WIGA-VOID-01 的场景处理★

#### Scenario: 已作废的登记放行全部漂移

- **WHEN** 重新开启时该自动化登记已作废（工作项组被删除所致），而 options / source / target 与历史记录不同
- **THEN** 系统 SHALL 接受该次重新开启并完成换代，MUST NOT 因逐字段相等而拒绝
