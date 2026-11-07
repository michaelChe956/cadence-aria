# logical-codebase-issue-entry Specification

## Purpose
为逻辑代码库（LC）提供**多仓 Issue 创建入口**：Issue 可绑定一组逻辑成员（involved 复选 + explicit / all_members 双路径），并由平台校验选择子集与焦点仓的一致性；单仓项目沿用既有入口，行为零回归。

## Requirements

### Requirement: 多仓 Issue 创建入口(REQ-MRE-01)

前端建 Lifecycle Issue 对话框 SHALL 要求先选定**一个逻辑代码库**，再在该 LC 上下文提供成员多选（复选）；提交 SHALL 将勾选集写入 `selection.focus_repository_ids`（复数字段），且勾选集 SHALL ⊆ LC active 成员。系统 MUST NOT 提供「单仓库代码库」选项，MUST NOT 接受 `logical_codebase_id: null` 的创建请求；缺 LC 归属的提交 SHALL 被稳定错误码拒绝。勾选恰 1 个成员时行为与现单选等价。

#### Scenario: 必须选定 LC 才能创建 issue

- **WHEN** 用户打开建 Lifecycle Issue 对话框
- **THEN** 对话框 SHALL 要求选择一个逻辑代码库；MUST NOT 出现「单仓库代码库」选项，MUST NOT 提供不带 LC 的提交路径

#### Scenario: 缺 LC 归属的提交被拒

- **WHEN** 提交请求不带逻辑代码库归属
- **THEN** 系统 SHALL 返回稳定错误码（`issue_not_bound_to_logical_codebase` 语义）并拒绝创建；MUST NOT 落库为「无归属 issue」

#### Scenario: 多选成员创建多仓 issue

- **WHEN** 用户在 LC 下勾选 4 个成员并提交
- **THEN** 创建的 issue `selection.focus_repository_ids` 含 4 个成员 id，durable 校验（focus ⊆ include）通过

#### Scenario: 单成员勾选等价单选

- **WHEN** LC 下仅勾选 1 个成员提交
- **THEN** 提交路径与 N 成员提交**完全相同**，`selection.focus_repository_ids` 恰含 1 个成员 id

#### Scenario: 勾选集即授权上界

- **WHEN** 后续 design/plan 链路任何环节产生涉及界外仓的意图
- **THEN** 该意图被钉定纪律或 preflight 拒绝，不得进入 coding
