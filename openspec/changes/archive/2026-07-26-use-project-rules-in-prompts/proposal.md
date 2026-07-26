## Why

多类 Provider prompt 目前内嵌了指向外部 Cadence-skills 目录的绝对路径，并将其声明为唯一流程权威。这会绕过目标代码库自身的 `AGENTS.md` 与 `CLAUDE.md`，也使 prompt 行为依赖本机目录布局而不是被初始化项目的规则。

## What Changes

- 以集中、项目相对的规则读取指令替换外部绝对路径、唯一权威声明及知识库禁令。
- 要求 Provider 在开始任务前直接读取当前目标仓库根目录的 `AGENTS.md` 与 `CLAUDE.md`，并以其中适用规则为准；任一文件不可读时停止并报告。
- 更新所有复用该集中指令的 Workspace、Coding、Tester、Work Item Draft 与运行时单元 prompt 契约测试。
- 不修改目标仓库的规则文件，不改变 Provider 的业务产物、JSON schema 或既有 Aria 人工审批边界。

## Capabilities

### New Capabilities

- `project-rule-aware-prompts`: 所有需要流程规则的 Provider prompt 使用当前目标仓库的 `AGENTS.md` 与 `CLAUDE.md` 作为规则依据，且不依赖外部 Cadence-skills 绝对路径。

### Modified Capabilities

- 无。

## Impact

- 后端：`src/product/cadence_skills/routing_reference.rs` 及其各类 prompt 构造器。
- 测试：集中引用测试与 Workspace、Coding、Tester、Work Item Draft、运行时模板的 prompt 契约断言。
- 无新增依赖、HTTP API、Provider turn 或持久化数据变化。
