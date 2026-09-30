## Purpose

确保流程型 Provider prompt 读取并遵循当前项目规则，同时不依赖外部 Cadence 路径或知识库禁令。

## Requirements

### Requirement: Provider prompt 使用当前项目规则（REQ-PROMPT-01）

逻辑代码库流程 SHALL 在实际 provider 运行或索引/规划准入前预检其将消费的聚合政策、成员规则材料和 capability/gateway 条件。缺失、非法、过期或 digest 不一致时，系统 SHALL fail-closed 并在 provider 启动前生成可诊断等待项，明确缺失材料、目标和准备/重试操作；用户完成准备后回到原链继续。有效 prompt 的规则依据仍 SHALL 来自 envelope 校验过的聚合政策正文或受控 artifact 引用，最小成员指针仅用于发现/路由，不能作为政策正文或缺失策略的 fallback。

#### Scenario: 逻辑代码库 prompt 以聚合政策为权威

- **WHEN** 逻辑代码库流程构建 planning/coding prompt
- **THEN** 规则依据 SHALL 来自 envelope 校验过的聚合政策；最小指针不作为政策正文；未加载有效政策时 SHALL 阻塞

#### Scenario: 逻辑代码库规则材料缺失

- **WHEN** 逻辑代码库构建 planning/coding prompt 前发现实际 provider 将消费的规则材料缺失
- **THEN** 系统 SHALL 在 provider 启动前停等并通知用户，提供可重入准备/重试操作；准备成功后使用校验过的聚合政策继续，不在运行时才以 Failed 暴露缺口

#### Scenario: 规则 digest 或 capability 不一致

- **WHEN** policy revision/digest、成员规则引用或 provider capability 与准入快照不一致
- **THEN** 系统 SHALL 拒绝该次启动并展示核验/重新准备操作，不回落到目标根同名文件、不伪造 capability 成功、不清除既有错误事实

### Requirement: Prompt 不依赖外部 Cadence 路径或知识库禁令（REQ-PROMPT-02）

逻辑代码库 prompt 对聚合政策与成员指针的引用 SHALL 由唯一 authority resolver 产生，声明权威根并携带 policy revision/digest；provider MUST NOT 通过目标 worktree、项目级历史布局或外部 Cadence 路径猜测同名材料。若 resolver 无法唯一解析或策略不可读，流程 SHALL 停等并提供核验/修复操作。

#### Scenario: 聚合政策引用须声明根

- **WHEN** prompt 引用聚合政策或成员指针
- **THEN** 引用 SHALL 声明权威根并携带 policy revision/digest 供审计，不得猜测同名路径

#### Scenario: policy 引用无法唯一解析

- **WHEN** 同名规则文件存在于多个布局，或 authority/path 与 policy digest 不一致
- **THEN** prompt 构建 SHALL 被拒绝并返回迁移/核验等待项，不选择“最新”或任一路径继续

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

