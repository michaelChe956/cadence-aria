# Cadence-Aria 一期收敛方案设计评审

> **版本**：v1.0
> **日期**：2026-04-17
> **评审对象**：`cadence/designs/2026-04-17_方案设计_Cadence-Aria一期收敛方案_v1.0.md`
> **关联文档**：
> - `cadence/designs/2026-04-16_方案设计_Cadence-Aria_v1.4.md`
> - `cadence/designs/2026-04-16_配套设计_Runtime-Schemas_Cadence-Aria_v1.0.md`
> - `cadence/designs/2026-04-16_配套设计_Implementation-Layout_Cadence-Aria_v1.0.md`

## 1. 评审目标

本次评审聚焦以下两类问题：

1. 一期收敛方案与主方案、配套 schema/实现布局文档之间是否存在口径冲突
2. 一期收敛方案是否已具备进入 implementation plan 的最小清晰度

本次评审不重写主方案，不提前展开实现细节，只识别会影响后续计划与实现边界的问题。

## 2. 评审结论

评审结论为：

**该方案可以进入下一步，但必须先完成若干关键口径修订。**

当前文档的主要风险不在方向错误，而在于：

1. 部分状态语义尚未钉死
2. `native issue` 与运行时 `source` 枚举之间存在术语歧义
3. 一期收敛方案与 Runtime Schemas 的“真源分工”尚未明确

如果不先修正这些问题，后续 implementation plan 容易在状态机、来源定义和 schema 归属上出现分叉。

## 3. 评审范围与方法

本轮采用“交叉一致性审稿 + 少量落地性检查”的方式。

评审基线如下：

1. 一期收敛方案负责定义业务语义、范围边界、状态推进原则与 Guard
2. 主方案负责上位架构、角色定位与总体技术路线
3. Runtime Schemas 负责字段、枚举和值域定义
4. Implementation Layout 负责实现落位与模块职责

判断标准如下：

1. 同一术语在不同文档中语义必须一致
2. 状态名、动作名、工件名必须能一一映射
3. 任何会导致 plan 分叉的歧义都视为有效问题

## 4. 必须修改项

### 4.1 状态语义需要进一步收紧

#### 问题 1：`dispatch` 与 `dispatched` 混用

一期收敛方案中同时出现 `dispatch` 与 `dispatched`，但未明确一个是动作、一个是状态。

这会导致后续实现时出现两种不同理解：

1. 将 `dispatch` 当作正式流状态
2. 将 `dispatch` 当作生成 `dispatch contract` 的动作

**修改建议：**

在一期收敛方案中明确约定：

1. `dispatch` 仅表示状态推进动作
2. `dispatched` 仅表示状态值
3. 状态串中统一使用 `dispatched`

并补充进入 `dispatched` 的条件，例如：

1. 已存在 approved `plan`
2. 至少一个合法 `dispatch contract` 已生成
3. 所有待执行单元的 contract 已通过 scope 与 capability 校验
4. `state.yaml` 的执行单元映射已完成初始化

#### 问题 2：`reviewing/testing` 是聚合状态，但离开条件未完全定义

一期收敛方案当前已经表达：

1. `review` 与 `test` 允许并行
2. 二者分别产出独立报告
3. `reviewing/testing` 是单一状态

但仍缺少一条关键规则：

只有当 `review` 与 `test` 均进入终态后，状态机才允许离开 `reviewing/testing`。

如果不明确这条规则，后续实现会出现：

1. 单一报告先完成就提前推进状态
2. `review failed` 与 `test pending` 时不清楚是否继续等待
3. 仲裁逻辑分散在 orchestrator、scheduler、report builder 中

**修改建议：**

在一期收敛方案中明确：

1. `reviewing/testing` 是聚合状态
2. `review` 与 `test` 可以串行或并行执行
3. 单一报告先完成不构成离开条件
4. 只有两类检查都进入终态后，才由 `arbitrator` 做统一判定

统一判定后的状态去向只允许为：

1. `verified`
2. `patching`
3. `blocked`

#### 问题 3：`retry`、`patching`、`blocked` 的职责边界尚需单点收口

当前文档已经区分：

1. 业务结果不通过进入 `patching`
2. 执行失败进入 `retry` 或 `blocked`

但仍缺少一个单点规则来说明：

谁负责做最终分流，`retry` 到底是动作还是状态。

**修改建议：**

在一期收敛方案中明确：

1. `retry` 是对 `exec unit` 或 `patch unit` 的重试动作，不是正式流顶层状态
2. `patching` 是业务不通过但仍可修补时进入的状态
3. `blocked` 是无法继续推进且不能由当前自动流程恢复时进入的状态

并补充统一分流原则：

1. 存在合法 must-fix 依据时进入 `patching`
2. 单元执行失败且满足重试条件时执行 `retry`
3. 关键能力缺失、关键工件非法、状态损坏或无法形成合法仲裁结论时进入 `blocked`

同时增加一条约束：

**纯执行失败不能直接进入 `patching`。**

### 4.2 `native issue` 与运行时 `source` 枚举存在术语冲突

一期收敛方案中多次使用 `native issue` 作为一期入口术语，但 Runtime Schemas 当前的 `source` 枚举为：

- `vk`
- `native`
- `aria-native`

目前文档没有说明：

1. `native issue` 在运行时应映射到 `native` 还是 `aria-native`
2. `native` 与 `aria-native` 的实际差异是什么

这会直接影响：

1. 状态初始化
2. CLI 命令设计
3. schema 定义

**修改建议：**

在一期收敛方案中加入术语定义：

`native issue` 特指通过 Aria 原生命令入口建立的任务；在运行时来源字段中建议映射为 `aria-native`。

### 4.3 一期收敛方案与 Runtime Schemas 的真源分工尚未明确

当前一期收敛方案的第 7 节列出“最小字段集合”，而 Runtime Schemas 文档又承担字段级定义，两者没有明确主从关系。

这会导致后续争议：

1. 字段级定义到底以哪份文档为准
2. 最小字段集合与完整 schema 出现差异时应如何处理

**修改建议：**

在一期收敛方案中加入如下规则：

1. 一期收敛方案负责业务语义、状态推进与 Guard 约束
2. Runtime Schemas 负责字段、枚举和值域定义
3. 第 7 节仅列一期必须出现的最小字段集合，不替代字段级真源

## 5. 建议修改项

### 5.1 收紧 `source` 枚举

如果坚持一期只支持 `native issue` 入口，则 Runtime Schemas 中保留 `native` 与 `aria-native` 两个近义枚举收益很低。

**建议：**

将 `source` 收敛为：

- `vk`
- `aria-native`

如果短期内暂不调整枚举，也至少应在 schema 文档中补充术语解释，避免两个值并存但边界不清。

### 5.2 主方案只做引用对齐，不重写正文

本次问题主要集中在一期收敛方案与 Runtime Schemas 的口径收口，不建议回头重写主方案。

**建议：**

仅在主方案中补两类引用：

1. 一期状态精确定义以一期收敛方案为准
2. 字段级 schema 定义以 Runtime Schemas 为准

### 5.3 最小字段集合建议与 Runtime Schemas 再对齐一次

一期收敛方案当前列出的最小字段与 Runtime Schemas 顶层字段表尚未完全一一对应。

建议重点核对以下字段：

1. `artifacts`
2. `capability_status`
3. `confirmation_mode`
4. `confirmation_artifact_path`
5. `patch_round`

这项不是当前最严重的问题，但如果不对齐，后续写 implementation plan 时仍可能出现“最小集合”与“字段真源”不一致。

## 6. 需同步修订的文档清单

### 6.1 一期收敛方案

文件：

`cadence/designs/2026-04-17_方案设计_Cadence-Aria一期收敛方案_v1.0.md`

必须补充或修正：

1. `dispatch` / `dispatched` 语义约定
2. `reviewing/testing` 的聚合状态定义
3. `retry` / `patching` / `blocked` 的边界说明
4. `native issue` 的术语定义
5. 与 Runtime Schemas 的真源分工说明

### 6.2 Runtime Schemas

文件：

`cadence/designs/2026-04-16_配套设计_Runtime-Schemas_Cadence-Aria_v1.0.md`

必须同步：

1. `dispatched` 是状态值，`dispatch` 不是状态值
2. `reviewing/testing` 的聚合离开条件
3. `review report.verdict` / `test report.verdict` 只表达报告结论，不直接决定最终状态分流
4. `source` 枚举与 `native issue` 术语映射关系

### 6.3 主方案

文件：

`cadence/designs/2026-04-16_方案设计_Cadence-Aria_v1.4.md`

建议仅补引用，不做结构性重写：

1. 一期状态精确定义引用一期收敛方案
2. 字段级 schema 定义引用 Runtime Schemas

## 7. 是否可进入下一步

结论如下：

1. 该方案具备继续推进基础
2. 但在进入 implementation plan 之前，必须先完成第 4 节列出的关键修订

如果完成这些修订，则一期收敛方案可以作为：

1. implementation plan 的业务语义基线
2. 后续 schema 对齐与状态机实现的评审依据

## 8. 推荐后续顺序

建议按以下顺序推进：

1. 先修改一期收敛方案，钉死状态语义与术语定义
2. 再同步修改 Runtime Schemas，完成字段与枚举对齐
3. 最后仅在主方案中补引用关系
4. 在上述修订完成后，再进入 implementation plan

这样可以保证：

1. 业务语义先定
2. schema 再对齐
3. implementation plan 以统一口径展开

## 9. 最终意见

本次评审结论不是“推翻方案”，而是“允许继续，但必须先收紧关键口径”。

一期收敛方案当前的主要问题不是能力范围错误，而是：

1. 个别状态语义尚未完全封口
2. 术语与 schema 枚举仍有重叠和歧义
3. 文档之间的主从关系尚未写明

这些问题一旦修正，方案就会从“方向正确”提升到“足够支撑下一步计划拆解”。
