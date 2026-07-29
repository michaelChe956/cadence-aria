# testing-stage-removal Specification

## Purpose
TBD - created by archiving change remove-testing-stage. Update Purpose after archive.
## Requirements
### Requirement: Coding Workspace 不含 Testing 阶段与 Tester 角色

Coding Workspace 的执行阶段集合 MUST NOT 包含 Testing 阶段，角色集合 MUST NOT 包含 Tester 角色。系统 MUST NOT 存在把 attempt 置入 Testing 阶段的路径。

#### Scenario: 阶段序列不含 Testing

- **WHEN** 一个 coding attempt 从创建执行到完成
- **THEN** 其经历的阶段 MUST 依次限于准备上下文、准备 worktree、Coding、Code Review、请求评审、最终确认，MUST NOT 出现 Testing 阶段

#### Scenario: 不存在 Tester 角色运行

- **WHEN** 一个 coding attempt 的任意阶段执行
- **THEN** MUST NOT 创建 Tester 角色的 role run，MUST NOT 为测试计划或测试报告调用 provider

#### Scenario: 阶段先后比较语义不变

- **WHEN** 比较两个阶段的先后
- **THEN** 移除后的相对顺序 MUST 与移除前一致（准备上下文 < 准备 worktree < Coding < Code Review < 请求评审 < 最终确认）

### Requirement: 测试计划与测试报告产物不再存在

系统 MUST NOT 生成、存储、读取或经接口暴露测试计划与测试报告产物。

#### Scenario: attempt 目录不产生测试产物

- **WHEN** 一个 coding attempt 执行完成
- **THEN** attempt 目录下 MUST NOT 出现测试计划目录与测试报告目录

#### Scenario: 会话状态不暴露测试报告

- **WHEN** 客户端订阅 coding workspace 会话状态
- **THEN** 返回的状态 MUST NOT 含测试报告字段

#### Scenario: 报告视图不含测试分区

- **WHEN** 用户查看某 attempt 的报告
- **THEN** 视图 MUST NOT 展示测试计划或测试报告分区

### Requirement: 恢复路径不指向已移除阶段

任何恢复、重试或门禁动作 MUST NOT 把 attempt 置入不存在编排入口的阶段。

#### Scenario: 计划修订的重校验恢复到代码审查

- **WHEN** 某 work item 的计划修订以"实现未变、需重新验证"的模式恢复
- **THEN** attempt 的阶段 MUST 恰为 Code Review

注：验收判据必须是阶段字面值。「能继续推进、不停机」在本变更前已成立（阶段按序号比较分派，Testing 会被直接跳过），断言它不构成回归覆盖。

#### Scenario: 恢复后阶段语义与实际执行一致

- **WHEN** attempt 完成恢复并继续执行
- **THEN** 其记录的阶段 MUST NOT 包含任何未实际执行的阶段

#### Scenario: 不提供测试相关门禁动作

- **WHEN** 任意门禁向用户呈现可选动作
- **THEN** 动作集合 MUST NOT 包含重试测试计划、重跑缺失步骤、重跑测试、接受测试结果

#### Scenario: 代码审查仍有重试入口

- **WHEN** Code Review 阶段落地阻塞门禁
- **THEN** 用户 MUST 仍能通过既有代码审查重试与分诊动作继续或终止流程

### Requirement: 测试证据不作为评审或完成依据

评审与完成判定 MUST NOT 读取或要求测试计划、测试报告及其派生字段。

#### Scenario: group final review 不因测试证据缺失判要求修改

- **WHEN** 整组 unit 的产品代码与契约一致，且不存在任何测试报告
- **THEN** group final review MUST NOT 因测试证据缺失、测试清单为空或测试结论缺失而给出要求修改或阻塞结论

#### Scenario: 评审上下文不含测试派生字段

- **WHEN** 为 reviewer 构建评估上下文
- **THEN** 上下文 MUST NOT 含测试执行清单与测试结论字段，提示词 MUST NOT 要求 reviewer 依据测试证据判断

#### Scenario: 完成判定不要求测试报告

- **WHEN** 某 attempt 的 Coding 与 Code Review 均通过且其他非测试门禁满足
- **THEN** attempt MUST 能进入完成状态，MUST NOT 因缺少测试报告被阻塞

### Requirement: 通用测试命令规划能力保留

移除范围 MUST NOT 触及从 work item 材料规划测试命令的能力：该能力被 Testing 阶段之外的路径消费。

#### Scenario: work item 上下文仍可规划测试命令

- **WHEN** 构建 work item 上下文需要从材料中得出计划内测试命令
- **THEN** 该能力 MUST 仍然可用，行为与移除前一致

#### Scenario: 失去消费者的执行能力不得保留为死代码

- **WHEN** Testing 阶段移除后，测试执行模块中某公开函数已无生产消费者
- **THEN** 该函数 MUST 被移除，MUST NOT 保留为死代码

### Requirement: 不保留 Testing 恢复兼容

系统 MUST NOT 保留用于未来恢复 Testing 的开关、占位枚举成员或不可达代码，也 MUST NOT 为含 Testing 阶段值或测试报告的历史持久化记录提供兼容层。

#### Scenario: 无残留占位

- **WHEN** 审查移除后的代码
- **THEN** MUST NOT 存在仅为兼容 Testing 而保留的枚举成员、配置开关或不可达分支

#### Scenario: 失去全部生产触发点的枚举成员不得保留

- **WHEN** 某枚举成员在 Testing 移除后失去全部生产产生点
- **THEN** 该成员 MUST 被移除

### Requirement: 非测试功能不得因移除而失去数据源

依赖测试报告存储但服务于非测试目的的功能 MUST 显式处置，MUST NOT 保留恒空而语义仍在的字段。

#### Scenario: 质量绕过审计的数据源

- **WHEN** 测试报告存储移除后，质量绕过审计原先从中读取的字段失去来源
- **THEN** 该字段 MUST 被移除或改为明确的空语义，MUST NOT 保留为语义仍在但恒空的字段

### Requirement: 保留节点的执行链契约变化必须显式

移除测试报告产物类型与其协议节点 MUST NOT 静默改变保留节点的必需产物集合与失败路由。

#### Scenario: 保留节点的必需产物集合

- **WHEN** 移除测试报告产物类型后校验保留节点的产物契约
- **THEN** 各保留节点的必需产物集合变化 MUST 被显式确认并有测试覆盖

#### Scenario: 保留节点的失败路由

- **WHEN** 移除测试报告对应的协议节点后校验执行链路由
- **THEN** 指向或来自该节点的失败路由 MUST 被显式重写，MUST NOT 留下指向已移除节点的路由

