## ADDED Requirements

### Requirement: 预检候选自动发现

登记预检必须支持以聚合根自动发现候选：当请求携带 auto_discover=true 时，服务端以聚合根下直接子目录中的 git 仓库为候选执行预检分类，无需调用方逐一提供候选路径；auto_discover 缺省或 false 时行为不变。

#### Scenario: 自动发现候选
- **WHEN** 调用预检端点携带 aggregate_root 与 auto_discover=true
- **THEN** 服务端扫描聚合根直接子目录，对其中发现的 git 仓库执行预检分类并返回带 class 的候选列表（快照照常冻结持久化）

#### Scenario: 默认行为兼容
- **WHEN** 调用预检端点未携带 auto_discover（或 false）
- **THEN** 服务端按 candidate_paths 显式候选执行预检，行为与既有实现一致

### Requirement: 登记向导候选自动发现交互

登记向导在用户填入聚合根后必须自动拉取候选并以勾选列表展示，不要求用户手工输入候选路径。

#### Scenario: 向导自动展示候选
- **WHEN** 用户在向导中填入聚合根并进入预检步骤
- **THEN** 前端以 auto_discover=true 请求预检，展示候选列表与分类徽标；eligible 候选默认勾选，needs_attention 候选需用户显式勾选（视为确认），其余分类默认不勾选

#### Scenario: 手填兜底
- **WHEN** 自动发现结果为空或用户选择手工模式
- **THEN** 向导仍允许手工输入候选路径列表（既有交互保留）
