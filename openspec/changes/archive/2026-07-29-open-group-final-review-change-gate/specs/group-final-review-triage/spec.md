## ADDED Requirements

### Requirement: internal PR review 的人工路由决策必须落地阻塞门禁

internal PR review 与 group final review 阶段，当评审结论推出需要人工介入的流程决策时，系统 MUST 落地阻塞门禁并把 attempt 置为阻塞状态。系统 MUST NOT 在该阶段静默退出流程。

需要人工介入的决策为：送回 Coder 返修、重试验证、人工分诊、运维阻塞。

#### Scenario: 要求修改结论落地门禁

- **WHEN** group final review 给出要求修改的结论，且流程决策为送回 Coder 返修
- **THEN** 系统 MUST 落地阻塞门禁，attempt 状态 MUST 为阻塞，MUST NOT 停留在运行中

#### Scenario: 验证证据不完整落地门禁

- **WHEN** internal PR review 的流程决策为重试验证
- **THEN** 系统 MUST 落地阻塞门禁，attempt 状态 MUST 为阻塞

#### Scenario: 人工分诊与运维阻塞落地门禁

- **WHEN** internal PR review 的流程决策为人工分诊或运维阻塞
- **THEN** 系统 MUST 落地阻塞门禁，attempt 状态 MUST 为阻塞

#### Scenario: 通过结论不落地门禁

- **WHEN** internal PR review 给出通过结论且流程决策为通过后继续
- **THEN** MUST NOT 落地阻塞门禁，完成路径 MUST 与本变更前一致

#### Scenario: 计划修订决策不落地门禁

- **WHEN** internal PR review 的流程决策为启动计划修订
- **THEN** MUST NOT 落地阻塞门禁，计划修订编排 MUST 与本变更前一致

### Requirement: 停机原因以互不相同的原因码区分

四个需要人工介入的决策 MUST 使用互不相同的门禁原因码。

#### Scenario: 四个决策原因码互不相同

- **WHEN** 分别以送回 Coder 返修、重试验证、人工分诊、运维阻塞四个决策落地门禁
- **THEN** 四次落地的原因码 MUST 互不相同

### Requirement: 同一次评审结论只落地一个阻塞门禁

系统 MUST NOT 为同一次评审结论落地多个阻塞门禁。门禁落地判定 MUST 由流程决策驱动，且各判定条件 MUST 互斥。

#### Scenario: 每次结论至多一个门禁

- **WHEN** internal PR review 完成任意一次评审并推出流程决策
- **THEN** 落地的阻塞门禁数量 MUST 不超过一个

#### Scenario: 阻塞结论与分诊决策不重复落地

- **WHEN** 评审结论为阻塞，且该结论推出的流程决策属于需要人工介入的四类之一
- **THEN** 系统 MUST 只落地一个阻塞门禁，MUST NOT 因结论与决策各自落地而产生两个

### Requirement: 分诊门禁提供可操作动作

分诊门禁 MUST 提供重试评审、人工继续、终止三个动作，使用户在任何停机原因下都能继续或结束流程。

#### Scenario: 门禁动作可用

- **WHEN** internal PR review 的分诊门禁向用户呈现
- **THEN** 动作集合 MUST 含重试评审、人工继续、终止

#### Scenario: 分诊动作不触发计划修订

- **WHEN** 用户在分诊门禁上执行任意动作
- **THEN** MUST NOT 唤起计划修订流程

### Requirement: 门禁原因码判定不含不可达分支

系统 MUST NOT 保留因前置条件永不满足而不可达的门禁原因码分支。判定条件 MUST 与实际可达的结论与决策组合一致。

#### Scenario: 无不可达分支

- **WHEN** 审查 internal PR review 的门禁原因码判定
- **THEN** 每个分支 MUST 在至少一种可达的结论与决策组合下生效

#### Scenario: 阻塞结论加可操作发现的组合仍可达

- **WHEN** 评审结论为阻塞且存在可操作发现，据此推出送回 Coder 返修决策
- **THEN** 该组合对应的原因码分支 MUST 保持生效，MUST NOT 被当作不可达分支移除
