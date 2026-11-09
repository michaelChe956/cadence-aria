## MODIFIED Requirements

### Requirement: 耐久授权与不乘法执行预算（REQ-SG-05）

每次自动结构执行 SHALL 绑定耐久授权kind、原node/run或SC turn、artifact/source/context hash、attempt key/上限、原业务请求、机器指令/prompt digest与审计。claim与真实start SHALL 分账，unknown外部调用等待不重播；实际start MUST 有ledger，WS/旧started不算证据。所有调用 MUST 经现gateway admission/full fingerprint，失配零adapter start/child、probe另计。anchor waiting SHALL 优先；显式fresh仍原run/turn/grant/attempt/prompt/hold、None resume、旧native失效，不重置尝试/结算。

SC 初轮的自动执行 SHALL 以该初轮既有的 GenerationRun 授权事实为唯一授权载体，并落在同一份耐久教学窗口上：窗口 SHALL 绑定原 generation/node/run 身份、业务请求及材料摘要、失败 source 引用与 hash、context digest、允许修改闭包、机器指令与 prompt digest、attempt key 及物理上限；窗口 MUST NOT 以重驱后的新 source hash 为锚，MUST NOT 因新的 session 身份或接管而铸造，MUST NOT 保留已关闭的窗口。窗口内的物理启动 MUST 记入既有 provider-start ledger 并与授权 claim 分账：授权 claim 成功不等于已启动，实际启动观察缺失时按 Unknown 等待而非按未启动重播；预留占位、旧 started 布尔与调用前时间戳不算实际启动证据。

#### Scenario: 逐载体授权与上限

- **WHEN** 系统为下列载体处理确定性结构拒收
- **THEN** SHALL 按表中的原授权、父身份、单权威及上限执行，普通/legacy/coding/review不得虚造SC三轮计费，上限/授权无法证明则零额外调用并显式系统故障；SC结算依REQ-CG-02/03/05，user/structural/mechanical事件及reason分层可重建

| 载体 | 授权与父身份 | attempt 单权威与物理上限 |
|---|---|---|
| SC 人工修订 | 已接受 typed feedback 的 turn/command + durable grant + source hash | 同 turn attempt key，含原调用最多2；L0/L1共用，无第3次 |
| SC 初轮 | 已有生成 run/reservation/phase（GenerationRun 授权事实 + 耐久教学窗口） | 仅已批准 RepairOnce 才原+1=2，否则1；不伪造人工 turn；教学重驱与原调用共享同一窗口，不另开 structure 槽 |
| Story/Design/单项普通 author | 持久 Revision node + 原 feedback + artifact hash；初轮用原 generation node | 已获单次 artifact retry 且 provider/路径合法时原+1=2；Pi/Kimi或无合法 gateway 续写资格为1 |
| legacy 整计划 revision | 原 revision run/候选版本/hash/overall feedback | 原总3次耐久单权威，不加第4次 |
| draft/batch | 原 generation round/outline/batch/draft run/feedback | 已有 RepairOnce 为原+1=2，无机会为1 |
| coding/review | 已批准work item/group/coder/reviewer原role-run/cycle、attempt/scope/hash及teaching/repair窗口 | coder每cycle3＋原恰一次teaching；code-review同cycle最多3；group-review原transport3＋一次repair（16KiB）。各窗口同耐久parent关联、L1只用原剩余机会；不统一重排为总3、不附加池、不借SC用户预算 |

#### Scenario: attempt2 已用于运行恢复

- **WHEN** SC 同 turn attempt2 已消费于死 provider 恢复而随后结构仍失败
- **THEN** 不启动第3次、不借用户剩余轮次、不按每 finding 重置；未合法交付则释放该 hold 并呈工程故障

#### Scenario: 指纹或 anchor join 错误

- **WHEN** full map/fingerprint 漂移或显式 fresh recovery/turn/grant/attempt/prompt digest 不匹配
- **THEN** 零 author adapter start/child，预算与 refs 不改；不 silent fresh、不伪造指纹相等

#### Scenario: 同键崩溃重放

- **WHEN** 指令冻结、claim、产物写入、current refs CAS、结算或复评各断点重启
- **THEN** 同 run/turn/attempt/prompt/receipt 继续，不重复 start/charge/release；Reviewing 只恢复绑定评审，已交付不误 Completed 或退款，已终态 grant 拒旁路

#### Scenario: SC 初轮窗口与真实启动分账

- **WHEN** SC 初轮在已批准的 RepairOnce 下申请一次教学重驱，平台已原子 claim 该 attempt key
- **THEN** 平台 SHALL 以同一 attempt key 把真实启动写入既有 provider-start ledger 并可关联真实 provider/run 与启动证据；claim 记录本身 MUST NOT 被当作实际启动，窗口内的物理尝试上限 MUST NOT 因类别变化而增加

#### Scenario: SC 初轮窗口重放与 Unknown 等待

- **WHEN** 同一 SC 初轮窗口在 claim 之后发生崩溃、重连重投、显式 fresh，或外部启动结果为 Unknown
- **THEN** 平台 SHALL 读取同一耐久窗口：已启动不重播、Unknown 沿原等待/对账、确定未启动才可在原 key 与原授权下恢复合法动作；MUST NOT 新增预算或物理机会，MUST NOT 因新的 session 身份铸造窗口
