## MODIFIED Requirements

### Requirement: 多仓 Issue 创建入口(REQ-MRE-01)

前端建 Lifecycle Issue 对话框 SHALL 要求先选定**一个逻辑代码库**，再在该 LC 上下文提供成员多选（复选）；提交 SHALL 将勾选集写入 `selection.focus_repository_ids`（复数字段），且勾选集 SHALL ⊆ LC active 成员。系统 MUST NOT 提供「单仓库代码库」选项，MUST NOT 接受 `logical_codebase_id: null` 的创建请求；缺 LC 归属的提交 SHALL 被稳定错误码拒绝。勾选恰 1 个成员时行为与现单选等价。

成员列表 SHALL 附带同一项目与 LC 当前验收 active 世代确认的无源码提示（同一成员及主 checkout），来源为 `REQ-BOOT-10` 的非阻断告警文案。选仓对话框 SHALL 在相应成员行标注「该仓无源码，只能改配置/文档类文件」。这条提示 MUST NOT 改变原复选/提交规则或权限，MUST NOT 将索引覆盖或告警存在性作为创建/目标仓校验门槛。成员可在既有工作区、checkout 和执行规则下成为配置/文档变更目标，无需 CodeGraph 检索该目标，且不绕过 LC 规划的可读索引前置。

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

#### Scenario: 无源码成员行有提示且可单独选中

- **WHEN** 用户选择有可用聚合索引的 LC，该 LC 有已确认跳过且满足原有创建条件的 active 配置成员
- **THEN** SHALL 显示同源提示并允许勾选、作为目标仓提交；创建仍校验 active 与选择子集，planning 保留该目标的有效 checkout，provider 在正常目标工作区直接处理配置/文档

#### Scenario: 成员提示缺失或切换 LC 不影响既有选择

- **WHEN** 当前 LC 无适用告警或用户切换至其他 LC
- **THEN** 不显示上一个 LC、历史世代或不匹配 checkout 的提示；没有提示的 active 成员仍按原有资格列出、选择和提交
