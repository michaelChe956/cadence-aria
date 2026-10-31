# 规范增量：候选编译失败必须可读、可行动、不可再死路

<!-- 来源：owner 2026-10-31 真机反馈（workspace_session_auto_3b4c7476-f300-40ff-abb7-81d0956f78fd）
     + 同日 owner 两条裁定：
     ① 「不应该出现这个成环的情况，应该拒绝」——编译不过的候选不应出现在 owner 可确认的集合里；
     ② 「就算成环了要有醒目的提示，让用户重新生成，并提醒用户可能 design 都需要重写，
        重写的时候着重注意哪些」。
     本 delta 全部为 ADDED：不改 REQ-WSC-01..17。前端门卡呈现由并行改动实现，契约条款在本 change 内成文。 -->

## ADDED Requirements

### Requirement: 候选编译失败必须以完整可读原因呈现在人工门（REQ-WSC-18）

SC Final Compile 的失败 MUST 以**完整**诊断 durable 落盘（不截断）：稳定码、人话原因、诊断原文。门卡 `session_state` MUST 带 `candidate_compile_failure{code,message,detail?,owner_hint?,rewrite_guidance?}`，刷新/重连后仍在。只有**当前候选**（source revision 与 candidate IR 双侧与会话 durable 值逐字相等）的失败才呈现；旧候选失败与任一侧 ref 缺席 MUST 不呈现。

#### Scenario: 依赖环失败在门卡上完整可读

- **WHEN** Final Compile 以 `final_compile_prepare_failed` / `dependency_cycle` 失败并落人工门
- **THEN** 该会话 durable 落有 `candidate-compile-status.json`，`detail` 含 `failure_id` 与 typed 渲染原文（逐字、不截断）；`session_state.candidate_compile_failure.message` MUST 是人话（点名哪两个 Work Item、谁把谁写成 provider、为什么与仓序自动边合成环）

#### Scenario: 刷新后原因不消失

- **WHEN** owner 刷新页面或服务重建会话
- **THEN** `session_state.candidate_compile_failure` MUST 仍带同一 `code` / `message` / `detail`（来自 durable 读取，不依赖内存 stash）

#### Scenario: 旧候选的失败不得冒充当前候选

- **WHEN** author 已重新生成候选（source revision 或 candidate IR 变化），库中仍留有上一候选的失败记录
- **THEN** 门卡 MUST NOT 呈现该失败，确认判据 MUST NOT 因此拦截

#### Scenario: 编译成功不得残留过期失败

- **WHEN** 同一候选的编译尝试后来成功
- **THEN** durable 候选编译事实 MUST 被覆盖为「无失败」；门卡 MUST NOT 继续呈现「当前候选编译不过」

### Requirement: 候选编译不过时确认动作不得形成死路（REQ-WSC-19）

`human_confirm` 之后的 Final Compile 是必经步骤，确认动作必然重跑同一次编译。当前候选已确定编译不过时，平台 MUST fail-closed 拒绝放行（零 durable 副作用、门保持打开）并给出原因与出路；`candidate_compile_failure` 在场即是该判据的呈现面。判据 MUST 在平台侧，MUST NOT 依赖前端自行猜测。

#### Scenario: 确认当前版本被拒且给出出路

- **WHEN** 当前候选的 Final Compile 已失败，owner 仍提交「确认当前版本」
- **THEN** 平台 MUST 以稳定码 `CANDIDATE_COMPILE_FAILED` 拒绝（会话状态、相位、门快照、预算 MUST 全部不变），文案 MUST 说明「确认会原样再失败一次」并给出下一步（让 author 重新生成 / 终止）

#### Scenario: 两条确认入口同一判据

- **WHEN** 确认来自人工门 close（`close_human_gate` Approve）或既有 confirm 端点（SC Approval 臂）
- **THEN** 两条入口 MUST 走同一平台判据并给出同一稳定码，MUST NOT 出现一条放过、一条拒绝

#### Scenario: 无法证明时不拦

- **WHEN** 失败记录或会话记录任一侧缺少候选 refs，无法证明失败属于当前候选
- **THEN** 确认 MUST NOT 被拦截（fail-open）；平台 MUST NOT 凭猜测把门永久挡死

#### Scenario: 终止不受影响

- **WHEN** 当前候选编译不过，owner 选择终止
- **THEN** 终止 MUST 照常执行（fail-closed 只作用于放行确认，不阻断退出）

### Requirement: 候选编译失败必须给 owner 出路与 author 重写要点（REQ-WSC-20）

候选编译类失败时，owner 面 MUST 醒目声明「该候选无效、不是待确认版本」，MUST 讲清是 design 的什么原因导致成环并无法推进、MUST 明确「需要 owner 修改 design」，MUST NOT 表述为「系统会自动去改 design」。同一组技术要点 MUST 进入给 author 的教学面，但 MUST NOT 要求 author 去改 design（design 只由 owner 决定是否修改）。

#### Scenario: 门卡醒目提示候选无效并点明 design 根因

- **WHEN** 门卡呈现 `candidate_compile_failure`
- **THEN** `owner_hint` MUST 说明该候选无效（非待确认）、讲清 design 的仓序/依赖声明与候选依赖方向矛盾导致成环、并要求 owner 修改 design（MUST NOT 写成平台自动改 design）；`rewrite_guidance` MUST 至少给出四条要点（provider 只能指向仓序在前者、同仓顺序只能靠 `depends_on`、依赖图必须无环、根因可能在 design 需 owner 决定）

#### Scenario: author 收到具体重写要点且不被要求改 design

- **WHEN** 当前候选编译不过，owner 在门内提交反馈触发 author 修订
- **THEN** 修订提示词 MUST 含 `[candidate_compile_failure]` 段：稳定码 + 人话原因 + 逐条技术要点，并明确「再犯同样的错仍会失败」；该段 MUST NOT 要求 author 去修改 design

#### Scenario: 无编译失败时既有提示词逐字不变

- **WHEN** 当前候选没有编译失败
- **THEN** 门内修订提示词 MUST NOT 出现该段，既有内容逐字保持不变

#### Scenario: 依赖环人话只写真实事实

- **WHEN** 环诊断由 change_order 与既有依赖合成
- **THEN** 人话 MUST 基于真实环 witness（有向链、各 Work Item 所在仓的 change_order 位次、逆仓序的那条依赖）；找不到 witness 时 MUST 给通用可行动文案，MUST NOT 虚构环形状

### Requirement: 候选编译类失败首次即提示 owner 并继续自动重写（REQ-WSC-21）

候选自身无效类失败（至少 `final_compile_prepare_failed` / `final_compile_structure_rejected`）首次发生时，平台 MUST 立即向 owner 呈现醒目提示与重写要点（不等重写结果），MUST 同时按剩余预算**真正驱动** author 重写（复用既有机械返修通道，不新造状态机）。预算 MUST 是**author 重写次数预算**（初值 `max_manual_repairs`，门卡上 owner 看到的数字同一口径），**平台自动重写与 owner 反馈触发的重写扣同一个池**，每轮消耗 1；编译成功 MUST 清零。`max_repairs`（机械返修池）MUST NOT 被当作自动重写预算。

#### Scenario: 第一次失败就出提示

- **WHEN** 候选第一次因依赖环失败
- **THEN** 门卡/气泡 MUST 立刻带 `candidate_compile_failure`（含 `auto_rewrite.phase="running"` 与剩余次数），MUST 说明「由于 design 的什么原因导致成环并无法推进、需要修改 design」，MUST 给出重写要点

#### Scenario: 提示不妨碍重写

- **WHEN** 提示已呈现且预算尚有余
- **THEN** 平台 MUST 继续自动重写，MUST NOT 因已提示而停止；剩余次数 MUST 随每次重写递减

#### Scenario: 预算用尽后停并交人工门

- **WHEN** author 重写次数预算（`max_manual_repairs`，含平台自动重写消耗）用尽
- **THEN** 平台 MUST 停止自动重写，MUST 以 `auto_rewrite.phase="stopped"`（`remaining=0`）呈现，并交人工门由 owner 决定

#### Scenario: 两个预算池互不挪用

- **WHEN** 平台自动重写触发一轮 author 重跑
- **THEN** MUST 消耗 author 重写次数预算（`max_manual_repairs` 口径，与 owner 反馈 turn 同一池）；MUST NOT 消耗 `max_repairs`（机械返修池，用于不重跑 author 的机械 repair）

#### Scenario: 非候选无效类失败不宣称自动重写

- **WHEN** 失败稳定码不属于候选自身无效族（如 store/IO 类 operational 失败）
- **THEN** 平台 MUST NOT 出现 `auto_rewrite` 字段，MUST NOT 声称已启动自动重写

### Requirement: 同一依赖环重复出现必须提前停并升级提示（REQ-WSC-22）

自动重写若产出**同一个依赖环**，平台 MUST 判定 plan 层面无法修复，MUST 停止剩余自动重写预算，MUST 把提示升级为「自动重写无法解决，请修改 design」，并 MUST 交人工门。同一环的判据 MUST 是结构（成员集合 + 边集合的规范化摘要），MUST NOT 用文案、行号或编译运行 id。

#### Scenario: 同一环第二次出现即停（与预算无关）

- **WHEN** 第二次失败的结构指纹与上一次相同（即使 author 重写预算仍有剩余）
- **THEN** 平台 MUST 立即停止自动重写（不再消耗剩余预算），呈现 `auto_rewrite.phase="stopped"` 与 `repeated` 次数，并进人工门——预算管「总共能重写几次」，结构判据管「这次重写有没有意义」，两者分别成立

#### Scenario: 换措辞/改标题但同一组环仍判重复

- **WHEN** author 重写后 Work Item 标题或依赖描述换了措辞，但环的成员与边集合不变
- **THEN** 平台 MUST 判为同一个环（结构指纹相同），MUST NOT 当成新事实继续消耗预算

#### Scenario: 另一个环是新事实

- **WHEN** 重写后出现结构不同的另一个依赖环
- **THEN** 平台 MUST 允许按剩余预算继续自动重写，`repeated` MUST 从 1 重新计

#### Scenario: 指纹跨尝试稳定

- **WHEN** 同一组环在不同编译尝试（编译运行 id 不同）中再次出现
- **THEN** 结构指纹 MUST 相同（成员用「编译序序号 + target 仓」表达，不掺运行身份）
