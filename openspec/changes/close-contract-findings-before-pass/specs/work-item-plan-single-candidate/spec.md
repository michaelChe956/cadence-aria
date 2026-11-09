# Spec Delta

## MODIFIED Requirements

### Requirement: 返修触发路径只认 actionable（REQ-WSC-17）

非 advisory 指纹闸门、transition budget 与 `revise` 归一 MUST 与阻断判定同口径：`advisory` MUST NOT 进指纹集合、MUST NOT 驱作者返修、MUST NOT 占 transition budget；`revise` 但无任何 actionable finding 时 MUST 归一为可通过或人工确认。`contract_gap` / `self_contradiction` MUST NOT 仅凭模型标 `advisory` 获得免阻断。

平台 SHALL 以唯一「有效必关闭」判定统一契约类 finding 的闭合、重开 `pass`、指纹、预算与呈现口径；未排除的本 Work Item 契约类 finding MUST 进入必关闭集合，模型标签不得放行；缺可验证定位或修复许可时 SHALL 转人工处理，MUST NOT 自动调用 author；owner 排除优先，范围外 advisory 不变。

例外作用域 SHALL 限于 WorkItemPlan 契约类，由可信入口传入，MUST NOT 由模型标签授予。

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

#### Scenario: 两级判定的分类落点

- **WHEN** 一条契约类 finding 分别处于三种状态：闭合资格为真（`effective_disposition == actionable` 且未被 owner 显式排除）且具备服务端授权的既有合法机械修复许可证据与可验证定位；闭合资格为真但许可证据与可验证定位任一缺失；闭合资格为假（例如已被 owner 显式排除）
- **THEN** 三种状态 MUST 分别落 `Repairable`（沿既有许可，不新造 L1）、`HumanRequired`（不得 `Advisory`，也不得 `Repairable`）、维持既有分类行为（范围外 advisory 不变）

#### Scenario: 缺可验证定位时转人工且不自动调用 author

- **WHEN** 一条闭合资格为真的契约类 finding 的自动修复许可为假——缺少可验证定位（`contract_field` 为空且无可定位的 typed 事实引用），或缺少服务端授权的既有合法机械修复许可，二者任一缺失
- **THEN** 平台 SHALL 将其分类为 `HumanRequired` 并转入既有人工处理通道；MUST NOT 判为 `Repairable` 或 `Advisory`，MUST NOT 因此自动调用 author，MUST NOT 新增自动重写次数，MUST NOT 仅凭 `category`、`severity` 或 `class_hint` 自行造出可自动修复的许可

#### Scenario: class_hint 不得单独构成自动调用许可

- **WHEN** 一条闭合资格为真的契约类 finding 携带 `class_hint=repairable`，或其分类兜底仅凭「`Revise` + 契约类 + `contract_field` 非空」推出可修复，但不存在服务端授权的既有合法机械修复许可证据
- **THEN** 平台 MUST NOT 将其判为 `Repairable` 或据此自动调用 author；无该许可证据时 MUST 落 `HumanRequired`；`class_hint`、`category`、`severity` 均不构成自动修复许可证据

#### Scenario: raw pass 在未重开路径上不得放行契约类 finding

- **WHEN** 一条未排除契约类 finding 的闭合资格为真，其所在轮次的原始判定为 `pass`（未经有效必关闭重开），且该 finding 经未发生重开的路径被消费——如读持久化旧 verdict，或 `routing_scope.rs:88`、`routing_platform_rescue.rs:71`、`prompts/review_context.rs:557` 上的分类与指纹消费点
- **THEN** 该原始 `pass` MUST NOT 使该 finding 自动放行或被判为 `Advisory`；各消费面 MUST 按有效必关闭口径得出同一结论

#### Scenario: 七面同口径

- **WHEN** 同一条契约类 finding 被 verdict 判定、必关闭清单渲染、owner 处置再归一、中央 policy 分类、非 advisory 指纹集合、返修轮作者呈现、返修影响闭合七个消费面分别处理
- **THEN** 七个消费面 MUST 得出同一「是否有效必关闭」结论；任一面 MUST NOT 因只读 `severity`、`class_hint` 或 `verdict` 字面值而得出与该结论相反的结果

#### Scenario: 本 Work Item 的界定与 WorkItemPlan 作用域取值

- **WHEN** 判定一条契约类 finding 是否属「本 Work Item」并是否启用契约类例外
- **THEN** 「本 Work Item」MUST 以该 finding 的 `target_work_item_id` 指向的 Work Item 界定；WorkItemPlan 的 `outline` / `item` / `batch` 三种 review scope 逐 finding 按该界定启用例外（`outline` 下无 Work Item target 的结构级 finding 不在例外内），非 WorkItemPlan 的 legacy 评审不启用；平台 disposition 硬化已在通用 strict / lenient 解析器无条件生效，MUST NOT 被收窄到 WorkItemPlan，且只有 severity 豁免是 WorkItemPlan 专属，MUST NOT 扩散到通用评审

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

- **WHEN** 以 `category`（`contract_gap` / `self_contradiction`）× `severity`（`suggestion` / `must_fix` / `blocking`）× `disposition`（`actionable` / `advisory` / 缺省）× `class_hint`（`repairable` / `human_required` / `advisory` / 缺省）× `verdict`（`pass` / `revise` / `needs_human`）构成矩阵，并代入含五条真实 finding 的基线样本
- **THEN** 恰有一条契约类 finding 进入必关闭集合，整轮结论 MUST NOT 为自动通过，该 finding 的原 severity 与 key MUST 不变；无 `contract_field` 的那条 MUST NOT 自动调用 author；其余四条 MUST 仍不扣预算、不阻断
