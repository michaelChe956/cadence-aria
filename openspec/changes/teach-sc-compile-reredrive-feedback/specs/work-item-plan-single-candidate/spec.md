# Spec Delta

## ADDED Requirements

### Requirement: 教学重驱载荷的完备性与降级可诊断性（REQ-WSC-32）

教学重驱的载荷 SHALL 完备：回灌给 author 的编译诊断 MUST 逐条含稳定码、行号、缺失字段或位置、消息与恰好一个可回喂修复示例；重驱 prompt MUST 同时携带单源结构契约（必需 section 清单、必需字段表）、最小合法 source 样例与上一版完整 source。MUST NOT 增减重驱次数、教学资格码集合、重驱机会上限语义或任何预算常量。教学机会耗尽进入终态前 MUST 发出结构失败诊断包事件，携带诊断全集与末次 source 引用及 hash，使降级产物可诊断。

#### Scenario: 回灌诊断不得丢弃缺失字段与修复示例

- **WHEN** 编译器产出的诊断被格式化为教学重驱的回灌理由
- **THEN** 每条理由 MUST 逐段含 `code:line:field:message:repair_example`；`field` 与 `repair_example` MUST NOT 被丢弃，缺失字段与修复示例 MUST 逐字呈现

#### Scenario: 两条诊断通道形态同构

- **WHEN** 同一编译器诊断分别经引擎侧与 runner 侧的诊断渲染路径呈现
- **THEN** 两条路径 MUST 产出同构形态，MUST NOT 出现一处含缺失字段与修复示例、另一处只含码行号消息的分叉

#### Scenario: 重驱 prompt 携带结构契约三件套

- **WHEN** 平台构造教学重驱 prompt
- **THEN** prompt MUST 含必需 section 清单、必需字段表与最小合法 source 标记；section 清单 MUST 与结构契约单源逐字一致且含「可选 section」与「空即合法 section」两条豁免表述，MUST NOT 另抄一份清单，MUST NOT 把全部 section 说成必需

#### Scenario: 上一版完整 source 随重驱回灌

- **WHEN** 平台发起教学重驱且持有上一版交付 source
- **THEN** prompt MUST 含 `[previous_source]` 段与该 source 全文；该 source 的引用与 hash MUST 取自耐久教学窗口模型，MUST NOT 由重驱路径另行抓取或重算出第二套事实

#### Scenario: 重驱补全结构后恢复为候选产出

- **WHEN** 首轮交付缺必需 section、重驱轮交付补全后可通过结构校验
- **THEN** 该 run MUST 产出候选并继续既有编译与评审流程，MUST NOT 进入失败终态

#### Scenario: 机会耗尽后发出结构失败诊断包

- **WHEN** 教学重驱后仍失败并进入终态
- **THEN** 终态失败落盘前 MUST 发出结构失败诊断包事件，其输出 MUST 含五段式诊断全集与最后一次交付 source 的 hash 与字节数；既有终态 message MUST 仍含「一次教学重驱后仍失败」的表述，MUST NOT 因新增诊断包而被改写

#### Scenario: 次数、资格码与预算常量不变

- **WHEN** 教学重驱载荷完备化生效
- **THEN** 模型调用次数、重驱次数、教学资格码集合、重驱机会上限与记账语义 MUST 逐字不变，MUST NOT 新增第三次机会，MUST NOT 上调固定规范面 23,500 B 与上下文面 32,000 B 的预算常量
