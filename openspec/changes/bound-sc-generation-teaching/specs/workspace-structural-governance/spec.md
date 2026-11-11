## MODIFIED Requirements

### Requirement: 耐久授权与不乘法执行预算（REQ-SG-05）

每次自动结构执行 SHALL 绑定耐久授权kind、原node/run或SC turn、artifact/source/context hash、attempt key/上限、原业务请求、机器指令/prompt digest与审计。claim与真实start SHALL 分账，unknown外部调用等待不重播；实际start MUST 有ledger，WS/旧started不算证据。所有调用 MUST 经现gateway admission/full fingerprint，失配零adapter start/child、probe另计。anchor waiting SHALL 优先；显式fresh仍原run/turn/grant/attempt/prompt/hold、None resume、旧native失效，不重置尝试/结算。

SC 初轮的自动执行 SHALL 绑定同一耐久教学窗口。已关闭窗口 MUST 保留其关闭与消耗事实供审计与接管引用；MUST NOT 被删除、清空、复用或以新 source 重新计数，接管 MUST NOT 凭新 session ID 重新铸造机会。

#### Scenario: 逐载体授权与上限
- **WHEN** 系统为下列载体处理确定性结构拒收
- **THEN** SHALL 按表中的原授权、父身份、单权威及上限执行，普通/legacy/coding/review不得虚造SC三轮计费，上限/授权无法证明则零额外调用并显式系统故障；SC结算依REQ-CG-02/03/05，user/structural/mechanical事件及reason分层可重建

| 载体 | 授权与父身份 | attempt 单权威与物理上限 |
|---|---|---|
| SC 人工修订 | 已接受 typed feedback 的 turn/command + durable grant + source hash | 同 turn attempt key，含原调用最多2；L0/L1共用，无第3次 |
| SC 初轮 | 已有生成 run/reservation/phase | 仅已批准 RepairOnce 才原+1=2，否则1；不伪造人工 turn |
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
- **THEN** 平台 SHALL 把真实启动写入既有 provider-start ledger（以 launch 身份为键）并携带该 attempt 键关联，可关联真实 provider/run 与启动证据，MUST NOT 为同一次启动另插第二条记录；claim 记录本身 MUST NOT 被当作实际启动，窗口内的物理尝试上限 MUST NOT 因类别变化而增加

#### Scenario: SC 初轮窗口重放与 Unknown 等待

- **WHEN** 同一 SC 初轮窗口在 claim 之后发生崩溃、重连重投、显式 fresh，或外部启动结果为 Unknown
- **THEN** 平台 SHALL 读取同一耐久窗口：已启动不重播、Unknown 沿原等待/对账、确定未启动才可在原 key 与原授权下恢复合法动作；MUST NOT 新增预算或物理机会，MUST NOT 因新的 session 身份铸造窗口

#### Scenario: SC 初轮以 GenerationRun 事实承载教学窗口

- **WHEN** SC 初轮的四类可教学失败申请教学重驱
- **THEN** 授权载体 SHALL 为已有生成 run/reservation/phase（GenerationRun 授权事实 + 耐久教学窗口），教学重驱与原调用共享同一窗口、不另开 structure 槽；上限仍为仅已批准 RepairOnce 才原调用在内 2 次，否则 1 次

#### Scenario: 已关闭窗口仍可被接管引用

- **WHEN** SC 初轮教学窗口已关闭后发生自动转交互接管或审计追溯
- **THEN** 平台 SHALL 保留该窗口的关闭与消耗事实供审计与接管引用；MUST NOT 删除、清空或复用该窗口

#### Scenario: 关闭窗口不以新 source 重新计数

- **WHEN** 已关闭窗口之后出现重驱产生的新 source、新 session 或同请求重放
- **THEN** 该已关闭窗口 MUST NOT 以新 source 重新计数或重开，不产生新的教学机会

#### Scenario: 接管不凭新 session ID 铸造机会

- **WHEN** 自动转交互接管以新 session ID 继续同一业务
- **THEN** 接管 MUST NOT 重新铸造 C 教学机会，原 session 的 provider-start ledger 保持权威，接管只引用原窗口的关闭与消耗事实

#### Scenario: 不具备合法观察准入的 provider 路径不获 C 机会

- **WHEN** provider 路径未具备合法观察/准入（真实启动观察不可绑定或准入事实不可证明）
- **THEN** 该路径不得获得 C 额外教学机会，按本条零额外调用并显式回显证据缺失，不默认为支持
