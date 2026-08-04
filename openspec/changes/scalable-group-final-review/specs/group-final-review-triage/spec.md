# group-final-review-triage Specification

## ADDED Requirements

### Requirement: 组级审查的材料与输出失败必须落地可区分的阻塞门禁

组级审查在产出评审结论之前发生的材料与输出失败 MUST 落地阻塞门禁，并 MUST 使用与既有人工路由决策互不相同的原因码。系统 MUST NOT 因这些失败而静默退出流程或产生通过结论。

需要区分的失败为：材料溢出、分片输出无效、归约输出无效、权威身份缺失。

#### Scenario: 材料溢出落地门禁

- **WHEN** 组级审查构建的分片或归约输入超过字节硬上限
- **THEN** 系统 MUST 落地阻塞门禁，attempt 状态 MUST 为阻塞，MUST NOT 调用 Provider

#### Scenario: 分片输出无效落地门禁

- **WHEN** 某个分片的 Provider 正常完成但未产出可解析结论，且结论转写补救仍未成功
- **THEN** 系统 MUST 落地阻塞门禁，attempt 状态 MUST 为阻塞

#### Scenario: 归约输出无效落地门禁

- **WHEN** 归约阶段的 Provider 正常完成但未产出可解析结论，且结论转写补救仍未成功
- **THEN** 系统 MUST 落地阻塞门禁，attempt 状态 MUST 为阻塞

#### Scenario: 权威身份缺失落地门禁

- **WHEN** 组级审查材料编译因权威校验不通过或单位审查结论身份缺失而失败
- **THEN** 系统 MUST 落地阻塞门禁，MUST NOT 调用 Provider

#### Scenario: 四类失败原因码互不相同

- **WHEN** 分别以材料溢出、分片输出无效、归约输出无效、权威身份缺失落地门禁
- **THEN** 四次落地的原因码 MUST 互不相同，且 MUST 与既有人工路由决策的原因码互不相同

### Requirement: 组级失败门禁必须提供按环节重试的动作

组级审查的材料与输出失败门禁 MUST 提供重试动作，且重试 MUST 只作用于失败环节。

#### Scenario: 分片失败门禁只重试该分片

- **WHEN** 用户在分片输出无效门禁上执行重试
- **THEN** 系统 MUST 仅重新执行该分片，MUST NOT 重新执行输入未变化的其他成功分片

#### Scenario: 归约失败门禁只重试归约

- **WHEN** 用户在归约输出无效门禁上执行重试
- **THEN** 系统 MUST 仅重新执行归约阶段，MUST NOT 重新执行输入未变化的成功分片

#### Scenario: 溢出门禁重试前必须重新度量

- **WHEN** 用户在材料溢出门禁上执行重试
- **THEN** 系统 MUST 重新编译材料并重新度量字节，仍超过硬上限时 MUST 再次落地溢出门禁

## MODIFIED Requirements

### Requirement: 同一次评审结论只落地一个阻塞门禁

系统 MUST NOT 为同一次评审结论落地多个阻塞门禁。门禁落地判定 MUST 由流程决策驱动，且各判定条件 MUST 互斥。

在分片与归约架构下，判定 MUST 以归约阶段产出的最终评审结论为依据。分片结论 MUST NOT 独立落地评审结论门禁。

#### Scenario: 每次结论至多一个门禁

- **WHEN** 组级审查完成任意一次归约并推出流程决策
- **THEN** 落地的阻塞门禁数量 MUST 不超过一个

#### Scenario: 阻塞结论与分诊决策不重复落地

- **WHEN** 归约结论为阻塞，且该结论推出的流程决策属于需要人工介入的四类之一
- **THEN** 系统 MUST 只落地一个阻塞门禁，MUST NOT 因结论与决策各自落地而产生两个

#### Scenario: 分片结论不独立落地评审门禁

- **WHEN** 某个分片给出要求修改或阻塞结论
- **THEN** 系统 MUST NOT 据此落地评审结论门禁，MUST 等待归约阶段产出最终结论后再判定

#### Scenario: 材料与输出失败门禁不与评审结论门禁叠加

- **WHEN** 组级审查因材料溢出、分片输出无效、归约输出无效或权威身份缺失而落地门禁
- **THEN** 系统 MUST NOT 同时落地由评审结论驱动的门禁
