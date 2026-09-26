## Purpose

确保流程型 Provider prompt 读取并遵循当前项目规则，同时不依赖外部 Cadence 路径或知识库禁令。

## Requirements

### Requirement: Provider prompt 使用当前项目规则（REQ-PROMPT-01）
系统 SHALL 使逻辑代码库流程的 prompt 规则依据来自 envelope 校验过的聚合政策正文（或受控 artifact 引用）；每仓最小指针仅作为发现/路由入口，不被当作政策正文执行；未加载有效政策时阻塞而非回落到目标根文件。

#### Scenario: 逻辑代码库 prompt 以聚合政策为权威
- **WHEN** 逻辑代码库流程构建 planning/coding prompt
- **THEN** 规则依据 SHALL 来自 envelope 校验过的聚合政策；最小指针不作为政策正文；未加载有效政策时 SHALL 阻塞

### Requirement: Prompt 不依赖外部 Cadence 路径或知识库禁令（REQ-PROMPT-02）
系统 SHALL 使逻辑代码库 prompt 对聚合政策/成员指针的引用声明权威根并携带 policy revision/digest；不得让 provider 在 target worktree 下猜测同名路径。

#### Scenario: 聚合政策引用须声明根
- **WHEN** prompt 引用聚合政策或成员指针
- **THEN** 引用 SHALL 声明权威根并携带 policy revision/digest 供审计，不得猜测同名路径

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

