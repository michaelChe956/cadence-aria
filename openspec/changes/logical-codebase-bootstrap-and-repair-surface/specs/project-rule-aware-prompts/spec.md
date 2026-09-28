# Spec Delta

## MODIFIED Requirements

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
