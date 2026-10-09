# Delta: logical-codebase-issue-entry

## ADDED Requirements

### Requirement: 多仓 Issue 创建入口(REQ-MRE-01)

前端建 Lifecycle Issue 对话框在 LC 上下文 SHALL 提供成员多选(复选);提交 SHALL 将勾选集写入 `selection.focus_repository_ids`(复数字段),且勾选集 SHALL ⊆ LC active 成员。单仓仓库路径(非 LC)行为不变;LC 路径勾选恰 1 个成员时行为与现单选等价。

#### Scenario: 多选成员创建多仓 issue

- **WHEN** 用户在 LC 下勾选 4 个成员并提交
- **THEN** 创建的 issue selection.focus_repository_ids 含 4 个成员 id,durable 校验(focus⊆include)通过

#### Scenario: 勾选集即授权上界

- **WHEN** 后续 design/plan 链路任何环节产生涉及界外仓的意图
- **THEN** 该意图被钉定纪律或 preflight 拒绝,不得进入 coding

### Requirement: 单仓零回归(REQ-MRE-02)

既有单仓 issue 创建与单仓 plan 链路 SHALL 保持行为零变化(含 preflight 恰一仓的等价语义在 focus=单成员时成立)。

#### Scenario: 单成员勾选等价单选

- **WHEN** LC 下仅勾选 1 个成员提交
- **THEN** 后续 story/design/plan 流与改动前完全一致
