# process-evidence-acceptance-boundary

## ADDED Requirements

### Requirement: 验收标准必须是可观测的结果状态

acceptance criterion MUST 描述可从最终代码状态、验证命令输出、人工检查结果或 handoff 字段观测的结果状态。acceptance criterion MUST NOT 描述实现完成后不可追补的过程事实。

过程事实判定标准为同时满足两条：无法从当前 diff、验证命令输出、handoff 字段或人工检查结果观测；即使返修也无法产出该证据。

#### Scenario: splitter 生成验收标准

- **WHEN** splitter provider 生成 canonical contract 的 `acceptance_criteria`
- **THEN** 每条 criterion 的 `statement` MUST 描述可观测的结果状态
- **AND** MUST NOT 以提交历史、提交顺序、开发时序或分支操作历史作为验收内容

#### Scenario: 契约中出现过程性验收标准

- **WHEN** canonical contract 校验发现某条 acceptance criterion 描述的是过程事实
- **THEN** 校验 MUST 产出 finding，MUST NOT 静默通过
- **AND** 该 finding MUST 指向该 criterion 的 `criterion_id`

#### Scenario: 检出不阻断候选

- **WHEN** 某候选除过程性 acceptance criterion 外无其他缺陷
- **THEN** 该检出 MUST NOT 使候选不可接受
- **AND** 该 finding MUST 出现在候选的校验结论中供用户查看

### Requirement: 证据类型语义必须在提示词中明确

`EvidenceKind` 的语义 MUST 在 splitter 与 reviewer 两侧提示词中一致说明。

#### Scenario: 说明非零测试执行的语义

- **WHEN** 提示词提及 `non_zero_test_execution`
- **THEN** MUST 说明其语义为验证命令执行时实际运行了非零数量的测试，是当前可观测的执行结果
- **AND** MUST 说明它不表达测试曾先失败、不表达提交顺序、不表达任何开发时序

### Requirement: 只读审查不得以过程事实否决实现

只读审查阶段 MUST NOT 以过程事实作为否决依据。

#### Scenario: reviewer 面对缺失的过程证据

- **WHEN** reviewer 在 Code Review 或 GroupFinalReview 阶段发现不存在 TDD red commit、不存在失败到通过的提交序列、提交顺序或开发时序不符合预期
- **THEN** reviewer MUST NOT 因此创建 finding
- **AND** MUST NOT 因此给出 `request_changes` 或 `blocked`
- **AND** MUST NOT 将其写入 verdict 或 summary 的否决理由
- **AND** MUST NOT 将其转换为 Coder 的 `required_action` 或任何返修要求

#### Scenario: 上游材料中写入了过程性要求

- **WHEN** Work Item、Design Spec、Verification Plan、handoff 或 EvaluationContextPack 中出现过程性要求
- **THEN** reviewer MUST NOT 将其转换为 finding、verdict 或 summary 的否决理由、Coder `required_action` 或任何返修要求

#### Scenario: 边界覆盖全部 reviewer 提示词构造路径

- **WHEN** 平台以任一路径构造 Code Review 或 GroupFinalReview 的 reviewer 提示词
- **THEN** 构造出的提示词 MUST 包含过程证据边界

#### Scenario: 边界覆盖 projection 渲染路径

- **WHEN** Code Review 阶段存在 unit run projection，提示词完全由 projection 渲染产生而不经传统提示词构造
- **THEN** 该提示词 MUST 仍包含过程证据边界

### Requirement: 可观测的测试证据仍必须审查

过程证据边界 MUST NOT 削弱对可观测测试证据的审查要求。

#### Scenario: 验证命令缺少执行证据

- **WHEN** reviewer 发现 required 验证命令缺少执行证据，或测试输出显示没有实际测试被执行，或测试输出与实现自相矛盾
- **THEN** reviewer MUST 仍按既有协议记录 finding 并按必要性给出 `request_changes` 或 `blocked`

#### Scenario: 测试覆盖不足

- **WHEN** reviewer 发现测试文件缺失或测试未覆盖需求场景
- **THEN** reviewer MUST 仍可据此创建 finding

### Requirement: 开发侧 TDD 要求不受影响

Coder 侧的 TDD 与测试要求 MUST 保持不变。

#### Scenario: Coder 执行协议

- **WHEN** 平台构造 Coder 执行提示词或增量执行提示词
- **THEN** 其 TDD 与测试要求 MUST 保持不变，MUST NOT 因过程证据边界被削弱

### Requirement: 不保留过程性验收标准的兼容豁免

平台 MUST NOT 为既有含过程性 acceptance criterion 的持久化契约提供豁免或兼容层。

#### Scenario: 既有契约触发检出

- **WHEN** 本变更实施后校验既有 canonical contract，且其中含过程性 acceptance criterion
- **THEN** 校验 MUST 照常产出 finding
- **AND** MUST NOT 提供豁免名单、宽限期或迁移层
