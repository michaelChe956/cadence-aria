# 共享结构化输出协议与 WorkItemPlan 返修路由修复技术方案

## 文档信息

- 文档类型：技术方案
- 版本：v1.0
- 日期：2026-07-10
- 目标分支：`feat-b-0709`
- 案例：`Work Item Plan #workspace_session_0003`
- 适用范围：Story Spec、Design Spec、Work Item、Work Item Plan 的 Workspace Review；Work Item Plan Outline 的人工返修路由与展示

## 1. 背景与问题结论

本案例由三类问题叠加形成。

### 1.1 流式 Provider 协议没有承载结构化结果

非流式 `AdapterOutput` 已包含 `structured_output`，但流式 `ProviderEvent::Completed` 仅包含：

- `full_output`
- `provider_session_id`

因此 Workspace Engine 必须再次从 `full_output` 中查找 `<ARIA_STRUCTURED_OUTPUT>` sentinel，并自行处理 nonce、JSON 与业务 Schema。这造成两套解析职责并存：

- `src/cross_cutting/provider_adapter.rs`：非流式结构化输出解析。
- `src/product/workspace_engine/parsers.rs`：Workspace 流式结构化输出解析。

两套实现的错误表达不同，流式实现使用 `Option` 丢失具体失败原因。

### 1.2 第二轮 reviewer 输出存在封装错误，但业务内容完整

第二轮 reviewer 输出包含完整 JSON，起始标签为：

```text
<ARIA_STRUCTURED_OUTPUT nonce="96aca42f">
```

结束标签却是：

```text
</ARIA_STRUCTURED_OUTPUT>
```

结束标签缺少相同 nonce。当前严格解析器拒绝该 block 是正确的安全行为，但后续降级逻辑把所有解析失败统一转换为：

```text
verdict = needs_human
summary = 需要人工确认
findings = []
work_item_plan_review = null
```

结果是：

- 前端无法展示 reviewer 已给出的结构化 findings。
- 用户看不到真实失败原因。
- Work Item Plan 专用 review 路由信息丢失。

### 1.3 解析失败触发了错误的 Work Item Plan 返修链路

结构化解析失败后，Outline Review 被降级到 `HumanConfirm`。用户选择“请求修改”时，`handle_human_confirm()` 进入通用 `Revision`：

- Prompt 要求完整 Markdown Work Item Plan。
- Artifact 校验要求“计划范围、任务拆分、依赖图、验证计划、执行顺序、风险、追踪关系”和 `[TASK-*]`。

但当前 artifact 实际是 Work Item Plan Outline candidate，应进入 Outline 专用生成与校验链路。最终 author 返回 Outline 风格 artifact，被通用 Work Item Plan Markdown 约束拒绝。

### 1.4 多轮 Review 中仍存在真实计划质量问题

程序问题不代表 reviewer 的业务判断错误：

- 第一轮指出 Repository POST 从同步 `200 Repository` 改为异步 `202 Job` 后，既有测试没有迁移 owner，判断成立。
- 第二轮确认第一轮返修漏掉 `tests/it_core/workspace_ws_integration/part_05.rs` 的直接调用，判断成立。
- 约 30 个 `it_web` / `it_core` 文件使用 `WebRuntime::new_fake`，而当前 `WebAppState::new` 不会自动注入 fake Provider 健康状态。BootstrapGuard 的测试兼容责任需要在计划中明确。

因此需要同时修复：

1. 结构化输出协议与可观测性。
2. Work Item Plan Outline 路由。
3. Outline 返修时的影响面闭合约束。

## 2. 设计目标

### 2.1 必须实现

- Provider Adapter 成为流式结构化输出语法解析的唯一边界。
- Workspace Engine 不再从原始文本中查找 sentinel。
- 保持首尾 nonce 严格一致，不通过放宽解析掩盖 Provider 格式错误。
- 结构化输出失败必须返回类型化错误，而不是 `None`。
- Reviewer 结构化输出格式失败时自动修复一次；修复只允许更正封装，不允许重新审核或改变 verdict/findings。
- 自动修复仍失败时，前端明确展示错误类型与可读 reviewer 原文。
- Work Item Plan Outline 从 Human Confirm 请求修改时必须进入 Outline 专用返修链路。
- Story Spec、Design Spec、Work Item、Work Item Plan 共用结构化输出错误模型。
- 新旧会话恢复必须兼容：旧数据没有诊断字段时按 `None` 处理。

### 2.2 非目标

- 本次不彻底删除文本 sentinel。
- 本次不要求 Codex 与 Claude Code 同时切换到各自原生 JSON Schema API。
- 本次不改变 reviewer 的业务判断标准或严重级别定义。
- 本次不自动修改 `.aria` 中当前案例的业务数据。
- 本次不重构全部 Provider 执行事件模型。
- 本次不修改 Coding Workspace 的 Code Review/Internal PR Review 业务协议；仅保证新增完成事件字段对其兼容。

## 3. 核心架构

### 3.1 分层职责

```text
Provider 原始流
    │
    ▼
Provider Adapter
    ├── 聚合 readable full_output
    ├── 校验 sentinel 起止标签与 nonce
    ├── 解析 JSON 语法
    └── 产出 StructuredOutputState
            │
            ▼
Workspace Engine
    ├── 校验 Review 业务 Schema
    ├── 校验 outline/draft/batch 引用
    ├── 计算 review gate/action
    ├── 必要时触发一次格式修复
    └── 持久化 verdict 或类型化诊断
            │
            ▼
WebSocket / Session Snapshot
    ├── 可信 verdict/findings
    └── structured_output_diagnostic
            │
            ▼
Web UI
    ├── 正常 findings 卡片
    └── 格式失败告警 + reviewer 可读原文
```

语法层与业务层严格分离：

- Provider Adapter 只判断“能否得到合法 JSON Value”。
- Workspace Engine 决定该 Value 是否满足 Review Schema 和业务引用约束。

### 3.2 流式完成结果模型

新增跨 Provider 的完成结果：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructuredOutputState {
    NotRequested,
    Parsed(serde_json::Value),
    Failed(StructuredOutputError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredOutputError {
    pub code: StructuredOutputErrorCode,
    pub message: String,
    pub expected_nonce: Option<String>,
    pub observed_nonce: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuredOutputErrorCode {
    MissingStartTag,
    MissingEndTag,
    MissingEndNonce,
    NonceMismatch,
    InvalidJson,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCompletion {
    pub full_output: String,
    pub structured_output: StructuredOutputState,
    pub provider_session_id: Option<String>,
}
```

`ProviderEvent::Completed` 调整为：

```rust
ProviderEvent::Completed(ProviderCompletion)
```

选择枚举而非两个独立 `Option`，避免出现以下非法组合：

- 同时存在 parsed value 和 error。
- 既没有 value，也没有 error，但无法判断是否要求结构化输出。

### 3.3 输入侧结构化契约

`StreamingProviderInput` 增加：

```rust
pub structured_output_contract: Option<StructuredOutputContract>
```

结构定义：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredOutputContract {
    pub nonce: String,
    pub schema_name: String,
}
```

第一阶段 `schema_name` 只用于诊断和测试，例如：

- `workspace_review`
- `work_item_plan_outline_review`
- `work_item_plan_item_review`
- `work_item_plan_batch_review`

Adapter 不理解业务 Schema，只使用 `nonce` 判断目标 block，避免误取历史会话中较早的 sentinel。

Prompt 构建器必须同时产出：

- 含 sentinel 的 Prompt 文本。
- 与该 sentinel 对应的 `StructuredOutputContract`。

禁止 Adapter 从 Prompt 文本反向猜测期望 nonce。

## 4. Provider Adapter 迁移策略

### 4.1 统一解析器

将 nonce sentinel 语法解析统一放在 `src/cross_cutting/provider_adapter.rs` 或新的聚焦文件：

```text
src/cross_cutting/structured_output.rs
```

该模块负责：

- 按指定 nonce 定位最后一个目标 block。
- 区分缺少结束标签与缺少结束 nonce。
- 拒绝 nonce mismatch。
- 解析 JSON object/array。
- 返回 `StructuredOutputState`。

非流式 `parse_last_structured_output()` 改为复用该模块，避免保留第三套实现。

### 4.2 Codex Provider

Codex 继续保留可读的 `full_output`。收到 turn completed 时：

1. 根据 `StreamingProviderInput.structured_output_contract` 解析结构化结果。
2. 构造 `ProviderCompletion`。
3. 发送 `ProviderEvent::Completed(completion)`。

### 4.3 Claude Code Provider

Claude Code 与 Codex 使用完全相同的完成结果模型。不得在 Claude Code adapter 内做 Work Item Plan verdict 解释。

### 4.4 Fake/Test Provider

Fake Provider 必须支持显式构造三类完成结果：

- `Parsed(value)`。
- `Failed(error)`。
- `NotRequested`。

测试 fixture 不再依赖手写 malformed 文本才能表达所有错误场景，但需保留至少一个端到端 malformed sentinel 测试，证明 Adapter 会生成正确错误码。

### 4.5 兼容 `run_streaming`

旧 `StreamChunk::Done { full_output }` 接口保持不变，以降低与非 Workspace 消费者的耦合。桥接层从 `ProviderCompletion` 中继续取 `full_output`；结构化字段由直接使用 `ProviderSession` 的 Workspace/Coding Engine 消费。

本次不扩大为全仓库 `StreamChunk` 协议迁移。

## 5. Workspace Review 处理

### 5.1 正常路径

Reviewer 完成后：

1. `StructuredOutputState::Parsed(value)` 进入业务解析。
2. 通用 Workspace 调用通用 Review Schema 解析。
3. Work Item Plan 根据 active node 分别调用 Outline/Item/Batch Schema 解析。
4. 业务解析成功后落盘 `ReviewVerdict`，发送 `review_complete`。

Workspace 不再调用 `extract_structured_json(full_output)`。

### 5.2 语法错误自动修复

以下错误允许自动修复一次：

- `MissingEndTag`
- `MissingEndNonce`
- `NonceMismatch`
- `InvalidJson`

自动修复使用同一 reviewer provider 的新一轮调用，并优先续接原生 provider session。修复 Prompt 只包含：

- 原始完整输出。
- 期望 nonce。
- 当前 schema 示例。
- 具体错误码。
- “不得重新审核、不得改变 verdict/findings/summary，只修复 JSON 与 sentinel 封装”的硬约束。

修复次数固定为 1，不做循环重试。

修复节点不创建新的业务 Review Round；仍归属于当前 review timeline node，并在 execution events 中记录一次 `structured_output_repair`。

### 5.3 业务 Schema 错误

JSON 语法合法但业务字段无效时，使用另一组业务错误码：

```rust
pub enum ReviewStructuredOutputErrorCode {
    MissingVerdict,
    InvalidVerdict,
    MalformedFindings,
    InvalidOutlineReference,
    InvalidGenerationRound,
}
```

业务 Schema 错误同样允许一次格式修复，但修复 Prompt 必须明确列出无效字段，不允许 reviewer重新分析候选内容。

### 5.4 最终降级

修复仍失败时创建安全降级 verdict：

```text
verdict = needs_human
review_gate = user_triage_required
findings = []
work_item_plan_review = null
structured_output_diagnostic = 具体错误
```

未通过校验的 JSON 不得进入可信 findings，也不得自动驱动 revise/pass。

同时保留去除 sentinel block 后的 reviewer 可读说明。如果无法安全剥离 malformed block，则保留原始文本但在 UI 中默认折叠。

## 6. 诊断持久化与 WebSocket 契约

`ReviewVerdict` 增加：

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub structured_output_diagnostic: Option<StructuredOutputDiagnostic>
```

诊断结构：

```rust
pub struct StructuredOutputDiagnostic {
    pub code: String,
    pub message: String,
    pub repair_attempted: bool,
    pub repair_succeeded: bool,
}
```

该字段需要贯通：

- timeline node detail verdict。
- session state 恢复。
- `review_complete` WebSocket 消息。
- TypeScript `ReviewVerdict`。
- chat entry/timeline rebuild。

兼容规则：旧 JSON 缺少字段时反序列化为 `None`。

## 7. 前端展示

### 7.1 正常 Review

保持现有 summary、findings、review gate 展示，不改变第一轮正常结果。

### 7.2 结构化输出失败

当 `structured_output_diagnostic` 存在时，Review 卡片顶部显示琥珀色告警：

```text
结构化审核结果解析失败
结束标签缺少 nonce。系统已自动修复 1 次，仍未成功。
```

同时提供：

- “查看 Reviewer 可读说明”区域。
- 默认折叠的“查看原始输出”区域。

不得将原始 JSON 渲染为 findings 卡片，避免用户误以为其已经通过可信校验。

### 7.3 刷新恢复一致性

实时 `review_complete` 与刷新后的 session state 必须生成相同 Review 卡片，避免首次连接与重建后的展示不同。

## 8. Work Item Plan Outline 专用返修路由

### 8.1 路由判定

新增统一判定：

```text
workspace_type == work_item_plan
AND 当前 artifact 是 outline candidate
AND 最近完成的 review node 是 work_item_plan_outline_review
```

满足时，即使 `work_item_plan_review` 因结构化失败为空，Human Confirm 的“请求修改”也必须进入 Outline 专用返修。

### 8.2 复用专用入口

抽取内部方法：

```rust
start_work_item_plan_outline_revision(
    feedback: Option<String>,
    source: WorkItemPlanOutlineRevisionSource,
) -> Result<ReviewDecisionOutcome, String>
```

调用方包括：

- Author Confirm 主动请求重写 Outline。
- Review Decision 要求 revise outline。
- Human Confirm 在 Outline Review 降级后的请求修改。

该入口统一执行：

- 标记最新 artifact rejected。
- session status 恢复为 open。
- stage 转为 running。
- 生成新的 Outline generation round。
- 使用 Work Item Plan Outline Prompt/Parser/Validator。

禁止进入 generic Markdown `Revision`。

### 8.3 恢复判定

路由依据必须来自已持久化数据：

- 当前 artifact 类型。
- timeline node type。

不得依赖仅存在于内存的临时布尔标记，否则服务刷新后会再次走错路由。

## 9. 减少真实计划缺口导致的重复 Review

程序修复不能替代计划质量修复。Outline author revision Prompt 增加影响面闭合约束：

```text
当 reviewer finding 涉及 API 契约、共享状态或测试迁移时：
1. 不得只修改 reviewer 点名的文件。
2. 必须重新检索 src/**、tests/it_web/**、tests/it_core/**、tests/it_product/**、web/src/** 中所有调用方。
3. 必须为每个受影响文件声明 owner，或明确说明无需修改的原因。
4. 必须在返修摘要中列出 searched_scopes、matched_files、owner_mapping。
```

Reviewer 后续轮次优先检查上一轮 finding 是否闭合，再检查返修引入的新影响面。

该 Prompt 改进用于降低遗漏概率，但不宣称能够通过 Prompt 完全替代 reviewer。

## 10. 当前案例的计划内容修正要求

当前 Work Item Plan 下一轮 Outline revision 至少需要：

1. 将 `tests/it_core/workspace_ws_integration/part_05.rs` 纳入 Repository API 契约迁移 owner。
2. 重新检索其他 `it_core` Repository POST 调用方并记录结果。
3. 在 `outline_backend_api_composition` 中明确：
   - `WebRuntime::new_fake` 通过依赖注入获得仅内存的 ready 健康快照。
   - fake override 不写入生产 Provider 健康快照。
   - 真实 runtime 无可用 Provider 时仍返回 `503 provider_bootstrap_blocked`。
4. 明确不要求约 30 个既有测试逐文件设置全局环境变量，避免并行测试污染。

本技术方案不直接改写当前 `.aria` artifact；代码修复后由用户在 UI 中重新触发 Outline revision。

## 11. 测试策略

### 11.1 Cross-cutting 单元测试

- 正确 nonce block 解析为 `Parsed`。
- 缺少结束标签返回 `MissingEndTag`。
- 结束标签缺少 nonce 返回 `MissingEndNonce`。
- nonce 不一致返回 `NonceMismatch`。
- JSON 非法返回 `InvalidJson`。
- 未请求结构化输出返回 `NotRequested`。
- 非流式 Adapter 与流式 Adapter 复用同一解析器。

### 11.2 Provider 测试

- Codex completion 携带 parsed structured output。
- Claude Code completion 携带 typed structured error。
- Fake Provider 可显式发送三种 `StructuredOutputState`。
- 旧 `run_streaming` 仍返回完整 `full_output`。

### 11.3 Workspace Review 表驱动测试

按项目三模块联动规则，通用 Review 行为使用表驱动覆盖：

- Story Spec。
- Design Spec。
- Work Item。
- Work Item Plan Outline。

验证：

- parsed value 正常形成 verdict/findings。
- syntax error 触发一次 repair。
- repair 成功后不产生额外业务 review round。
- repair 失败后 diagnostic 被持久化。
- 刷新恢复后 diagnostic 与可读说明不丢失。

### 11.4 Work Item Plan 专用测试

- Outline Review 解析失败后进入 Human Confirm。
- Human Confirm 请求修改进入 Outline 专用 revision。
- 不进入 generic `Revision`。
- 新 artifact 使用 Outline candidate validator，而不是 Markdown `[TASK-*]` 约束。
- item/batch review 的既有 `plan_reopen_required` 行为保持通过。

### 11.5 前端测试

- 正常 findings 展示不回归。
- diagnostic 告警包含错误原因和 repair 状态。
- 未校验 JSON 不被渲染为可信 findings。
- 实时消息与 session rebuild 展示一致。
- Story/Design/Work Item/Work Item Plan 的通用诊断卡片一致。

## 12. 实施拆分

建议按五个可独立评审的阶段实施：

1. 共享结构化输出模型与解析器。
2. Codex/Claude Code/Fake 流式 completion 协议迁移。
3. Workspace Review 消费、一次 repair 与诊断持久化。
4. Work Item Plan Outline Human Confirm 专用返修路由。
5. 前端诊断展示、恢复一致性与全量回归。

每个阶段必须遵循 TDD，先增加失败测试，再实现最小改动。

## 13. 风险与控制

### 13.1 `ProviderEvent::Completed` 构造点较多

风险：大量测试 fixture 直接构造该事件。

控制：先提供 `ProviderCompletion::plain(full_output, session_id)` 与测试 helper，降低机械迁移噪音；业务测试需要结构化结果时使用显式 constructor。

### 13.2 Repair 可能改变 reviewer 结论

风险：Provider 在修格式时重新审核。

控制：修复 Prompt 禁止改变业务字段；修复成功后比较原始 JSON 候选与修复 JSON 的业务字段。若能够从 malformed block 中提取 JSON candidate，则要求 canonical JSON 等价；不等价时转人工确认。

### 13.3 原生 Provider 能力不一致

风险：未来切换 native structured output 时 Codex/Claude Code 行为不同。

控制：Workspace 只依赖 `StructuredOutputState`；原生能力差异封装在各 Provider Adapter 内。

### 13.4 旧数据恢复

风险：旧 node detail 不含 diagnostic。

控制：所有新增持久化字段必须 `serde(default)`，旧数据恢复为 `None`。

### 13.5 共享 parser 影响多种 Workspace

风险：修复 Work Item Plan 时破坏 Story/Design/Work Item。

控制：通用行为必须表驱动覆盖三种产物 Workspace，并单独覆盖 Work Item Plan 扩展。

## 14. 验收标准

- Workspace Engine 不再直接从 reviewer `full_output` 提取 sentinel JSON。
- 流式与非流式 Provider 使用同一结构化输出语法解析实现。
- 第二轮案例中的缺失结束 nonce 能被识别为 `missing_end_nonce`，而不是无信息的 `needs_human`。
- 格式错误自动修复最多一次，且不增加业务 Review Round。
- 修复失败时 UI 清楚展示错误原因、是否已重试以及 reviewer 可读原文。
- Work Item Plan Outline 在 Human Confirm 请求修改后进入 Outline 专用 revision，不再触发 Markdown heading/`[TASK-*]` 校验错误。
- Story Spec、Design Spec、Work Item、Work Item Plan 的结构化错误恢复测试全部通过。
- 当前案例下一轮 Outline revision 明确补齐 `it_core` Repository POST 调用方和 fake runtime BootstrapGuard owner。
- Rust 标准验证命令通过，且任何 Cargo 命令均不使用 `-j 1`。

