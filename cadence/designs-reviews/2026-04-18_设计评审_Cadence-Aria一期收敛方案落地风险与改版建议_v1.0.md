# Cadence-Aria 一期收敛方案落地风险与改版建议

> **版本**：v1.0
> **日期**：2026-04-18
> **评审对象**：`cadence/designs/2026-04-17_方案设计_Cadence-Aria一期收敛方案_v1.0.md`
> **对照文档**：
> - `cadence/designs/2026-04-16_方案设计_Cadence-Aria_v1.4.md`
> - `cadence/designs/2026-04-16_配套设计_Runtime-Schemas_Cadence-Aria_v1.0.md`
> - `cadence/designs/2026-04-16_配套设计_Implementation-Layout_Cadence-Aria_v1.0.md`
> **评审前提**：按“正式工程骨架，最小闭环”标准评估一期可落地性

## 1. 评审结论

当前一期收敛方案的方向正确，已经完成了三项关键收敛：

1. 一期主入口收敛到 `native issue`
2. 后段自动化闭环成为实现重点
3. `OpenSpec` 与 `superpowers` 被明确为正式流基础设施，而非外挂能力

但按“一期必须真实可落地”标准判断，当前文档仍缺少一层关键约束：  
需要把“角色分工、状态机、工件、能力注入、恢复机制”的概念表述，进一步钉成明确的运行时规则。

若不补这些规则，一期存在较高概率做成“概念完整、实现依赖隐式约定”的系统。

## 2. 评审范围与方法

本次评审以收敛方案为主文档，必要时对照主方案与配套设计，重点检查以下五类问题：

1. 状态是否真的可推进
2. 角色切换是否真的可执行
3. 工件是否足够支撑自动化
4. 异常路径是否形成闭环
5. 一期实现顺序是否符合最小骨架原则

结论分为三类：

- **必须改**：不改会明显阻碍一期实现，或导致运行时不稳定
- **建议改**：不改也能推进，但会提高一期成本或后续返工概率
- **可后置**：当前无需纳入一期闭环

## 3. 必须改项

### 3.1 明确运行时双真源关系

必须在收敛方案中明确：

- `OpenSpec` 是**正式工件内容真源**
- `state.yaml` 是**运行时状态与冻结引用真源**

一期后段所有动作只允许消费 `state.yaml` 中记录的冻结引用，不允许在运行时临时读取“最新 spec / plan”作为正式输入。

建议在运行时至少保留下列字段：

- `approved_spec_ref`
- `approved_plan_ref`
- `active_result_set_id`

并增加以下状态约束：

1. `spec-review -> spec-approved` 时必须写入 `approved_spec_ref`
2. `plan-review -> plan-approved` 时必须写入 `approved_plan_ref`
3. `dispatch` 只能基于 `approved_plan_ref`
4. `exec / patch / review / test / verify` 只能基于当前冻结引用与活动结果集推进
5. 一旦 `spec` 或 `plan` 重批，旧引用立即失效，不得跨轮混用

### 3.2 将“能力注入”落成结构化执行上下文

当前文档中“Aria 注入 `OpenSpec + superpowers`”仍主要停留在概念层。  
一期必须把这一动作落成明确的运行时对象，而不是依赖自由文本 prompt。

建议新增 `execution context bundle`，并明确其属于 `dispatch contract` / `patch contract` 的正式组成部分。

一期最小字段建议如下：

- `spec_ref`
- `plan_ref`
- `scope_constraints_ref`
- `required_methods`
- `workspace_context`
- `verification_requirements`
- `prompt_template_ref`

同时应明确：

1. `Aria` 的能力注入在实现上以 `context bundle` 落地
2. 执行、重试、patch、审计都必须可回溯到对应 bundle
3. prompt 只是 contract 的呈现方式，不得替代结构化注入对象

### 3.3 为 review/test 仲裁建立稳定结果基线

当前收敛方案已经把 `reviewing/testing` 定义为聚合状态，但仲裁仍缺少稳定结果边界。

建议新增 `result_set` 机制，并要求：

- `review report` 绑定 `result_set_id`
- `test report` 绑定 `result_set_id`
- `arbitrator` 只允许基于同一个 `result_set_id` 做统一判定

建议 `result_set` 至少包含：

- `result_set_id`
- `task_id`
- `source_unit_ids`
- `result_refs`
- `created_at`

并增加运行时规则：

1. 每次合法执行完成后生成新的 `result_set_id`
2. 发生 retry 或 patch 后，旧结果集不得继续参与当前轮仲裁
3. `patch` 成功后必须创建新结果集，再进入 `reviewing/testing`

### 3.4 把 dispatch/patch contract 提升为正式执行契约

一期若要实现“边界受控的执行与修补”，`dispatch contract` 与 `patch contract` 必须承担正式执行契约职责，而非仅作为文档占位。

建议在方案层明确两类 contract 的最小强制字段：

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

并明确：

1. `Codex` 的正式输入应以 contract 为准
2. 自然语言任务描述不能替代 contract
3. 缺少冻结引用、范围约束或验收条件的 contract 不得进入执行态

### 3.5 把用户确认点定义为正式事件

当前 `confirmation_pending`、`confirmation_mode`、`confirmation_artifact_path` 已说明确认点是运行时门禁，但还缺少正式事件模型。

建议新增最小 `confirmation event` 结构：

- `task_id`
- `confirmation_type`
- `artifact_ref`
- `decision`
- `actor`
- `timestamp`
- `note`

并增加状态机约束：

1. `spec-review -> spec-approved` 必须由合法 `confirmation event` 触发
2. `plan-review -> plan-approved` 必须由合法 `confirmation event` 触发
3. 若 `artifact_ref` 与当前待确认工件不一致，则事件无效
4. 自动确认与人工确认结构一致，仅 `actor` 与来源不同

### 3.6 为 blocked/retry/cancel 建立最小恢复模型

当前方案已要求一期支持 `blocked / retry / cancel / status / result` 基本闭环，但恢复语义仍不够具体。

建议一期只保留四类阻塞：

- `capability_blocked`
- `input_blocked`
- `execution_blocked`
- `decision_blocked`

每个阻塞实例至少应包含：

- `block_reason_code`
- `blocking_stage`
- `retryable`
- `required_action`

同时明确恢复规则：

1. `retry` 仅对 `retryable = true` 的阻塞或失败执行生效
2. `decision_blocked` 必须通过新的确认事件或补充输入恢复
3. `capability_blocked` 必须通过 `doctor` 或依赖修复恢复
4. `input_blocked` 必须通过重新生成合法工件或 contract 恢复

### 3.7 统一一期入口与 source 术语

当前文档中同时存在 `native issue`、`aria-native`、`vk | native | aria-native` 三类表述，容易导致一期范围理解漂移。

建议一期统一口径如下：

1. 一期正式支持的任务建立入口只有 `aria-native`
2. `Vibe Kanban` 只作为后续兼容方向，不进入一期必交付范围
3. 一期运行时实例值只允许 `aria-native`
4. 若 schema 仍暂时保留 `vk` 或 `native` 枚举，必须标注为保留值，不得在一期正式流中产生实例

## 4. 建议改项

### 4.1 增加一期分层实现顺序

建议在收敛方案中明确三层交付顺序：

1. **Layer 1：单任务、单执行单元、串行正式闭环**
2. **Layer 2：Patch 与 Retry**
3. **Layer 3：多执行单元与并行调度**

每层建议职责如下：

#### Layer 1

最小目标：

- 打通 `intake -> ... -> dispatch -> exec -> review/test -> verified | patching | blocked`
- 仅支持单 `exec unit`
- 不支持并行调度
- 具备 `approved refs`、`dispatch contract`、`exec result`、`review report`、`test report`、`verification summary`

#### Layer 2

在 Layer 1 稳定后补齐：

- `patch contract`
- `patch result`
- `result_set_id` 切换
- `blocked` 分类
- `retry` 恢复规则
- `doctor` 能力诊断

#### Layer 3

最后引入：

- 多 `exec units`
- worktree 分配
- 并行上限控制
- 依赖解析
- 多单元结果汇总与统一仲裁

### 4.2 明确 Claude 与 Aria 的编排层级关系

建议增加一句硬规则：

> `Claude` 是用户交互与前段任务组织入口；`Aria` 是正式流状态推进与工件编排真源。

并补充规则：

1. 用户命令先进入 `Claude` 插件入口
2. 一旦形成正式任务，状态推进、contract 生成、报告归档都必须经由 `Aria`

### 4.3 固定 review/test 的流程归属

建议明确：

1. `Aria` 是 `review/test` 的流程发起者与结果归档者
2. `Claude` 是默认分析执行者
3. 可自动执行的 superpowers 能力由 `Aria` 调用
4. 无论由谁执行，正式 `review report / test report` 都必须由 `Aria` 归档

### 4.4 将后续扩展接口降级为保留点

建议将 `branch / PR / merge / release` 相关内容从一期核心运行时设计中降级为“后续保留点”，并明确：

1. 一期 schema、状态机、contract 不为这些后续角色预留运行时字段
2. 仅允许在目录布局或文档注释中说明未来扩展方向
3. 任何一期实现代码中若仅为后续能力预留而当前无消费者，应优先视为可删减对象

### 4.5 增加跨文档联动修订规则

建议在收敛方案中追加一节“联动修订要求”，至少明确以下映射关系：

1. **状态语义变更**：同步更新收敛方案与 Runtime Schemas
2. **角色职责变更**：同步更新主方案与收敛方案
3. **contract/report 字段变更**：同步更新 Runtime Schemas 与 Implementation Layout
4. **模块边界变更**：同步更新 Implementation Layout 与主方案

## 5. 建议采用的改版设计

基于本次评审，建议将一期收敛方案下一版补强为以下四个新增或重写小节：

### 5.1 运行时真源规则

明确：

- `OpenSpec` 是正式工件内容真源
- `state.yaml` 是运行时状态与冻结引用真源
- 一期后段只允许消费冻结引用

### 5.2 执行契约模型

明确：

- `dispatch contract`
- `patch contract`
- `execution context bundle`

三者之间的关系、字段边界与执行约束。

### 5.3 仲裁与恢复模型

明确：

- `result_set_id`
- `confirmation event`
- `blocked` 分类
- `retry` 恢复条件

### 5.4 一期实现顺序与范围控制

明确：

- Layer 1：串行最小闭环
- Layer 2：Patch 与 Retry
- Layer 3：并行执行单元

并加三条控制规则：

1. 未完成 Layer 1 前，不为未来并行场景引入额外抽象层
2. 未完成 Layer 2 前，不拆分复杂恢复策略系统
3. Layer 3 只扩执行组织方式，不改 Layer 1/2 已确定的核心语义

## 6. 联动修改建议

若采纳本评审结论，建议至少同步修订以下文档：

1. `cadence/designs/2026-04-17_方案设计_Cadence-Aria一期收敛方案_v1.0.md`
2. `cadence/designs/2026-04-16_方案设计_Cadence-Aria_v1.4.md`
3. `cadence/designs/2026-04-16_配套设计_Runtime-Schemas_Cadence-Aria_v1.0.md`
4. `cadence/designs/2026-04-16_配套设计_Implementation-Layout_Cadence-Aria_v1.0.md`

## 7. 最终判断

当前一期收敛方案已经具备正确骨架，但仍缺“可执行运行时约束”这一层补强。  
下一版文档不需要重写总体方向，重点应放在以下四个问题上：

1. 谁是真源
2. 什么是正式执行契约
3. 如何基于稳定结果做仲裁与恢复
4. 一期应按什么分层顺序落地

只要上述四点被正式写入方案，并同步到 schema 与 implementation layout，一期从“概念成立”走到“可执行设计”就是可控的。
