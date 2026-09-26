## MODIFIED Requirements

### Requirement: 多仓库模式 opt-in

创建 project 时必须可选择启用多仓库模式；创建后不可切换。模式通过创建弹窗内的单选表达：选择多仓库即启用，选择单仓库即原流程；不存在独立的勾选确认环节。

#### Scenario: 创建多仓 project
- **WHEN** 创建 project 请求携带 multi_repo=true
- **THEN** project 持久化 multi_repo 标志；仓库存储层按项目启用逻辑代码库 feature；多仓链路端点可用

#### Scenario: 默认单仓不变
- **WHEN** 创建 project 不携带 multi_repo（或 false），或读取旧版本 project 数据
- **THEN** project 为单仓模式，全部既有单仓行为保持不变

#### Scenario: 弹窗模式单选
- **WHEN** 用户在创建 project 弹窗中选择「多仓库」选项
- **THEN** 提交请求携带 multi_repo=true；选择「单仓库」（默认）携带 false 或不携带
