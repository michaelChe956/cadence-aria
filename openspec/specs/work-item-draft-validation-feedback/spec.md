# work-item-draft-validation-feedback Specification

## Purpose
TBD - created by archiving change improve-work-item-draft-generation-reliability. Update Purpose after archive.
## Requirements
### Requirement: Draft 校验失败确认区反馈
系统 SHALL 在 Work Item Draft 确认节点的当前确认区直接展示校验失败状态。该反馈 MUST 显示失败总数、至少前三条错误摘要和可展开的完整 findings；提示 MUST 使用可访问的告警语义。

#### Scenario: Draft 包含校验错误
- **WHEN** 当前节点为 Work Item Draft 确认且 `can_accept=false` 并存在 validator findings
- **THEN** 确认区 MUST 显示“Draft 校验失败，暂不能接受”、错误总数、前三条错误码与说明，以及查看全部错误的入口

#### Scenario: 失败 Draft 没有可用 findings
- **WHEN** 当前节点为 Work Item Draft 确认且 `can_accept=false` 但 findings 为空或不可读取
- **THEN** 确认区 MUST 显示通用的不可接受原因和重写指引，且不得静默隐藏接受动作

#### Scenario: Draft 校验通过
- **WHEN** 当前节点为 Work Item Draft 确认且 `can_accept=true`
- **THEN** 确认区 MUST 显示接受动作，且不得显示校验失败告警

### Requirement: 带校验反馈的重写
系统 SHALL 在用户从失败 Draft 发起重写时，将当前 validator findings 与用户附加反馈合并后传入下一轮 Draft 生成上下文。

#### Scenario: 用户重写失败 Draft
- **WHEN** 用户在失败 Draft 的确认区点击“根据校验错误重写”
- **THEN** 下一轮 Draft Provider 输入 MUST 包含当前 findings 的错误码和说明，并且保持用户提供的非空附加反馈

#### Scenario: 用户暂停失败 Draft
- **WHEN** 用户在失败 Draft 的确认区点击暂停
- **THEN** 系统 MUST 保持失败 Draft 与 findings 可查看，并进入人工处理状态，不得触发新的 Provider 生成

