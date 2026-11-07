## ADDED Requirements

### Requirement: 已收口的人工门卡不得再以行动号召形态呈现（REQ-WSC-26）

人工门卡一旦已收口（`resolved`），SHALL 以只读陈述形态呈现：陈述式标题、不渲染人话选项块与作答指引、决定收口时不渲染「为什么找你」meta 行，并在独立的结果徽章旁给出一句来源与后果说明。本 requirement 只约束已收口门卡；未收口的活门卡与 REQ-WSC-11 逐字不变。

#### Scenario: 已终止的 triage 门卡不再呈现行动号召

- **WHEN** 一张携带人话裁决（`owner_question` + ≥2 个 `owner_options`）的 triage 门卡 `resolved=true`、`resolution="terminate"`
- **THEN** 标题 SHALL 逐字为 `本轮分诊已结束`，MUST NOT 出现 `需要判断 reviewer 意图`
- **AND** MUST NOT 渲染 `owner-guidance` 块、`owner-option` 条目与「选一个，把选项名原文作为补充指令发回去即可」
- **AND** MUST NOT 渲染 `gate-meta` 行
- **AND** 「已终止」徽章 SHALL 是独立元素、文案恰为 `已终止`（`getByText("已终止")` 精确匹配只命中它）
- **AND** SHALL 另渲染 `data-testid="gate-resolution-note"`，文案逐字为 `这一轮已终止，之前的选择未生效，无需再操作`

#### Scenario: 有被中断修订证据的已终止卡明说选择未生效

- **WHEN** 门卡 `resolved=true`、`resolution="terminate"`，且时间线存在 `status=aborted`、`summary` 含 `修订被中断，你的选择未生效` 的节点
- **THEN** `gate-resolution-note` SHALL 逐字为 `这一轮修订被中断，你的选择未生效，无需再操作`
- **AND** 无该证据时 MUST NOT 宣称「修订被中断」

#### Scenario: 其余收口标题与说明逐字固定

- **WHEN** 一张门卡 `resolved=true`
- **THEN** 标题 SHALL 逐字为：triage 门 `本轮分诊已结束`；批次确认门 `本轮发布确认已结束`；其余 `本轮人工确认已结束`
- **AND** MUST NOT 使用命令式标题 `需要判断 reviewer 意图` / `需要人工确认` / `确认发布整组 Work Items`
- **AND** `resolution="confirm"` 的说明 SHALL 为 `已按当前版本确认，这一轮无需再操作`；`resolution="request-change"` 的说明 SHALL 为 `已要求修改，这一轮已结束，无需再操作`
- **AND** 「已确认」/「已要求修改」徽章 SHALL 保持独立元素且文案逐字不变

#### Scenario: 既有已关元素保持关闭

- **WHEN** 一张门卡 `resolved=true`
- **THEN** 需人工 chip、下一步指引、why 行、findings 折叠、距通过清单、跨轮 delta、动作区 SHALL 继续不渲染

#### Scenario: 留档卡保留历史 meta 行但同样去行动化

- **WHEN** 一张门卡 `resolved=true`、`resolution="superseded"`，携带人话裁决
- **THEN** 标题 SHALL 为陈述式，MUST NOT 渲染人话选项块与作答指引
- **AND** SHALL 保留既有 `gate-archive-note`、「已留档」徽章与 `gate-meta`（含「预算历史不可用」）

#### Scenario: 活门卡不受影响

- **WHEN** 一张门卡 `resolved` 不为 true，携带人话裁决
- **THEN** SHALL 照旧渲染命令式标题与人话选项块、作答指引（REQ-WSC-11 不变）

### Requirement: 人工门卡必须表达修订与复评的在途态且刷新重连后不得退化为全新可操作卡（REQ-WSC-27）

当前门在 single_candidate 流 `human_confirm` 阶段存在在途 provider run 时，门卡 SHALL 表达在途态并 MUST NOT 提供会被后端判 Busy 的提交通路。在途态由服务端权威事实派生：`active_run_id` 非空且活动节点 `status=active`，节点类型 `author_run` 为修订中，`reviewer_run` 为复评中，其余类型不判在途。

#### Scenario: 刷新后修订中的门卡不是全新可操作卡

- **WHEN** 刷新或 WS 重连后收到 `session_state`：`stage=human_confirm`、`flow_kind=single_candidate`、`active_run_id` 非空、活动节点 `author_run` 且 `active`，store 的 `humanGateTurn` 为 null，当前门卡未收口
- **THEN** 门卡 SHALL 渲染 `gate-revision-status`，文案逐字为 `已提交，正在按反馈修订`
- **AND** MUST NOT 渲染反馈编辑器（`gate-feedback-editor`）与「确认当前版本」按钮
- **AND** SHALL 仍渲染「终止此门」
- **AND** 顶部指引 SHALL 逐字为 `引擎正在按你的反馈修订，请稍候；完成后会自动复评`，不指向「确认当前版本」

#### Scenario: 复评阶段文案区别于修订阶段

- **WHEN** 同上，但活动节点为 `reviewer_run` 且 `active`
- **THEN** 状态行 SHALL 逐字为 `修订完成，正在复评`，MUST NOT 为 `已提交，正在按反馈修订`
- **AND** 顶部指引 SHALL 逐字为 `引擎正在复评修订后的版本，请稍候；结论出来后再决定`
- **AND** 反馈编辑器与确认按钮 MUST NOT 渲染

#### Scenario: run 结束后随节点终态自动解锁

- **WHEN** 在途 run 结束，活动节点经 `timeline_node_updated` 变为终态（`completed` / `aborted` / `failed`）
- **THEN** 在途态 SHALL 为空，门卡 SHALL 按既有规则恢复，MUST NOT 因陈旧的 `active_run_id` 永久锁住

#### Scenario: 已收口卡不被在途态重新激活

- **WHEN** 会话已 `terminated`（门收口，`resolution=terminate`），但 store 里 `active_run_id` 与 `active` 的 `author_run` 节点仍陈旧存在
- **THEN** 该卡 SHALL 仍为只读陈述形态（REQ-WSC-26），MUST NOT 渲染在途状态行、反馈编辑器或确认按钮

#### Scenario: live-turn 路径与不判在途的情形不变

- **WHEN** store 存在与本卡 `turn_id` 匹配的 open / busy / awaiting_confirm `humanGateTurn`，或活动节点缺失、非 `active`、类型不是 `author_run` / `reviewer_run`，或 `active_run_id` 为空
- **THEN** 在途态选择器 SHALL 为空，门卡 SHALL 按既有路径渲染，行为不变
- **AND** 已知边界：`active_run_id` 为进程内字段，服务进程重启后为空，重启丢失的在途 turn 由既有恢复链收口，不属于本 requirement
