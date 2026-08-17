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
