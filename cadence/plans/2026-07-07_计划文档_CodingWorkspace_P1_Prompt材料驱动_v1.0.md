# CodingWorkspace P1 Prompt 材料驱动 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 Coding Workspace 的 coder、code reviewer、GroupFinalReview prompt 改为从任务材料提取执行/审查要求，删除平台固定技术栈硬编码。

**Architecture:** 本计划只改 prompt 生成和 prompt 单测，不改状态机、UI、attempt 创建和返修额度。固定模板只保留语言无关流程约束，技术栈要求只能来自 Work Item、VerificationPlan、EvaluationContextPack、项目规则或仓库事实。

**Tech Stack:** Rust backend, existing `CodingWorkspaceEngine` prompt helpers, `cargo test --locked --lib parser_prompt`, standard Rust verification commands.

---

## Scope

实现来源：

- `cadence/designs/2026-07-07_技术方案_CodingWorkspace材料驱动Prompt协议_v1.0.md`
- `cadence/designs/2026-07-07_技术方案_CodingWorkspace流程精简补充Delta_v1.0.md` 的第 10 节 Prompt 衔接

不做：

- 不改 `max_auto_rework` 配置来源。
- 不改 runner 流转。
- 不改前端配置页。
- 不按 07-06 全量重做。

## Files

- Modify: `src/product/coding_workspace_engine/prompts.rs`
- Modify: `src/product/coding_workspace_engine/tests/parser_prompt.rs`
- Possible modify: `src/product/coding_workspace_engine/internal_pr_review.rs`
- Possible modify: `src/product/coding_workspace_engine/code_review.rs`

## Task 1: 建立 Prompt 固定模板禁止词测试

- [ ] 在 `src/product/coding_workspace_engine/tests/parser_prompt.rs` 新增或扩展 coder prompt 测试。
- [ ] 构造一个 neutral Work Item markdown，不包含 Rust/Node/Java/tooling 词。
- [ ] 断言 full prompt 固定模板不包含以下平台默认词：
  - `pnpm`
  - `node_modules`
  - `tsc`
  - `vitest`
  - `cargo`
  - `crate`
  - `.rs`
  - `mvn`
  - `gradle`
  - `.java`
  - `source set`
- [ ] 断言 prompt 包含：
  - `从 Work Item`
  - `Verification Plan`
  - `执行清单`
  - `不得用平台默认技术栈假设`
- [ ] 运行：

```bash
cargo test --locked --lib parser_prompt
```

Expected before implementation: 新测试失败，失败点应指向旧 `dependency_bootstrap_guidance()` 或 `coding_self_check_contract()` 的固定词。

## Task 2: 替换 coder full/delta prompt 协议

- [ ] 在 `prompts.rs` 删除或停止调用旧 `dependency_bootstrap_guidance()` 中的 pnpm/node_modules/tsc/vitest 固定说明。
- [ ] 删除或停止调用旧 `coding_self_check_contract()` 中的 `.rs`、crate 挂载固定说明。
- [ ] 新增语言无关 helper：
  - `no_default_stack_assumption_contract()`
  - `coding_execution_protocol()`
  - `coding_completion_report_contract()`
  - `coding_delta_execution_protocol()`
- [ ] `build_coding_prompt` 保留元数据、`验证命令`、`已确认 Work Item` 原文，然后追加：
  - 修改前先提取执行清单。
  - 执行清单覆盖实现目标、写入范围、禁止范围、TDD/测试、依赖初始化或环境诊断、验证命令、完成前自检、handoff。
  - 材料没给语言/构建/包管理器/测试框架时不得臆造。
  - 完成报告列执行清单、修改文件、命令完整输出、diff stat、Forbidden scope 检查。
- [ ] `build_coding_delta_prompt` 追加：
  - 继续以本会话 Work Item 和 VerificationPlan 为任务来源。
  - 重新核对本轮返修要求、补充上下文和原 Work Item。
  - 人工返修意见优先级最高。
  - 无人工意见时，最新 reviewer findings 优先于更旧上下文。
- [ ] 保留 Work Item markdown 原文注入，确保当 Work Item 自身包含 Rust/Java/Node 内容时不会被过滤。

## Task 3: CodeReviewer 材料驱动审查协议

- [ ] 在 `build_code_review_prompt` 中加入 `code_review_material_protocol()`。
- [ ] 协议必须要求 reviewer：
  - 只分析 diff，不修改代码。
  - 从原始需求上下文和 EvaluationContextPack 提取审查清单。
  - 审查实现目标、写入范围、Forbidden scope、验证证据、自检要求、handoff 承诺。
  - required 验证证据缺失时必须记录 finding，必要时 `request_changes` 或 `blocked`。
  - `0 tests` 或无实际测试执行不能直接视为覆盖。
  - 不提出任务材料外的技术栈默认要求。
- [ ] 保持 JSON schema：

```json
{"verdict":"approve|request_changes|blocked","summary":"...","findings":[]}
```

## Task 4: GroupFinalReview prompt 语义

- [ ] 在现有 internal PR review prompt 生成点引入 GroupFinalReview 语义。
- [ ] 可以复用既有字段集合，但 prompt 文案应使用 `GroupFinalReview` / `WorkItemGroup GroupFinalReview`。
- [ ] findings 的 source stage 使用 `group_final_review`，不再在新 prompt 中要求 `internal_pr_review`。
- [ ] GroupFinalReview 协议必须要求：
  - 从 Completed Units、unit handoff、EvaluationContextPack、完整 diff 提取整组审查清单。
  - 检查每个 completed unit 的 handoff 承诺是否闭环。
  - 检查依赖 handoff 是否断裂。
  - 检查整组 diff 是否越过任一 unit 的 Forbidden Write Scopes。
  - 审查 ReviewRequest commit 与 completed units、diff、验证证据是否一致。
- [ ] 单 WorkItem scope 不生成此 prompt。若生成路径属于 runner，本计划只改 prompt 生成函数和测试；runner 删除放到 P3。

## Task 5: Prompt 单测补齐

- [ ] Coder full prompt:
  - neutral Work Item 不出现固定技术栈词。
  - Rust Work Item 原文包含 `cargo fmt --check` 时 prompt 保留该原文。
  - Java/Maven Work Item 原文包含 `mvn test` 时 prompt 保留该原文。
- [ ] Coder delta prompt:
  - 包含人工返修意见最高优先级说明。
  - 不包含固定技术栈词。
- [ ] CodeReviewer prompt:
  - 包含“从原始需求上下文和 EvaluationContextPack 提取审查清单”。
  - 包含 required 验证证据审查。
  - 保持 CodeReviewer JSON schema 稳定。
- [ ] GroupFinalReview prompt:
  - 包含 Completed Units、handoff 闭环、ReviewRequest commit、完整 diff 审查要求。
  - 不包含平台固定技术栈词。
  - 单 WorkItem 不生成 GroupFinalReview prompt 的测试如需要 runner 配合，留到 P3。

## Task 6: Verification

- [ ] Run focused tests:

```bash
cargo test --locked --lib parser_prompt
```

- [ ] Run standard backend checks:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
```

- [ ] Run diff check:

```bash
git diff --stat
```

- [ ] Confirm changed files are limited to prompt generation and prompt tests unless a direct compile error requires a small caller adjustment.

## Completion Criteria

- 固定 prompt 模板不再输出 Rust/Node/Java/package-manager/tooling 默认规则。
- 任务材料中自然出现的技术栈内容仍保留。
- coder、CodeReviewer、GroupFinalReview 都明确从任务材料提取执行/审查清单。
- 人工返修意见优先级已出现在 coder delta prompt。
- 本计划不改状态机和 UI。
