# 设计：SC generate 段教学重驱的单窗口收口（通道 C）

## Context

**动机见 `proposal.md` 的 Why**，本节只列约束设计的事实。

- **契约面**：`openspec/specs/work-item-plan-single-candidate/spec.md:478` 的 REQ-WSC-09 明文「每 candidate 至多一次额外驱动」；同文件 `:885` 的 REQ-WSC-25 要求每次候选生成/编译追加流水且**流水写失败不得阻断**。`openspec/specs/workspace-structural-governance/spec.md:80` 的 REQ-SG-05 要求「实际 start MUST 有 ledger」「上限/授权无法证明则零额外调用」，其逐载体表的「SC 初轮」行写明「仅已批准 RepairOnce 才原+1=2，否则 1；不伪造人工 turn」。
- **实现面现状**：handler `src/web/workspace_ws_handler/run/single_candidate.rs:1211-1212` 用两个进程内布尔量 `compile_ir_rerun_used` / `structure_rerun_used` 表示两个互不挤占的槽（`:1222`、`:1306`、`:1389` 消费，`:1227`、`:1307`、`:1397` 置位——`compile_ir_rerun_used` 亦在 `:1307` 置位），最坏造成原调用之外两次额外驱动，且它们随进程消失——重连、恢复或同请求重放即可重新获得机会。
- **账本面现状**：`src/product/models/workspace.rs:844` 的 `provider_start_ledger` 是唯一的实际启动账；`src/product/work_item_plan_policy/types.rs:166` 的 `is_actual_launch_observation` 已要求 `started && provider.is_some() && started_at.is_some()`，即旧预留占位不构成实际启动证据。`src/product/lifecycle_store/workspace.rs:899` 的 `claim_provider_start_with_details` 对同一 key 直接返回 `false`，**只拒绝不回填**，也没有上限语义。
- **占位与派生的现状**：`src/product/lifecycle_store/workspace_single_candidate.rs:127` 的 `reserve_single_candidate_provider_start` 在预留时（占位写入实为 `:197-203`）即写入 `started: true, provider: None, started_at: None` 的占位条目（占位值须改为 `started=false`，见 D4 与 tasks 2.2）；`src/product/workspace_engine/single_candidate.rs:155-157` 用 `provider_start_ledger.len()` 反推「当前 author attempt」——这是隐式的长度语义，不是显式的原授权窗口语义。
- **A 池载体**：`src/product/work_item_plan_policy/evaluate.rs:203-222` 的 `RepairReservation` 只有 `token / owner_session_id / owner_run_id / provider_start_idempotency_key / state / commit_id`，既无所需 hash/上限，也已承担 A 池接力。
- **授权事实面**：`src/product/planning_failure.rs:310-338` 已有 `StructuralAuthorizationFacts { authorization_kind: GenerationRun, attempt_no, max_attempts, active, anchor_waiting, launch_unknown, ... }`，`:392-435` 的 `classify_failure_set` 已是「混合集合整批裁决」的纯函数。
- **物理启动口**：`src/web/workspace_ws_handler/run/gateway_start.rs:583` 的 `start_work_item_plan_author` 是该 path 唯一的物理启动口（caller 覆盖面引用 `:594` 注释「七个 caller 共用」的说法而非本文断言；实测非测试调用点为 11 处）；`src/cross_cutting/provider_resources.rs:1409-1418` 的 `release_after_session_tail` 会在会话尾部注销资源登记。
- **接管面**：`src/product/lifecycle_store/workspace.rs:481-499` 的自动转交互接管复制 history 但清空 `repair_reservation` / `human_gate_reservation` / `provider_start_ledger`。

**本设计的边界**：只交付通道 C 所需的完整 GenerationRun 执法闭环，不复开已归档的结构治理 change，不把 C 塞进 author 重写/反馈共池 change。

## Goals / Non-Goals

**Goals:**

- 四类可教学失败（parse 语法、事实完整的 typed lowering、事实完整的 typed IR 校验、确定性结构拒收）共享**同一个**耐久授权窗口；原调用在内最多两次；取消独立 structure 第三次。
- 通道 C 的预算归属独立有界：不扣 A 池 `max_repairs`、不扣 B 池 `max_manual_repairs`，且不得由 A/B 余额兜底 C 的第三次。
- claim 与 actual start 分账，真实启动只以 `provider_start_ledger` 为准；窗口在崩溃、重连、恢复、重放后仍指向同一 attempt。
- 流水按同一 generation 与 provider attempt 关联，且流水失败与执法失败分流。
- 停止语义明确：第二次交付仍失败或越权即停止该生成自动链并保留 last-valid refs。

**Non-Goals:**

- **不复开已归档的结构治理 change**：`openspec/changes/archive/2026-11-02-fix-error-experience-structural-governance/` 保持归档状态；其 SG4a、SG6a.2 的验收直接被引用，不重做、不改归档目录，也不据此宣称归档 SG 全量重实施完成。
- **不把通道 C 塞进 `fix-author-rewrite-quota-pool`**：该 change 继续只修门段 A/B 混扣；C 不进入它的预算域，也不改它的扣减语义。
- **不修改 `mechanicalize-plan-compile-repair` 的目录**：资格清单的 typed IR 与 structure 扩充由集成 owner 在该 change 的 MODIFIED REQ-WSC-09 中执行（见「跨 change 共享面与排序」）。
- **不改 REQ-SG-02A 的内容修复授权边界**、不改 REQ-SG-07 的「不会 spawn 第 3 次」语义。
- **不扩张公共配置面**：effort/model 等档位不在本 change 范围；effort 语义不在本 change，冻结执行策略摘要字段由本 change 提供（D1、REQ-WSC-28B，值由 effort 侧填写）。
- 本 change 的边界仅由决策报告 §1（决策一）给定；报告内其它章节列出的、需后续立项的独立范围不属本 change，本设计不复述其内容。

## Decisions

### D1 · 用一个耐久 GenerationRun 教学窗口取代两个进程内布尔槽

新增一份 SC 生成教学窗口（durable generation teaching window），挂在会话记录上，与原生成 run/reservation/phase 绑定。窗口持有：authorization kind、原 generation/node/run 身份、业务请求与材料摘要、失败 source 引用与 hash、context digest、允许修改闭包、机器指令 digest、完整 prompt 引用与 digest、attempt key、物理上限 max ∈ {1,2}（由 RepairOnce 批准状态决定：已批准且耐久绑定时原调用在内最多两次，无批准时最多一次）、RepairOnce 批准状态、批准规则版本、冻结输入摘要、冻结执行策略摘要（通用字段：本 change 只定义字段，值由 `explicit-sc-author-effort` 填写，effort 语义不在本 change）、已消费 attempt 序号、claim 状态与 ledger key。

handler 的 `compile_ir_rerun_used` / `structure_rerun_used` 两个布尔量**删除**（不保留兼容 shim），改为消费窗口的 claim 结果与 typed assessment。结构失败耗尽不得落到另一自动通道。

**D1-a · RepairOnce「批准」由谁授予、按什么判据（owner 裁决 2026-11-09）**

裁决 §1.3 ¶2 要求「只有**已批准且耐久绑定**的 RepairOnce 才可使用原调用之外的一个物理机会，原调用在内最多两次；无批准时最多一次」，§1.7 验收含「**无 RepairOnce 0 额外**」一档；但「批准」的主体与判据此前无处落点（审阅报告 §5.1 列为未裁决缺口）。**owner 已拍板：窗口建立即批准。**

- **无独立批准主体**：不新增签发方、不新增授权载体（明确**不**把现 `RepairReservation` 改名当 C grant —— 裁决 §1.5 已否决）。「已批准」是**窗口自身的一项耐久状态**，不是另一次签发动作。
- **授予时机与判据**：初轮新授权在 session 同锁/CAS 下**建立窗口的那一次事务**内，一次性核验并冻结：① 前置授权存在且可证明（原生成 run/reservation/phase）；② provider 路径合法且具备可用的真实启动观察能力（裁决 §1.5「未具备合法观察/准入的 provider 路径不得获 C 额外机会」）；③ 剩余物理机会 ≥ 1；④ 失败事实完整（可定位的 typed 事实，见裁决 §1.5 四类 IR 诊断的携带要求）。四项**全部成立**才记为「已批准」并写入**批准规则版本**。
- **耐久性与不可变性**：批准状态与批准规则版本随窗口持久化，**不随部署升级、重连、恢复、显式 fresh 或改档而变化**。批准规则版本随窗口冻结，后续规则变更**不得回溯改写**既有窗口的批准结论。
- **无原授权即无批准**：无原授权、无窗口、或窗口建立时四项核验任一不成立 ⇒ 窗口的批准状态为「未批准」，物理上限 `max = 1`（原调用在内最多一次），**零额外调用**。历史会话（含只有旧 `started` 占位或无可证明原授权的记录）**不得补造**批准证据（裁决 §1.5「历史旧样本不得事后补造 grant 或 actual-start 证据」）。
- **审计口径**：批准结论以**窗口字段**（批准状态 + 批准规则版本 + 核验时的原授权引用）为准，**不**依赖独立的签发记录。裁决 §1.5「授权、材料、上限或持久 claim 不可证明时 MUST 零额外调用并显式系统故障」仍然适用。

**D1-a 的备选与否决**：

| 备选 | 否决理由 |
|---|---|
| policy 层显式签发独立 RepairOnce 授权（新增第三套授权载体） | 授权与执行分离虽更清晰，但需新增载体与签发点、改动面最大；裁决 §1.5 已明确否决「拿现 `RepairReservation` 改名当 C grant」，再加第三套载体须再走一轮设计。owner 已选「窗口建立即批准」 |
| 上提到初轮生成授权携带「允许一次教学重驱」标志 | 最贴「前置授权」字面含义，但要改原生成授权链路并回填历史语义；旧授权一律无标志 ⇒ 一律拿不到第二次机会，可能让「有合法原授权但无标志」的历史会话失去通道 C 资格 |

**备选与否决**：

| 备选 | 否决理由 |
|---|---|
| 保留两个独立槽 | 需同时扩大 REQ-WSC-09 与 SG-05 初轮上限，且与 SG-07「不会 spawn 第 3 次」相撞；无证据证明第三次能避免整篇回归 |
| C 入 A 池 | 抢占评审后唯一机械聚合返修额度，混合生成修复与 review cycle，需改 A 池用途与 cycle 计费，不是本次缺口的必要解 |
| C 入 B 池 | 需扩大 REQ-WSC-21 触发域、改 quota-pool「编译成功清零」语义与初轮授权承载，且在用户反馈前即消耗反馈预算 |
| 单槽 + 只补 WSC-25 日志 | 无耐久 CAS 上限、无真实 start、无 unknown 执法，重连仍可重新获得机会；日志关不掉真实启动 |

### D2 · 键形如 `sc_generation:{session}:{generation}:attempt:{1|2}`，状态机 `Claimed → ActualStarted → Terminal` 且 `LaunchUnknown` 可恢复

窗口内的每个物理尝试由显式 attempt 序号构造 key：

```
sc_generation:{session}:{generation}:attempt:{1|2}
```

`{generation}` 取原生成链的稳定标识（不得取重驱后的新 source hash，否则窗口会随 source 漂移）；attempt1 与 attempt2 **同用**该键式。provider 技术运行使用独立 launch 身份关联该 attempt：旧键 `single_candidate_author:{id}:{n}` 即 provider launch 身份，**保留为 ledger 自身的键**，不与 attempt 键混用；provider_start_ledger 条目**以其 launch 身份为键**，并**新增 attempt 键关联字段**。因此「同一 attempt key 写入既有 ledger」的准确含义是：该 attempt 的实际启动 MUST 写入既有 provider_start_ledger（以 launch 身份为键）并携带 attempt 键关联，MUST NOT 为同一次启动另插第二条记录。attempt 次数由窗口显式字段给出，**不得**用 `provider_start_ledger.len()` 反推。

状态机：`Claimed`（已原子 claim、尚未证明启动）→ `ActualStarted`（真实启动观察已回填）→ `Terminal`（终态：已交付、已耗尽或已停止）。外部结果不确定时进入 `LaunchUnknown`，沿原等待/对账，**不自动重播**；确定未启动（`KnownNotStarted`）可在原 key、原授权下继续同一 claim，**不增加机会**；已实际启动的不以重投方式重播。

**为什么显式 attempt 序号**：`src/product/workspace_engine/single_candidate.rs:155-157` 现有的长度反推语义会被 C 的记账污染（记一次 C 就会移动 A 路径的 attempt 判定），必须改成显式窗口字段。

**必须迁移的耦合读写点**（裁决 §1.5 明列三处）：

| 读写点 | 现状 | 迁移要求 |
|---|---|---|
| `src/product/workspace_engine/single_candidate.rs:138-167,244-256` | `:244-256` 取「ledger 中最后一个前缀键」当 reservation，`:138-167` 依赖长度派生 attempt | 取消「全 ledger 长度＝当前 author attempt」与「最后一个前缀 key＝当前 reservation」的隐式关系；C 用显式原窗口当前 key，A 路径仍用其 repair reservation |
| `src/product/review/routing_scope.rs:20-46` | 以 `ledger.len()` 预占 A 路径键 | 改为显式键派生，避免 C 记账后误接或挤占 A 的键；验收含「记一次 C 后 A 路径键不变」（tasks 1.2、8.3） |
| durable→runtime 同步与 policy CAS（`src/product/workspace_engine/types.rs:95-102,162-168`、`src/product/lifecycle_store/workspace_policy_route.rs:104-109`） | 同步 window/ledger 镜像 | policy CAS 不得把旧 runtime 镜像覆盖到已更新 ledger；真实 start 回填后重载/合并最新 expected，冲突不得静默丢条目 |

### D3 · `RepairReservation` 不改名、不复用为 C 的 grant

`src/product/work_item_plan_policy/evaluate.rs:203-222` 的 `RepairReservation` **只是 A 池接力载体**：字段只有 token/owner/key/state/commit，既无所需 hash 与上限，也已绑定 A 池的接力语义。把它改名成「C grant」会让 C 的授权看起来有上限有 hash，实际是 A 池的余额。

C 使用自己的 GenerationRun 窗口载体；窗口与 `RepairReservation` 互不替代，也不得互相投影成对方的余额。

### D4 · `provider_start_ledger` 仍是唯一实际启动账；新增受控 observe/CAS 接口

不变的事实：`src/product/models/workspace.rs:844` 的 `provider_start_ledger` 是唯一实际启动账，`is_actual_launch_observation` 的口径（`started` + provider + started_at）保持。

问题：`src/product/lifecycle_store/workspace.rs:899` 的 `claim_provider_start_with_details` 对同一 key **只返回 false、不回填**，也没有上限语义，因此无法表达「先占位、同 key 原位补全真实启动证据」。

因此需要**新增专用受控 observe/CAS 接口**（不改变既有接口语义）：在同一 session 排它锁下，按 attempt 键关联把占位条目**原位**补全为带真实 provider/run 与启动证据的条目（条目以 launch 身份为键，见 D2），并校验该 attempt 属于已 claim 的窗口、窗口未超上限、同键幂等。**不为同一次启动另插第二条记录**，也不得由调用方自报「已启动」。**占位值 MUST 为 `started=false`**：`reserve_single_candidate_provider_start`（`workspace_single_candidate.rs:127`，占位写入在 `:197-203`）现写 `started: true, provider: None`，须改为 `started=false`，改动点列于 tasks 2.2。

真实启动证据来自 spawn 身份确认（`TrustedLaunchOwnership` 的真 PID/starttime 确认点），在 spawn 身份确认后、协议驱动继续前同步持久回填，并在 `provider_resources.rs:1409-1418` 的会话尾部注销前验证回填完成；回填失败由既有资源收口停止进程，保留 claim 与资源证据并进入 Unknown，**不自动重播**。

进程在 spawn 与回填之间崩溃而无法证明有无启动时，结论是 Unknown，**不是** `start=0`。本设计不宣称跨进程 exactly-once；验收要证明的是有界、可追溯与不盲目重试。

### D5 · 耐久窗口 → store CAS → 真实 spawn 观察 → handler 单槽 → 流水/故障重放是**不可拆开上线的纵向闭环**

这五段构成一个闭环：没有耐久窗口就没有 claim 上限；没有 store CAS 就没有原子性；没有真实 spawn 观察就无法区分「已启动」与「已占位」；没有 handler 单槽就会双槽超发；没有流水与故障重放就无法审计与恢复。

因此**不得分段上线**：先落窗口与 CAS 与观察接线，再落 handler 单槽收口，最后才开放新的教学资格与载荷。只上其中一段会得到「日志看起来对、真实启动却超发」或「单槽已收口但重连即重发」的状态。

落地次序（对齐决策报告 §1.4）：**先批准 WSC-09 / WSC-28 / SG-05 的规划条款 → 做 SG 初轮最小完整执法 → 最后开放 B/C 的新教学资格与载荷。** 不是「写完 WSC-28 就先上线重驱」，也不是「等完整 SG 项目重实施完才补这一处」。

### D6 · 预算归属独立有界，且不得由 A/B 余额兜底

C 不增加 `repairs_used`、不增加 `manual_repairs_used`、不伪造用户 turn/hold/receipt；A/B 池的触发域、余额与计费语义一字不改。**明确禁止**：A/B 池还有余额时给 C 补第三次——那会把 C 的失败转嫁到与 C 无关的池，破坏两套池各自的语义。

C 的预算表达是「窗口内 attempt 序号 + 物理上限 max ∈ {1,2}（由 RepairOnce 批准状态决定，判据与授予时机见 **D1-a**）」，与 A/B 池余额无关。

### D7 · 旧记录不倒填；接管不铸造新机会

- 历史会话只有旧 `started` 占位、或没有可证明的原授权/窗口时，读取历史**不产生授权**：零额外调用并显式说明证据缺失。**不得**从「计数为零」推断「还有机会」。
- 自动转交互接管（`src/product/lifecycle_store/workspace.rs:481-499` 会清空 ledger 与 reservation）只能引用原窗口的关闭/消耗事实，**MUST NOT 凭新的 session ID 重新铸造 C 机会**；原 session 的 ledger 保持权威。恢复沿原 attempt 证据继续，**不得**把「旧字段为 None」或「ledger 为空」解释为「还剩两次」。
- 新生成窗口只在**初轮新授权**时建立；A/B 的合法接力仍消费各自的 repair/turn 载体，不把每次 handler 进入都当成新初轮。

### D8 · 流水与执法失败分流，二者不得混用

流水（REQ-WSC-25）只用于审计/度量，**不是启动许可**；流水写失败按 WSC-25 记运行日志且不阻断。授权 claim / 实际启动记录的持久化失败则是执法失败：不得继续无账启动或重试，已发生的外部动作按真实证据进入 Unknown 或补偿，**不得伪装未启动**。

流水记录须带阶段（generate / parse / typed lowering / typed IR 校验 / 结构拒收）与 provider attempt 关联，与原调用同属一个 generation 序列。

### D9 · 决策报告 §1.3 的条款全文按自然段拆为六条 requirement

决策报告 §1.3 以**单条** requirement 给出全文。OpenSpec 对 ADDED requirement 的描述文本有 500 字符硬上限（`MAX_REQUIREMENT_TEXT_LENGTH`），`openspec validate --strict` 不过即失败，而 MODIFIED 豁免该上限。若照抄单条，校验必然失败；若删减正文，则丢掉 §1.3 的规范性语义。

取舍：按「一条 requirement 只陈述一个行为」把 §1.3 拆为 REQ-WSC-28 / 28A / 28B / 28C / 28D / 28E，**逐句保留、不删减、不放宽**，并在本节留下逐段映射以备审阅。

### §1.3 条款全文 → delta 条款映射

| §1.3 自然段 | 内容 | delta 条款 |
|---|---|---|
| ¶1 | 绑定原 run/reservation/phase + SC 初轮载体；资格以 REQ-WSC-09 为唯一清单（四类）；不因阶段/数量/节点/类别建独立机会；mixed Unknown/Operational 等不授予 | REQ-WSC-28 |
| ¶2 | 以原生成授权身份为锚；每 candidate 至多一次额外驱动；已批准 RepairOnce 才原+1=2，否则 1；恢复已用第二次则不得第三次 | REQ-WSC-28 |
| ¶2 末句 | 新 source revision / 新 native 会话 / 重连 / 恢复 / 同请求重放 MUST NOT 重置窗口 | REQ-WSC-28C（与恢复重放同段） |
| ¶3 | C 独立有界；不扣 A/B 池；不增计数；不伪造 turn/hold/receipt；不改 A/B 语义；不得由 A/B 余额兜底第三次 | REQ-WSC-28A |
| ¶4 | 原子 claim + 耐久绑定字段清单；初次调用无候选 source 的记录口径；失败原文与 hash 可验证；不可证明则零额外调用并显式系统故障 | REQ-WSC-28B |
| ¶5 | claim 与 actual start 分账；同 attempt key 写入既有 provider-start ledger；占位/时间戳/WS/旧 started 不冒充；Unknown 沿原等待；gateway admission/full fingerprint；显式 fresh 保留原授权与冻结输入 | REQ-WSC-28C |
| ¶6 | 流水按 WSC-25 追加并关联 generation 与 attempt；无法形成 source 时记录缺失原因；流水非启动许可；流水失败与执法失败分流 | REQ-WSC-28D |
| ¶7 | 第二次仍失败或越权即停止该生成自动链；保留 last-valid refs；不提升坏 candidate、不发起 review/confirm、不旁路；不由平台自动造新 turn/candidate/generation | REQ-WSC-28E |
| §1.3 场景 1 | IR 拒收使用唯一额外机会 | REQ-WSC-28C 的场景 |
| §1.3 场景 2 | 跨类别不乘法 | REQ-WSC-28 的场景 |
| §1.3 场景 3 | 业务或运行错误不借教学 | REQ-WSC-28 的场景 |
| §1.3 场景 4 | 同键重放与不确定启动 | REQ-WSC-28C 的场景 |
| §1.3 场景 5 | 旧记录不可补造授权 | REQ-WSC-28B 的场景 |
| §1.3 场景 6 | 流水与启动账本失败处理不同 | REQ-WSC-28D 的场景 |
| §1.7 验收条目 | 一次 C 教学后 A=0、B=0、C 已消费 1、实际启动合计 2 | REQ-WSC-28A 的场景 |
| §1.3 ¶4 末句（fail-closed） | claim 不可证明即零额外调用并显式系统故障 | REQ-WSC-28B 的场景 |
| §1.6 行 1 | 原生成 attempt1：真实 start 一条，失败 source/hash 与 typed IR 事实关联流水 | REQ-WSC-25 / REQ-WSC-28C、28D |
| §1.6 行 2 | 唯一教学 attempt2：已批准 RepairOnce 且 IR 事实完整才允许；真实 start 再一条；A/B 不扣 | REQ-WSC-28C 场景「IR 拒收使用唯一额外机会」+ REQ-WSC-28A 场景 |
| §1.6 行 3 | attempt2 合法：复验正常推进，窗口不能再授第三次 | REQ-WSC-28 场景「跨类别不乘法」 |
| §1.6 行 4 | attempt2 再次结构失败：第三次 start=0；不借 A/B、保留合法 refs 与失败证据 | REQ-WSC-28E 场景「第二次仍失败即停止」 |
| §1.6 末句 | 历史旧样本不得事后补造 grant 或 actual-start 证据 | REQ-WSC-28B 场景「历史样本不得补造 grant 与 actual-start」 |

## 跨 change 共享面与排序

> 本节只登记**共享面与归属**。本 change 不修改任何其它 change 目录；下列「由集成 owner 执行」的条目不在本 change 的制品内落地。

### 共写文件与串行次序（对齐处置裁决 §2.2 全局表）

**决策六（拆分大文件）先落**是本 change 的硬前置：`run/single_candidate.rs`（handler）与 `workspace_engine/prompts.rs` 的纯移动拆分必须先于本 change 的任何实质新增（handler 仅剩 6 行余量）。全局串行序：决策六(0) → mechanicalize(1) → quota-pool(2) → **bound(3，本 change)** → effort(4) → isolate(5) → close(6)。

| 对端包 | 共写面 | 相对次序 |
|---|---|---|
| `explicit-sc-author-effort` | `models/mod.rs`、`models/workspace.rs`、`lifecycle_store/workspace.rs`、`workspace_engine/types.rs`、`run/single_candidate.rs`、`workspace_engine/prompts.rs` | bound → effort（effort 侧共享面表与本表互认） |
| `isolate-plan-criterion-namespace` | `work_item_plan_policy/types.rs`（本 change 写 ledger entry，isolate 后续写代际字段） | bound → isolate |
| `mechanicalize-plan-compile-repair` | MODIFIED REQ-WSC-09（typed IR + structure 资格清单）；经决策六拆分后的 `run/single_candidate.rs` 判据面 | mechanicalize（序 1）→ bound（序 3）：本 change 依赖其资格清单 |
| `fix-author-rewrite-quota-pool` | MODIFIED REQ-WSC-21（门段 `final_compile_*` 触发域）；同一 session record 的 CAS | quota-pool（序 2）→ bound（序 3）：本 change 只引用其触发域口径，不改其合同；CAS 合并由集成 owner 串行 |

**effort #6「compile 教学块迁出 prompts.rs」的归属澄清**：该项属**决策六**（纯移动到 `prompts/sc_compile_teaching.rs`），**不是 bound** 的交付物；bound 的 tasks 不承接该项。

### 与 `mechanicalize-plan-compile-repair`

| 共享面 | 内容 | 归属 |
|---|---|---|
| MODIFIED REQ-WSC-09 | 决策报告 §1.4 要求：**在 mechanicalize 已有的 MODIFIED WSC-09 中加入 typed IR 与 structure**，标题扩为「generate 候选拒收」；「恰好一次」明确以前置授权、合法 provider 与剩余机会为条件；保留总额外 ≤1；禁止以 `any(可教)` 放过混合 operational 集合 | **集成 owner 执行**，本 change 不改该目录 |
| REQ-WSC-25 | mechanicalize 已引用并声明该条由归档 change 承接；本 change 的 MODIFIED REQ-WSC-25 只**增加**阶段枚举与 provider attempt 关联，**不改** `:894-897` 的 non-blocking 义务 | 本 change 落地 |
| REQ-SG-05 | mechanicalize 的 design 声明「REQ-SG-05 的 attempt 单权威与上限不变」；本 change 的 MODIFIED REQ-SG-05 只改「SC 初轮」行 | 本 change 落地；归档顺序与 mechanicalize 协调 |

**去重纪律**：本 change 的 REQ-WSC-28 系列**只写**窗口、预算归属、账本分账、流水关联与停止语义；**不重写** REQ-WSC-09 的资格清单正文（typed IR 与 structure 的资格条目由 mechanicalize 侧补齐）。两份 delta 合起来构成「唯一清单 + 唯一窗口」，不得出现两套资格解释器。

**归档硬门禁**：mechanicalize 的 MODIFIED REQ-WSC-09 完成 typed IR + structure 资格扩充后，本 change 方可归档；在此之前，REQ-WSC-28 的「资格以 WSC-09 为唯一清单」不得实施。本 change 不在自身 delta 内重复 MODIFIED WSC-09（裁决 §1.4）。

### 与 `fix-author-rewrite-quota-pool`

| 共享面 | 内容 | 归属 |
|---|---|---|
| 预算域 | quota-pool 把自动重写与门反馈并入同一 `max_manual_repairs` 余额；本 change 的 C 窗口**不进入**该余额域 | 两者并行，语义互不覆盖 |
| session 记录 | 两者都要在同一 session record 上做 CAS（quota-pool 扣余额、C 窗口 claim 与回填） | **集成 owner 串行合并**，避免两处 CAS 互相覆盖 |
| 明确不做 | 本 change **不**把 C 塞进 quota-pool，也不改 quota-pool 的扣减语义 | — |

### 与已归档的结构治理 change

`openspec/changes/archive/2026-11-02-fix-error-experience-structural-governance/` 的 **SG4a、SG6a.2 验收直接引用，不复开、不重做、不改归档目录**。本 change 只交付 C 所需的完整 GenerationRun 执法，**不得**据此宣称归档 SG 全量重实施完成。

### 排序

1. **决策六拆分先落（硬前置）**：`run/single_candidate.rs` 与 `workspace_engine/prompts.rs` 的纯移动拆分先于本 change 的任何实质新增（见「共写文件与串行次序」）。
2. **规划条款先批准**：WSC-09（mechanicalize 侧）+ WSC-28 系列 + SG-05。
3. **纵向闭环执法就位**：耐久窗口 → store CAS → 真实 spawn 观察 → handler 单槽 → 流水/故障重放（D5，五段不可拆开上线）。
4. **最后开放新资格与载荷**：新教学资格与教学输入在此之前不得对用户开放。
5. **共享接线的串行归属**：`mechanicalize` 的 MODIFIED WSC-09 扩充、quota-pool 与本 change 的 session CAS 合并，均由**集成 owner**串行执行；本 change 的制品只登记归属，不代做。

## Risks / Trade-offs

- **[窗口与账本面改动面大]**（涉及 session CAS、真实 spawn 观察、fresh 链） → 缓解：先落耐久模型与受控 observe/CAS 接口并单独验收，再动 handler 侧槽位；D5 的纵向闭环不得拆段上线。
- **[spawn 与回填之间的崩溃窗无法证明 exactly-once]** → 缓解：结论定级为 Unknown 而非 `start=0`；不盲目重试；验收口径是「有界、可追溯、不盲目重试」，验收断言写 actual starts ≤2 与第三次为 0，不宣称跨进程 exactly-once。
- **[由布尔槽改为耐久窗口会改变既有调用序列]** → 缓解：窗口字段一律缺省即「未知、零额外调用」，旧会话不因缺字段而被判为「还有机会」；旧 `started` 占位不构成实际启动证据。
- **[§1.3 拆成六条 requirement 偏离了报告的「单条」形态]** → 缓解：逐段映射表公开可审；语义逐句保留、不删减不放宽；理由是校验器的 500 字符硬上限，MODIFIED 豁免而 ADDED 不豁免。
- **[流水被误当作授权凭证]** → 缓解：REQ-WSC-28D 与 REQ-WSC-25 双向写明流水非启动许可；D8 的分流在验收中以「注入流水写失败 = 不阻断」「注入 ledger 写失败 = fail-closed」两个独立用例证明。
- **[与其它 change 的 session CAS 互相覆盖]** → 缓解：D5/「共享面与排序」把合并交给集成 owner 串行执行，本 change 不代做。

## Migration Plan

1. **兼容期**：新增窗口字段与 ledger 观察字段一律可缺省；缺省即「未知、零额外调用」。旧会话按旧路径读取，不产生授权、不倒填证据。
2. **切换期**：在耐久窗口与受控 observe/CAS 接口就位后，删除 handler 的两个布尔槽并改为消费窗口 claim 结果；此步与 D5 的纵向闭环同批上线。
3. **数据面**：已写新格式的数据只能由兼容版本读取；不批量改写历史 durable 记录文件（读侧派生），不清空历史 ledger。
4. **回退**：停止授予新的 RepairOnce 后，原调用与已存在的合法产物仍可读；已 claim / Unknown 的按原证据收口；**不清 ledger、不恢复双槽、不借预算重跑**。回退不回退到「布尔槽 + 只记日志」的旧形态，因为那不是本次交付前的合法状态。

## Open Questions

无阻塞项：本设计的边界、归属与次序均由决策报告 §1 给定，未留下需要 owner 裁决才能推进的关键决策。
