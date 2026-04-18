# Cadence-Aria 一期收敛方案设计

> **版本**：v1.2
> **日期**：2026-04-18
> **定位**：在 [`cadence/designs/2026-04-16_方案设计_Cadence-Aria_v1.4.md`](../designs/2026-04-16_方案设计_Cadence-Aria_v1.4.md) 基础上，对一期实现范围、能力边界与落地顺序做收敛设计。

## 1. 设计目标

本设计文档不替代主方案，而是回答三个一期实现必须先定清楚的问题：

1. 一期到底要交付到什么边界
2. `OpenSpec`、`superpowers`、`Aria`、`Claude`、`Codex` 在全流程中如何协作
3. 哪些能力必须强约束保证，哪些内容可以留到后续迭代

本次收敛结论为：

- 一期入口只做 `aria-native`
- 一期保留正式流全状态
- 前段允许人工或半自动推进
- 后段以 `dispatch / exec / review / test / patch` 自动化闭环为重点
- `OpenSpec` 与 `superpowers` 不是阶段性外挂，而是贯穿正式流的两套基础设施

## 2. 一期范围

### 2.1 包含范围

一期必须包含以下能力：

1. 从 `aria-native` 建立正式任务
2. 跑通完整正式流状态：
   `intake -> clarification -> spec-drafting -> spec-review -> spec-approved -> planning -> plan-review -> plan-approved -> dispatched -> executing -> reviewing/testing -> patching(按需) -> verified -> done`
3. 支持前段正式工件：
   - `task intake card`
   - `spec artifact`
   - `plan artifact / plan brief`
4. 支持后段正式工件：
   - `dispatch contract`
   - `exec result`
   - `review report`
   - `test report`
   - `patch contract`
   - `patch result`
   - `verification summary`
   - `closure summary`
5. 保证 `exec / patch` 在 `OpenSpec + superpowers` 依赖就绪的前提下由 `Codex` 执行
6. 支持 `blocked / retry / cancel / status / result` 的基本闭环

### 2.2 不包含范围

一期明确不做以下内容：

1. `Vibe Kanban` 入口集成
2. 双入口或多入口
3. `merge / release / archive` 等交付后角色
4. 重型策略引擎或全量调用审计系统
5. 复杂跨多阶段自动回退
6. 大规模可配置执行模式

### 2.3 术语约定

本文中的 `native issue` 特指通过 `Aria` 原生命令入口建立的任务。

在运行时来源字段中，一期正式流实例值统一使用 `aria-native`。

## 3. 架构总览

一期采用“状态机优先，能力路由居中”的结构。

### 3.1 分层

1. **Command Layer**
   - 对应 `aria:intake / start / run / status / result / cancel / retry / doctor`
   - 负责参数解析、调用 orchestrator、输出 CLI 结果

2. **Orchestrator Layer**
   - 负责驱动正式流
   - 负责按状态决定下一步动作
   - 协调 state machine、capability router、scheduler、arbitrator、persistence

3. **State Machine + Guards**
   - 作为唯一状态推进真源
   - 定义进入条件、退出条件、阻塞条件、回退条件
   - 对关键节点执行能力来源和工件合法性校验

4. **Capability Router + Adapters**
   - 负责把阶段映射为实际执行上下文
   - Adapter 至少包括：
     - `OpenSpecAdapter`
     - `SuperpowersAdapter`
     - `CodexAdapter`
     - `HostAdapter`

5. **Runtime Artifacts**
   - 作为状态推进、结果仲裁、异常恢复的正式依据
   - 不以聊天文本作为正式运行时输入

### 3.2 核心原则

`Aria` 不是替代 `OpenSpec` 或 `superpowers` 的业务能力层，而是：

- 检查它们是否可用
- 在正确阶段把它们注入给 `Claude` 或 `Codex`
- 回收产出并写成结构化 runtime 工件
- 用 guard 决定状态能否继续推进

其中，`Codex` 消费的是由 `Aria` 注入的 `OpenSpec` / `superpowers` 上下文引用，而不是直接承担这两套基础设施的生命周期管理与正式主线维护。

## 4. 角色与职责边界

### 4.1 OpenSpec

`OpenSpec` 是正式任务主线基础设施，负责：

- 正式工件主线
- 任务边界
- 正式输入输出合法性
- spec / plan 以及后续执行所依赖的基准约束

`OpenSpec` 不是只在前段出现一次的文档工具，而是整个正式流的基础约束来源之一。

### 4.2 superpowers

`superpowers` 是方法层基础设施，负责：

- 澄清方法
- spec / plan 形成方法
- review 方法
- verification / debugging / TDD / 并行执行等方法能力

`superpowers` 不是后处理插件，而是会在前后段工作中被实际使用的方法系统。

### 4.3 Claude

`Claude` 是前段主编排入口，负责：

- 接收用户输入
- 在 `Aria` 编排下使用 `OpenSpec + superpowers`
- 推进 `clarification / spec / plan / review-confirmation`
- 把前段结果落成正式工件或正式状态

### 4.4 Codex

`Codex` 是后段直接执行者，负责：

- 执行 `exec`
- 执行 `patch`

但 `Codex` 不是裸执行器。它必须在 `Aria` 注入的 `OpenSpec + superpowers` 上下文中完成编码和修补。

### 4.5 Aria

`Aria` 是 runtime 编排层，负责：

- 状态推进
- 能力检查
- 上下文注入
- 工件收束
- 结果仲裁
- 错误处理与阻塞控制

`Aria` 不直接复制 `OpenSpec` 和 `superpowers` 的核心职责。

## 5. 正式流状态设计

### 5.1 状态划分

一期将状态划分为两段：

1. 前段人工或半自动阶段
   - `intake`
   - `clarification`
   - `spec-drafting`
   - `spec-review`
   - `spec-approved`
   - `planning`
   - `plan-review`
   - `plan-approved`

2. 后段自动化阶段
   - `dispatched`
   - `executing`
   - `reviewing/testing`
   - `patching`
   - `verified`
   - `done`
   - 以及异常态 `blocked`、`cancelled`

### 5.2 状态推进原则

1. 前段允许人工推进，但每一步都必须写回 `state.yaml`
2. 一旦 `spec` 或 `plan` 进入 approved 状态，后续阶段必须使用对应正式工件引用
3. 后段不允许绕过 state machine 直接跳步
4. `review` 与 `test` 在状态机上合并为 `reviewing/testing`，执行上允许并行
5. `patching` 为条件状态，仅在 `arbitrator` 判定存在必须修补项时进入

### 5.3 `dispatch` 与 `dispatched` 的语义约定

为避免动作名与状态名混用，一期统一采用以下约定：

- `dispatch` 表示状态推进动作，含义为：基于 approved `plan` 生成并下发 `dispatch contract`
- `dispatched` 表示正式流状态值，含义为：任务已完成派发，并具备进入 `executing` 的前置条件

因此，`dispatch` 在本文中不作为状态值使用；状态机中的对应状态统一使用 `dispatched`。

进入 `dispatched` 的条件至少包括：

1. 已存在 approved `plan`
2. 至少一个合法 `dispatch contract` 已生成
3. 所有待执行单元的 `dispatch contract` 已通过 scope 与 capability 校验
4. `state.yaml` 中的执行单元映射已完成初始化

### 5.4 `reviewing/testing` 的聚合状态定义

`reviewing/testing` 是一期正式流中的聚合状态。

在该状态内：

- `review` 与 `test` 是两类独立执行活动
- 两者可以串行执行，也可以并行执行
- 两者分别产出独立的 `review report` 与 `test report`
- 状态机不因单一报告先完成而提前离开该状态

`review` 的终态定义为：已生成合法 `review report`，且其 `verdict` 属于可判定终值。  
`test` 的终态定义为：已生成合法 `test report`，且其 `verdict` 属于可判定终值。

只有当 `review` 与 `test` 均进入终态后，任务才允许离开 `reviewing/testing`，并由 `arbitrator` 基于两类报告进行统一判定。

统一判定后的状态去向仅允许为：

1. `verified`：满足 review/test 通过条件
2. `patching`：存在必须修补项，且仍处于合法可修补范围内
3. `blocked`：结果证据不足、运行失败、状态不一致，或无法形成合法仲裁结论

## 6. 运行时真源、执行契约与恢复模型

### 6.1 运行时双真源规则

一期正式流中，运行时真源必须拆分为两类：

- `OpenSpec`：正式工件内容真源，承载 approved `spec` 与 approved `plan` 的内容本体
- `state.yaml`：运行时状态与冻结引用真源，承载当前状态、确认点、执行单元状态、活动结果集与恢复元数据

一期后段所有动作只允许消费 `state.yaml` 中显式记录的冻结引用，不允许在运行时临时读取“最新 spec / plan”作为正式输入。

### 6.2 冻结引用与结果集字段

一期运行时至少保留以下字段：

- `approved_spec_ref`
- `approved_plan_ref`
- `active_result_set_id`

并满足以下约束：

1. `spec-review -> spec-approved` 时必须写入 `approved_spec_ref`
2. `plan-review -> plan-approved` 时必须写入 `approved_plan_ref`
3. `dispatch` 只能基于 `approved_plan_ref`
4. `exec / patch / review / test / verified` 只能基于当前冻结引用与活动结果集推进
5. 一旦 `spec` 或 `plan` 重批，旧冻结引用立即失效，不得跨轮混用

### 6.3 执行契约模型

一期中，`dispatch contract` 与 `patch contract` 是 `Aria` 下发给执行端的最小正式契约。`Codex` 的正式输入以 contract 为准，自然语言 prompt 只是 contract 的呈现载体，不能替代结构化契约对象。

两类 contract 至少共享以下字段：

- `task_id`
- `unit_id`
- `contract_type`
- `based_on_spec_ref`
- `based_on_plan_ref`
- `goal_statement`
- `allowed_paths`
- `blocked_paths`
- `acceptance_checks`
- `context_bundle_ref`
- `output_schema_ref`
- `generated_at`

`patch contract` 额外必须包含：

- `based_on_result_set_id`
- `patch_reason`
- `must_fix_items`

缺少冻结引用、范围约束或验收条件的 contract 不得进入执行态。

### 6.4 `execution context bundle`

`Aria` 对 `OpenSpec + superpowers` 的能力注入，一期必须落成结构化运行时对象 `execution context bundle`，并作为 `dispatch contract` / `patch contract` 的正式组成部分。

一期最小字段如下：

- `spec_ref`
- `plan_ref`
- `scope_constraints_ref`
- `required_methods`
- `workspace_context`
- `verification_requirements`
- `prompt_template_ref`

同时满足以下约束：

1. 执行、重试、patch、审计都必须可回溯到对应 `execution context bundle`
2. prompt 只是 contract 的渲染结果，不得替代 bundle 本体
3. `Codex` 接收的上下文必须来自 `Aria` 已冻结的 bundle 引用，而不是自由拼接文本

### 6.5 仲裁结果基线

一期为 review/test 仲裁建立稳定结果基线，采用 `result_set` 机制。

`result_set` 至少包含：

- `result_set_id`
- `task_id`
- `source_unit_ids`
- `result_refs`
- `created_at`

并满足以下运行时规则：

1. 每次合法执行完成后生成新的 `result_set_id`
2. `review report` 与 `test report` 必须绑定同一个 `result_set_id`
3. `arbitrator` 只允许基于同一个 `result_set_id` 做统一判定
4. 发生 retry 或 patch 后，旧结果集不得继续参与当前轮仲裁
5. `patch` 成功后必须创建新结果集，再重新进入 `reviewing/testing`

### 6.6 用户确认事件模型

`confirmation_pending`、`confirmation_mode`、`confirmation_artifact_path` 只描述确认门禁，不替代正式事件。
一期必须引入最小 `confirmation event` 结构：

- `task_id`
- `confirmation_type`
- `artifact_ref`
- `decision`
- `actor`
- `timestamp`
- `note`

状态机约束如下：

1. `spec-review -> spec-approved` 必须由合法 `confirmation event` 触发
2. `plan-review -> plan-approved` 必须由合法 `confirmation event` 触发
3. 若 `artifact_ref` 与当前待确认工件不一致，则该事件无效
4. 自动确认与人工确认结构一致，仅 `actor` 与来源不同

### 6.7 `blocked / retry / cancel` 恢复模型

一期只保留四类阻塞：

- `capability_blocked`
- `input_blocked`
- `execution_blocked`
- `decision_blocked`

每个阻塞实例至少包含：

- `block_reason_code`
- `blocking_stage`
- `retryable`
- `required_action`

恢复规则如下：

1. `retry` 仅对 `retryable = true` 的阻塞或失败执行生效
2. `decision_blocked` 必须通过新的确认事件或补充输入恢复
3. `capability_blocked` 必须通过 `doctor` 或依赖修复恢复
4. `input_blocked` 必须通过重新生成合法工件或 contract 恢复
5. `execution_blocked` 只能在执行条件重新满足后，通过新的合法执行轮恢复

## 7. 节点-能力映射

下表定义每个节点如何使用 `OpenSpec` 与 `superpowers`。

| 节点 | `OpenSpec` 如何使用 | `superpowers` 如何使用 | 直接工作者 | `Aria` 职责 |
|---|---|---|---|---|
| `intake` | 判定正式主线入口与初始边界 | 提供 intake 分析与风险收敛方法 | Claude | 建档、初始化状态 |
| `clarification` | 约束澄清围绕正式边界展开 | 提供澄清方法，如 `brainstorming` | Claude | 组织澄清回合并沉淀结果 |
| `spec-drafting` | 承载正式 spec 主线与工件合法性 | 提供 spec 形成与 scope 控制方法 | Claude | 记录 spec 工件来源与引用 |
| `spec-review` | 作为被确认的正式 spec 工件 | 提供 spec review 方法 | Claude + 用户 | 管理确认点与退回 |
| `spec-approved` | 冻结 spec 为后续正式输入 | 承接前置评审结果 | Claude + 用户 | 将 spec 写入 runtime |
| `planning` | 承载正式 plan 主线并映射 approved spec | 提供写计划、拆单元、门禁设计方法 | Claude | 生成 plan artifact / brief |
| `plan-review` | 作为被确认的正式 plan 工件 | 提供 plan review 方法 | Claude + 用户 | 管理确认点与退回 |
| `plan-approved` | 冻结 plan 为 dispatch 基准 | 承接前置评审结果 | Claude + 用户 | 将 plan 写入 runtime |
| `dispatch` | 将 spec / plan 边界写入 contract | 将执行方法要求写入 contract | Aria | 生成 `dispatch contract` |
| `exec` | 作为实现边界、目标与验收依据 | 作为 coding 方法能力被实际使用 | Codex | 检查能力、注入上下文、记录结果 |
| `review` | 作为审查判定基准 | 提供 review 方法与结构化输出方式 | Claude 或被编排能力 | 收集 `review report` |
| `test` | 作为验证目标与门禁依据 | 提供验证、调试、测试组织方法 | Claude 或被编排能力 | 收集 `test report` |
| `patch` | 约束修补不能偏离正式边界 | 提供修复、调试、修后验证方法 | Codex | 生成 `patch contract` 并回收结果 |
| `verified` | 判定是否满足正式主线闭环要求 | 提供 verification 方法 | Aria | 汇总结果并执行 guard |
| `done` | 标记正式主线闭环完成 | 前序方法保证已兑现 | Aria | 输出 `closure summary` |

## 8. 运行时工件与最小 Schema

状态推进语义、角色边界与 Guard 约束以本方案为准；字段级 schema、枚举值和值域定义以 Runtime Schemas 配套文档为准。

### 7.1 一期最小工件集合

一期至少保留以下工件：

1. `state.yaml`
2. `task intake card`
3. `spec artifact`
4. `plan artifact / plan brief`
5. `dispatch contract`
6. `exec result`
7. `review report`
8. `test report`
9. `patch contract`
10. `patch result`
11. `verification summary`
12. `closure summary`

### 7.2 统一来源字段

关键工件统一保留以下来源字段：

- `producer`
- `source_capabilities`
- `artifact_refs`
- `generated_at`

执行类结果统一保留以下证明字段：

- `capabilities_used`
- `openspec_refs_consumed`
- `superpowers_refs_consumed`
- `degraded`
- `degradation_reason`

### 7.3 关键工件最小要求

#### `state.yaml`

至少包含：

- `task_id`
- `source`
- `flow_type`
- `risk_level`
- `status`
- `current_round`
- `approved_spec_ref`
- `approved_plan_ref`
- `active_result_set_id`
- `confirmation_pending`
- `confirmation_mode`
- `confirmation_artifact_path`
- `review_status`
- `test_status`
- `patch_required_by`
- `patch_round`
- `active_exec_units`
- `exec_units`
- `patch_units`
- `artifacts`
- `capability_status`
- `block_reason_code`
- `blocking_stage`
- `retryable`
- `required_action`
- `created_at`
- `updated_at`

#### `dispatch contract`

至少包含：

- `task_id`
- `unit_id`
- `contract_type`
- `based_on_spec_ref`
- `based_on_plan_ref`
- `goal_statement`
- `allowed_paths`
- `blocked_paths`
- `acceptance_checks`
- `context_bundle_ref`
- `output_schema_ref`
- `generated_at`

#### `exec result`

至少包含：

- `task_id`
- `exec_unit_id`
- `status`
- `changed_files`
- `summary`
- `capabilities_used`
- `openspec_refs_consumed`
- `superpowers_refs_consumed`
- `degraded`
- `degradation_reason`
- `started_at`
- `finished_at`

#### `review report`

至少包含：

- `task_id`
- `result_set_id`
- `exec_units_reviewed`
- `baseline_refs`
- `method_refs`
- `blockers`
- `suggestions`
- `verdict`
- `producer`
- `source_capabilities`
- `generated_at`

#### `test report`

至少包含：

- `task_id`
- `result_set_id`
- `exec_units_tested`
- `baseline_refs`
- `method_refs`
- `commands_run`
- `failures`
- `passed_count`
- `failed_count`
- `verdict`
- `producer`
- `source_capabilities`
- `generated_at`

#### `patch contract`

至少包含：

- `task_id`
- `unit_id`
- `contract_type`
- `based_on_spec_ref`
- `based_on_plan_ref`
- `based_on_result_set_id`
- `patch_reason`
- `must_fix_items`
- `context_bundle_ref`
- `output_schema_ref`
- `generated_at`

#### `execution context bundle`

至少包含：

- `spec_ref`
- `plan_ref`
- `scope_constraints_ref`
- `required_methods`
- `workspace_context`
- `verification_requirements`
- `prompt_template_ref`

#### `confirmation event`

至少包含：

- `task_id`
- `confirmation_type`
- `artifact_ref`
- `decision`
- `actor`
- `timestamp`
- `note`

## 9. 强约束与 Guard

### 9.1 整体保证策略

一期采用：

- **整体接口级保证**
- **关键主线节点运行时级保证**

### 9.2 关键 Guard

1. **Spec Guard**
   - 无合法 `spec artifact` 不能进入 `spec-review`
   - 无 `OpenSpec` 来源证明不能进入 `spec-approved`

2. **Plan Guard**
   - 无合法 `plan artifact` 不能进入 `plan-review`
   - 无 `OpenSpec` 来源证明不能进入 `plan-approved`

3. **Exec / Patch Guard**
   - 没有 `OpenSpec + superpowers` 不允许启动 `exec`
   - 没有 `OpenSpec + superpowers` 不允许启动 `patch`
   - 缺少能力消费记录视为证据不足

4. **Review / Test Guard**
   - 没有合法 `review report` / `test report` 不能离开 `reviewing/testing`
   - 缺少 `baseline_refs` 或 `method_refs` 视为未完成
   - `review report` 与 `test report` 不绑定同一 `result_set_id` 时，不得进入统一仲裁
   - 仅当 `review` 与 `test` 均进入终态后，才允许触发统一仲裁
   - 单一报告先完成不构成离开 `reviewing/testing` 的条件

5. **Confirmation Guard**
   - 无合法 `confirmation event` 不能进入 `spec-approved` 或 `plan-approved`
   - `artifact_ref` 与当前待确认工件不一致的确认事件必须视为无效

6. **Verification Guard**
   - 未满足 review/test 通过条件，不能进入 `verified`

## 10. 错误处理、降级与恢复

### 10.1 错误分类

一期将错误分为：

1. 能力不可用
2. 工件缺失或不合法
3. 执行失败
4. 结果不通过
5. 状态损坏或不一致

### 10.2 降级策略

允许的有限降级只有：

1. 前段人工推进
   - `clarification`
   - `spec-review`
   - `plan-review`

2. 方法辅助降级的定义预留在 schema 中，但一期默认不开放自动降级执行

不允许的降级包括：

1. 脱离 `OpenSpec` 形成正式 `spec`
2. 脱离 `OpenSpec` 形成正式 `plan`
3. 在 `OpenSpec` 或 `superpowers` 缺失时启动 `exec`
4. 在 `OpenSpec` 或 `superpowers` 缺失时启动 `patch`
5. 缺少正式基准或方法依据时，把 `review/test` 视为已完成

### 10.3 `blocked` / `retry` / `patching` 边界

一期对三者的语义约定如下：

- `patching`：业务结果不通过，但任务输入、能力与状态仍合法，因此进入修补闭环
- `retry`：对 `exec unit` 或 `patch unit` 的重试动作，不作为正式流顶层状态
- `blocked`：任务无法继续推进，且不能通过当前自动流程恢复的正式状态

统一分流原则如下：

1. 业务结果不通过，但存在合法 `review report` / `test report` 或合法仲裁结果证明存在 must-fix 项：
   - 进入 `patching`

2. 执行单元运行失败、超时或取消，且满足重试条件：
   - 不直接改变任务主状态终点
   - 由 orchestrator 对对应 `exec unit` / `patch unit` 执行 `retry`

3. 出现以下情况之一：
   - 关键能力缺失
   - 关键工件缺失或非法
   - guard 不满足
   - runtime 状态损坏
   - 外部依赖不可恢复失败
   - 无法形成合法仲裁结论
   - 进入 `blocked`

补充约束：

- 纯执行失败不能直接进入 `patching`
- 只有在正式基准、方法依据与问题项证据齐备时，才允许生成 `patch contract`
- `retry` 解决的是执行动作重试问题，不替代正式流状态仲裁

### 10.4 review/test 流程归属

一期中 `review/test` 的流程归属固定如下：

1. `Aria` 是 `review/test` 的流程发起者与结果归档者
2. `Claude` 是默认分析执行者
3. 可自动执行的 superpowers 能力由 `Aria` 调用

## 11. 一期实现顺序与范围控制

一期按三层交付顺序推进：

### 11.1 Layer 1

单任务、单执行单元、串行正式闭环。

最小目标：

- 打通 `intake -> ... -> dispatch -> exec -> review/test -> verified | patching | blocked`
- 仅支持单 `exec unit`
- 不支持并行调度
- 具备 `approved refs`、`dispatch contract`、`exec result`、`review report`、`test report`、`verification summary`

### 11.2 Layer 2

在 Layer 1 稳定后补齐：

- `patch contract`
- `patch result`
- `result_set_id` 切换
- `blocked` 分类
- `retry` 恢复规则
- `doctor` 能力诊断

### 11.3 Layer 3

最后引入：

- 多 `exec units`
- worktree 分配
- 并行上限控制
- 依赖解析
- 多单元结果汇总与统一仲裁

## 12. 一期落地顺序

建议按 4 个切片实现：

### 切片 1：状态机与工件骨架

产出：

- state machine
- guards
- artifact schemas
- persistence 基础能力

### 切片 2：前段正式主线

产出：

- intake card
- spec artifact
- plan artifact / brief
- approval transitions

### 切片 3：后段执行闭环

产出：

- capability check
- context injection
- codex execution path
- arbitrator
- patch loop

### 切片 4：异常恢复与 CLI 投影

产出：

- `status / result / cancel / retry / doctor`
- blocked / retry flows
- 错误码基础域

## 13. 一期验收口径

一期完成时至少满足以下条件：

1. 可从 `aria-native` 建立正式任务
2. 可形成并确认正式 `spec` 与 `plan`
3. 可生成带 `OpenSpec + superpowers` 依赖的 `dispatch contract`
4. `Codex` 可在该上下文中完成 `exec`
5. `review` / `test` 可形成结构化报告
6. 出现 must-fix 时能按 `exec unit` 生成 `patch contract` 并回环
7. 缺关键能力时进入 `blocked`，而不是静默降级
8. 最终可产出 `verification summary` 与 `closure summary`

## 14. 结论

一期推荐实现形态为：

**全状态正式流 + 前段人工承接 + 后段强约束自动闭环**

在这个版本中：

- `OpenSpec` 是正式主线基础设施
- `superpowers` 是方法层基础设施
- `Claude` 在前段使用两者
- `Codex` 在后段使用两者
- `Aria` 负责保证它们在正确阶段被检查、注入、记录、校验并收束

这是一版足以进入 implementation plan 的收敛设计，同时保持与主方案一致，不把一期做成重型泛化平台。
