# CodingWorkspace 材料驱动 Prompt 协议技术方案

## 文档信息

- 文档类型：技术方案
- 版本：v1.0
- 日期：2026-07-07
- 范围：Coding Workspace 的 coder、code reviewer、WorkItemGroup GroupFinalReview prompt
- 目标：去除平台固定 prompt 中的语言、包管理器、构建工具和测试框架硬编码，改为由 Work Item Draft、Final Compile、Verification Plan、EvaluationContextPack 等任务材料驱动执行与审查。

## 背景

当前 Coding Workspace 的 coder prompt 中存在平台层硬编码的项目特定提示，例如 pnpm、node_modules、tsc、vitest、.rs、crate 等。这类内容对当前 Rust/前端项目有帮助，但会污染 Java、Python、Go、Node、混合仓库等其他项目场景，不符合 Aria 作为全语言平台的目标。

同时，Coding Workspace 已经具备较完整的任务材料链路：

- Work Item Draft author 生成 `implementation_context`、`handoff_summary`、write scopes、forbidden scopes、verification plan。
- Final Compile 将 accepted draft 固化为 `LifecycleWorkItemRecord` 与 `VerificationPlan`。
- Coding prompt 已能注入 `Final Compile Work Item`、`Verification Plan`、`Source Draft Supplement`、`EvaluationContextPack`、git diff 等上下文。

因此，平台不应继续维护语言模板库，而应通过 prompt 明确要求 provider 从这些材料中提取本次任务自己的执行和审查清单。

## 设计目标

1. 平台固定 prompt 不写死 Rust、Java、Node、pnpm、Maven、Gradle、Cargo、crate、source set、node_modules 等技术栈知识。
2. Coder 必须通过 prompt 被要求先从 Work Item / Final Compile / Verification Plan 中提取执行清单，再修改代码。
3. CodeReviewer 必须通过 prompt 被要求从同一批任务材料中提取审查清单，再审查 diff。
4. 单个 WorkItem 不再运行组级最终审查；CodeReviewer approve 后由 ReviewRequest commit/push 完成。
5. WorkItemGroup 的 GroupFinalReview 必须从 ReviewRequest、Completed Units、handoff、EvaluationContextPack、完整 diff 中提取整组审查清单。
6. 技术栈特异要求只能来自 Draft、Final Compile、VerificationPlan、EvaluationContextPack、项目规则、仓库事实或用户补充上下文。
7. Prompt 输出字段集合尽量稳定，避免扩大 parser、前端和状态机联动范围；但单 WorkItem InternalPrReview 入口、命名和 UI 不做历史兼容。

## 非目标

- 不新增后端语言模板库。
- 不由后端根据 Java/Rust/Node 等语言生成具体自检内容。
- 不在本方案中重构 Work Item Draft schema。
- 不为单 WorkItem 保留 InternalPrReview prompt、provider run、timeline 或 UI 入口。
- WorkItemGroup 最终审查使用新的 GroupFinalReview 语义；实现时可以迁移既有字段集合，但不得以历史兼容为理由保留 InternalPrReview 对外命名或单 WorkItem 分支。
- 不改变 VerificationPlan 作为验证命令来源的职责。
- 不改变 provider 真实 adapter 接入方式。

## 总体原则

平台固定 prompt 只允许表达语言无关的过程约束：

- 先读任务材料，再执行或审查。
- 结论必须基于证据。
- Required 验证命令失败不能视为完成。
- 没有测试被执行不能直接视为测试覆盖。
- diff 必须遵守 Exclusive / Forbidden Write Scopes。
- 如果材料不足，不得臆造技术栈假设，必须报告不确定性。

平台固定 prompt 禁止表达具体技术栈规则：

- 禁止固定出现 `cargo`、`mvn`、`gradle`、`pnpm` 等命令。
- 禁止固定出现 `.rs`、`.java`、`crate`、`source set`、`node_modules` 等项目约定。
- 禁止固定推荐任何包管理器、构建工具或测试框架。

上述词汇只有在 Work Item markdown、Source Draft Supplement、VerificationPlan、EvaluationContextPack、diff、项目规则或用户补充上下文中自然出现时，才允许出现在最终 prompt 中。

## Coder Prompt 设计

### Full Prompt 结构

保留现有元数据：

- Project
- Issue
- Work Item
- Attempt
- Branch
- Worktree Path

保留 `验证命令` 区块，但说明其来源和执行责任：

```text
验证命令:
以下命令来自当前 Work Item 的 Verification Plan。你必须按 Work Item 要求和命令 required 标记执行；如果无法执行，必须在最终报告中说明阻塞原因。
- {command}
```

保留 `已确认 Work Item` 原文：

示例形态：

    已确认 Work Item:
    ````markdown
    {work_item_markdown}
    ````

新增语言无关的 `Coder 执行协议`：

```text
Coder 执行协议:
- 在修改代码前，必须先阅读“已确认 Work Item”，并从其中提取本次任务的执行清单。
- 执行清单必须覆盖：实现目标、允许修改范围、禁止修改范围、TDD/测试要求、依赖初始化或环境诊断要求、验证命令与执行顺序、完成前自检要求、handoff 中要求交付给下游的契约。
- 如果 Work Item、Source Draft Supplement、Verification Plan 已明确给出某项要求，必须按其内容执行。
- 如果执行材料没有给出语言、构建系统、包管理器或测试框架相关要求，不得臆造具体技术栈命令。
- 需要判断环境或依赖问题时，必须优先根据 Work Item、Verification Plan、仓库文件和项目规则判断。
- 如果判断依据不足，必须在最终报告中说明“不足以确定”，并列出需要人工确认的问题。
- 不得用平台默认技术栈假设替代 Work Item 内容。
```

新增语言无关的 `完成报告要求`：

```text
完成报告要求:
- 先列出你从 Work Item / Final Compile / Verification Plan 提取出的执行清单。
- 列出实际修改文件。
- 列出实际执行的验证命令。
- 粘贴每条验证命令的完整输出。
- 报告 git diff --stat。
- 明确说明是否触碰 Forbidden Write Scopes。
- 如果测试命令显示没有测试被执行，不能直接视为已覆盖；必须说明处理方式或风险。
- 如果某项要求无法执行，说明阻塞原因、已尝试的诊断步骤和需要人工确认的内容。
```

### Delta Prompt 结构

Delta prompt 不重新注入完整 Work Item，但必须要求 provider 重新核对既有材料和新增返修要求：

```text
Coder 增量执行协议:
- 继续以本会话中的“已确认 Work Item”和 Verification Plan 作为任务来源。
- 在继续修改前，必须重新核对本轮返修要求、补充上下文和原 Work Item 中的执行要求。
- 若存在人工返修意见，人工返修意见优先级最高；当人工返修意见与 reviewer findings、原 Work Item 或既有上下文冲突时，优先遵循人工返修意见，并在最终报告说明冲突和取舍。
- 若没有人工返修意见，但本轮 reviewer findings 与原 Work Item 冲突，优先遵循更具体、更新的本轮 reviewer findings；同时在最终报告说明冲突和取舍。
- 不得引入平台默认技术栈假设；语言、构建系统、包管理器、测试框架相关动作必须来自 Work Item、Verification Plan、仓库文件或项目规则。
- 如果判断依据不足，必须在最终报告中说明“不足以确定”，并列出需要人工确认的问题。
```

### 删除或替换的旧片段

删除旧 `dependency_bootstrap_guidance()` 中的 pnpm、node_modules、tsc、vitest 相关硬编码。

删除旧 `coding_self_check_contract()` 中的 `.rs`、crate 挂载相关硬编码。

替换为：

- `coding_execution_protocol()`
- `coding_completion_report_contract()`
- `coding_delta_execution_protocol()`

这些函数只输出语言无关内容。

## CodeReviewer Prompt 设计

### 当前职责

CodeReviewer 只分析当前变更 diff，不修改代码、不执行写操作。其输出 schema 保持：

```json
{"verdict":"approve|request_changes|blocked","summary":"...","findings":[...]}
```

### 新增审查协议

在现有“代码规范”前或后增加材料驱动审查协议：

```text
CodeReviewer 审查协议:
- 只分析当前变更 diff，不修改代码、不执行写操作。
- 在给出 verdict 前，必须从“原始需求上下文”和 EvaluationContextPack 中提取本次任务的审查清单。
- 审查清单必须覆盖：实现目标、允许修改范围、禁止修改范围、TDD/测试要求、验证命令与证据、完成前自检要求、handoff 承诺、需求/设计追踪关系。
- 必须审查 diff 是否满足 Work Item 的实现目标、写入范围、禁止范围、验证计划、自检要求和 handoff 承诺。
- 如果 coder 报告或 EvaluationContextPack 中缺少 required 验证命令的执行证据，必须作为 finding 记录；若该证据是完成本 Work Item 的必要条件，verdict 应为 request_changes 或 blocked。
- 如果测试输出显示没有实际测试被执行，不能把它当作有效覆盖；必须结合 Work Item 要求判断是否需要返修。
- 不得提出执行材料之外的技术栈默认要求。
- findings 必须包含 severity、file_path、line、message、required_action、source_stage=code_review。
```

### 保持不变

- 仍注入 Work Item markdown。
- 仍注入 EvaluationContextPack。
- 仍注入 git diff。
- 仍只输出 JSON。
- 不改变 parser schema。

## WorkItemGroup GroupFinalReview Prompt 设计

### 当前职责

GroupFinalReview provider 只在 `CodingAttemptScope::WorkItemGroup` 的所有 coding units 完成后使用，对整组 PR 做最终功能审查。单个 WorkItem scope 不生成 GroupFinalReview。

输出 schema 保持整组 PR 审查需要的字段集合。实现上可以从现有 `InternalPrReview` parser 迁移，但命名和语义应收敛为 `GroupFinalReview`：

```json
{
  "verdict": "approve|request_changes|blocked",
  "summary": "...",
  "findings": [],
  "impact_scope": [],
  "pr_description": "...",
  "commit_message_suggestion": "..."
}
```

### 新增整组 PR 级审查协议

```text
WorkItemGroup GroupFinalReview 审查协议:
- 你必须从 Completed Units、unit handoff、EvaluationContextPack 和完整 diff 中提取整组审查清单。
- 必须确认每个 completed unit 的 handoff 承诺是否体现在最终 diff 或最终报告中。
- 必须检查依赖 handoff 是否断裂：上游 unit 承诺的 API、状态、文件、测试证据是否被下游正确消费。
- 必须检查整组 diff 是否越过任何 unit 的 Forbidden Write Scopes。
- 如果某个 unit 的验证证据缺失、handoff 未闭环、或最终 PR 描述遗漏关键影响，必须 request_changes 或 blocked。
- 如果 ReviewRequest 已 push 的 commit 与 completed units、diff 或验证证据不一致，必须 request_changes 或 blocked。
- impact_scope、pr_description、commit_message_suggestion 必须基于实际 diff、completed units 和 handoff，不得编造未实现内容。
- 不得用平台默认技术栈假设替代 unit handoff 或 Work Item 内容。
- findings 必须包含 source_stage=group_final_review。
```

### 保持不变

- 仍注入 Completed Units。
- 仍注入 EvaluationContextPack。
- 仍注入完整 git diff。
- 仍注入 ReviewRequest id、remote、commit。
- 仍只输出 JSON。
- 字段集合保持稳定，但类型、函数、timeline 和 UI 文案不再保留单 WorkItem InternalPrReview 语义。
- 单 WorkItem scope 不生成此 prompt，不创建 InternalPrReview provider run。

## 共享 Prompt 片段

为避免 coder、reviewer、group final reviewer 之间规则漂移，建议新增共享 helper：

```text
no_default_stack_assumption_contract()
```

内容：

```text
不得用平台默认技术栈假设替代任务材料。语言、构建系统、包管理器、测试框架、依赖初始化和模块接入要求，必须来自 Work Item、Source Draft Supplement、Verification Plan、EvaluationContextPack、项目规则、仓库文件事实或用户补充上下文。若材料不足，必须报告不确定性，不得臆造具体命令或工具。
```

建议按角色拆分 helper：

- `coding_execution_protocol()`
- `coding_delta_execution_protocol()`
- `coding_completion_report_contract()`
- `code_review_material_protocol()`
- `group_final_review_material_protocol()`

## 数据流

```text
Work Item Draft author
  生成项目特定 implementation_context / verification_plan / handoff_summary / scopes
        ↓
Final Compile
  原样固化为 LifecycleWorkItemRecord + VerificationPlan
        ↓
CodingExecutionContext
  读取 Final Compile Work Item、Source Draft Supplement、VerificationPlan
        ↓
Coder Prompt
  要求 coder 从材料提取执行清单并执行
        ↓
CodeReviewer Prompt
  要求 reviewer 从材料和 diff 提取审查清单并审查
        ↓
WorkItemGroup GroupFinalReview Prompt
  仅在 WorkItemGroup 全部 units 完成后运行，要求 reviewer 从 Completed Units、handoff、ReviewRequest、diff 提取整组审查清单
```

## 示例：平台固定 Prompt 中不应出现的内容

以下内容不得由平台固定模板输出：

- `cargo fmt --check`
- `cargo test`
- `mvn test`
- `gradle check`
- `pnpm install`
- `node_modules`
- `.rs`
- `.java`
- `crate`
- `source set`

如果这些内容来自 Work Item markdown 或 EvaluationContextPack，则允许自然出现在 prompt 中。

## 测试方案

### Coder Prompt 单测

- Full prompt 包含“从 Work Item / Final Compile / Verification Plan 提取出的执行清单”要求。
- Full prompt 不包含 pnpm、node_modules、tsc、vitest、cargo、crate、.rs 等固定技术栈词。
- 当 Work Item markdown 自身包含 Rust 要求时，prompt 保留该内容。
- 当 Work Item markdown 自身包含 Java/Maven 要求时，prompt 保留该内容。
- Delta prompt 包含重新核对返修要求、补充上下文和原 Work Item 的要求。

### CodeReviewer Prompt 单测

- Prompt 包含“从原始需求上下文和 EvaluationContextPack 提取审查清单”。
- Prompt 要求审查 required 验证命令证据。
- Prompt 不包含平台固定技术栈词。
- Prompt 保持 CodeReviewer JSON schema 稳定。

### WorkItemGroup GroupFinalReview Prompt 单测

- Prompt 包含 Completed Units、handoff 闭环、依赖 handoff 断裂检查。
- Prompt 要求检查整组 diff 是否越过 unit forbidden scopes。
- Prompt 要求审查 ReviewRequest commit、完整 diff、verification evidence、handoff 和 forbidden scope。
- Prompt 不包含平台固定技术栈词。
- Prompt 保持整组审查字段集合稳定。
- 单 WorkItem scope 不应生成 InternalPrReview prompt；WorkItemGroup 才能生成 GroupFinalReview prompt。

## 实施顺序建议

1. 新增共享语言无关 prompt helper。
2. 替换 coder prompt 的旧依赖初始化和自检硬编码。
3. 更新 coder prompt 单测。
4. 更新 CodeReviewer prompt，补充材料驱动审查协议。
5. 更新 WorkItemGroup GroupFinalReview prompt，补充整组 handoff 审查协议。
6. 删除单 WorkItem scope 的 InternalPrReview prompt 生成路径，并把组级最终审查命名迁移到 GroupFinalReview。
7. 补齐 prompt 单测，确认固定模板不再包含技术栈硬编码。
8. 运行 Rust 标准验证命令。

## 风险与处理

### 风险：上游 Draft 没有生成足够项目特定要求

处理：coder prompt 要求报告“不足以确定”，不得臆造。后续可单独增强 Draft author prompt，让项目特定要求更多来自 Draft。

### 风险：Reviewer 过度依赖 coder 报告

处理：CodeReviewer 和 WorkItemGroup GroupFinalReview 必须同时读取 Work Item、EvaluationContextPack 和 diff。coder 报告只是证据之一，不是唯一证据。

### 风险：删除硬编码后提示变弱

处理：保留语言无关底线，包括 required 命令证据、0 tests 风险、write scope、forbidden scope、handoff 承诺、证据可追溯。

### 风险：测试用“禁止词”误伤 Work Item 原文

处理：单测应区分固定模板和注入材料。可用空 Work Item 或 neutral Work Item 测固定模板无技术栈词，再用带技术栈内容的 Work Item 测原文保留。

## 验收标准

- Coder prompt 固定模板不再输出语言、包管理器、构建工具、测试框架硬编码。
- CodeReviewer prompt 固定模板不再输出技术栈默认审查要求。
- WorkItemGroup GroupFinalReview prompt 固定模板不再输出技术栈默认审查要求。
- 单 WorkItem scope 不再生成 InternalPrReview prompt。
- 技术栈特异内容只能来自任务材料或仓库事实。
- JSON 输出字段集合按新链路保持稳定。
- Prompt 单测覆盖 coder、code reviewer、WorkItemGroup GroupFinalReview。
