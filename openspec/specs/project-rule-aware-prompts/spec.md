## Purpose

确保流程型 Provider prompt 读取并遵循当前项目规则，同时不依赖外部 Cadence 路径或知识库禁令。

## Requirements

### Requirement: Provider prompt 使用当前项目规则
系统 SHALL 在所有需要流程规则的 Provider prompt 中包含统一的项目规则读取要求：Provider MUST 使用原生文件读取能力直接读取当前目标仓库根目录的 `AGENTS.md` 和 `CLAUDE.md`，并以其中适用规则为准。

#### Scenario: 生成流程型 Provider prompt
- **WHEN** 系统生成 Workspace、Coding、Tester、Work Item Draft、Web Workspace Context 或运行时模板的流程型 Provider prompt
- **THEN** prompt SHALL 包含一次统一的项目规则读取要求，并要求先读取 `AGENTS.md` 与 `CLAUDE.md`

#### Scenario: 项目规则文件不可读
- **WHEN** Provider 无法直接读取当前目标仓库根目录的 `AGENTS.md` 或 `CLAUDE.md`
- **THEN** prompt MUST 要求 Provider 停止并报告阻塞，且不得继续生成候选产物、代码、审查结论或 JSON

### Requirement: Prompt 不依赖外部 Cadence 路径或知识库禁令
系统 SHALL 不在流程型 Provider prompt 中声明外部 Cadence-skills 绝对路径、`[cadence_original_routing_rules]` 标签、外部规则的唯一权威地位，或知识库 Skill/manifest 的禁止性约束。

#### Scenario: 渲染统一规则片段
- **WHEN** 系统渲染统一项目规则读取片段
- **THEN** 片段 SHALL 仅引用当前目标仓库的 `AGENTS.md` 与 `CLAUDE.md`，且不得包含 `/home/michaelche/workspace/github/Cadence-skills/`、`[cadence_original_routing_rules]`、`KnowledgeBase` 或“唯一流程权威”

#### Scenario: 复用规则片段的生命周期 prompt
- **WHEN** 系统构造任一复用统一规则片段的 Provider 生命周期 prompt
- **THEN** 该 prompt SHALL 保留其既有业务阶段、输出格式和 Aria gate 约束，同时符合项目规则读取与外部路径排除要求
