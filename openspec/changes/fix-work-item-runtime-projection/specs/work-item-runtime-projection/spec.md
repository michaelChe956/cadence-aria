## ADDED Requirements

### Requirement: 初始编译生成运行期 Work Item 投影
系统 SHALL 在初始 Final Compile 成功发布每个 Work Item 的正式 revision 后、创建对应子 Workspace 前，为每个逻辑 Work Item 生成或确认一个来源可追溯的运行期 Work Item 记录。正式 revision SHALL 保持 Work Item 内容与版本的唯一权威。

#### Scenario: 新 Group 的编译产生可执行投影
- **WHEN** Work Item Group 的全部 Draft 已通过校验并执行 Initial Final Compile
- **THEN** 系统为每个已编译逻辑 Work Item 建立与正式 revision 一致的运行期记录，并使该记录可由既有 Workspace 与 Coding 链路读取

#### Scenario: 同一编译重试复用一致投影
- **WHEN** 同一 Initial Final Compile 在已创建运行期记录后重试
- **THEN** 系统仅复用来源与该编译一致的记录，且不得创建重复记录或覆盖来源不一致的记录

#### Scenario: 已存在记录的来源不一致
- **WHEN** Initial Final Compile 发现同一逻辑 Work Item 已存在来源不一致的运行期记录
- **THEN** 系统 SHALL 失败关闭并返回可诊断错误，且不得继续创建子 Workspace 或报告确认成功

### Requirement: Plan 确认以子 Workspace 就绪为完成条件
系统 SHALL 仅在所有已编译 Work Item 的运行期投影、子 Workspace 和启动上下文均准备成功后，才将 Work Item Plan 报告为已确认。

#### Scenario: 全部子 Workspace 初始化成功
- **WHEN** 初始 Final Compile 为所有逻辑 Work Item 成功生成运行期投影并初始化子 Workspace 上下文
- **THEN** 系统将 Plan 标记为已确认，并返回包含全部子 Workspace 的成功确认结果

#### Scenario: 一个子 Workspace 上下文初始化失败
- **WHEN** 任一 Work Item 子 Workspace 的启动上下文无法初始化
- **THEN** 系统 SHALL 返回该初始化错误，且不得将该 Plan 对外报告为确认成功

### Requirement: 历史已确认 Group 不自动回填
系统 SHALL 只将运行期投影机制应用于本变更发布后的新 Initial Final Compile，不得自动修改已确认 Work Item Group 的运行数据或状态。

#### Scenario: 读取历史已确认 Group
- **WHEN** 系统启动或读取本变更发布前已确认的 Work Item Group
- **THEN** 系统不得因本变更自动创建、修改或删除该 Group 的运行期 Work Item 记录

### Requirement: 其他产物 Workspace 保持兼容
系统 SHALL 保持 Story Spec 和 Design Spec Workspace 的既有上下文初始化行为，不得要求它们依赖 Work Item 运行期投影。

#### Scenario: 初始化 Story 或 Design Workspace
- **WHEN** 系统初始化 Story Spec 或 Design Spec Workspace 的启动上下文
- **THEN** 系统继续使用各自产物的既有数据路径完成初始化，且不读取或创建 Work Item 运行期投影
