# Work Item Plan 单会话最少拆分技术方案

## 文档信息

- 文档类型：技术方案
- 日期：2026-07-16
- 版本：v1.0
- 适用范围：Work Item Plan Outline 生成、校验、审核以及后续串行/批量 Draft 生成
- 目标分支：`feat-b-0715`

## 一、背景

当前 Work Item Plan Outline 将 `estimated_context_tokens < 20k` 作为单个 coding session 可完成的硬标准。该约束同时存在于 Outline 生成 Prompt、JSON Schema、后端 Validator 和 Reviewer Prompt 中，因此 Provider 必须把所有 Outline 压到 20k 以下，超过时只能继续拆分。

现有实际样本中，一个跨后端、前端与集成测试的 Issue 最终生成了 10 个 Work Item、13 份 Draft 记录。10 个 Outline 的估算值平均约为 15.25k，并明显集中在 20k 上限附近。与此同时，最终 Work Item 的默认 `target_context_k` 为 `30-50`，说明 Outline 硬限制与后续 coding session 的既有目标上下文范围不一致。

问题的本质不是需要规定一个理想 Work Item 数量，而是当前拆分目标错误地偏向“每项尽量小”。正确目标应为：在每个 Work Item 能由单个 coding session 可靠完成的约束下，使 Work Item 数量最少。

## 二、目标与非目标

### 2.1 目标

1. 将单 Work Item 的正常预算范围调整为不超过 40k，允许 Reviewer 确认后接受 40k–50k 的 Work Item，并以 50k 作为硬上限。
2. 将 Outline Author 的生成目标改为“最大内聚、最少拆分”，避免把实现步骤、测试或同一目标下的相邻工作机械拆成独立 Outline。
3. 让 Reviewer 同时检查任务过大与过度拆分，并能要求合并不必要的 Outline。
4. 保持逐个生成和批量生成两套 Draft 流程不变，两者消费同一份已确认 Outline。
5. 对齐结构化 Work Item Plan 与 Markdown Work Item Plan 两条生成路径的拆分标准。

### 2.2 非目标

1. 不迁移、不重算、不修改任何已有 Outline、Draft、Work Item 或历史 Artifact。
2. 不自动合并已经持久化的旧 Work Item Plan。
3. 不改变逐个生成和批量生成按钮的产品语义、状态机或交互流程。
4. 不新增 Outline、Draft 或 Work Item 数据字段。
5. 不以固定 Work Item 数量作为生成或验收目标。

## 三、核心约束

Work Item Plan 的优化目标定义为：

> 在所有 Work Item 均能由单个 coding session 可靠完成的前提下，最小化 Work Item 数量。

每个 Work Item 必须同时满足以下条件：

- 具有单一、内聚且可验收的目标。
- 所需依赖在 session 开始时已经具备，或能由同一 session 内的前序子步骤产生。
- 编码、测试、Reviewer 返修与最终验证能够在同一 session 内闭环。
- 写入范围、依赖交接和上下文代理指标未超过现有安全边界。
- `estimated_context_tokens` 不超过 50k。

## 四、预算规则

| 预算范围 | 处理方式 |
|---|---|
| `1..=40000` | 正常范围，Schema 与 Validator 通过 |
| `40001..=50000` | 合法范围，Reviewer 必须确认单 session 可完成 |
| `>50000` | 硬拒绝，必须继续拆分 |
| 缺失或为 `0` | 保持现有行为，视为无效 Outline |

40k 是软警戒线，不应由确定性 Validator 返回阻断错误。40k–50k 的判断涉及目标内聚性、写入范围、验证复杂度和外部中断点，应由 Reviewer 结合完整 Outline 语义判断。

50k 是新生成和新返修 Outline 的硬上限。历史数据不应用新阈值重新校验或回写。

## 五、Outline Author 最少拆分规则

Outline Author 必须采用“先合并、后证明必须拆”的策略。

### 5.1 必须优先合并

满足以下条件的相邻工作应优先生成一个 Outline：

- 服务于同一个用户可见结果或同一个技术目标。
- 写入范围相同、重叠，或存在紧密的直接生产消费关系。
- 合并后依赖仍可在同一 session 内满足。
- 合并后可以通过同一组测试和验收结果完成闭环。
- 合并后的 `estimated_context_tokens` 不超过 50k。
- 不违反用户显式选择的拆分选项。

API、数据层、UI、测试、迁移脚本或若干 TDD 子步骤本身不能作为独立拆分理由；只要它们共同完成一个内聚目标且单 session 可闭环，就可以作为同一个 Work Item 的子步骤。

### 5.2 允许拆分

只有出现以下情况时才允许拆成多个 Outline：

- 合并后预计超过 50k。
- 用户选项明确要求拆分，例如启用了前后端强制拆分。
- 存在必须等待用户决策、外部资源、权限或前序执行结果的中断点。
- 两部分需要独立回滚或独立验收，无法在同一 session 内安全闭环。
- 合并后写入范围、依赖交接或验证复杂度超过现有上下文代理指标。
- 两部分目标缺乏内聚性，合并会形成跨多个独立结果的 Issue 级任务。

## 六、Reviewer 双向审核

Reviewer 必须同时审核“是否过大”和“是否过度拆分”。

### 6.1 过大检查

- 不超过 40k 时按现有单 session 适配规则审核。
- 40k–50k 时必须明确判断编码、测试、返修和验证是否能在单 session 内完成。
- 超过 50k 时必须返回返修，要求继续拆分。

### 6.2 过度拆分检查

当两个或多个 Outline 可以在不违反显式选项、50k 上限和安全边界的情况下合并时，Reviewer 应返回 `revise`，并产生 `outline_unnecessary_split` finding。该 finding 至少应包含可合并的 Outline ID，并说明合并依据。

`outline_unnecessary_split` 必须路由回 Outline 返修，不得进入 Draft 局部返修。只有 Outline 通过过大检查与过度拆分检查后，才允许进入人工确认和 Draft 生成阶段。

## 七、流程与组件改动

### 7.1 结构化 Work Item Plan 路径

1. `src/product/work_item_split_engine/prompts.rs`
   - 将 `<20k` 调整为 `<=50k`。
   - 写明 40k 软警戒线和 50k 硬上限。
   - 增加最大内聚、最少拆分、合并优先和允许拆分条件。
2. `src/product/work_item_split_engine/schema.rs`
   - 将 `estimated_context_tokens.maximum` 从 `19999` 调整为 `50000`。
3. `src/product/work_item_split_validator/outline.rs`
   - 接受 `1..=50000`。
   - `>50000` 继续返回 `outline_exceeds_single_session_budget`。
   - 缺失或为 `0` 的行为保持不变。

### 7.2 Reviewer 路径

`src/product/workspace_engine/prompts/review.rs` 需要增加：

- 40k–50k 单 session 可完成性审核。
- Outline 合并可行性审核。
- `outline_unnecessary_split` finding 约定。
- 超过 50k 时继续拆分的硬要求。

现有 Review 结构和 finding 展示可继续复用，不新增前端专用交互。

### 7.3 Markdown Work Item Plan 路径

`src/web/workspace_context/prompts.rs` 中 Markdown Work Item Plan 与 Work Item 的 20k 文案必须同步调整，保证普通 Workspace 路径与结构化 Split Engine 路径采用相同标准。

Story Spec 与 Design Spec 的生成边界不变，仅补充回归测试确认其 Prompt 未被本次调整影响。

## 八、数据流与失败处理

新生成流程保持现有阶段，只调整 Outline 质量门禁：

1. 用户提交 Work Item Plan 生成请求与拆分选项。
2. Outline Author 以最大内聚、最少拆分为目标生成候选。
3. JSON Schema 拒绝缺失、为零或超过 50k 的预算。
4. Validator 执行确定性 ID、追踪、scope、依赖和 50k 硬上限检查。
5. Reviewer 执行 40k–50k 单 session 检查和过度拆分检查。
6. Reviewer 通过后进入人工 Outline 确认。
7. 已确认 Outline 根据用户按钮进入逐个 Draft 或批量 Draft 生成。

失败处理规则：

- Schema 或 Validator 失败：停留在 Outline Author 返修路径。
- Reviewer 判断过大：返回 Outline 返修并要求继续拆分。
- Reviewer 判断过度拆分：返回 `outline_unnecessary_split` 并要求合并。
- Outline 未通过前不得创建新的 Draft。
- 旧 Outline、旧 Draft 和旧 Work Item 不受任何失败处理或后台重校验影响。

## 九、兼容性与历史数据

本方案采用纯前向兼容策略：

- 新规则仅作用于方案上线后的新 Outline 生成和用户主动触发的 Outline 返修。
- 已持久化的 Outline 不重新估算、不重新审核、不自动合并。
- 已生成的 Draft 和已编译 Work Item 保持原样。
- 已确认或已完成的 Work Item Plan 状态不变化。
- 不新增迁移脚本，不在应用启动时扫描或修改历史数据。

## 十、测试设计

### 10.1 Prompt 与 Schema

- Outline Author Prompt 包含最少拆分、合并优先、40k 软线和 50k 硬线。
- Draft Prompt 不重新引入 20k 限制。
- JSON Schema 的最大值为 `50000`。
- Markdown Work Item Plan 与结构化 Work Item Plan 的阈值一致。

### 10.2 Validator

- `40000` 通过。
- `40001` 通过，留给 Reviewer 判断。
- `50000` 通过。
- `50001` 返回 `outline_exceeds_single_session_budget`。
- 缺失或为 `0` 继续返回预算必填错误。

### 10.3 Reviewer

- 两个目标相同、范围可合并且各约 18k 的 Outline 应被要求合并。
- 合并后超过 50k 时允许保留拆分。
- 启用显式前后端强制拆分时允许保留拆分。
- 存在外部中断点、独立回滚或独立验收边界时允许保留拆分。
- 40k–50k Outline 必须在 Reviewer Prompt 中进行单 session 可完成性判断。

### 10.4 流程回归

- 串行 Draft 和批量 Draft 使用同一份已确认 Outline。
- `outline_unnecessary_split` 路由到 Outline 返修，而不是 Draft 返修。
- Story Spec 与 Design Spec Prompt 行为不变。
- 历史 Outline、Draft 和 Work Item 读取不受影响。

## 十一、验收标准

1. 所有新生成路径都以 40k 为软警戒线、50k 为硬上限。
2. Outline Author 明确以单 session 可完成前提下的最少拆分为目标。
3. Reviewer 能拒绝超过 50k 的 Outline，也能拒绝不必要的碎片化拆分。
4. 两条 Work Item Plan 生成路径没有阈值或语义漂移。
5. 串行和批量 Draft 流程不发生行为变化。
6. 不迁移、不重算、不修改任何历史数据。
