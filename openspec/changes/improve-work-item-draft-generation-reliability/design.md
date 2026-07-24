## Context

Work Item Draft author 当前接收封闭 JSON 字段契约，并在生成后执行 Canonical Contract 与 Outline 的本地校验。实际运行已证明该协议缺少跨字段引用语义：Provider 能输出可解析 JSON，却可能使 `done_when_refs`、`reviewer_check_refs`、`target_contract_refs` 和必需验证命令互相不一致。失败结果保存为 `validation_failed`，前端只因 `can_accept=false` 隐藏接受按钮，未在确认区解释原因。

本变更同时解决生成质量和失败可见性。既有 WebSocket Draft decision 已支持可选 feedback，既有 artifact 已携带 `validator_findings`；应复用这些契约，而不是新增消息类型。

## Goals / Non-Goals

**Goals:**

- 使 Provider 在首次生成时能够构造语义闭合的 Canonical Contract，并对可修复错误执行一次有界自动修复。
- 将 Draft 语义校验失败转化为当前确认区可读、可访问且可操作的反馈。
- 在 Claude Code 上执行临时 Prompt 试运行：使用一至两个脱敏案例、每个案例 20 次，以首次输出的本地校验通过率至少 95% 作为进入人工验证的门槛。
- 保留无效 Draft 不可接受的硬门禁。

**Non-Goals:**

- 不放宽 Canonical Contract、Outline 或写入范围校验来提高通过率。
- 不在每次常规 CI 中执行消耗 Provider 配额的真实评估。
- 不把真实业务 Prompt、Story/Design 内容或完整 Provider 输出写入评估报告。
- 不改变 Story Spec 或 Design Spec 的普通作者确认流程。

## Decisions

### 1. 使用“注册表、投影、自检”三段式 Draft Prompt

Draft Prompt SHALL 先要求 Provider 在脑中建立 contract、criterion、traceability 和可信验证命令的注册表，再从该注册表投影任务、交接和 blocker，最后执行闭合自检。Prompt 以明确集合关系约束：任务完成条件只引用 criterion；需求引用只引用 traceability；reviewer 检查覆盖全部 criterion；blocker 目标只引用输入或输出 contract；必需验证项拥有非空命令。

这比继续增加零散禁止语句更容易与后端 validator 一一对应，也能把本次 20 个错误码归入可验证的规则。JSON Schema 对 `required=true` 与非空 `command` 等静态关系加条件约束；动态引用关系仍由后端 validator 保持唯一事实来源。

**Alternatives considered:**

- 仅扩大 JSON Schema：拒绝。JSON Schema 不能表达候选内动态 ID 集合的全量引用关系。
- 放宽 validator：拒绝。会使错误 contract 进入后续 Work Item 编译和执行阶段。

### 2. 将可信验证命令作为显式 Prompt 输入

生成调用 SHALL 从已确认的 Design/Outline 证据构建“可信验证命令目录”，每项包含命令、工作目录、用途和证据来源。必需验证项只能选用该目录中的命令；目录不足以支持必需验证时，Draft SHALL 走明确 blocker/context 路径，而不是输出 `required=true` 且 `command=null` 的候选。

这样避免 Provider 根据 WorkItem 类型猜测 `cargo`、`pnpm` 或 `node` 命令，并将“命令缺失”转化为可处理的上下文问题。

### 3. 校验失败后只自动修复一次

后端在 Provider 输出已成功解析、但本地语义校验失败时，格式化全部 `validator_findings` 并发起一次同一 Draft 的自动修复。修复 Prompt 保留原始上下文、候选和错误码；修复结果必须重新经过完整解析和校验。

- 修复成功：仅将有效 Draft 作为当前候选，并在 artifact/timeline 中保留自动修复诊断。
- 修复失败：保存失败候选和 findings，进入人工可见的重写/暂停状态。
- 每次生成或人工重写最多一次自动修复；不得递归重试。

**Alternatives considered:**

- 只在用户点击重写后回传错误：拒绝。无效输出仍会先中断用户流程，无法提高首次交付体验。
- 无限自动重试：拒绝。成本、延迟和失败定位都会失控。

### 4. 失败反馈置于确认区，并由服务端统一注入重写上下文

Work Item Draft 确认区 SHALL 在 `can_accept=false` 时显示共享的校验失败提示：错误总数、前三条摘要、可展开的完整列表、无障碍告警语义和“根据校验错误重写”动作。Artifact 保留完整 JSON 和 findings 作为深度查看入口。

重写时由后端从当前 artifact 统一合并用户 feedback 与 validator findings，写入下一轮 Provider 上下文。服务端注入保证 Chat 输入区、staged panel 和未来客户端得到一致行为，前端不负责拼接权威错误文本。

### 5. 确定性回归与临时 Claude Code Prompt 试运行

质量验证分两层：

1. 常规 CI 的确定性单元、集成和前端测试，覆盖每个语义规则、自动修复边界和用户提示。
2. 仅在操作者明确授权后，对 Claude Code 执行一至两个脱敏案例的临时试运行；每个案例固定执行 20 次，以首次输出通过既有本地 Validator 为成功。一个案例须至少 19/20 成功；两个案例合计须至少 38/40 成功。达标后立即停止自动化调优，转入人工验证。

试运行不新增产品模块、CLI、持久化报告、版本控制语料或 CI 步骤。若不达标，只改 Prompt 文案这一变量，先重跑确定性回归，再重跑同一组临时案例；不得通过放宽 Validator、Schema 或接受门禁提高成功率。

## Risks / Trade-offs

- [真实评估成本和 Provider 波动] → 真实评估显式触发，按 Provider 单独统计，记录 Prompt 版本；以两轮独立结果而非单次结果决定发布。
- [Prompt 增加上下文导致输出质量下降] → 使用紧凑注册表和错误码映射；为 Prompt 长度保留现有上下文预算测试。
- [自动修复掩盖系统问题] → 限制为一次，持久化诊断，并将失败码纳入基准语料和评估报表。
- [命令目录缺失阻塞 Draft] → 明确呈现 context/blocker，而非生成无法确认的虚假成功 Draft。
- [错误列表过长影响确认区可读性] → 默认显示前三项并提供完整展开；Artifact 仍提供原始细节。

## Migration Plan

1. 先发布保持现有 validator 的 Prompt、Schema、自动修复和确定性测试。
2. 再发布确认区反馈；已有持久化 artifact 没有 findings 时使用通用失败提示，保持向后兼容。
3. 建立基线评估并完成调优；未达到门槛时不将候选 Prompt 设为默认版本。
4. 回滚时恢复前一 Prompt 版本并关闭自动修复；validator 和失败提示保留，防止无效 Draft 被接受或静默失败。
