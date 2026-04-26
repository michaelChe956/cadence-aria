# 设计评审：Aria 一期方案与实施计划研发可落地性 Review

**文档信息**
- **创建日期**：2026-04-26
- **版本**：v2.0
- **评审范围**：Aria 一期 MVP 设计、IO 协作协议、实现总契约、研发导读、评审后实施规格补齐、P1-P4 实施计划
- **评审目标**：验证研发人员拿到方案和计划后能清楚知道如何落地实现
- **评审基准**：以 MVP 设计 v1.2 和总设计（实现总契约 v1.0 + 评审后实施规格补齐 v1.2）为准

---

## 1. 总体评价

**结论：方案设计完整，文档体系层次清晰，MVP 边界合理。实施计划与设计对齐度较高。没有发现 MVP 设计或总设计的根本性错误。**

具体评价：

| 维度 | 评分 | 说明 |
|------|------|------|
| 设计完整性 | 高 | 从系统边界到代码级规格形成完整链条 |
| 文档一致性 | 中高 | 核心枚举值和路由规则一致，个别字段定义分散在三份文档中 |
| 研发可落地性 | 中 | 大部分 task 有明确步骤，但存在 P0 级缺口需修复 |
| 实施计划覆盖度 | 高 | 21 个实现单元到 P1-P4 task 的映射完整，无遗漏 |

---

## 2. P0 级问题（阻塞研发落地，必须修复）

### P0-1：`AdapterInput` / `AdapterOutput` Rust 类型定义缺失

**现状**：
- 实施规格补齐 v1.2 第 4.7 章给出了 `ProviderContextPackage`、`ProviderRunRecord` 的完整 Rust 类型
- 实现总契约 v1.0 第 6.7.1 章给出了 `ProviderContextPackage -> AdapterInput` 的字段映射表
- 但 **`AdapterInput` 和 `AdapterOutput` 本身的 Rust struct 定义从未给出**

**影响**：
- P3 Task 1 Step 1 要求"定义 AdapterInput / AdapterOutput 运行时封装"
- 研发需要自己根据映射表和 ProviderContextPackage 类型反推这两个 struct
- fake provider 和 CLI adapter 的共用 DTO 缺乏基准定义，可能导致两者分叉

**修复建议**：
- 在实施规格补齐 v1.2 中新增 `AdapterInput` 和 `AdapterOutput` 的 Rust 类型定义
- 或在 P3 Task 1 步骤中给出明确的结构签名
- 参考映射表：

````text
AdapterInput:
  providerType: ProviderType
  role: AdapterRole
  worktreePath: Option<String>
  prompt: String
  contextFiles: Vec<String>
  outputSchema: String
  timeout: u64
  maxRetries: u32

AdapterOutput:
  exitCode: Option<i32>
  stdout: String
  stderr: String
  structuredOutput: Option<Value>
  filesModified: Vec<String>
  durationMs: u64
  timeoutStatus: TimeoutStatus
````

**涉及文档**：实施规格补齐 v1.2

---

### P0-2：`document_ops.rs` 双文件职责不明

**现状**：
- P2 目标文件结构同时列出 `src/protocol/document_ops.rs` 和 `src/cross_cutting/document_ops.rs`
- P2 Task 1 Files 也同时创建这两个文件
- 但没有任何文档或 task 步骤解释两者的职责划分

**影响**：
- 研发无法判断类型定义放哪里、实现放哪里
- 可能导致 protocol/ 下放了实现逻辑，或 cross_cutting/ 下放了类型定义

**修复建议**：
- 方案 A（推荐）：合并为单一 `src/cross_cutting/document_ops.rs`，类型定义也在同一文件。protocol/ 下的类型统一通过 `mod.rs` re-export。
- 方案 B：明确划分 —— `protocol/document_ops.rs` 放 `DocumentModel`、`DocumentSection`、`DocumentBlock` 等纯类型定义；`cross_cutting/document_ops.rs` 放 `read_document_model`、`upsert_section`、`extract_projection_source` 等实现函数。
- 无论选哪种，需在 P2 Task 1 中显式说明。

**涉及文档**：P2 实施计划 v1.1

---

### P0-3：节点 handler 统一接口 / trait 未定义

**现状**：
- 21 个实现单元各有自己的 `.rs` 文件
- 但没有一个统一的 `RuntimeUnit` trait 或接口定义
- 研发不知道：
  - 节点 handler 函数签名是什么
  - 输入怎么传入（`CanonicalNodeInput`？）
  - 输出怎么返回（返回哪些产物？触发哪些事件？）
  - 错误怎么处理

**影响**：
- 每个研发可能自己发明不同的 handler 接口
- P3/P4 的 runtime_units/ 下的实现可能风格不一致
- 状态机如何调用节点 handler 的衔接点不清晰

**修复建议**：
- 在实施规格补齐或 P1 中定义统一 trait：

```rust
trait RuntimeUnit {
    fn node_id(&self) -> NodeId;
    fn covered_protocol_nodes(&self) -> Vec<NodeId>;
    fn execute(&self, input: CanonicalNodeInput, ctx: &mut DaemonContext) -> RuntimeUnitResult;
}

struct RuntimeUnitResult {
    pub artifacts: Vec<ArtifactRef>,
    pub events: Vec<Event>,
    pub next_route: Option<String>,
}
```

- 或至少在 P1 Task 4 中给出一个最小示例 handler 签名

**涉及文档**：实施规格补齐 v1.2 或 P1 实施计划 v1.0

---

## 3. P1 级问题（影响研发效率，建议修复）

### P1-1：`ProviderContextPackage` 三份文档字段逐步增加，需明确以哪个为准

**现状**：
- IO 协作协议 v1.0 第 6.1 章：12 个字段（简版）
- 实现总契约 v1.0 第 6.7 章：20 个字段（完整版）
- 实施规格补齐 v1.2 第 4.7 章：22 个字段（加了 sessionId、taskId、advisoryOnly）

三者之间字段逐步增加，但没有明确的"以此为准"声明。

**影响**：
- 研发可能看了 IO 协议就开始写代码，遗漏后续补充的字段
- 三份文档交叉阅读负担大

**修复建议**：
- 在研发导读或实施规格补齐中增加一行明确声明：**"ProviderContextPackage 的 Rust 类型以实施规格补齐 v1.2 第 4.7 章为准；IO 协作协议的字段表仅做概念说明"**
- 或在 IO 协议中标注"本节为概念模型，实现类型见实施规格补齐"

**涉及文档**：研发导读 v1.0 或 IO 协作协议 v1.0

---

### P1-2：P2 Task 2 和 Task 3 共用测试文件名 `tests/spec_projection.rs`

**现状**：
- Task 2（三层 validator）的测试文件：`tests/spec_projection.rs`
- Task 3（SpecProjection compiler）的测试文件：`tests/spec_projection.rs`
- 同一个文件被两个不同 task 引用

**影响**：
- 研发不清楚 validator 测试和 compiler 测试是写在同一个文件还是分开
- 两个 task 的提交可能互相冲突

**修复建议**：
- Task 2 的测试改为 `tests/artifact_validate.rs`
- Task 3 的测试保持 `tests/spec_projection.rs`
- 这样职责分离更清晰

**涉及文档**：P2 实施计划 v1.1

---

### P1-3：P3 规划节点 task 对"统一执行链"引用不显式

**现状**：
- 实现总契约 v1.0 第 8.1 章定义了 19 步统一执行链
- 这是所有 Agent 节点的固定调用流程
- 但 P3 Task 4/5 的步骤只说"调用 fake provider"、"产出 clarification_record"
- 没有显式引用统一执行链

**影响**：
- 研发可能跳过 context builder 组装、归一化、校验等步骤
- 直接把 fake provider 输出当成正式产物

**修复建议**：
- 在 P3 Task 4 Step 2-4 的每个节点实现步骤中，增加引用：
  - "按实现总契约 §8.1 统一执行链执行：组装 CanonicalNodeInput -> 读取 projection/bundle -> 组装 ProviderContextPackage -> 调用 provider -> 收集 run record -> 归一化 -> artifact_validate -> 写 checkpoint"
- 或在 P3 Task 4 Step 1 的测试断言中覆盖完整链路

**涉及文档**：P3 实施计划 v1.1

---

### P1-4：P4 `traceability_refs` 生成职责描述不精确

**现状**：
- 实施规格补齐 v1.2 第 10 章明确：`_aria.traceability_refs` 由 daemon 在归一化时生成，provider 只提供候选
- 但 P4 Task 2 Step 2 说"填充 `_aria.traceability_refs`"
- Step 3 说 testing_report 和 code_review_report"都具备 `_aria.traceability_refs`"

**影响**：
- 研发可能理解为在节点 handler 中手动填充
- 实际应该是 daemon 归一化阶段自动生成

**修复建议**：
- P4 Task 2 的步骤改为：
  - Step 2：`coding_report` 归一化后由 daemon 自动生成 `_aria.traceability_refs`（按实施规格补齐第 10 章算法）
  - Step 3：testing_report 和 code_review_report 归一化后同样自动生成
  - 测试断言：验证 `_aria.traceability_refs` 与 PlanProjection 的 traceability_refs 一致

**涉及文档**：P4 实施计划 v1.0

---

### P1-5：P1 `nodes.rs` 和 `artifacts.rs` 内容来源不明

**现状**：
- P1 Task 1 文件列表包含 `src/protocol/nodes.rs` 和 `src/protocol/artifacts.rs`
- 但没有任何 task 步骤说明这两个文件的内容
- Task 1 只建空模块骨架，后续 task 也没有提及

**影响**：
- 研发不知道这两个文件应该放什么内容
- 实施规格补齐 v1.2 的第 4 章有基础类型定义，但没有明确说放到哪个文件

**修复建议**：
- 在 P1 Task 1 Step 3 中明确：
  - `nodes.rs`：存放协议节点 ID 常量（`N00-N28`、`X01-X09`）、节点类型枚举、节点上下游路由映射
  - `artifacts.rs`：存放 `ArtifactKind` 枚举、`ArtifactRef`、`ArtifactStatus` 等引用类型（来自实施规格补齐第 4.1-4.2 章）

**涉及文档**：P1 实施计划 v1.0

---

### P1-6：P2 的横切能力覆盖未在总览中列出

**现状**：
- 总览计划 §5 的横切节点覆盖矩阵只列出了 P1、P3、P4
- P2 主要负责 X05（artifact_validate），但矩阵中没有出现

**影响**：
- 研发可能不清楚 P2 需要覆盖哪些横切能力

**修复建议**：
- 在总览计划 §5 覆盖矩阵中增加 P2 行：
  - `X05 artifact_validate`：P2 完整实现 canonical + phase1 profile validator

**涉及文档**：实施计划总览 v1.0

---

## 4. P2 级问题（建议优化）

### P2-1：P1 Task 4 缺少入口函数签名示例

P1 Task 4 是最核心的任务，但步骤只描述了要做什么，没有给出函数签名。建议至少给出：

```rust
// session_bootstrap.rs
fn bootstrap(workspace_root: &Path) -> Result<SessionState>;

// intake_capture.rs
fn capture_intake(request_text: &str, session: &SessionState) -> Result<IntakeBrief>;

// task_init.rs
fn init_task(intake: &IntakeBrief, session: &mut SessionState) -> Result<TaskInitResult>;

struct TaskInitResult {
    task_id: TaskId,
    change_id: ChangeId,
    effective_policy: PolicyRef,
    openspec_bootstrap_status: BundleStatus,
}
```

---

### P2-2：P3 N06 advisory 调用逻辑不够明确

MVP v1.2 §2.5 明确了 N06 可以调用 Codex 做 advisory，但 P3 Task 4 Step 3 只说"N06 由 daemon 生成 spec_gate_decision"。

建议在 Step 3 中补充：
- "N06 先执行 `artifact_validate` 校验 spec"
- "若配置启用 advisory review，通过 context builder 构建 advisory 请求并调用 Codex"
- "最终 `spec_gate_decision` 仍由 daemon 按固定协议字段生成"

---

### P2-3：P4 缺少带 provider run 和 worktree 的 recovery 测试计划

P1 Task 5 只做了最小 recovery smoke test（无 provider run、无 worktree）。

后续 P3/P4 引入了 provider run 和 worktree 后，recovery 逻辑需要增强，但没有对应的 task 覆盖。

建议：
- 在 P4 Task 5（一期闭环 smoke）中增加一个 recovery 验证步骤
- 断言：daemon 重启后能恢复 open gates、in-progress worktree leases、pending provider run 状态

---

### P2-4：Task 间数据流可更显式

目前各 task 只描述了自己的文件和步骤，但 task 之间的数据流（特别是跨阶段的数据流）需要研发自己理解。

建议：
- 在总览计划中增加一节"关键数据流图"，用 ASCII 或 mermaid 画出：
  - P1 产出 -> P2 消费
  - P1+P2 产出 -> P3 消费
  - P1+P2+P3 产出 -> P4 消费
- 这样研发能一目了然地看到自己负责的模块需要消费哪些前置产出

---

## 5. MVP 设计与总设计审查结论

**核心问题：MVP 设计或总设计有没有出错？**

**没有发现根本性设计错误。** 以下是验证过的关键设计决策：

| 设计决策 | 验证结论 |
|----------|----------|
| 双层模型（协议层 + 实现层） | 合理，避免实现折叠污染协议定义 |
| 21 个实现单元划分 | 合理，合并的都是相邻且职责相近的内部节点 |
| BYO CLI 策略 | 合理，降低一期工程复杂度 |
| OpenSpec 强约束 | 合理，防止需求扩散；时序上 P2 bootstrap 在 N04 之前完成，逻辑通顺 |
| runtime_snapshot 与业务产物分离 | 合理，避免混淆 checkpoint 和节点输出 |
| 三层 validator（canonical / projection / phase1_profile） | 合理，各层职责清晰 |
| N26 一期不自动派生 | 合理，避免无限补丁循环 |
| 集成队列 FIFO + 回流上限 | 合理，实现成本低的兜底机制 |

---

## 6. 修复优先级与建议

### 必须在研发启动前修复（P0）

| 编号 | 问题 | 修复位置 | 建议方式 |
|------|------|----------|----------|
| P0-1 | AdapterInput/AdapterOutput 类型缺失 | 实施规格补齐 v1.2 | 新增 §4.7.3 定义 Rust 类型 |
| P0-2 | document_ops.rs 双文件职责不明 | P2 实施计划 v1.1 | Task 1 中明确职责划分或合并 |
| P0-3 | 节点 handler 统一接口未定义 | 实施规格补齐 v1.2 或 P1 计划 | 新增 RuntimeUnit trait 定义 |

### 建议在对应阶段启动前修复（P1）

| 编号 | 问题 | 修复位置 |
|------|------|----------|
| P1-1 | ProviderContextPackage 三文档字段对齐 | 研发导读或 IO 协议 |
| P1-2 | P2 Task 2/3 测试文件名共用 | P2 实施计划 |
| P1-3 | 统一执行链引用不显式 | P3 实施计划 |
| P1-4 | traceability_refs 生成职责不精确 | P4 实施计划 |
| P1-5 | P1 nodes.rs/artifacts.rs 内容不明 | P1 实施计划 |
| P1-6 | P2 横切能力覆盖缺失 | 总览计划 |

### 建议优化（P2）

| 编号 | 问题 | 修复位置 |
|------|------|----------|
| P2-1 | P1 入口函数签名缺失 | P1 实施计划 |
| P2-2 | N06 advisory 调用逻辑不明确 | P3 实施计划 |
| P2-3 | P4 recovery 测试计划缺失 | P4 实施计划 |
| P2-4 | Task 间数据流可更显式 | 总览计划 |

---

## 7. 实施计划与 21 个实现单元映射验证

以下验证 21 个实现单元是否在 P1-P4 中都有对应的 task 和步骤：

| 实现单元 | 覆盖协议节点 | 计划 Task | 状态 |
|---------|-------------|-----------|------|
| `M00 session_bootstrap_impl` | `N00` | P1 Task 4 Step 2 | 覆盖 |
| `M01 intake_capture_impl` | `N01` | P1 Task 4 Step 3 | 覆盖 |
| `M02 task_init_impl` | `N02 + N03` | P1 Task 4 Step 4 | 覆盖 |
| `M04 clarification_impl` | `N04` | P3 Task 4 Step 2 | 覆盖 |
| `M05 spec_authoring_impl` | `N05` | P3 Task 4 Step 3 | 覆盖 |
| `M06 spec_gate_review_impl` | `N06` | P3 Task 4 Step 3 | 覆盖 |
| `M07 design_authoring_impl` | `N07` | P3 Task 4 Step 4 | 覆盖 |
| `M08 design_review_impl` | `N08` | P3 Task 5 Step 2 | 覆盖 |
| `M09 design_revision_impl` | `N09` | P3 Task 5 Step 2 | 覆盖 |
| `M10 plan_dispatch_impl` | `N10 + N11 + N12` | P3 Task 5 Step 3-4 | 覆盖 |
| `M13 execution_setup_impl` | `N13 + N14 + N15` | P4 Task 1 Step 2-3 | 覆盖 |
| `M16 coding_impl` | `N16` | P4 Task 2 Step 2 | 覆盖 |
| `M17 testing_impl` | `N17` | P4 Task 2 Step 3 | 覆盖 |
| `M18 code_review_impl` | `N18` | P4 Task 2 Step 3 | 覆盖 |
| `M19 rework_impl` | `N19` | P4 Task 2 Step 4 | 覆盖 |
| `M20 integration_prepare_impl` | `N20 + N21 + N22` | P4 Task 3 Step 2-3 | 覆盖 |
| `M23 integration_execute_impl` | `N23` | P4 Task 3 Step 4 | 覆盖 |
| `M24 integration_verify_impl` | `N24` | P4 Task 3 Step 5 | 覆盖 |
| `M25 final_review_impl` | `N25` | P4 Task 4 Step 2 | 覆盖 |
| `M27 final_summary_impl` | `N27` | P4 Task 4 Step 5 | 覆盖 |
| `M28 session_closeout_impl` | `N28` | P4 Task 4 Step 6 | 覆盖 |

**结论：21 个实现单元全部在 P1-P4 中有对应 task，无遗漏。**

---

## 8. 枚举值与关键字段一致性验证

| 检查项 | MVP v1.2 | 实现总契约 | 实施规格补齐 | 实施计划 | 结果 |
|--------|----------|-----------|-------------|----------|------|
| `ExecutionMode` | 未显式定义 | `agent_only/human_assisted/human_required` | §4.1 Rust enum | P3 完成判定 | 一致 |
| `review_decision` | `pass/revise/conditional_pass` | §9.2 矩阵 | N08 模板差异项 | P3 Task 5 | 一致 |
| `overallDecision` | `pass/followup/fail` | §9.1 矩阵 | §4.7 未显式定义 | P4 Task 4 | 一致 |
| `pass_fail_status` | `pass/fail/partial/skip` | 引用 | 未显式定义 | P4 Task 2 | 一致 |
| `integrationMode` | `merge/rebase/cherry_pick` | 引用 | §13.3 cherry-pick | P4 Task 3 | 一致 |
| `timeoutStatus` | 未显式定义 | §6.8.1 | §4.7.1 Rust enum | P3 Task 2 | 一致 |
| N26 路由规则 | §2.7 五种路由 | §9.1 矩阵 | §14.3 gate 规则 | P4 Task 4 | 一致 |
| Provider 错误码 | 未显式定义 | §6.8.1 表格 | §4.7.2 表格 | P3 Task 2 | 一致 |
| OpenSpec bundle 字段名 | 未显式定义 | §6.3.1 camelCase | §4.6 camelCase | P2 Task 5 | 一致 |

---

## 9. 研发执行建议

### 9.1 文档阅读顺序（精简版）

研发按以下顺序阅读，4 份即可开始编码：

1. **研发导读 v1.0** —— 10 分钟，理解团队怎么拆、怎么排
2. **MVP 精简设计 v1.2** —— 30 分钟，理解一期系统边界
3. **实现总契约 v1.0** —— 60 分钟，理解接口、对象模型和收口规则
4. **评审后实施规格补齐 v1.2** —— 60 分钟，看 Rust 类型和编译规则

IO 协作协议和节点文档集作为参考手册，遇到具体节点时再查阅。

### 9.2 阶段准入前检查清单

| 阶段 | 必须确认 |
|------|----------|
| P1 | 补齐规格第 4.1-4.3、4.8、7、8 章已理解 |
| P2 | 补齐规格第 4.5-4.6、5、6、8、10、15 章已理解 |
| P3 | 补齐规格第 4.7、9 章已理解；P0-1 已修复 |
| P4 | 补齐规格第 10-14 章已理解；P0-3 已修复 |

### 9.3 最容易做错的事情（重申）

1. 让 provider 直接推进状态 → 禁止，只有 daemon 能推进
2. 直接解析 Markdown 原文做 routing → 禁止，必须走 projection
3. 把实现扩展字段混入 canonical 最小字段 → 禁止，走 `_aria` 命名空间
4. 把 OpenSpec 当可选 → 禁止，一期是强约束
5. 让 Superpowers 输出直接成为系统真相源 → 禁止，必须归一化

---

## 10. 一句话总结

方案设计没有根本性错误，文档体系完整，21 个实现单元到 P1-P4 的映射无遗漏。主要风险集中在研发落地层面：`AdapterInput/AdapterOutput` 类型缺失、节点 handler 统一接口未定义、文档间字段定义分散。修复 3 个 P0 问题后，研发可以按计划启动。
