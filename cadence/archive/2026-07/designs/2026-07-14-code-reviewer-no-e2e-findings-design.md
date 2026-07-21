# Code Reviewer 禁止 E2E Findings 设计

## 目标

调整 Coding Workspace 的 Reviewer Prompt，使单 Work Item `CodeReviewer` 与整组 `GroupFinalReview` 都不得把 E2E、Playwright 或浏览器自动化测试转化为 finding、否决理由或 Coder 返修要求，同时保留 Reviewer 对其他测试与验证方式的正常审查能力。

## 问题背景

当前 Reviewer Prompt 要求模型从 Work Item、EvaluationContextPack、验证证据和设计追踪关系中提取审查清单，但没有排除 E2E 或 Playwright。上游 Design Spec 包含全局 E2E 验证策略时，Reviewer 可能把该策略扩大为当前 Coder 的返修要求，导致 Coder 执行浏览器测试、修改端口、安装浏览器并产生与当前编码任务无关的副作用。

## 设计范围

本次只调整 Reviewer Prompt 和对应 Prompt 契约测试：

- 新增一个共享的 Reviewer 测试边界 Prompt 片段。
- 将该片段分别注入 `build_code_review_prompt()` 与 `build_group_internal_pr_review_prompt()`。
- 补充 `src/product/coding_workspace_engine/tests/parser_prompt.rs` 中的 Prompt 契约测试。

本次明确不做：

- 不修改 Coder Prompt 或 Coder 执行优先级。
- 不把 Verification Plan 变成 Reviewer 可提出测试要求的白名单。
- 不修改 Review JSON schema、parser、报告落盘、rework 或 gate 流程。
- 不增加 E2E 关键字检测、过滤器或运行时特殊拦截。
- 不假设被开发项目使用 Rust、React、Cargo、Vite、Vitest 或其他固定技术栈。

## Reviewer 行为规则

`CodeReviewer` 与 `GroupFinalReview` 使用同一语义边界：

1. Reviewer 不得创建以新增、执行、补充、修复、配置或安装以下内容为目的的 finding：
   - E2E 测试或端到端测试。
   - Playwright 测试。
   - 浏览器自动化测试。
   - 为运行上述测试而进行的浏览器、Chrome、Chromium 或同类运行环境安装与配置。
2. Reviewer 不得因为 E2E、Playwright 或浏览器自动化测试缺失、失败或缺少证据而给出 `request_changes` 或 `blocked`。
3. 即使 Work Item、Design Spec、Verification Plan、handoff 或 EvaluationContextPack 提到上述内容，Reviewer 也不得将其转换成 finding、否决理由或 Coder 返修要求。
4. Reviewer 仍可根据需求、当前 diff、仓库事实、测试证据与代码风险提出其他合理验证要求，包括但不限于：
   - 单元测试。
   - 非浏览器自动化的集成测试。
   - 编译或构建验证。
   - 类型检查。
   - 静态分析、格式检查或 lint。
   - 与当前项目技术栈相符的其他非 E2E 验证。
5. Reviewer 提出测试要求时不受当前 Verification Plan 已列命令的严格限制，但测试框架、命令和技术栈判断必须来自任务材料、仓库事实或项目规则，不得凭平台默认假设生成。

## Prompt 组织方式

在 `prompts.rs` 中新增一个复用的 Reviewer 测试边界协议函数，由 `prompts.rs::build_code_review_prompt()` 和 `internal_pr_review.rs::build_group_internal_pr_review_prompt()` 分别注入。协议使用测试类别描述允许范围，只对明确禁止的 E2E/Playwright/浏览器自动化类别进行点名，不写入任何项目专属语言或工具命令。

复用同一协议可以避免两个 Reviewer Prompt 日后出现规则漂移，同时不影响两种 Reviewer 各自已有的审查职责。

## 测试设计

这里的自动化测试验证的是“平台生成的 Reviewer Prompt 是否正确包含已确认规则”，不声称能够确定性证明外部 AI Provider 在所有情况下都会服从 Prompt。

### Prompt 契约测试

在 `parser_prompt.rs` 中验证共享 Reviewer 测试边界协议明确表达：

- CodeReviewer 与 GroupFinalReview 禁止 E2E、Playwright 和浏览器自动化 findings。
- E2E 缺失或失败不能成为 `request_changes` 或 `blocked` 的依据。
- 单元测试、非浏览器集成测试、编译、类型检查和静态分析仍被允许。
- Reviewer 的测试建议不被限制为 Verification Plan 已列命令。
- 规则没有引入当前平台之外的固定技术栈假设。

### Prompt 接线测试

分别构造 CodeReviewer 和 GroupFinalReview Prompt，确认两者都包含共享规则；Coder Prompt 不包含该 Reviewer 专属规则。

### 当前平台验证

Prompt 构造逻辑位于 Rust 后端，因此本次变更的定向与完整验证运行相关 Rust 单元测试，以及仓库标准 Rust 格式、Clippy、编译和测试命令。前端代码不在本次变更范围，Vitest 与 TypeScript 构建不作为本次 Prompt 修改的必要门禁；执行平台整体回归时可以运行这些前端验证，但不运行 Playwright/E2E。

## 验收标准

- 两种 Reviewer Prompt 都包含一致的 E2E/Playwright 禁止规则。
- Reviewer 仍被明确允许提出与实际项目相符的其他测试要求。
- Reviewer 测试建议不被限制为 Work Item 或 Verification Plan 的命令白名单。
- Coder Prompt、Review parser、报告持久化、返修和 Gate 行为不发生变化。
- Prompt 不引入 Rust、React 或其他固定开发技术栈假设。
- 相关 Rust 测试和仓库标准非 E2E 验证通过。
