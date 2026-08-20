# Delta Spec: project-rule-aware-prompts

## ADDED Requirements

### Requirement: 单仓生成类 Prompt 的规则引用按需化（REQ-PROMPT-03）

系统 SHALL 在 Legacy（单仓）上下文中，对生成类 prompt（outline、draft、plan、revision 及其生成侧 review）的规则引用仅声明目标仓库规则文件位置并允许按需查阅；规则读取 MUST NOT 作为生成输出的前置条件，规则文件或读取工具不可用时 MUST NOT 阻塞生成或强制转入 blocker。Legacy 上下文中的 coding 阶段 prompt 不受本要求约束，维持既有完整读取与失败关闭行为。

#### Scenario: 单仓 outline 生成不因规则文件缺失而阻塞
- **WHEN** Legacy 上下文构建 Work Item Group outline 生成 prompt 且目标仓库缺失 AGENTS.md 或 CLAUDE.md
- **THEN** prompt SHALL 仅包含规则位置声明与按需查阅提示，Provider SHALL 可正常输出候选 outline，不得被要求先阻塞报告

#### Scenario: 单仓 draft 生成不强制完整读取规则文件
- **WHEN** Legacy 上下文构建 Work Item Draft 生成 prompt
- **THEN** prompt SHALL NOT 包含"完整读取 AGENTS.md 与 CLAUDE.md 后才允许输出 JSON"类前置门禁

#### Scenario: coding 阶段保持既有规则门禁
- **WHEN** Legacy 上下文构建 coding 阶段 prompt
- **THEN** 规则引用 SHALL 维持现状（完整读取与失败关闭），不受本变更影响

#### Scenario: 逻辑代码库政策门禁不受影响
- **WHEN** Logical 上下文构建任一生成类或 coding prompt
- **THEN** 聚合政策权威加载与"未加载即阻塞"行为 SHALL 与既有 REQ-PROMPT-01/02 完全一致
