# Tasks: bound-sc-generation-teaching

> 精确文件、命令、测试与提交步骤由 `writing-plans` 写入 `cadence/plans/`；本清单只列高层可验收工作包。每个工作包都标注它所映射的 requirement。
>
> 落地次序遵循 `design.md` 的 D5：耐久窗口 → store CAS → 真实 spawn 观察 → handler 单槽 → 流水/故障重放是**不可拆开上线的纵向闭环**。**新的教学资格与载荷只在该闭环整体就位后才开放。**

## 1. 通道 C 的耐久授权窗口模型

- [ ] 1.1 建立 SC 生成教学窗口的耐久模型：承载原 generation/node/run 身份、业务请求与材料摘要、失败 source 引用与 hash、context digest、允许修改闭包、机器指令 digest、prompt 引用与 digest、attempt key、物理上限 max ∈ {1,2}（由 RepairOnce 批准状态决定）、RepairOnce 批准状态、批准规则版本、冻结输入摘要、冻结执行策略摘要（本 change 只定义该通用字段，值由 explicit-sc-author-effort 填写）、已消费 attempt 序号；窗口仅在初轮新授权时建立，A/B 合法接力不得重建窗口（映射 REQ-WSC-28、REQ-WSC-28B；验证：新窗口的字段投影可完整还原一次 claim 所需的全部绑定事实）
- [ ] 1.2 定义窗口的 attempt 键形如 `sc_generation:{session}:{generation}:attempt:{1|2}`（attempt1/attempt2 同用该键式；旧键 `single_candidate_author:{id}:{n}` 是 provider launch 身份、保留为 ledger 自身键，ledger 条目以 launch 身份为键并新增 attempt 键关联字段），并实现 `Claimed → ActualStarted → Terminal` 与 `LaunchUnknown` 的状态迁移；attempt 次数取窗口显式字段，不以启动账长度反推；迁移 `workspace_engine/single_candidate.rs:138-167,244-256` 与 `review/routing_scope.rs:20-46` 的隐式键派生读写点，durable→runtime 同步与 policy CAS 不得覆盖已更新 ledger（映射 REQ-WSC-28B、REQ-WSC-28C；验证：窗口 attempt 序号与账本条目数解耦，改变账本长度不改变窗口次数；记一次 C 后 A 路径键不变）
- [ ] 1.3 在会话排它锁下提供窗口的原子 claim 与持久化能力：同一 attempt key 幂等、并发 claim 只有一个成功者、上限超限时拒绝并给出可判定原因（映射 REQ-WSC-28B；验证：并发 claim 同一键后窗口内只有一个有效 claim，其余被拒且不产生额外启动）
- [ ] 1.4 窗口缺省即「未知、零额外调用」：历史会话缺字段、无可证明原授权或只有旧 `started` 占位时，读取历史不产生授权，且不从计数为零推断存在机会；旧记录不得事后倒填 grant 或 actual-start 证据（映射 REQ-WSC-28B；验证：旧格式会话上的教学请求产生零额外调用并给出证据缺失说明）
- [ ] 1.5 实现 RepairOnce 批准核验（owner 裁决 2026-11-09「窗口建立即批准」）：在**初轮新授权建立窗口的同一事务内**一次性核验并冻结四项判据 —— ① 原授权可证明（原生成 run/reservation/phase）② provider 路径合法且具备可用的真实启动观察能力 ③ 剩余物理机会 ≥ 1 ④ 失败事实完整（可定位 typed 事实）；四项**全部成立**才写「已批准」并冻结批准规则版本，物理上限为「原调用在内最多两次」，任一不成立则「未批准」、上限为「原调用在内最多一次」且零额外调用并显式系统故障。**不新增签发方或授权载体**，不把现 `RepairReservation` 改名当 C grant；批准状态 MUST NOT 因部署升级、重连、恢复、显式 fresh 或改档而变化，MUST NOT 事后补造（映射 REQ-WSC-28B；验证：四条正例各一条用例（批准→上限 2）、四条反例各一条用例（未批准→上限 1 且零额外调用），并有一条用例证明批准结论在重连与显式 fresh 后不变）
- [ ] 1.7 **【防退化 · 新增】「未启用通道 C 时行为与现状逐字相同」的基线** —— 本 change 用耐久窗口**替换**现有两个进程内布尔量（`compile_ir_rerun_used` / `structure_rerun_used`），属**替换既有路径**而非新增路径；1.3 / 2.2 / 8.2 给的都是**新断言**（幂等、并发唯一、超上限拒绝、实际启动合计 2），**缺**「旧路径原本是什么行为」的对照 —— 一旦新实现把不该重驱的场景也重驱、或把该重驱的漏掉，现有断言未必抓得住。故须在**未改实现**的状态下记录：① 无通道 C 机会时**不发生任何额外 provider 启动**；② 有机会时**恰好一次**额外启动且预算计数与现状一致；③ 失败后的终态呈现（错误面/提示文案）与现状一致。改实现后逐条比对**可观察行为**（不只是计数），任一条不一致即判为退化。验证：基线可在**未改实现**上复现；改后逐条比对

## 2. 真实启动观察与既有启动账接线

- [ ] 2.1 保持既有 provider-start 账为唯一实际启动账，并沿用「需要 started 事实且有 provider 与启动证据」的观察口径；预留占位、调用前时间戳、WS 事件与旧 `started` 布尔均不得冒充真实启动（映射 REQ-WSC-28C；验证：仅占位条目的会话不会被判为已启动）
- [ ] 2.2 新增受控的启动观察/CAS 接口：该 attempt 的实际启动写入既有 provider-start ledger（以 launch 身份为键）并携带 attempt 键关联，把占位条目**原位**补全为带真实 provider/run 与启动证据的条目，校验该 attempt 属于已 claim 的窗口、窗口未超上限、同键幂等；不为同一次启动另插第二条记录，不接受调用方自报「已启动」；把占位值改为 `started=false`（改动点：`reserve_single_candidate_provider_start`，`workspace_single_candidate.rs:127`、占位写入 `:197-203`，现写 `started:true, provider:None`）（映射 REQ-WSC-28C；验证：同键重复观察只得到一条补全后的记录，超上限的观察被拒；占位条目 `started=false` 且不被判为实际启动）
- [ ] 2.3 把真实 spawn 身份确认接入启动观察：在 spawn 身份确认后、协议驱动继续前同步持久回填，并在会话尾部资源注销前验证回填完成；回填失败由既有资源收口停止进程并保留 claim 与资源证据（映射 REQ-WSC-28C；验证：注入回填失败时进程被收口、证据保留且状态为 Unknown 而非未启动）
- [ ] 2.4 允许启动的路径继续经过既有 gateway admission 与 full fingerprint 校验；显式 fresh 保留原授权、窗口与冻结输入，且不获取新的 RepairOnce（映射 REQ-WSC-28C；验证：指纹漂移时零 adapter start 且预算与 refs 不变；fresh 后窗口仍是同一窗口）

## 3. generate 段教学重驱的单槽收口

- [ ] 3.1 让 parse 语法、事实完整的 typed lowering、事实完整的 typed IR 校验、确定性结构拒收四类失败统一消费同一窗口；移除「按失败阶段或类别另开机会」的能力（映射 REQ-WSC-28；验证：四类失败各自都能消费同一窗口，且同链只存在一个窗口）
- [ ] 3.2 取消结构类的独立槽：删除两套并存的进程内重驱布尔量与结构类独立槽接口，改为消费窗口的受控 claim 结果与 typed assessment；不保留兼容 shim（映射 REQ-WSC-28、REQ-WSC-28E；验证：源码与接口中不再存在独立 structure 重驱槽或等价布尔授权）
- [ ] 3.3 资格判定只依据唯一清单与事实完整性：mixed Unknown/Operational、事实不完整、权限/上下文/基线不可证明的失败一律零额外调用，并按已有等待/失败面回显真实原因，不渲染成模型可修语法错误（映射 REQ-WSC-28；验证：混合失败集合产生零额外调用，且错误面显示真实原因）
- [ ] 3.4 窗口内原调用在内最多两次：原窗口的运行恢复已用掉第二次物理机会时，教学重驱不再产生第三次（映射 REQ-WSC-28；验证：恢复已消费第二次后触发教学请求，实际启动数为 2）

## 4. 预算归属与停止语义

- [ ] 4.1 落实通道 C 的独立有界归属：教学重驱不消耗评审机械池 `max_repairs`、不消耗 author 重写/反馈池 `max_manual_repairs`，不增加对应已用计数，不伪造用户 turn/hold/receipt（映射 REQ-WSC-28A；验证：一次 C 教学后两池余额均未变动）
- [ ] 4.2 明确并实现「A/B 池余额不得为 C 的第三次调用兜底」：C 的上限只由窗口的 attempt 上限表达；A/B 池余额充足也不改变 C 的上限（映射 REQ-WSC-28A；验证：A/B 有余额时 C 的第三次启动仍为 0）
- [ ] 4.3 实现停止语义：第二次交付仍失败或出现修复范围越权时停止该生成自动链，保留 last-valid refs 与失败历史，不提升坏 candidate、不发起 review/confirm、不旁路至另一自动重写入口（映射 REQ-WSC-28E；验证：第二次失败后无第三次启动、refs 保持为最后一次合法值）。「修复范围越权」的判据新码 `repair_scope_violation` 由裁决 §7 的 C 包交付，C 包落地前该分支不可判定（见 REQ-WSC-28E 占位场景）。
- [ ] 4.4 确保停止后不由平台自动制造新 turn、candidate 或 generation 来补机会；后续只能由真实新业务授权按其既有合同继续（映射 REQ-WSC-28E；验证：停止后平台侧无新增 turn/candidate/generation 记录）

## 5. 尝试流水的阶段与 attempt 关联

- [ ] 5.1 为生成/parse/typed lowering/typed IR 校验/结构拒收补齐真实阶段字段，并让成功与失败都关联同一 generation 与本次 provider attempt（映射 REQ-WSC-25、REQ-WSC-28D；验证：一次原调用加一次教学重驱在同一条流水序列上可分别列出两个 attempt 的阶段与处置结论）
- [ ] 5.2 失败原文先落不可变 source 再判定：原始输出无法形成 source 时明确记录缺失原因，不伪造引用，失败产物不提升 current refs（映射 REQ-WSC-28D；验证：无法形成 source 的记录带缺失原因且 current refs 未被提升）
- [ ] 5.3 保持 REQ-WSC-25 的 non-blocking 义务逐字成立：流水写失败只记运行日志，不阻断候选生成或确认（映射 REQ-WSC-25；验证：注入流水写失败时生成链路继续且日志留痕）
- [ ] 5.4 与执法失败分流：授权 claim 或实际启动记录持久化失败时不得继续无账启动或重试，已发生的外部动作按真实证据进入 Unknown 或补偿，不伪装未启动（映射 REQ-WSC-28D；验证：注入 ledger 写失败时链路 fail-closed，且与 5.3 的不阻断用例互不干扰）

## 6. 恢复、接管与旧数据兼容

- [ ] 6.1 恢复沿原 attempt 证据继续：同键重放读取同一耐久窗口，已启动不重播、Unknown 等待、确定未启动才可在原 key 与原授权下恢复合法动作，且不新增预算或物理机会（映射 REQ-WSC-28C；验证：崩溃后重放不产生第二次启动）
- [ ] 6.2 自动转交互接管只引用原窗口的关闭与消耗事实，不凭新的 session 身份铸造 C 机会；原 session 的启动账保持权威（映射 REQ-WSC-28B、REQ-WSC-28C；验证：接管后新 session 不产生新的教学机会）
- [ ] 6.3 「旧字段为 None」与「账本为空」不得被解释为「还剩两次」；新增字段缺省即零额外调用（映射 REQ-WSC-28B；验证：缺省场景下窗口机会数为 0 而非 2）
- [ ] 6.4 新增字段可缺省且兼容旧记录读取；不批量改写历史 durable 记录文件、不清空历史账本（映射 REQ-WSC-28B；验证：旧格式记录可被新代码读取且数值不被改写）

## 7. 结构治理侧接线

- [ ] 7.1 在 REQ-SG-05 的 SC 初轮行上接线：初轮以既有 GenerationRun 授权事实为唯一授权载体，物理启动记入既有 provider-start 账并与授权 claim 分账；SC 人工修订的计费、上限与结算语义保持不变（映射 REQ-SG-05；验证：SC 人工修订路径的扣减与结算断言不受本 change 影响）

## 8. 验收

- [ ] 8.1 序列验收：parse→structure、IR→structure、structure→parse 三组序列下实际启动数均 ≤2，第二失败后第三次为 0，且不新开结构槽（映射 REQ-WSC-28、REQ-WSC-28E；验证：三组序列各自给出实际启动计数与第三次为 0 的断言）
- [ ] 8.2 预算验收：一次 C 教学后断言 A=0、B=0、C 已消费 1、实际启动合计 2；不得把「`manual_repairs_used == 0`」单独称为红测（映射 REQ-WSC-28A；验证：四个数值同时断言）
- [ ] 8.3 故障与重放验收：同键重连、并发 claim、启动前后崩溃、Unknown 等待各有一条通过用例；source/context/prompt 漂移时实际启动为 0；旧占位不算实际启动；无 RepairOnce 时零额外；记一次 C 后 A 路径键不变（映射 REQ-WSC-28B、REQ-WSC-28C；验证：每种故障各有一条独立用例与明确期望值）
- [ ] 8.4 失败分流验收：分别注入流水写失败与账本写失败，证明一个不阻断、一个 fail-closed，且 best-effort 流水不被当作授权凭证（映射 REQ-WSC-25、REQ-WSC-28D；验证：两个注入用例的期望结果不同且都通过）
- [ ] 8.5 归档门禁：`mechanicalize-plan-compile-repair` 的 MODIFIED REQ-WSC-09 完成 typed IR 与 structure 资格扩充之前，本 change 不得归档；REQ-WSC-28 的「资格以 WSC-09 为唯一清单」在该扩充落地前不得实施（映射 REQ-WSC-28；验证：本门禁与 design「归档硬门禁」及 Workflow follow-up 登记一致，未满足时归档被拒）

## Workflow follow-up / 授权后

- 集成 owner 在本 change 归档前完成 `mechanicalize-plan-compile-repair` 侧 MODIFIED REQ-WSC-09 的 typed IR 与 structure 扩充，并与本 change 的条款做去重复核。
- 集成 owner 串行合并本 change 与 `fix-author-rewrite-quota-pool` 涉及同一会话记录的 CAS 变更。
- 引用已归档结构治理 change 的 SG4a、SG6a.2 验收，不复开、不重做、不修改归档目录；本 change 不据此宣称归档 SG 全量重实施完成（流程项，不映射 REQ-SG-05 的可验收产出）。
- 与 `mechanicalize-plan-compile-repair` 的 MODIFIED REQ-WSC-09 去重复核：本 change 的条款只写窗口、预算归属、账本分账、流水关联与停止语义，不重写资格清单正文（流程项，不映射 REQ-WSC-28 的可验收产出）。
- 与 `fix-author-rewrite-quota-pool` 的边界协调：C 不进入共池余额域，不改其扣减语义；两者涉及同一会话记录的 CAS 由集成 owner 串行合并（流程项，不映射 REQ-WSC-28A 的可验收产出）。
- 按项目规则执行受影响的定向测试与全量门禁（含格式、静态检查与行数门禁），并由集成 owner 统一执行而非各工作包并行执行（授权后执行，不映射可验收产出）。
- 真实 provider 的通道 C 验收另行授权后执行；本文档的 tasks 不含任何真实 provider 调用。
