## Context

当前所有流程型 Provider prompt 都经由 `direct_cadence_routing_rules_reference()` 插入同一段固定文本。该文本把两个外部 Cadence-skills 文件的绝对路径描述为唯一权威，并额外禁止知识库能力；它与目标仓库自身已生成的 `AGENTS.md`、`CLAUDE.md` 脱节，且依赖执行主机目录。

该函数被 Workspace、Coding、Tester、Work Item Draft、Web Workspace Context 与运行时模板复用。因此本次是跨模块的 prompt 契约变化，但不涉及 API、存储或 Provider 输出 schema。

## Goals / Non-Goals

**Goals:**

- 使所有复用集中规则段的 Provider prompt 以当前目标仓库根目录的 `AGENTS.md` 与 `CLAUDE.md` 为流程规则依据。
- 要求 Provider 在任务开始前使用其原生文件读取能力直接读取两份文件；任一文件无法读取时停止并报告，不继续生成候选、代码、审查结论或 JSON。
- 移除外部 Cadence-skills 绝对路径、"唯一流程权威"表述和知识库禁令，并以契约测试防止回归。

**Non-Goals:**

- 不改写、生成或同步目标仓库的 `AGENTS.md`、`CLAUDE.md`。
- 不改变既有 Aria 审批 gate、Provider 调用次数、业务 artifact 或 JSON schema。
- 不把规则正文复制到 prompt，也不增加读取外部规则路径的回退。

## Decisions

### 1. 保留集中入口，替换其文本内容

`routing_reference` 继续作为唯一的 prompt 片段来源，但其语义改为“项目规则读取要求”。所有既有调用点继续引用该入口，避免让多套 prompt 文本重新分叉。

替代方案是分别修改每个 prompt 构造器。该方案会造成规则文本重复，并使后续调整遗漏某个 Provider 生命周期，因此不采用。

### 2. 明确要求读取两个当前项目文件

集中片段将明确指向当前目标仓库根目录的相对文件名 `AGENTS.md` 和 `CLAUDE.md`，要求先直接读取二者并遵循其中适用规则；文件不可读即停止并报告。它不得声明任何外部路径或另一个规则来源为权威。

替代方案是完全移除规则片段。该方案无法确保各类 Provider 在生成前读取当前项目规则，因此不采用。

### 3. 以正反向 prompt 契约测试锁定边界

集中入口测试验证两个项目文件、直接读取和失败关闭语义。各类现有 prompt 契约测试改为断言共享片段出现一次、旧标签和外部路径不再出现。测试覆盖 Workspace、Coding、Tester、Work Item Draft 与运行时模板，保证复用入口未被任一生命周期绕过。

## Risks / Trade-offs

- [目标仓库缺少任一规则文件会阻断 Provider] → 这是明确的失败关闭语义；初始化流程负责生成规则文件，调用方可据报告补齐后重试。
- [两个项目规则内容彼此冲突] → prompt 只要求读取并遵循当前仓库规则，不在应用层定义额外冲突裁决或复制摘要。
- [旧断言仍依赖外部文本] → 先以失败测试替换集中断言，再逐类更新 prompt 契约测试，最终全量检索确认旧标签和路径不存在。
