# 规范增量：把审查 finding 绑定到 Work Item 范围

<!-- 来源：owner 2026-10-30 真机反馈（workspace_session_auto_e5f6b290-f231-479a-9c5c-c7d63c93515e）
     与会话裁定：取舍 1=B（范围外事实报但降级 advisory）、取舍 2=A（逐条「排除此项」落 durable
     并回喂下一轮）。本 delta 全部为 ADDED：不改 REQ-WSC-01..11（既有单候选契约与 owner 人话裁决
     呈现义务），不改并行改动新增的 REQ-WSC-15（门卡「排除此项」UI 呈现）。 -->

## ADDED Requirements

### Requirement: 审查 finding 必须落在 Work Item 范围内，范围外事实降级为 advisory（REQ-WSC-12）

每条 finding MUST 显式声明 `disposition`：`actionable`（本 Work Item 可通过修订自己收敛）或 `advisory`（只作提示）。平台 MUST 只把 `actionable` 计入阻断判定、返修触发与必关闭 finding 清单。

指向本 Work Item 范围之外的事实 MUST 标 `advisory`，其 `required_action` MUST NOT 要求作者执行。缺失 `disposition` MUST 视作 `actionable`（fail-safe）；值非法 MUST 按既有 `invalid_finding_field` 判不合规；历史记录 MUST 照旧可解析。finding MAY 携带 `target_work_item_id`。返修触发路径与契约类缺口的平台加固见 REQ-WSC-17。

#### Scenario: 范围外文档缺口降级为 advisory、不阻断

- **WHEN** reviewer 发现仓库里某份产品侧规范文档仍写着「中文 locale 尚未配置」，而本 Work Item 的写入范围只有代码与语言包
- **THEN** 该 finding MUST 带 `disposition=advisory`，`required_action` 只作提示而不要求作者执行；平台 MUST NOT 因此判定返修，owner MUST 仍能在门卡上看到它

#### Scenario: 本 Work Item 的验收标准缺口仍是 actionable

- **WHEN** reviewer 发现本 Work Item 的 `busi/gateway` 能力契约缺少「非数字 id → 400」的验收标准
- **THEN** 该 finding MUST 带 `disposition=actionable`（或缺失，按 fail-safe 视作 actionable），MUST 进入阻断与返修通道

#### Scenario: 模型把 advisory 误标为 must_fix 也不阻断

- **WHEN** reviewer 输出 `severity=must_fix` 同时 `disposition=advisory`
- **THEN** 平台 MUST NOT 因该 finding 把 `pass` 归一为返修、MUST NOT 把它列入必关闭清单——判定在平台侧，不依赖模型自觉标对

#### Scenario: 缺失 disposition 的历史记录仍阻断

- **WHEN** 平台解析一条 wire 上既无 `disposition` 也无 `target_work_item_id` 的 finding（历史记录或旧 provider 输出）
- **THEN** 解析 MUST 成功且该 finding 语义上等同 `actionable`，既有阻断行为逐字不变

#### Scenario: 非法 disposition 值判不合规

- **WHEN** reviewer 输出 `disposition: "maybe"`
- **THEN** 系统 MUST 以既有 `invalid_finding_field` 判结构化输出不合规，MUST NOT 猜测成任一合法值


### Requirement: 返修触发路径只认 actionable（REQ-WSC-17）

非 advisory 指纹闸门、transition budget 与 `revise` 归一 MUST 与阻断判定同口径：`advisory` MUST NOT 进指纹集合、MUST NOT 驱作者返修、MUST NOT 占 transition budget；`revise` 但无任何 actionable finding 时 MUST 归一为可通过或人工确认。`contract_gap` / `self_contradiction` MUST NOT 仅凭模型标 `advisory` 获得免阻断。

#### Scenario: 指纹闸门与返修预算只认 actionable

- **WHEN** 同一轮 verdict 里既有 `advisory`（哪怕标成 `must_fix`/`human_required`）又有 `actionable`
- **THEN** 非 advisory 指纹集合与 transition budget MUST 只由 `actionable` 构成；只有 `advisory` 时该指纹集合 MUST 为空、MUST NOT 消耗返修预算

#### Scenario: revise 但无 actionable 不驱作者返修

- **WHEN** reviewer 输出 `verdict=revise`，但全部 finding 都是 `advisory`
- **THEN** 平台 MUST 归一为可通过（或人工确认）的结论，MUST NOT 进入作者返修通道，MUST NOT 因此消耗 transition budget

#### Scenario: 契约类缺口不得以 advisory 漏检

- **WHEN** reviewer 把 `category=contract_gap` 或 `category=self_contradiction` 的本 Work Item 缺口标成 `disposition=advisory`
- **THEN** 平台 MUST 按 `actionable` 处理，MUST 让它进入必关闭清单（判定在平台侧，不依赖模型自觉）

### Requirement: owner 排除项落 durable 并回喂下一轮评审（REQ-WSC-13）

门卡 SHALL 为每条 finding 提供结构化「排除此项」，身份 MUST 是平台算的内容 key（`sha256(severity + \u{1} + message)` 前 16 位小写 hex）。该 key MUST 由平台在 WS 出站与门卡 metadata 给出，模型 MUST NOT 生成；live 与 rebuild MUST 算出同一个 key。

owner 的排除 MUST 落到会话 durable 已处置事实清单（含 key、message 摘要、处置方式），同 key 重复提交 MUST 幂等。下一轮评审 prompt MUST 携带该清单并声明已处置事实不得再报为 actionable；只有拿得到上轮上下文的路径才携带。

`human_gate_feedback` 的 `excluded_finding_ids` 缺省或空 MUST 与改动前逐字相同。排除-only 形态（`feedback` 为空且至少一个合法 key）MUST 被受理：落 durable 即成立，MUST NOT 建修订 turn，MUST NOT 以「反馈为空」拒收。

#### Scenario: owner 排除一条后不再被报为 actionable

- **WHEN** owner 在门卡上对「中文 locale 尚未配置」这条 finding 点「排除此项」，随后再提交一次反馈
- **THEN** 该 finding 的 key 与摘要 MUST 落 durable 已处置事实清单，下一轮 reviewer prompt MUST 携带该清单并声明不得重复报为 actionable

#### Scenario: 未处置的事实下一轮仍可报

- **WHEN** owner 只排除了 A 条，B 条（本 Work Item 的验收标准缺口）未处置
- **THEN** 下一轮 reviewer MUST 仍可把 B 报为 `actionable` 并阻断

#### Scenario: 重启后清单仍可读

- **WHEN** 平台重启后再次为该会话构建评审 prompt
- **THEN** 已处置事实清单 MUST 从 durable 读回同一批条目，MUST NOT 依赖内存态

#### Scenario: live 与 rebuild 算出同一个 key

- **WHEN** 同一份 findings 先经 live 解析投影到 WS 出站，再经 durable verdict 反序列化重建
- **THEN** 两条路径 MUST 给出逐字相同的 `finding_key`，前端不需要自己实现算法

#### Scenario: 排除-only 帧被受理且不建修订 turn

- **WHEN** owner 在门卡上点「排除此项」，客户端发来 `feedback=""` 且 `excluded_finding_ids` 含合法平台 key 的帧
- **THEN** 平台 MUST 把该 key 落 durable 已处置清单并回受理回执，MUST NOT 返回 `INVALID_HUMAN_GATE_FEEDBACK`，MUST NOT 建修订 turn（零 provider 启动）

### Requirement: 重复报告必须说明未收敛原因（REQ-WSC-14）

reviewer 重复报告同一事实时 MUST 在 `message` 说明「上一轮处置为何没有解决它」。事实已被 owner 处置且未变化时 MUST NOT 再报为 `actionable`。

已处置事实 MUST 由**平台**裁决，而不只是注入 prompt 期望模型自觉：`finding_key` 命中已处置清单的 finding MUST 在平台侧按 `advisory` 处理——即便模型仍报 `actionable`，MUST NOT 因此驱作者返修、MUST NOT 进必关闭清单、MUST NOT 占 transition budget。

门 turn 以 `validation_reject` 失败时，平台 MUST 给出被拒反馈原文、可读原因（不是错误码）、至少一条下一步动作与逐字诊断，并 MUST 落 durable 使重启后可读。

作者返修输入面 SHALL 同口径：被降级的 finding MUST NOT 以 `message`/`required_action` 进入作者返修 prompt 的「必须修复」清单，至多「仅供提示，本轮不要修改」且无正文。

#### Scenario: 同一事实再次出现必须带未收敛说明

- **WHEN** reviewer 在第二轮再次报告第一轮已报告过的同一缺口
- **THEN** 该 finding 的 `message` MUST 说明上一轮处置为何没有解决它，MUST NOT 只是换措辞重复原文

#### Scenario: 已处置且事实未变不得报为 actionable

- **WHEN** 某事实已在 `[human_dispositions]` 清单里，且事实本身没有变化
- **THEN** reviewer MUST NOT 把它报为 `actionable`；若平台仍收到 `actionable`，该 finding 的处置归属以 owner 已处置事实为准，MUST NOT 因此把流程卡在返修循环

#### Scenario: 门 turn 被拒后 owner 能看到原因与下一步

- **WHEN** 门 turn 因 `validation_reject` 终态失败（例如 owner 的反馈无法被平台转成可执行修订指令）
- **THEN** 平台 MUST 给出被拒反馈原文、可读原因、至少一条下一步动作与逐字诊断，并 MUST 把它们落到该会话的 durable 记录，使重启后仍可复盘

#### Scenario: 模型仍报 actionable 也不驱返修

- **WHEN** 某事实已在会话已处置清单里，而某轮 reviewer 仍把它报为 `actionable`（措辞相同、key 命中）
- **THEN** 平台 MUST 把它按 `advisory` 处理——MUST NOT 进入返修通道、MUST NOT 进必关闭清单、MUST NOT 占 transition budget；该判定 MUST 不依赖模型是否照 prompt 听话

#### Scenario: 刷新后被拒原因仍可读

- **WHEN** owner 在门 turn 被拒后刷新页面或重连
- **THEN** 门卡 SHALL 仍能呈现最近一次拒收的四要素（来自 durable 记录的重建投影），MUST NOT 只剩一条失败行

#### Scenario: 降级后的 finding 不进入作者返修输入面

- **WHEN** owner 已排除事实 A，某轮 reviewer 再报 A + B（B 是本 Work Item 可收敛的缺口），平台已把 A 降级为 `advisory`
- **THEN** 作者返修 prompt MUST 只把 B 的 `message` / `required_action` 列入「必须修复」清单；MUST NOT 含 A 的 `message` / `required_action`，A 至多以「仅供提示，本轮不要修改」的无正文提示出现；上一轮 finding 身份清单同样 MUST 先按会话已处置清单过滤，同一份 prompt MUST NOT 既称其为「上一轮未收敛项」又称其为「已被排除」

### Requirement: 人工门在拒绝与 advisory 场景下必须可操作（REQ-WSC-15）

一次反馈触发的修订被判 `validation_reject` 时，门卡 SHALL 呈现被拒反馈原文、可读原因、至少一条下一步动作与逐字编译诊断；这些字段全缺席时 MUST NOT 渲染空白块（回落既有失败行）。门卡 SHALL 把 `disposition: "advisory"` 的 finding 与需 owner 处理的 finding 分组隔离，原因行「需要处理 N 项」SHALL 只计 actionable，advisory 的 `required_action` MUST NOT 呈现为要求 owner 执行的祈使样式。

门卡 SHALL 提供逐条 finding 的「排除此项」，仅当该 finding 带平台算的 finding key 时才渲染（旧记录无 key → fail-closed）；发起失败 MUST NOT 显示为已排除。排除成功后 SHALL 即时显示「已排除」与「下一轮评审不再报为需处理」的说明。

#### Scenario: 被拒后门卡给出四要素

- **WHEN** 门 turn 以 `validation_reject` 终态失败且 durable 拒收记录四要素齐备
- **THEN** 门卡 SHALL 逐字呈现被拒反馈、可读原因、下一步动作与诊断原文，owner 无需猜测「预判」是什么

#### Scenario: 四要素全缺席不渲染空白块

- **WHEN** 门卡 metadata 不含任何拒收字段（旧记录或非拒收失败）
- **THEN** 系统 MUST NOT 渲染空白或只有标题的拒收块，MUST 回落既有失败行呈现

#### Scenario: advisory 与 actionable 视觉可分且计数只计 actionable

- **WHEN** 本轮 findings 含 1 条 advisory 与 2 条 actionable 的 must_fix
- **THEN** advisory 落「仅供参考（不阻断）」组且 `required_action` 呈弱化提示样式；原因行 SHALL 写「有 2 条必须处理项」，MUST NOT 计入 advisory

#### Scenario: 无平台 key 的 finding 不渲染排除操作

- **WHEN** 某 finding 来自历史记录（无平台算的 finding key）
- **THEN** 门卡 MUST NOT 渲染该行的「排除此项」，MUST NOT 以无 key 的引用发起排除

#### Scenario: 排除成功即时可见

- **WHEN** owner 点击某行的「排除此项」且发送成功
- **THEN** 该行 SHALL 立即显示「已排除」状态，门卡 SHALL 说明「下一轮评审不再报为需处理」，MUST NOT 等到下一轮刷新才可见

#### Scenario: 排除发送失败不冒充已排除

- **WHEN** 排除动作的发送返回 false 或抛错（门面不可用/被阻断）
- **THEN** 该行 MUST NOT 显示「已排除」状态，MUST 保持可再次发起

### Requirement: owner 可以撤销已提交的排除（REQ-WSC-16）

owner 排错后 SHALL 能撤销某条 finding 的排除。撤销 MUST 删除该会话 durable 已处置清单里的该 key（幂等），此后该 finding MUST 恢复为「可报为 actionable」：下一轮 reviewer prompt MUST NOT 再携带它，平台裁决 MUST NOT 再把它降级，撤销 MUST NOT 影响清单里的其它条目。

#### Scenario: 撤销后该 finding 恢复可报为 actionable

- **WHEN** owner 撤销某条此前排除的 finding key
- **THEN** 该条目 MUST 从 durable 清单删除；下一轮 reviewer 把它报为 `actionable` 时 MUST 照常阻断（与从未排除一致）

#### Scenario: 撤销幂等

- **WHEN** 同一 key 被重复撤销，或该 key 本不在清单里
- **THEN** 清单 MUST 保持不变、平台 MUST NOT 报错

#### Scenario: 撤销入口形状

- **WHEN** 客户端向 `POST /api/workspace-sessions/{session_id}/human-actions` 发送
  `{"type":"revoke_finding_exclusions","command_id":"<id>","finding_keys":["<key>"]}`
- **THEN** 平台 MUST 返回 200 与 `{"command_id":"<id>","state":"accepted","remaining_finding_keys":[...]}`（撤销后仍生效的排除项）
