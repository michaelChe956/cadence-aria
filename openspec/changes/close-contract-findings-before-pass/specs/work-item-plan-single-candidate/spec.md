# Spec Delta

## MODIFIED Requirements

### Requirement: 返修触发路径只认 actionable（REQ-WSC-17）

非 advisory 指纹闸门、transition budget 与 `revise` 归一 MUST 与阻断判定同口径：`advisory` MUST NOT 进指纹集合、MUST NOT 驱作者返修、MUST NOT 占 transition budget；`revise` 但无任何 actionable finding 时 MUST 归一为可通过或人工确认。`contract_gap` / `self_contradiction` MUST NOT 仅凭模型标 `advisory` 获得免阻断。

平台 SHALL 维护唯一「有效必关闭」判定（effective closure），并以该判定作为契约类 finding 是否进入必关闭集合、是否重开 `pass`、是否进入非 advisory 指纹与返修预算、是否被中央策略判为可自动返修的唯一口径；verdict 判定、必关闭清单渲染、owner 处置再归一、policy 分类四个消费面 MUST 使用同一判定，不得各设门槛。

未被 owner 显式排除的本 Work Item `contract_gap` / `self_contradiction` finding，MUST 进入必关闭集合；模型的 `severity=suggestion`、`class_hint=advisory` 或 `verdict=pass` 均不得使其自动放行。平台 SHALL 保留原 severity 及内容身份（finding key 逐字不变），以有效必关闭语义归一路由；缺少可验证定位或修复许可时 SHALL 转既有人工处理，不得因此自动调用 author。owner 显式排除优先，范围外 advisory 规则不变。

有效必关闭的例外作用域 SHALL 限于 WorkItemPlan 的既有契约类 finding；通用 Story/Design 等其他评审的既有门槛 MUST NOT 因此扩大。作用域 SHALL 由可信 session / 解析入口传入，MUST NOT 由模型标签授予。

#### Scenario: 指纹闸门与返修预算只认 actionable

- **WHEN** 同一轮 verdict 里既有 `advisory`（哪怕标成 `must_fix`/`human_required`）又有 `actionable`
- **THEN** 非 advisory 指纹集合与 transition budget MUST 只由 `actionable` 构成；只有 `advisory` 时该指纹集合 MUST 为空、MUST NOT 消耗返修预算

#### Scenario: revise 但无 actionable 不驱作者返修

- **WHEN** reviewer 输出 `verdict=revise`，但全部 finding 都是 `advisory`
- **THEN** 平台 MUST 归一为可通过（或人工确认）的结论，MUST NOT 进入作者返修通道，MUST NOT 因此消耗 transition budget

#### Scenario: 契约类缺口不得以 advisory 漏检

- **WHEN** reviewer 把 `category=contract_gap` 或 `category=self_contradiction` 的本 Work Item 缺口标成 `disposition=advisory`
- **THEN** 平台 MUST 按 `actionable` 处理，MUST 让它进入必关闭清单（判定在平台侧，不依赖模型自觉）

#### Scenario: 模型标签不得使契约类 finding 自动放行

- **WHEN** 一条 `category=contract_gap` 的本 Work Item finding 同时带 `severity=suggestion`、`class_hint=advisory`，且整轮 `verdict=pass`
- **THEN** 该 finding MUST 进入必关闭集合；平台 MUST NOT 因上述任一模型标签把它放行，MUST NOT 让整轮结论保持 `pass`

#### Scenario: 保留原 severity 与内容身份

- **WHEN** 平台把一条契约类 finding 判为有效必关闭
- **THEN** 该 finding 的 `severity` MUST 保持模型原值（`suggestion` 不得被改写为 `must_fix` 或 `blocking`）；其内容身份 key MUST 仍按 `sha256(severity + \u{1} + message)` 前 16 位小写 hex 计算且逐字不变；owner 既有排除项的匹配与幂等语义 MUST NOT 因此改变

#### Scenario: owner 显式排除优先且排除项不得复活

- **WHEN** 一条契约类 finding 的 key 已在会话 durable 已处置事实清单中（owner 已显式排除）
- **THEN** 该 finding MUST NOT 进入必关闭集合、MUST NOT 触发作者返修；owner 排除判定 MUST 先于有效必关闭计算，且排除生效后 MUST NOT 再次被平台硬化恢复为必关闭项

#### Scenario: 缺可验证定位时转人工且不自动调用 author

- **WHEN** 一条被判为有效必关闭的契约类 finding 没有任何可验证定位（例如缺 `contract_field` 或其值不可用），且不存在已有的合法机械修复证据
- **THEN** 平台 SHALL 将其转入既有人工处理通道；MUST NOT 因此自动调用 author，MUST NOT 新增自动重写次数，MUST NOT 仅凭 `category` 或 `severity` 自行造出可自动修复的许可

#### Scenario: 四面同口径

- **WHEN** 同一条契约类 finding 被 verdict 判定、必关闭清单渲染、owner 处置再归一、中央 policy 分类四个消费面分别处理
- **THEN** 四个消费面 MUST 得出同一「是否有效必关闭」结论；任一面 MUST NOT 因只读 `severity`、`class_hint` 或 `verdict` 字面值而得出与该结论相反的结果

#### Scenario: 例外作用域不扩到通用评审

- **WHEN** 一次通用 Story/Design 等非 WorkItemPlan 评审产出 `category=contract_gap` 且 `severity=suggestion` 的 finding
- **THEN** 该 finding MUST 继续按该评审既有门槛处理，MUST NOT 因本例外进入必关闭集合；本例外的作用域 MUST 由可信 session / 解析入口确定，MUST NOT 由模型输出的任何标签授予或扩大

#### Scenario: 严格、宽松与历史入口同一规则

- **WHEN** 同一条契约类 finding 分别经严格解析入口、宽松解析入口与历史展示重建入口被处理
- **THEN** 三条入口 MUST 使用同一有效必关闭判定并得出同一结论；历史展示重建 MUST 重新归一，但 MUST NOT 自动启动 provider，MUST NOT 回滚已经确认或已发布的历史产物

#### Scenario: 范围外 advisory 不被放大

- **WHEN** 一条 `category` 不属于契约类的 advisory finding（例如范围外事实或完备度意见）出现
- **THEN** 该 finding MUST 继续保持 advisory，MUST NOT 进入必关闭集合，MUST NOT 计入非 advisory 指纹，MUST NOT 消耗返修预算，MUST NOT 阻断流程

#### Scenario: 矩阵覆盖与基线一致

- **WHEN** 以 `category`（`contract_gap` / `self_contradiction`）× `severity`（`suggestion` / `must_fix` / `blocking`）× `disposition`（`actionable` / `advisory` / 缺省）× `class_hint`（三类）× `verdict`（`pass` / `revise` / `needs_human`）构成矩阵，并代入含五条真实 finding 的基线样本
- **THEN** 恰有一条契约类 finding 进入必关闭集合，整轮结论 MUST NOT 为自动通过，该 finding 的原 severity 与 key MUST 不变；无 `contract_field` 的那条 MUST NOT 自动调用 author；其余四条 MUST 仍不扣预算、不阻断
