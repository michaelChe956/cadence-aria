# Spec Delta

## ADDED Requirements

### Requirement: 面向 owner 的人工裁决呈现义务（REQ-WSC-11）

reviewer 输出 `verdict=needs_human` 时，系统 SHALL 要求其 findings 至少一条同时携带 `owner_question`（一句话人话提问：卡在哪 + 为什么要你决定）与 `owner_options`（**至少 2 个**互斥选项，每项含非空 `label` 与 `consequence`；不设上限）。二者 SHALL 用日常语言表达，MUST NOT 以内部字段名、枚举值、指纹或追踪 ID 作为唯一呈现。缺失或空白 SHALL 判结构化输出不合规，并按既有 `structured_output_diagnostic` 通道收口。`pass` 与 `revise` SHALL NOT 被要求提供这两个字段，其呈现 MUST 保持不变。

#### Scenario: needs_human 带选项 → 界面呈现人话问题与选项

- **WHEN** 某轮 reviewer 输出 `verdict=needs_human`，且其一条 finding 携带 `owner_question="WI-004 还要不要继续负责 scripts/dev.sh 的改动？"` 与两个 `owner_options`（各含 `label`/`consequence`）
- **THEN** 结论卡**与人工门卡主区** SHALL 先呈现该人话问题、逐个选项的 `label` 与 `consequence`，以及「把选项名原文作为补充指令发回去」的指引（门卡是 owner 真正动手的地方，不得只给技术原文）
- **AND** 该轮技术 findings（`message`/`evidence`/`required_action`）SHALL 折叠在「技术细节」内、默认收起，不被删除

#### Scenario: needs_human 缺字段 → 判不合规并走既有诊断通道

- **WHEN** reviewer 输出 `verdict=needs_human`，但没有任何 finding 同时携带非空 `owner_question` 与非空 `owner_options`（缺失、全空白或空数组）
- **THEN** 系统 SHALL 判该结构化输出不合规，诊断码为 `missing_owner_guidance`；有效选项仅 1 条（不构成互斥选择）同判不合规
- **AND** 该轮 SHALL 经既有 `structured_output_diagnostic` 通道落 `needs_human` 结论卡（保留原始输出预览与 reviewer comments），MUST NOT 新造第二套上报或展示机制

#### Scenario: 选项字段半截 → 判不合规

- **WHEN** 某条 finding 只填了 `owner_question` 或只填了 `owner_options`，或某个选项缺 `label`/`consequence`（trim 后为空）
- **THEN** 系统 SHALL 判结构化输出不合规（`invalid_finding_field`），不得把半截字段呈现给 owner

#### Scenario: pass 与 revise 呈现不变

- **WHEN** reviewer 输出 `verdict=pass` 或 `verdict=revise`（含 `revise_batch`、`plan_reopen_required` 等自动返修结论），且未携带人话字段（即全部契约合规 payload 与全部存量记录）
- **THEN** 系统 SHALL NOT 要求 `owner_question`/`owner_options`，结论卡与门卡 SHALL 沿用既有 findings 分组呈现，不出现人话裁决区，也不折叠技术 findings

#### Scenario: 要求写的人话字段必在至少一张卡可见

- **WHEN** 模型宣称 `verdict=needs_human` 并写出人话字段，但同轮存在 `blocking`/`must_fix` finding 使归一后 verdict 变为 `revise`
- **THEN** 该 payload 的人话字段 SHALL 至少在结论卡或人工门卡之一可见（呈现以「是否携带人话字段」为准，不以 verdict 字面为准），MUST NOT 出现「被强制要求写、却永不上屏」的死字段

#### Scenario: 存量 reviewer 消息重解析不被新判据拦下

- **WHEN** 系统从本 change 之前的 reviewer 消息文本重解析出 `needs_human`（该记录不可能带人话字段）
- **THEN** 历史重解析入口 SHALL 不强制人话字段，SHALL 保留既有 `summary` 与 `findings`（作者修订轮仍需待关闭 finding 清单）；live 入口对同一 payload 仍判不合规

#### Scenario: 复评轮漏写人话字段不被升级成协议失败

- **WHEN** 单候选 Verification scope 的复评轮模型输出 `needs_human` 但未写人话字段
- **THEN** 系统 SHALL 保留诊断码 `missing_owner_guidance` 并落 needs_human 结论卡，MUST NOT 升级为 `verification_scope_violation` / durable failed

#### Scenario: 全部面向 owner 的审查路径同一口径

- **WHEN** 任一 reviewer prompt builder 构建输出契约（通用 workspace 评审、单候选 WorkItemPlan outline 评审、Plan Projection 评审、Work Item Plan Outline 评审、单 Work Item Draft 评审、整 Batch 评审、单候选 Plan 评审）
- **THEN** 该 prompt SHALL 携带同一份面向 owner 的呈现硬约束，且其示例 schema SHALL 给出 `owner_question`/`owner_options` 字段形状，使各路径不出现「一类有人话、一类没有」的分叉
