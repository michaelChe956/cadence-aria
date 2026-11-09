## ADDED Requirements

### Requirement: generate 段教学重驱的统一授权窗口（REQ-WSC-28）

SC generate 段的教学重驱 SHALL 绑定原生成 run/reservation/phase，并使用 REQ-SG-05 的 SC 初轮载体。资格 SHALL 以 REQ-WSC-09 为唯一清单：parse 语法、事实完整的 typed lowering、事实完整的 typed IR 校验和确定性结构拒收均只可消费该同一窗口；MUST NOT 因失败阶段、诊断数量、provider 节点变化或类别变化建立独立机会。mixed Unknown/Operational、事实不完整、权限/上下文/基线不可证明的失败 MUST NOT 授予教学重驱。

同一候选生成链 SHALL 以原生成授权身份为锚，而非以重驱后新 source hash 为锚；每 candidate 至多一次额外教学驱动。只有已批准且耐久绑定的 RepairOnce 才可使用原调用之外的一个物理机会，原调用在内最多两次；无批准时最多一次。原窗口中的运行恢复已使用第二次物理机会时，教学重驱 MUST NOT 再产生第三次。

#### Scenario: 跨类别不乘法

- **WHEN** 同一生成链 attempt1 命中 parse/IR，attempt2 又命中 structure，或顺序相反
- **THEN** 不产生第三次启动，不新开 structure 槽，按 typed 工程故障停止并保持合法 refs

#### Scenario: 业务或运行错误不借教学

- **WHEN** 失败集合含 Operational/Unknown、基线读取失败、授权缺失或缺少必要 typed 事实
- **THEN** 零额外 author 调用；按已有等待/失败面回显真实原因，不把其渲染成模型可修语法错误

#### Scenario: 窗口只在初轮新授权建立

- **WHEN** 初轮新授权建立教学窗口后，A 或 B 的合法接力（机械返修、author 重写/反馈）再次进入生成链
- **THEN** 教学窗口 MUST NOT 被重建或重开；A/B 接力仍消费各自的 repair/turn 载体，不把 handler 再次进入当成新初轮

### Requirement: 通道 C 的独立有界预算归属（REQ-WSC-28A）

通道 C SHALL 是独立有界的生成教学机会，不消耗评审机械池 `max_repairs`，不消耗 author 重写/反馈池 `max_manual_repairs`；MUST NOT 增加 `repairs_used`、`manual_repairs_used` 或伪造用户 turn/hold/receipt。该归属不改变 A/B 池原有触发域、余额或计费语义，也不得由 A/B 池余额为 C 的第三次调用兜底。

#### Scenario: 一次教学不扣 A/B 池余额

- **WHEN** 通道 C 消费了唯一一次教学重驱机会并产生一次真实启动
- **THEN** 评审机械池 `max_repairs` 与 author 重写/反馈池 `max_manual_repairs` 的已用计数均不递增，不产生用户 turn/hold/receipt；该次教学自身的窗口消费计数递增且实际启动合计为 2

### Requirement: 教学机会的原子 claim 与耐久绑定（REQ-WSC-28B）

每次机会 SHALL 在任何外部启动前原子 claim，耐久绑定 authorization kind、原 generation/node/run 身份、业务请求及材料摘要、失败 source 引用与 hash、context digest、允许修改闭包、机器指令 digest、完整 prompt 引用与 digest、attempt key、物理上限与冻结执行策略摘要（通用字段，值由 explicit-sc-author-effort 填写）。初次调用尚无候选 source 时 SHALL 明确记录该事实并绑定原请求/材料，MUST NOT 伪造候选引用；重驱前失败原文与其 hash 必须可验证。授权、材料、上限或持久 claim 不可证明时 MUST 零额外调用并显式系统故障。

#### Scenario: 旧记录不可补造授权

- **WHEN** 历史会话只有旧 `started` 占位或没有可证明的原授权/窗口
- **THEN** 读取历史不产生授权，零额外调用并说明证据缺失，不从计数为零推断存在机会

#### Scenario: claim 不可证明即零额外调用

- **WHEN** 授权、材料、上限或持久 claim 中任一项不可证明
- **THEN** 平台 MUST 零额外调用并显式系统故障，MUST NOT 以「尚未记账」推断仍有剩余机会

#### Scenario: 历史样本不得补造 grant 与 actual-start

- **WHEN** 读取历史旧样本且缺少 grant 或实际启动证据
- **THEN** MUST NOT 事后补造 grant 或 actual-start 记录；缺失按证据缺失处理，零额外调用

### Requirement: 教学重驱的 claim 与真实启动分账（REQ-WSC-28C）

claim 与 actual start SHALL 分账。该 attempt 的实际启动 MUST 写入既有 provider-start ledger（以 launch 身份为键）并携带 attempt 键关联，可关联真实 provider/run 与启动证据；MUST NOT 为同一次启动另插第二条记录。预留占位、调用前时间戳、WS 事件及旧 `started=true` MUST NOT 冒充真实启动。外部结果 Unknown 时 SHALL 沿原等待/对账，不自动重播。新 source revision、新 native 会话、重连、恢复及同请求重放 MUST NOT 重置窗口。所有允许启动仍须经过原 gateway admission/full fingerprint；显式 fresh 保留原授权/窗口/冻结输入，不获取新 RepairOnce。

#### Scenario: IR 拒收使用唯一额外机会

- **WHEN** 原调用产生事实完整的 typed IR 失败，原生成授权批准 RepairOnce，尚有一个剩余物理机会
- **THEN** 平台以完整失败 source、typed 定位和允许修改闭包 claim attempt2，原 author 重驱一次，形成两个真实 start 证据；C 消费一次，A/B 池均不扣

#### Scenario: 同键重放与不确定启动

- **WHEN** claim 后崩溃、启动回执未知、重连重投或显式 fresh
- **THEN** 读取同一 durable 窗口；已启动不重播、Unknown 等待；确定未启动才可在原 key/授权下恢复合法动作，不新增预算或物理机会

### Requirement: 教学尝试流水按 attempt 关联（REQ-WSC-28D）

候选完整输出及每次 compile/IR/structure 判定 SHALL 按 REQ-WSC-25 追加可追溯流水，关联同一 generation 与 provider attempt，包含 source ref/hash、阶段、稳定码、typed 种类、环指纹、规范化/修复事实和处置结论。原始输出无法形成 source 时 SHALL 明确记录缺失原因，不伪造引用。流水只用于审计/度量，不作为启动许可；流水写失败按 WSC-25 记日志且不阻断，授权/ledger 的持久执法失败则按 SG-05 停止或进入 Unknown 等待，二者不得混用。

#### Scenario: 流水与启动账本失败处理不同

- **WHEN** WSC-25 流水追加失败，或者 SG-05 claim/实际启动记录持久化失败
- **THEN** 前者记运行日志且不阻断；后者不得继续无账启动/重试，已发生外部动作按真实证据进入 Unknown/补偿，不伪装未启动

### Requirement: 第二次交付失败后的停止与越权处置（REQ-WSC-28E）

第二次交付仍失败或出现修复范围越权时 MUST 停止该生成自动链、保留 last-valid refs 与失败历史，MUST NOT 提升坏 candidate、发起 review/confirm 或旁路至另一自动重写入口。后续只有真实新业务授权可按其既有合同继续，MUST NOT 由平台自动制造新 turn、candidate 或 generation 来补机会。本条仅适用于 **generate 段**原生成链的第二次交付；**门段** `final_compile_*` 候选编译类失败不属于本条，其自动重写按 REQ-WSC-21 既有合同处理，REQ-WSC-21 的触发域不因本条收窄或扩张。「修复范围越权」的判据新码 `repair_scope_violation` 由裁决 §7 的 C 包交付，C 包落地前该判据不可判定。

#### Scenario: 第二次仍失败即停止

- **WHEN** 唯一教学机会已消费且第二次交付仍未通过编译或仍未满足结构判据
- **THEN** 平台停止该生成自动链并保留 last-valid refs 与失败历史；不产生第三次启动、不借 A/B 池余额、不自动制造新 turn 或 candidate 继续

#### Scenario: 门段候选编译失败仍按 REQ-WSC-21 驱动重写

- **WHEN** 门段 `final_compile_*` 候选编译类失败发生
- **THEN** 其自动重写按 REQ-WSC-21 既有合同处理（提示 owner、扣 author 重写次数预算），本条停止语义不适用于该门段失败

#### Scenario: 越权判据由 C 包交付（占位）

- **WHEN** 修复范围越权判据（新码 `repair_scope_violation`）尚未由裁决 §7 的 C 包交付
- **THEN** 该越权分支在 C 包落地前不可判定，本条不据此产生可验收断言；第二次交付失败的停止语义不受此占位影响

## MODIFIED Requirements

### Requirement: 候选生成尝试可序列观测（REQ-WSC-25）

平台 MUST 以追加写记录每次候选生成与编译尝试，MUST NOT 只保留最近一次。每条记录 MUST 含尝试序号、source revision 引用、阶段、稳定码、typed 种类、环结构指纹、已应用的自动修复与等价规范化清单、处置结论。阶段 MUST 能区分 generate、parse、typed lowering、typed IR 校验与结构拒收；每条记录 MUST 关联同一 generation，若本次触发了 provider 启动则 SHALL 记录其 attempt 序号与 attempt key，使教学重驱与原调用出现在同一可追溯序列上，若本次未触发 provider 启动则 SHALL 显式记录「无 provider 启动」，MUST NOT 伪造关联。由此 MUST 可计算首次通过率、机械自修收敛率与模型重写次数。

#### Scenario: 连续失败可逐次追溯

- **WHEN** 同一 issue 连续发生多次候选失败
- **THEN** 平台 MUST 能列出每次失败所在阶段、稳定码与应用的自动修复，MUST NOT 只剩最后一次

#### Scenario: 记录读写失败不阻断生成

- **WHEN** 追加记录时发生写入错误
- **THEN** 平台 MUST 把该错误记入运行日志，MUST NOT 因此使候选生成或确认失败

#### Scenario: 阶段与 attempt 关联可追溯

- **WHEN** 同一 generation 发生原调用与一次教学重驱，且失败阶段跨越 parse 与 typed IR
- **THEN** 平台 MUST 能在同一流水序列上列出两个 attempt 各自的阶段、稳定码、source 引用与处置结论，MUST NOT 把两者合并为一条无法区分阶段的记录
