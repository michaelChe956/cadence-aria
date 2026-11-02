# Spec Delta

## Purpose

五阶段 workspace 的确定性结构问题由系统事实、同源规则、机器指令及既有授权 author 闭环处理，避免技术治理外包给用户；合法产物、业务裁决、安全授权、耐久预算与失败审计保持分域且可重建。

## ADDED Requirements

### Requirement: 四层判定与五阶段覆盖（REQ-SG-01）

系统 SHALL 将失败区分为 L0 同输入运行恢复、L1 确定性结构治理、L2 已授权范围技术逻辑及 L3 业务决策。L1 资格 MUST 来自完整服务端事实与可生成精确指令的确定性规则，不能取自由文本、模型 category/class_hint/severity 或未知码为许可；结构解不唯一不改变 L1 资格。story/design/plan/coding/review SHALL 均在各自真实 schema、scope、权限及已授权 run 内进行 L1 自动结构修复，不只消费统一文案，不给 Story 或 coding/review 虚造 WI 规划图。只有证据证明全部可行方案必须改变已声明业务范围、AC、真实数据流或优先级时 SHALL 请求用户业务裁决；机器未找到解不是这种证明。operational、安全/授权事实缺失及 reviewer 协议坏 SHALL 沿原安全等待/协议恢复，不借 L1 启动 author。

#### Scenario: 技术多解不要求用户填结构
- **WHEN** 有效图环可通过两个保持业务要求的结构方案消除
- **THEN** 系统将事实、候选约束与保护项交既有 author 选择，用户不必选仓 UUID、删依赖或写修订 prompt

#### Scenario: coding 与 review 纳入而不扩权限
- **WHEN** coding/review 载体在已授权执行或评审中出现其确定性 schema/scope 结构问题
- **THEN** 服务端生成机器指令，按既有该载体 author/coder/协议修复授权与剩余物理机会自动执行并复验；未知启动结果或无机会时等待/工程故障，不能改称用户技术反馈或新开池

#### Scenario: 真实业务矛盾
- **WHEN** 证据证明保留全部既有 AC 与业务交付顺序无法同时满足
- **THEN** 原业务门只呈业务后果选项，用户决定转成结构约束；不让用户修 heading 或填写仓 UUID

### Requirement: 全量结构门与机器指令（REQ-SG-02）

系统 SHALL 每次交付先执行适用的 grammar/schema、canonical、split/preflight、target/成员归属、有效图及 scope/intent/trusted commands/projection 同构全量校验，再提升当前合法候选并启动语义 reviewer。失败原始输出 SHALL 留作 attempt evidence，不成为可审核 current artifact。机器指令 MUST 绑定 failure/repair identity、规则版本、source hash/context digest、定位、实际边及来源、允许修改闭包、protected business facts、精确操作或候选、中文 required_action、expected predicates、可用历史 refs 与 instruction digest；未解析先修格式，不猜 IR 图。

#### Scenario: 第414行非法二级标题
- **WHEN** 输出第414行出现非法非 WI `##` heading，诊断同时含共享 code、field、line 与 grammar example
- **THEN** 系统形成精确 typed heading finding，保留原业务段与行，机器指令携合法模板重跑原 author；不按共享 unknown key 码删行，不提前推用户 Failed，不提升或评审坏 source

### Requirement: 内容修复作者来源（REQ-SG-02A）

所有新内容修复，包括局部唯一表达修复、原范围内 autorepair 补 capability/删 key/handoff/ref 等变化，SHALL 经原授权 author 真实调用、不可变新产物及全量复验/原复评，MUST NOT 系统直 patch 已持久或已评审 Markdown/IR。已有交付前前言修剪、固定中文 heading→英文和 EARS 空白规范化仍可保留，但不得作为新语义修改旁路；保护项 SHALL 拒绝删除真实业务供需/AC 或扩大权限的假修复。

#### Scenario: 唯一解仍保作者来源
- **WHEN** 固定 grammar 映射或完整枚举局部域证明唯一修复
- **THEN** 系统自动选定该指令交 author 执行，用户零操作；新 refs 经完整校验与评审，不原位修改 immutable 对象

#### Scenario: 禁止假删边
- **WHEN** author 删除必需 Inputs/AC、关闭必需 E2E 选项、只改 Dependencies 但 Inputs 不变或改变已确认仓序来消环
- **THEN** 保护项及全量校验拒绝，不能以无环替代业务正确或扩大权限

### Requirement: 系统门与 reviewer 责任分立（REQ-SG-03）

reviewer SHALL 只消费已通过系统结构门的同代 refs、报告/context digest 及修复审计，负责开放语义判断；模型 Pass SHALL NOT 覆盖系统阻断。疑似结构问题 SHALL 仅触发服务端同套件复检，服务端未证实则返回原语义 finding，不把模型提示当修复许可。Final Compile SHALL 再验证冻结候选与当前合法上下文，不静默重新 lower 最新 artifact 偷换批准物；refs/context 漂移是具体工程等待/故障。review 通过仍回原人工确认门，不自动 confirm/compile/advance。晚发现结构缺口仅能沿仍有效原授权及剩余机会处理或保留/恢复合法 refs 回原门；终态 grant 不可新开 author。

#### Scenario: 坏候选不送 reviewer
- **WHEN** candidate 的 Inputs×仓序有效图有环
- **THEN** 当前 refs 不提升、该候选 reviewer start 为零、确认按钮不可用，机器修复先行；模型 Pass 不能解除阻断

#### Scenario: Final Compile 晚发现
- **WHEN** 冻结候选的最新成员/Design/context 不再合法且原 author 授权已终态
- **THEN** 系统阻止发布并保留最近合法 refs，呈具体系统故障/现恢复操作；不新 turn、不要求用户写拓扑、不把 compile recovery continue 变 author 生成

### Requirement: 合法历史 checkpoint 与结构防回归（REQ-SG-04）

系统 SHALL 为合法交付保存 grant结束后仍保留的耐久 checkpoint，关联 artifact version、source node、source/IR/report refs、hash、context digest、authorization、delivery receipt 及 review scope/status。历史恢复 SHALL 按当前 Design/member/options/baseline 重验同scope/hash整组refs；缺join只能供author参考，不补造旧评审IR。仅无发布/执行副作用且当前请求同授权业务digest或本轮无新增业务请求时可纯状态回绑；否则经author融合。回绑 MUST 同CAS失效旧approval/approved_at/compile reservation/publication provenance、append审计，不复活批准、不把旧receipt计本轮交付；原审核不能绑定当前refs/context/scope则复评。

#### Scenario: R3 重新引入冲突
- **WHEN** 上一合法候选已解 WI-005/WI-004 冲突而新输出再次引入该冲突
- **THEN** 系统在提升 current refs 前阻断，指令含 resolved predicates、旧合法 refs 与差异；不能只依赖模型记忆或用户再次代写反馈

#### Scenario: 旧结构满足本轮业务
- **WHEN** 历史 source/IR/report 完整、当前上下文重验通过且同业务请求已证明
- **THEN** 可整组纯状态恢复、不调用 author，旧 approval/reservation 失效，审核事实不能 join 时复评，不自动批准发布

#### Scenario: 历史与新业务漂移
- **WHEN** 历史缺 IR/report、hash/scope 不符、成员/Design 变化或新业务反馈尚未落实
- **THEN** 禁止整篇回退假交付或编造 refs；可用旧结构仅作 author 指令参考，新业务反馈完整保留

### Requirement: 结构谓词防回归（REQ-SG-04A）

系统 SHALL 耐久保存已解决结构谓词与证据，每次输出全量检查；相同冲突重现 MUST 产生结构回归 typed finding，携旧合法 refs/差异/保护项，剩余机会内靶向修复，耗尽工程故障。允许保持业务要求的另一合法重组，不永久锁死边/target。失败指纹、指令digest、context与attempt身份 SHALL 防重复派发，context变化时重验旧约束。

#### Scenario: 回归与合法替代
- **WHEN** 后续候选重引旧冲突或采用另一保持业务要求的合法重组
- **THEN** 前者在refs提升前阻断并携旧事实靶向修复，后者允许通过；指令同context不无限派发，新context重新验证

### Requirement: 耐久授权与不乘法执行预算（REQ-SG-05）

每次自动结构执行 SHALL 绑定耐久授权kind、原node/run或SC turn、artifact/source/context hash、attempt key/上限、原业务请求、机器指令/prompt digest与审计。claim与真实start SHALL 分账，unknown外部调用等待不重播；实际start MUST 有ledger，WS/旧started不算证据。所有调用 MUST 经现gateway admission/full fingerprint，失配零adapter start/child、probe另计。anchor waiting SHALL 优先；显式fresh仍原run/turn/grant/attempt/prompt/hold、None resume、旧native失效，不重置尝试/结算。

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

### Requirement: 同源用户面与机器建议预填（REQ-SG-06）

live/rebuild/observer、通用 error、protocol_error、门局部 failure 与 cockpit inbox SHALL 使用同 failure identity 与 typed presentation。处理中主显“系统正在整理方案结构，保留你的业务要求”，已解决主显等待正常业务审核；中间结构拒收不刷红错、不建需人工技术项。最终工程故障 SHALL 如实显示未交付/已交付事实、合法版本保留和未确认发布状态，可退出/等待现恢复；安全可读详情含真实 code/line/field、机器指令、规则版本、before/after refs、attempt/reservation/actual start、预算 receipt。主区默认不刷 attempt，详情分别示已提交/已交付/非结构失败扣次/可用预算，不能混计。

#### Scenario: 安全可读的主区和详情
- **WHEN** 系统处理中、已解决或工程耗尽停点被投影
- **THEN** SHALL 主区默认不刷attempt，详情区分别展示安全真实诊断、原始code/line/field、指令/规则/refs、reservation/actual start/receipt和已提交/已交付/非结构失败扣次/可用预算；不混计、不隐失败

### Requirement: 独立机器建议与交互可达性（REQ-SG-06A）

machine repair finding SHALL 由服务端产生并沿现 required_action 投影，保持既有 class hint/severity；独立机器建议入口不能把 blocking 混入 advisory。采纳 SHALL 仅当前身份/context/hash合法、可编辑、非observer的编辑器去重append、保草稿/focus，零WS/REST/provider，真实提交才建turn；自动闭环不依赖点击。旧卡/漂移/关闭门/处理中或已解决项不冒称“采纳才能继续”。终态command_id SHALL 稳定拒收并可见回显，非终态复用ID、终态后真实业务重试新ID；沿现守卫不另造判断。


#### Scenario: 可达性与终态重放
- **WHEN** 用户通过键盘查看历史、点击建议或重放终态command
- **THEN** SHALL 复用中性白卡/现primary-secondary、polite状态/最终停点一次alert、可见焦点/44px触达/正文4.5:1/reduced-motion；终态重放拒收结果可见且零重扣/重启，不静默蒸发

#### Scenario: 机器建议一键采纳
- **WHEN** 当前合法可编辑门点击机器建议
- **THEN** 文本恰一次预填、原草稿和焦点保留、出站计数为零；自动结构链无需该点击

#### Scenario: 旧卡与 observer
- **WHEN** observer 或旧 source/context/关闭门点击同建议
- **THEN** 只读拒绝，不启动新链、不丢草稿；失败结果仍可见

### Requirement: R1 R2 R3 连续事故链可验收（REQ-SG-07）

系统 SHALL 连续覆盖显式供需×自动仓序环、非法 heading、已解决结构再次引入：每次确定性拒收不得要求用户代写技术 prompt，坏产物不审核/批准发布，所有问题/指令/attempt/refs/预算审计历史完整。连续问题超过同一授权物理机会时 SHALL 如实停止工程故障，不为了完成链偷借新 turn、取消 Design 仓序/必需 E2E、放宽权限或无限循环；未交付结构耗尽的用户 hold 按 REQ-CG-02 释放，已交付事实不抹除。

#### Scenario: 三连撞在同一 turn
- **WHEN** 同一业务反馈链依次出现 R1机器修复后R2 heading错，再R3依赖回退风险
- **THEN** 在2 attempts内能合法即完整交付/复评，否则到顶停工程故障并保持合法 refs；不会spawn第3次、白吃新用户轮次或循环追逐错误，主区只有一条当前状态，历史保三类问题与处理事实

#### Scenario: 跨合法业务轮次的完整链
- **WHEN** 等价完整 fixture 将 R1/R2/R3 分配到各有合法原授权的真实业务轮次
- **THEN** 每轮只按自己的物理上限执行，结构未交付release/合法交付settle/已交付复评不退款均恰一次；下轮继承 checkpoint/resolved predicates且预算不重置
