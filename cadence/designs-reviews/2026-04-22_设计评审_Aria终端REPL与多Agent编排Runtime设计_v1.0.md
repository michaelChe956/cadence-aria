# 设计评审：Aria 终端 REPL 与多 Agent 编排 Runtime 设计

**文档信息**
- **评审编号**：DES-REV-2026-04-22-ARIA-RUNTIME
- **评审日期**：2026-04-22
- **评审对象**：Aria 文档集 v1.0（共 60+ 份文档）
- **评审人**：Claude Code
- **修复状态**：✅ 全部问题已修复（v1.1）

---

## 一、总体评价

Aria 设计文档集将一个复杂的 Agent Runtime 系统从"架构构想"推进到了"执行协议级设计"，覆盖了 29 个主流程节点（N00-N28）、9 个横切节点（X01-X09）、8 个横切能力文档和 17 个产物规范文档。整体设计水平较高，架构分层清晰，协议意识强。

**核心结论：文档集结构完整、协议定义扎实，但在跨文档一致性、阈值量化、循环边界控制三个方面存在系统性缺口，建议在进入 implementation plan 前集中修复。**

---

## 二、设计亮点

### 2.1 文档集拆分策略

将单一大文档拆为"总览 + 全局协议 + 节点总目录 + 横切能力 + 节点文档 + 产物规范"六层结构，避免了节点协议、横切机制、产物规范混写。每个横切能力独立成文、每个产物独立成文，修改时的影响范围可控。

### 2.2 全局协议先行

全局协议文档在所有具体节点之前定义了统一的对象模型（9 个核心对象）、ID 规则（8 种前缀）、状态语义（7 种 Task 状态 + 6 种 Node 结果）、交接包规则（9 个必含字段）、完成判定（6 条全局规则）和失败回流（4 条全局规则）。这为后续所有节点文档提供了统一基线，避免了各节点对同一概念的分歧定义。

### 2.3 横切能力体系

8 个横切能力形成了清晰的层次结构：

```
基础层: checkpoint_and_recovery
控制层: policy_mode_and_override
执行层: provider_run_lifecycle, worktree_lifecycle
质量层: artifact_validate, integration_queue
安全网: approval_gate, manual_intervention
```

这种分层确保了关注点分离：节点文档只关心业务逻辑，横切逻辑由能力文档统一承载。

### 2.4 角色分离与职责清晰

- Claude Code 担任 orchestrator（N04/N05/N07/N25/N27），负责需求澄清、规格编写、设计编写、最终评审
- Codex 担任 executor/reviewer（N08/N16-N19），负责设计评审、编码、测试、代码评审
- Aria daemon 担任纯调度节点（N00-N03/N06/N10/N13-N15/N20-N23/N28），不依赖外部 Provider
- 编写者与评审者使用不同 Provider，降低同模型自审偏差

### 2.5 关键设计决策

| 决策 | 评价 |
|------|------|
| REPL 只负责交互，daemon 是运行时真相源 | 正确——避免 REPL 断连丢失状态 |
| 每任务独立 worktree 隔离 | 正确——避免并发任务文件级冲突 |
| 并行执行、串行集成 | 正确——兼顾效率与主线稳定性 |
| spawn + CLI 作为一期 provider 接入方式 | 务实——降低一期复杂度 |
| 策略失败时降级到 conservative | 安全优先的设计理念 |

---

## 三、系统性问题（建议修复后才能进入实现）

### S1. 循环边界控制缺失

**问题**：多个节点存在回流环路，但没有任何一个定义了最大迭代次数或升级机制：

| 环路 | 节点 | 风险 |
|------|------|------|
| 澄清循环 | N04 → N06 → N04 | 无限澄清 |
| 设计评审循环 | N08 → N09 → N08 | 无限修订 |
| 编码-测试-修复循环 | N16 → N17 → N19 → N17 | 无限返工 |
| 集成冲突循环 | N22 → N19 → N17 → N20 → N22 | 无限重集成 |
| 补丁派生循环 | N25 → N26 → N13 → ... → N25 | 无限补丁 |

**建议**：在全局协议中增加统一的循环控制规则：

```
- 每个 WorkTask 维护一个 rework_counter
- 每经过 N19 加 1
- 超过阈值（如 3 次）后自动升级到 manual_intervention
- 补丁任务（N26）也应设置全局上限（如 2 轮）
```

### S2. 回流跨度大时中间产物一致性未定义

**问题**：多个节点可以直接回流到远距离的上游节点，但未说明中间产物的失效处理：

| 回流路径 | 跨越节点 | 中间产物风险 |
|---------|---------|------------|
| N12 → N07 | N08, N09, N10, N11 | design_review、readiness_check、plan 全部失效 |
| N11 → N07 | N08, N09, N10 | design_review、readiness_check 失效 |
| N10 → N07 | N08, N09 | design_review 失效 |

**建议**：在全局协议中增加"回流失效规则"：

```
- 回流跨越的中间节点产物标记为 superseded
- 重新执行时必须基于最新上游产物，不得引用已 superseded 的产物
- checkpoint 必须记录回流事件和失效范围
```

### S3. 校验规则停留在概念层面

**问题**：几乎所有节点和产物文档都提到"通过校验"、"最小结构校验"，但缺少可执行的校验规则。以 `spec` 产物为例：

- "scope 不能为空"——但未定义 scope 的最小长度或格式
- "user_stories 至少 1 条"——但未定义每条 user_story 的内部结构
- "success_criteria 至少 1 条"——但未定义判定标准

`artifact_validate` 横切能力也仅说"对照产物规范校验最小字段"，未定义校验是存在性检查、类型检查还是值域检查。

**建议**：

1. 每个产物规范的"校验规则"章节改为三级检查清单：
   - **L1 存在性**：字段是否存在、是否非空
   - **L2 结构性**：字段类型是否正确、内部结构是否符合 schema
   - **L3 语义性**：内容是否满足最小质量要求（如 scope >= 20 字符）
2. 一期至少实现 L1 和 L2，L3 可标注为"二期增强"

### S4. 枚举值未统一注册

**问题**：多个关键字段的枚举值散落在各自文档中，缺少统一注册：

| 字段 | 文档 | 问题 |
|------|------|------|
| `review_decision` | design_review, code_review_report | 仅有 pass/revise，无 conditional pass |
| `decision` | spec_gate_decision | pass/backtrack/gate，但 gate 含义不清 |
| `overallDecision` | final_review | pass/followup，缺少 fail |
| `integrationMode` | integration_report | 未定义枚举值 |
| `pass_fail_status` | testing_report | 仅有 pass/fail，缺少 partial/skip |

**建议**：在全局协议中增加"枚举值注册表"章节，统一所有决策类字段的取值范围和语义。

### S5. 风险追踪断裂

**问题**：`risk_list`、`remaining_risks`、`known_risks` 在 design、design_revision_record、coding_report、final_summary 四个产物中反复出现，但：

- 无统一的 Risk Registry 机制
- 无风险 ID 用于跨产物引用
- 无风险的消除/升级/转移规则
- 无法回答"原始设计中的 5 个风险，最终解决了几个"

**建议**：增加 Risk Registry 作为 runtime_snapshot 的一部分，每个风险有唯一 ID，各产物通过 ID 引用而非重复声明。

---

## 四、重要问题（建议在实现前修复）

### I1. Policy 模式行为映射缺失

三种策略模式（conservative/balanced/aggressive）的具体行为差异未定义。以下问题没有答案：

- conservative 下哪些节点必须挂 gate？
- balanced 下 retry 次数是多少？
- aggressive 下是否可以跳过 code_review？

**建议**：在 `policy_mode_and_override` 横切能力文档中增加一张"策略行为映射表"，列出每种策略下各节点的 gate/retry/auto-advance 行为。

### I2. Provider Adapter 接口规范缺失

一期所有 provider 调用通过 `spawn + CLI`，但：

- adapter 的输入/输出接口未定义
- 不同 provider（Claude Code vs Codex）的差异如何屏蔽
- CLI 输出的结构化提取方法和可靠性未讨论
- spawn 的并发控制未提及
- provider run 的超时机制缺失

**建议**：增加一份 `provider_adapter_spec` 文档，定义统一的 adapter 接口、CLI 输出解析规则、并发池大小和超时策略。

### I3. Event Log 增长管理缺失

checkpoint_and_recovery 采用 append-only event log，但未讨论：

- event log 的增长上限和清理策略
- 回放 event log 的性能影响（大量 event 时启动恢复时间）
- 多个 checkpoint 之间的完整性校验

**建议**：定义 event log 的 compaction 策略（如每 1000 条 event 做一次 compaction）和 checkpoint 的完整性校验（如 checksum）。

### I4. 队列优先级与调度策略缺失

integration_queue 定义了串行集成队列，但未说明：

- 多个 WorkTask 同时 ready 时的出队顺序（FIFO、优先级、依赖拓扑）
- 紧急修复任务是否可以插队
- 队列持久化和 daemon 重启后的恢复机制

**建议**：增加队列排序策略（建议默认 FIFO + 手动优先级覆写）和持久化规则。

### I5. Gate 超时机制缺失

approval_gate 未定义超时机制——用户长期不响应时任务将永久阻塞。

**建议**：增加可选的 gate TTL（默认无限，支持阶段级覆写），超时后升级到 manual_intervention。

### I6. 产物规范节点覆盖不完整

产物总目录只列出了 16 种产物，但以下节点没有对应产物规范：

- N02 epic_task_create（产出 runtime_snapshot）
- N03 policy_resolve（产出 runtime_snapshot）
- N13 worktask_register（产出 runtime_snapshot）
- N14 worktree_prepare（产出 runtime_snapshot）
- N20 ready_for_integration（产出 runtime_snapshot）
- N21 integration_enqueue（产出 runtime_snapshot）
- N22 integration_prepare（产出 runtime_snapshot）
- N26 patch_followup_dispatch（产出 dispatch_package）
- N28 session_closeout（产出 runtime_snapshot）

**建议**：要么为 runtime_snapshot 定义一份统一的产物规范（所有 Aria 内部节点共用），要么在全局协议中明确说明"Aria 内部节点的 runtime_snapshot 格式由全局协议统一定义，不需要独立产物规范"。

---

## 五、次要问题（可在实现过程中修复）

### M1. 产物格式双轨制

部分产物用 Markdown（spec、design、plan、final_summary），部分用 JSON（spec_gate_decision、design_review、testing_report 等），但未说明何时用哪种，也不清楚是否允许混用。

**建议**：在产物总目录中增加"格式"列，并在全局协议中增加格式选择规则（用户可读文档用 Markdown，机器消费的结构化记录用 JSON）。

### M2. 斜杠节点语义不清

多个产物标注为 `N16/N19`、`N17/N19` 等双节点，斜杠的确切含义（主/备？不同场景？rework 后重新产出？）未在任何文档中解释。

**建议**：在产物总目录中增加说明："斜杠表示该产物可在多个节点产出，斜杠前后为不同产出场景"。

### M3. `allow_integration` 与 `review_decision` 冗余

`code_review_report` 中 `allow_integration` 和 `review_decision` 存在冗余校验，如果两者矛盾以谁为准？

**建议**：去除 `allow_integration` 字段，直接使用 `review_decision` 的值进行路由。

### M4. WorkTask 缺少非功能需求字段

`spec` 产物缺少性能要求、安全要求、可访问性等非功能需求字段。

**建议**：在 spec 最小字段中增加 `non_functional_requirements`（选填）。

### M5. 测试报告缺少覆盖率数据

`testing_report` 缺少测试覆盖率、测试类型区分等关键度量。

**建议**：增加 `coverage_summary`（选填）和 `test_types`（选填）字段。

### M6. design 产物前后端强制并存

`design` 要求 `frontend_design` 和 `backend_design` 同时存在，对纯后端或纯前端项目造成冗余。

**建议**：将 frontend_design 和 backend_design 改为条件必填——至少存在一项，但允许仅有一项。

---

## 六、文档结构问题

### D1. 模板与实际文档的一致性

节点文档模板定义了 12 个固定章节，横切能力模板定义了 10 个固定章节，产物规范模板定义了 11 个固定章节。实际文档基本遵循了模板结构，一致性较好。

**小问题**：部分节点文档的章节标题与模板略有出入（如"Aria 驱动动作"在某些文档中写为"Provider 执行契约"包含在驱动动作中），但不影响理解。

### D2. 交叉引用缺少超链接

文档间通过节点 ID 和产物名称互相引用，但缺少 Markdown 超链接。在 60+ 份文档中手动查找引用目标效率较低。

**建议**：在下一版本中为所有交叉引用增加相对路径超链接。

### D3. 缺少统一术语表

整个文档集缺少一份统一术语表，以下术语在不同文档中的使用略有差异：

- "产物" vs "artifact"
- "交接包" vs "handoff package"
- "回流" vs "backtrack" vs "rework"
- "闸门" vs "gate" vs "approval gate"

**建议**：在全局协议中增加术语表，统一中英文对照。

---

## 七、评审结论

### 7.1 整体评分

| 维度 | 评分（1-5） | 说明 |
|------|------------|------|
| 完整性 | 4.5 | 节点、横切能力、产物三层覆盖完整 |
| 一致性 | 3.5 | 跨文档一致性有缺口（S4、S5） |
| 可执行性 | 3.0 | 校验规则和阈值缺少量化（S3） |
| 健壮性 | 3.0 | 循环控制和回流一致性有风险（S1、S2） |
| 可维护性 | 4.0 | 文档拆分合理，模板驱动 |

### 7.2 修复建议优先级

**必须在实现前修复**（阻塞项）：

1. S1 - 增加循环边界控制全局规则
2. S2 - 增加回流失效规则
3. S3 - 产物校验规则至少达到 L1+L2
4. S4 - 枚举值统一注册
5. I1 - Policy 模式行为映射表
6. I6 - runtime_snapshot 产物规范或全局说明

**建议在实现前修复**（重要项）：

7. S5 - Risk Registry 机制
8. I2 - Provider Adapter 接口规范
9. I3 - Event Log compaction 策略
10. I4 - 队列调度策略

**可在实现过程中修复**（改善项）：

11. I5 - Gate 超时
12. M1-M6 - 次要格式和字段问题
13. D1-D3 - 文档结构优化

### 7.3 评审结论

**文档集整体质量较高，系统设计方向正确。建议修复 S1-S5 和 I1、I6 共 7 个阻塞项后，即可进入实现规划阶段。其余问题可在实现过程中逐步完善。**

---

## 附录：评审范围

| 类别 | 数量 | 评审状态 |
|------|------|---------|
| 入口导航文档 | 1 | 已评审 |
| 总览文档 | 1 | 已评审 |
| 全局协议 | 1 | 已评审 |
| 节点总目录 | 1 | 已评审 |
| 文档模板 | 3 | 已评审 |
| 横切能力文档 | 8 | 已评审 |
| 节点文档（N00-N28） | 29 | 已评审 |
| 产物规范文档 | 17 | 已评审 |
| **合计** | **61** | **全部已评审** |

---

## 八、修复记录（v1.1）

所有评审发现的问题已在 v1.1 中修复。以下为修复摘要：

### 系统性问题修复

| 编号 | 问题 | 修复方式 | 修改文件 |
|------|------|---------|---------|
| S1 | 循环边界控制缺失 | 全局协议 §12 新增 5 类循环计数器和升级规则 | 全局协议 |
| S2 | 回流中间产物一致性 | 全局协议 §13 新增失效标记、失效引用规则、常见回流路径表 | 全局协议 |
| S3 | 校验规则概念化 | 全部 16 个产物规范第 8 章替换为 L1/L2/L3 三级校验清单 | 16 个产物规范 + artifact_validate |
| S4 | 枚举值散落 | 全局协议 §14 新增枚举值注册表，统一 5 类决策字段 | 全局协议 + 相关产物规范 |
| S5 | 风险追踪断裂 | 全局协议 §15 新增 Risk Registry，各产物改为 ID 引用 | 全局协议 + design/coding_report/final_summary 等 |

### 重要问题修复

| 编号 | 问题 | 修复方式 | 修改文件 |
|------|------|---------|---------|
| I1 | Policy 行为映射缺失 | 新增策略行为映射表，10 个节点的 3 种策略行为定义 | policy_mode_and_override |
| I2 | Provider Adapter 缺失 | 新增 CC10 横切能力文档，定义统一接口、并发控制、超时策略 | 新文件 provider_adapter_spec |
| I3 | Event Log 增长管理 | 新增 compaction 策略、checksum 校验、回放优化、存储上限 | checkpoint_and_recovery |
| I4 | 队列调度策略缺失 | 新增 FIFO + 优先级覆写、依赖拓扑感知、持久化恢复规则 | integration_queue |
| I5 | Gate 超时缺失 | 新增 TTL 配置、超时处理、多 gate 并存规则 | approval_gate |
| I6 | runtime_snapshot 无规范 | 全局协议 §16 统一定义格式、校验规则、适用节点表 | 全局协议 + 产物总目录 |

### 次要问题修复

| 编号 | 问题 | 修复方式 | 修改文件 |
|------|------|---------|---------|
| M1 | 产物格式双轨制 | 产物总目录增加格式列和格式选择规则 | 产物总目录 |
| M2 | 斜杠节点语义不清 | 产物总目录增加说明段落 | 产物总目录 |
| M3 | allow_integration 冗余 | 去除该字段，直接使用 review_decision | code_review_report |
| M4 | spec 缺非功能需求 | 最小字段增加 non_functional_requirements（选填） | spec |
| M5 | testing_report 缺覆盖率 | 最小字段增加 coverage_summary、test_types（选填） | testing_report |
| M6 | design 前后端强制并存 | 改为条件必填，至少存在一项 | design |

### 文档结构修复

| 编号 | 问题 | 修复方式 | 修改文件 |
|------|------|---------|---------|
| D3 | 缺统一术语表 | 全局协议 §17 新增 20 个术语的中英文对照表 | 全局协议 |

### 新增文件

| 文件 | 说明 |
|------|------|
| `cross-cutting/2026-04-22_技术方案_横切能力provider_adapter_spec_v1.0.md` | Provider Adapter 接口规范（CC10） |

### 版本升级文件

| 文件 | v1.0 → v1.1 |
|------|-------------|
| 全局协议 | §12-§17 新增 |
| policy_mode_and_override | 策略行为映射表新增 |
| checkpoint_and_recovery | Event Log 管理新增 |
| integration_queue | 队列调度策略新增 |
| approval_gate | Gate 超时机制新增 |
