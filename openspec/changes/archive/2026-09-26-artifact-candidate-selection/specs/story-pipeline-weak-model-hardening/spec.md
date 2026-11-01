# story-pipeline-weak-model-hardening Delta

## MODIFIED Requirements

### Requirement: Story author artifact 一次成功（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

标准 Story author 的"一次成功"定义为：首次 provider turn 输出中存在**唯一一个通过 artifact gate 的完整候选**（一级标题、必需 heading、稳定 ID、追踪 token、禁止项以 `artifact-candidate-selection` REQ-ACS-01 的候选选择为准），未触发自动 retry。provider 原文含额外的未通过候选（示意 block、自检推演）不单独构成失败；两个及以上通过 gate 的候选仍构成歧义失败。

#### Scenario: author 首次通过

- **WHEN** author 首次输出即为完整合规 artifact（无论原文是否含前置未通过候选 block）
- **THEN** 不触发 `build_artifact_retry_prompt`，记一次 author 成功

#### Scenario: 前置示意 block 不破坏一次成功

- **WHEN** author 首次输出在最终合规候选之前另含一个未通过 gate 的示意 artifact block 与过程思考文本
- **THEN** 按候选选择规则选中唯一通过候选，仍记一次 author 成功、不触发 retry

#### Scenario: 多个有效候选不算一次成功

- **WHEN** author 首次输出含两个各自通过 gate 的候选
- **THEN** 候选歧义失败关闭，不计一次成功（后续恢复路径按既有契约）
