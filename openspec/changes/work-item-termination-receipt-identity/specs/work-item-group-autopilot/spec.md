## ADDED Requirements

### Requirement: 终止回执的身份限定与换代失效（REQ-WIGA-14）

终止回执 SHALL 携带**计划身份**（`plan_id`；绑定世代 `binding_version` 若响应可得 SHALL 一并携带），SHALL NOT 仅以 `(project_id, issue_id)` 为身份。

展示终止横幅前 SHALL 校验回执身份与当前计划/会话一致；不一致 ⇒ 不展示。

enrollment **换代**（supersede）后**旧代**回执 SHALL 立即失效，新代界面 SHALL NOT 叠加它。

回执**缺失计划身份**时 ⇒ SHALL **fail-closed 不展示**。

身份源 SHALL 在**终止态仍可得**（终止会清空停点形态投影如 `planControl`）⇒ SHALL NOT 以停点形态为唯一身份源；**本计划被终止时其横幅 SHALL 仍渲染**；旧代 `outcome` SHALL NOT 叠加到新代横幅。

既有终止语义与文案不变；本 requirement 只约束不得跨计划/跨世代复用回执，SHALL NOT 削弱 REQ-WIGA-09 或推翻 DRG 的「销账 MUST NOT 删除 enrollment 记录」。

#### Scenario: 同 issue 的旧回执不得叠加到新计划

- **WHEN** 某 issue 上先有一次 work item 自动化被终止并留下终止回执，随后同 issue 新建了另一个 work item 自动化
- **THEN** 新计划的界面 SHALL NOT 渲染旧回执对应的「自动化已终止」横幅
- **AND** 新计划的终止语义 SHALL 与旧计划互不影响

#### Scenario: 回执缺计划身份时不展示

- **WHEN** 终止回执缺失 `plan_id` 或 `binding_version`
- **THEN** 界面 SHALL fail-closed 不展示终止横幅
- **AND** SHALL NOT 由此推断当前计划已终止

#### Scenario: 换代后旧回执立即失效

- **WHEN** enrollment 换代（换绑新 plan/session、清空 `terminated_at`）后，旧代终止回执仍存在于客户端存储
- **THEN** 旧代回执 SHALL 不再用于任何界面判定
- **AND** 新代界面 SHALL 按新代身份独立判定

#### Scenario: 终止语义本身不被弱化

- **WHEN** 当前计划的终止回执身份与当前界面一致
- **THEN** 终止横幅 SHALL 照既有文案渲染（含「本计划全部仓库已停止，不可继续；已产出文件保留。」与 `cleanup_required` 的管理员介入说明）

#### Scenario: 本计划被终止时横幅仍然渲染

- **WHEN** 当前计划自身被终止（其停点形态投影已被清空）
- **THEN** 该计划的终止横幅 SHALL 仍然渲染，且文案与既有语义一致

#### Scenario: 旧代 outcome 不得叠加到新代

- **WHEN** 新一代计划的界面存在，而客户端存储里留有旧一代的终止回执（含 `cleanup_required`）
- **THEN** 新一代界面 SHALL NOT 渲染旧一代回执的横幅，也 SHALL NOT 显示旧一代的 `outcome` 文案
