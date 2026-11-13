## MODIFIED Requirements

### Requirement: 候选编译类失败首次即提示 owner 并继续自动重写（REQ-WSC-21）

候选自身无效类失败（至少 `final_compile_prepare_failed` / `final_compile_structure_rejected`）首次发生时，平台 MUST 立即向 owner 呈现醒目提示与重写要点（不等重写结果），MUST 同时按剩余预算**真正驱动** author 重写（复用既有返修路由通道，不新造状态机）。预算 MUST 是**author 重写次数预算**（初值 `max_manual_repairs`，门卡上 owner 看到的数字同一口径），**平台自动重写与 owner 反馈触发的重写扣同一个池**，每轮消耗 1；编译成功 MUST 清零**该候选编译链上「平台自动重写」这一段的已计费消费**（同一 logical gate 内已由 owner 反馈消费或已预留的额度 MUST NOT 因此被返还，按 REQ-HGC-01 的同门接续与 REQ-CG-02 的预留 / 结算 / 退款既有语义处理）。`max_repairs`（机械返修池）MUST NOT 被当作自动重写预算。

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

#### Scenario: 自动重写与反馈互扣同一余额

- **WHEN** 平台自动重写已消耗 1 次 author 重写额度，且 owner 随后在同一 logical gate 内提交一次反馈
- **THEN** 平台 MUST 从**同一个** author 重写次数预算中再扣 1 次（可用余额如实递减，如 3→2→1）；MUST NOT 为两者各保留一份互不相干的余额；MUST NOT 因 owner 反馈而使自动重写侧的可用额度被重置或另算一份

#### Scenario: 余额只有一个权威事实

- **WHEN** 平台查询 author 重写次数预算的可用余额
- **THEN** MUST 只有**一个**权威余额（同一预算域、同一扣减序列）；MUST NOT 出现「自动重写侧余量」与「反馈侧余量」两个互不相干的数字；MUST NOT 由展示端对两个计数取 max / min / 相加推算余额；同一 command 重放 MUST NOT 重复扣减

#### Scenario: 编译成功只清编译链上的自动重写消费

- **WHEN** 候选编译成功
- **THEN** 平台 MUST 清零该候选编译链上「平台自动重写」的已计费消费（其诊断计数随之归零）；MUST NOT 返还同一 logical gate 内已由 owner 反馈消费或已预留的门反馈额度；该门反馈额度 MUST 继续按 REQ-HGC-01（同一 logical gate 内 carry-forward）与 REQ-CG-02（预留 / 结算 / 退款）的既有语义处理

#### Scenario: 平台自动重写不得消耗机械返修池

- **WHEN** 平台因候选编译失败触发一轮 author 重写（平台自动重写）
- **THEN** 该轮 MUST NOT 递增机械返修池的轮内计数（`repairs_used`）与会话观测计数；MUST NOT 因机械返修池已用尽而阻断平台自动重写；平台自动重写能否继续 MUST 只由 author 重写次数预算的可用余额决定
