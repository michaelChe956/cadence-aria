# 评审 · WIG 四个 OpenSpec Change · bound-sc-generation-teaching v1.0

> 审阅者：ReviewChangeC（anthropic-task）｜日期：2026-11-09｜只读审查，未改任何被审文件，未 git add。
> 代码根：`.worktrees/feat-wig-optimize-11-08/src/`；决策来源：`cadence/designs-reviews/2026-11-08_决策_WIG六项owner裁决_v1.0.md`（下称「决策」）。

## 1. 状态

**BLOCKED** —— 机械面全绿（validate --strict 通过、MODIFIED 场景零丢失、ADDED 均 ≤500 字符、决策 §1.3 七段逐句保留），
但存在 **2 项阻断级语义缺陷**：(I-01) REQ-SG-05 delta 写入「MUST NOT 保留已关闭的窗口」，与决策「窗口保存历史，关闭不删除」及本 change 自己的 D7/任务 6.2 直接相反；
(I-02) REQ-WSC-28E「不得旁路至另一自动重写入口」与主规格 REQ-WSC-21 对 `final_compile_structure_rejected` 的「首次即驱动 author 自动重写」字面冲突，且现代码结构槽耗尽后正是落到该通道，design 未消解。
另有 1 项会使归档后门禁变红的问题（I-03，`validate --strict` 对归档后主规格 exit=1）。

## 2. 问题清单

| 编号 | 严重度 | 位置 | 问题 | 证据 | 建议修法 |
|---|---|---|---|---|---|
| I-01 | 阻断 | `specs/workspace-structural-governance/spec.md:7` | SG-05 新增段写「窗口 … MUST NOT 保留已关闭的窗口」。决策要求窗口**关闭不删除、保存历史、不通过新 source 重新计数**；design D7（`design.md:103`）与 tasks 6.2（`tasks.md:45`）都依赖「接管只引用原窗口的关闭/消耗事实」，窗口若不得保留则无事实可引。规范性文本反转了 owner 裁决 | 决策 L123「窗口保存历史，关闭不删除、不通过新source重新计数」；delta L7 原文「MUST NOT 保留已关闭的窗口」 | 改为「已关闭窗口 MUST 保留其关闭/消耗事实用于审计与接管引用，MUST NOT 被重新打开、复用或重置计数」 |
| I-02 | 阻断 | `specs/work-item-plan-single-candidate/spec.md:65-72`（28E）× 主规格 `spec.md:820`（REQ-WSC-21） | 28E：第二次交付仍失败 MUST 停止自动链、MUST NOT 旁路至另一自动重写入口。WSC-21 触发域写「至少 `final_compile_prepare_failed`/`final_compile_structure_rejected` 首次发生 MUST 真正驱动 author 重写（扣 `max_manual_repairs`）」，正文无「仅门段」限定。现代码结构槽占用后返回 `Routed`＝「平台已在自动重写或人工门态」，即落入该通道。两条对同一失败码给出相反动作；design 无一处消解 | `src/product/workspace_engine/single_candidate.rs:30-31`；`src/web/workspace_ws_handler/run/single_candidate.rs:1419-1421`；决策 L36（C 不能因复用 provider 函数就推断属 A/B）；决策 L119「结构失败耗尽禁止落到另一自动通道」 | 在 28E 增写限定句「generate 段窗口耗尽的结构拒收不适用 REQ-WSC-21 的自动重写触发，仅按本条停止/转既有人工面」，并加一条场景；WSC-21 归 quota-pool，须由集成 owner 确认其触发域口径，本 change 不得改该文件 |
| I-03 | 重要 | `specs/workspace-structural-governance/spec.md:7` | 归档后主规格 SG-05 正文由 370 增至 780 字符（>500），`openspec validate workspace-structural-governance --strict --type spec` 由 exit=0 变 exit=1（change 级 validate 因 MODIFIED 豁免而通过，属归档后才暴露）。同类问题亦见其它 change（见 D 维度） | 见 §3-A 实测；`MAX_REQUIREMENT_TEXT_LENGTH=500` | 把新增段压到 ≤130 字符（只留「载体=GenerationRun 授权事实+耐久教学窗口，claim 与真实 start 分账」），窗口字段清单移入 28B、行为移入已有场景 |
| I-04 | 重要 | `spec.md:5`（28）依赖 REQ-WSC-09 | 28 把 REQ-WSC-09 定为「唯一清单」且含 typed IR 与确定性结构拒收，但主规格 WSC-09（`spec.md:480`）写「非 parse 语法类不触发重驱」，mechanicalize 的 MODIFIED WSC-09（其 `spec.md:10`）仍只含 parse+lowering 且「恰好一次」无条件，与 28「只有已批准 RepairOnce 才可用额外机会、无批准 1 次」不一致。四个 IR 诊断码的 typed producer（决策 L125 `ir_fault.rs`）与 RepairOnce「批准」判据在任何 active change 中均无 owner（grep `ir_fault|unknown_done_when_ref|RepairOnce` 仅见主规格 SG-05 表与本 change） | mechanicalize `design.md`/`tasks.md` 无 IR/structure/WSC-28 引用；决策 L97 | 属跨 change 归档顺序依赖：登记为「mechanicalize 的 WSC-09 扩充先于本 change 归档」的硬门禁，并由 owner 指定 RepairOnce 批准判据的落点 |
| I-05 | 重要 | `design.md:41,98`（D1/D6）vs `spec.md:5-7`、SG-05 表 | D1/D6 把物理上限写死 `max=2`；规格与 SG-05 表要求「无批准 1、批准 2」。D1 字段清单也漏了决策 L110 要求的「RepairOnce 批准规则版本、max=1/2、冻结输入」 | 决策 L110；design L41 | D1 窗口字段加「RepairOnce 批准状态/规则版本」「冻结输入摘要」，上限写成 {1,2} 由批准状态决定 |
| I-06 | 重要 | `spec.md:30`（28B）、`design.md:34,41`、`tasks.md:9` | 决策 §3 与 effort 的 design.md:132/136、tasks 4.1 都要求窗口「冻结输入摘要」纳入 effort 策略摘要，且声明「摘要构造在 bound 目录内」。bound 的 28B 字段清单、D1、tasks 1.1 均无「冻结执行策略摘要」字段，Non-Goals 还写「effort 不在本 change 范围」，该承诺悬空无 owner | effort `design.md:132-136`；决策 L236-238 | 28B/D1/1.1 增加通用字段「冻结执行策略摘要」（不定义 effort 本身），由 effort 填值 |
| I-07 | 重要 | `design.md:139-170`「共享面与排序」 | 共享面登记不对称/互相矛盾：effort 登记 8 个与 bound 的共享面（models/mod.rs、models/workspace.rs、lifecycle_store/workspace.rs 接管段、workspace_engine/types.rs、handler single_candidate.rs、prompts.rs、冻结摘要），bound 一个未登记；isolate `design.md:135` 声称 `work_item_plan_policy/*`「本 change 独占」，而 bound D4 要扩 `ProviderStartLedgerEntry`（`work_item_plan_policy/types.rs:148-167`）；effort #6 把「compile 教学块迁出 prompts.rs」记到 bound，bound 无此交付；effort 要求「决策六拆分先落」，bound 排序不含 | 见 §3-D | bound 共享面补登记上述文件与排序；与 isolate 协调 types.rs 归属；澄清 prompts.rs 迁出归属 |
| I-08 | 重要 | `design.md:66`（D2）、`tasks.md:10` | D2 只点名 `workspace_engine/single_candidate.rs:155-157`，漏了同一耦合的另三处：`:244-256` 取「ledger 中最后一个前缀键」当 reservation；`review/routing_scope.rs:20-46` 以 `ledger.len()` 预占 A 路径键；决策 L115-116 还要求 durable→runtime 同步与 policy CAS 不得覆盖已更新 ledger。tasks/验收均无对应项，C 记账会误移动 A 路径键 | 代码行见左；决策 L115-116 | tasks 1.2 与 8.3 增列这些读写点迁移及「记 C 后 A 路径键不变」验收 |
| I-09 | 重要（需 owner） | `design.md:56-62`（D2）、`spec.md:44`（28C） | 现有 attempt1 键为 `single_candidate_author:{id}:{n}`（`single_candidate.rs:158-164` 预留），新键为 `sc_generation:{session}:{generation}:attempt:{1\|2}`；28C 要求「同一 attempt key 写入既有 ledger」且不另插一条。未说明 attempt1 复用旧键还是新键，两套键并存会对同一次物理启动记两条。占位值亦未定：决策 L112 要求先占位 `started=false`，现实现占位为 `started:true, provider:None`（`workspace_single_candidate.rs:200`），D4 未表态 | 代码行见左 | owner 裁决键对应关系与占位值，写入 D2/D4 |
| I-10 | 次要 | 28E / tasks 4.3 | 「修复范围越权」触发源无 producer、无稳定码（决策 §7 C 要求新码 `repair_scope_violation`）、无场景；tasks 4.3 验证只覆盖「第二次失败」分支 | 决策 L433 | 注明越权判定由包 C 交付，或加场景占位说明 |
| I-11 | 次要 | design D7、tasks 1.1/6.2、决策 L118 | 以下规则只在 design/tasks，无 requirement/场景承载：接管不铸造新机会（仅 SG-05 散文）；窗口只在初轮新授权建立、A/B 接力不重建；不具备合法观察/准入的 provider 路径不得获 C 机会（决策 L118）；历史样本不得补造 grant/actual-start（28B 场景仅部分覆盖） | grep 接管：delta 仅 SG-05 L7 | 各补一个短场景 |
| I-12 | 次要 | WSC-25 MODIFIED（`spec.md:78`） | 要求「每条记录关联本次 provider attempt（attempt 序号与 attempt key）」，但现 `attempt_seq/attempt_key` 是 store 分配的会话内流水号（`candidate_attempts.rs:144-146`），stage 枚举仅 Structure/Final（`:90-95`），无 provider 启动的记录（如机械重编译）无法满足 | 同左 | 改为「若本次触发 provider 启动则关联其 attempt key，否则显式记录无」 |
| I-13 | 次要 | design.md 各引用 | 行号小偏差：`models/workspace.rs:843`→实 844；`workspace_single_candidate.rs:129`→实 127（占位写入 197-203）；`gateway_start.rs:585`→实 583；「七个 caller」引自 `gateway_start.rs:594` 注释，实际非测试调用点 11 处；handler `:1307` 亦置位 `compile_ir_rerun_used`，design 只列 1227 | §3-E | 更正行号，「七个」改为引用而非断言 |
| I-14 | 次要 | design 映射表 / tasks 7.2-7.4、8.5 | 映射表「§1.3 末段(fail-closed)」实为 ¶4 末句；无 §1.6 场景映射。tasks 7.2/7.3/7.4/8.5 为流程/协调项而非可验收工作包，却映射到 REQ-SG-05/WSC-28/28A | `design.md:137`；`tasks.md:51-62` | 流程项移入 Workflow follow-up |

## 3. 逐维度结论

### A. Delta 机械正确性 —— 通过（附 I-03 归档后门禁风险）

**A1 MODIFIED 场景保全（逐字比对，脚本 + diff）**
- REQ-SG-05：主规格 `openspec/specs/workspace-structural-governance/spec.md:80-107` 4 场景（逐载体授权与上限 / attempt2 已用于运行恢复 / 指纹或 anchor join 错误 / 同键崩溃重放）→ delta 6 场景，原 4 个标题逐字在、后 3 个正文逐字相同；「逐载体授权与上限」场景体仅「SC 初轮」一行改动（载体表 6 行，其余 5 行含「SC 人工修订 … 无第3次」逐字保留）。新增「SC 初轮窗口与真实启动分账」「SC 初轮窗口重放与 Unknown 等待」。
- REQ-WSC-25：主 2 场景（连续失败可逐次追溯 / 记录读写失败不阻断生成）→ delta 3 场景，原 2 个标题与正文逐字相同；`:894-897` 的 non-blocking 义务未动；新增「阶段与 attempt 关联可追溯」。
- 命令：`diff <(grep -v '^\s*$' sg05_main.txt) <(grep -v '^\s*$' sg05_delta.txt)` 仅出现 2 处差异（新增段、载体表 SC 初轮一行）；WSC-25 差异为正文新增一句与新增 1 场景。
**A2** 8 个 requirement 均 ≥1 场景，脚本核查全部 WHEN/THEN 齐全（`WHEN/THEN_all=True`）。
**A3 真实输出**
```
$ openspec validate "bound-sc-generation-teaching" --strict
Change 'bound-sc-generation-teaching' is valid          (exit=0)
$ openspec show bound-sc-generation-teaching --deltas-only --json   (deltaCount=8)
WSC ADDED x6 (正文长度/场景数)：28=454/2, 28A=210/1, 28B=299/2, 28C=339/2, 28D=279/1, 28E=184/1
WSC MODIFIED REQ-WSC-25 306/3 ；SG MODIFIED REQ-SG-05 779/6
```
**A4** ADDED 正文均 ≤500（28 的两自然段合计 455，最大值）。MODIFIED 豁免属实：validator 注释「MODIFIED is left alone」（`dist/core/validation/validator.js:268-271`）。
**归档模拟（/tmp 副本，仓库未动）**：`openspec archive bound-sc-generation-teaching -y` 成功（+6 added, ~2 modified），归档后 WSC 主规格 `--strict` 通过，**SG 主规格 `--strict` exit=1**：`requirements[6] Requirement text is very long (>500)`（SG-05 370→780 字符，原仓库 exit=0）→ I-03。
同法模拟其它三 change：close-contract 使 WSC-17 达 883 字符、effort 使「明确 fresh 只换 native 会话」达 649 字符，归档后各自主规格 `--strict` 同样 exit≠0（mechanicalize 另有 Purpose 占位警告，非本批次引入）。属系统性问题，建议 controller 统一处置。

### B. 与决策 §1.1~§1.7 的语义一致性 —— 条款本体一致，窗口/归属细节有 I-01/I-05/I-06/I-09/I-11
逐条核对（决策 → delta/design）：
- §1.1 取消独立 structure 槽并入同一 RepairOnce；C 不入 A/B；「独立有界」仅资格/计费归属 → 28/28A、design D1/D6 一致。资格四类共用 → 28 ¶1 逐字。
- §1.2 三备选否决 → design D1 表四行逐项对应，理由无放宽。
- §1.3 七段与六场景：28 ¶1/¶2（含「原调用在内最多两次，无批准时最多一次」「恢复已用第二次则不得第三次」）逐字；¶2 末句移入 28C；28A/28B/28C/28D/28E 对 ¶3~¶7 逐句保留，MUST/MUST NOT/SHALL 强度与原文一致（核对 `不得由 A/B 池余额兜底`、`MUST NOT 伪造候选引用`、`MUST 零额外调用并显式系统故障`、`MUST NOT 提升坏 candidate`），无软硬互换。六场景全在（IR→28C、跨类别→28、业务/运行错误→28、同键重放→28C、旧记录→28B、流水/账本→28D），另有 28A 的 A=0/B=0/C=1/启动合计2 场景（决策 §1.7）与 28B fail-closed 场景。
- §1.4 去重纪律（不在第二 change 重复 MODIFIED WSC-09；SG-05 只改初轮行、不改 SC 人工计费；WSC-25 不改 non-blocking）→ 守住（见 D）。落地次序 → design D5/排序一致，但缺「决策六拆分先落」（I-07）。
- §1.5：C 不复用 RepairReservation（D3 ✓）；ledger 唯一实际启动账与 observe/CAS（D4 ✓，`claim_provider_start_with_details` 同键返回 false 已核实 `lifecycle_store/workspace.rs:914-918`）；真实启动在 spawn 身份确认后回填、尾部注销前验证、崩溃窗为 Unknown（D4 ✓）；接管不铸造（D7 ✓ 但规范文本缺场景，I-11）；键与状态机（D2 ✓）。差异见 I-05/I-06/I-08/I-09。
- §1.6 回放表：run1→run2 保留、第三次 start=0 → 28C/28E 场景覆盖；「历史样本不得补造」→ 28B 场景部分覆盖（I-11）。
- §1.7 验收：三组序列、同键/并发/崩溃/Unknown、漂移 0 start、旧占位、无 RepairOnce、A=0/B=0/C=1/启动2、流水 vs ledger 失败分流 → tasks 8.1~8.4 逐项对应。
- 擅自扩大检查（决策 §9）：grep `TTL|review_rounds|low 是否|旧存量AC|legacy JSON|语义等价|全载体` 于本 change 全部文件无命中；SG-05 表其余 5 行未动；design 明示「不复述其它章节」。未发现 §9 越界。

### C. 与主规格冲突（字面级）
- **冲突 1（I-02）** 28E × REQ-WSC-21（见上）。
- **冲突 2（I-04）** 28 ¶1「资格以 WSC-09 为唯一清单，含 typed IR/结构拒收」× 主规格 WSC-09 `spec.md:480`「非 parse 语法类失败不触发重驱」× mechanicalize 现 MODIFIED WSC-09（仅 parse+lowering、「恰好一次」无条件）。归档顺序若本 change 先于 mechanicalize 扩充，则主规格内两条互斥。
- 无冲突：REQ-SG-07「不会 spawn 第 3 次」`spec.md:139` 与 28/28E 一致；REQ-SG-02A 未被触及；REQ-WSC-09 与本 change 的「至多一次额外驱动」一致；fresh 语义与 `workspace-session-resume-recovery`「不增加自动重试或替代动作授权」兼容。

### D. 跨 change 冲突（已实际读取其余 3 个 design.md 的「共享面与排序」与各 delta 标题）
- **REQ 编号**：`grep '^### Requirement:.*REQ-'` 全 openspec（排除 archive）：WSC-28/28A~28E（bound）、WSC-29（effort）、WSC-30/31（isolate）、TOP-07（close）；主规格最大 WSC-27、TOP-06，archive 与 specs 对 `REQ-WSC-2[89]|3[01]|TOP-07` 无命中。**无重号。**
- **同一 requirement 被两 change MODIFIED**：bound＝WSC-25、SG-05；isolate＝TOP-04、TOP-05；close＝WSC-17（+ADDED TOP-07）；effort＝resume-recovery「明确 fresh 只换 native 会话」；mechanicalize＝WSC-09；quota-pool＝WSC-21。**两两无交集**，已确认。
- **MODIFIED REQ-WSC-09 归属**：仅 mechanicalize 持有；bound/isolate/close/effort 四个 delta 均不含 WSC-09（bound 的 tasks 7.3 与 design「去重纪律」明确不写；effort `design.md:137`、isolate `design.md:136` 同声明）。**守住。** 但 mechanicalize 侧的 typed IR/structure 扩充尚未发生（其 delta 仅 parse+lowering），见 I-04。
- **共享文件声明矛盾**（I-07）：① effort `design.md:125-132` 登记 8 项与 bound 共享（`models/mod.rs`、`models/workspace.rs`、`lifecycle_store/workspace.rs` 接管段、`workspace_engine/types.rs`、`run/single_candidate.rs`、`workspace_engine/prompts.rs`、冻结输入摘要），bound `design.md:139-170` 只登记 mechanicalize/quota-pool/归档 SG，**对称性缺失**；② isolate `design.md:135` 声明 `work_item_plan_policy/*` 本 change 独占，bound D4 要扩 `work_item_plan_policy/types.rs` 的 `ProviderStartLedgerEntry`（`types.rs:145-169`），close `design.md:162` 亦改 `classify.rs`，三方对该目录归属互相冲突；③ effort #6 把「compile 教学块迁出 prompts.rs」记在 bound 名下，bound 的 proposal/design/tasks 无此交付；④ effort #5 与 isolate `design.md:137-139` 均要求「决策六纯拆分先落、再接线」，bound 排序四步无此前置；⑤ `sc_author_teaching.rs`、`prompt_contract*.rs`、compiler fixtures、`single_candidate.rs`（engine）：bound 均未涉及，isolate/mechanicalize 串行链也未含 bound；`run/single_candidate.rs` 与 `lifecycle_store/workspace.rs:481-499` 三方（bound/effort/决策六）均要改，只有 effort 登记了先后（先冻结继承再窗口只引用），bound 未呼应。
- 其余：close 与 quota-pool/mechanicalize 的排序（`close design.md:165`）与 bound 无交集；quota-pool 与 bound 的「同一 session record CAS 由集成 owner 串行」双方一致。

### E. 与实现的接地（抽查 14 处，worktree `src/`）
✓ 一致：`run/single_candidate.rs:1211-1212` 两个布尔量；`:1222`/`:1306`/`:1389` 消费、`:1227`/`:1307`/`:1397` 置位；`:1202` 注释「额外 provider 驱动总上限 2 次」；`product/models/workspace.rs:844 provider_start_ledger`；`work_item_plan_policy/types.rs:166-168 is_actual_launch_observation = started && provider.is_some() && started_at.is_some()`；`lifecycle_store/workspace.rs:899` `claim_provider_start_with_details` 同键 `Ok(false)`；`workspace_engine/single_candidate.rs:155-157` 以 `ledger.len()`（Generate 相 `-1`）反推 attempt；`work_item_plan_policy/evaluate.rs:203-222` RepairReservation 仅 token/owner_session_id/owner_run_id/provider_start_idempotency_key/state/commit_id；`planning_failure.rs:310-338` `StructuralAuthorizationFacts`（含 `authorization_kind/attempt_no/max_attempts/active/anchor_waiting/launch_unknown`）、`StructuralAuthorizationKind::GenerationRun` 存在，`:404 classify_failure_set` 为纯函数且 Unknown/事实不完整先于 Operational、`attempt_no>=max_attempts→UnknownStop`；`provider_resources.rs:1414 release_after_session_tail`；`lifecycle_store/workspace.rs:497-499` 接管清 `repair_reservation`/`human_gate_reservation`/`provider_start_ledger`；`TrustedLaunchOwnership` 存在于 claude_code/codex provider。
✗ 偏差（均已列入 I-08/I-09/I-13）：`workspace_single_candidate.rs` 占位写入在 `:197-203`（`started: true, provider: None, started_at: None`）而决策要求 `started=false`；`gateway_start.rs` 函数在 `:583`；非测试 `start_work_item_plan_author(` 调用点 11 处（followups×4、legacy_outline×2、plan_batch、plan_draft、sc_manual_revision、single_candidate×2）vs 注释「七个」；`routing_scope.rs:20-46` 与 `single_candidate.rs:244-256` 的耦合 design 未点名。

### F. 门禁与可执行性
- 触及 OpenSpec 阈值的内容：500 字符上限（ADDED 全过，MODIFIED 豁免但归档后失效 → I-03）；每 requirement ≥1 场景 ✓。
- tasks.md：无文件路径/命令/测试/提交步骤（grep `\.rs|cargo|src/|git ` 仅 `design.md` 引用），每包均写「映射 + 验证」，粒度为高层工作包 ✓；例外：7.2/7.3/7.4/8.5 为流程/协调项（I-14）；1.2 内嵌字面键格式与状态名，边界偏细但仍属契约级，可接受。
- 不可测/自相矛盾：① I-01 使 6.2（接管引用关闭事实）与 SG-05 文本不可同时成立；② I-02 使 4.3「不旁路至另一自动重写入口」与现行 WSC-21 行为无法同时验收；③ 4.3 的「越权」分支无可测场景（I-10）；④ 8.2 的 A=0/B=0/C=1/启动合计2 可测 ✓。

## 4. 已核验通过项（证明确实逐条查过）
1. 6 条 ADDED 与 2 条 MODIFIED 的标题均不与主规格/其它 change 重名；归档模拟成功（+6 ~2）。
2. 决策 §1.3 的 ¶1~¶7 与 28/28A/28B/28C/28D/28E 逐句一致，无删减、无放宽、无软硬互换（MUST NOT 伪造候选引用、零额外调用并显式系统故障、不得由 A/B 余额兜底第三次均在）。
3. 决策 §1.3 六个场景 + §1.7 的 A=0/B=0/C=1/启动2 场景 + fail-closed 场景，全部有对应 delta 场景。
4. SG-05：除 SC 初轮一行与新增一段外，原段落与 5 行载体表、4 个原场景逐字保留；SC 人工修订「最多2」行未动，与「SC 初轮 原+1=2」两行载体口径一致。
5. WSC-25：non-blocking 场景与义务逐字保留；28D 与 25 双向写明「流水非启动许可」。
6. 去重：REQ-WSC-09 未被 bound 改写；quota-pool、mechanicalize 目录未触碰（`git status --short` 行数 0 于审查前后；审阅期间仅在 /tmp 做归档模拟）。
7. 决策 §9「仍未决」条目在 bound 全部文件无越界复述（grep 零命中）。
8. 编号：28/28A~28E 与 29、30/31、TOP-07 无重号；26/27 已归档占用，未复用。
9. 其余 5 个 change 的 `openspec validate --strict` 均 valid（bound、isolate、close、effort、mechanicalize、quota-pool 共 6 个均 exit 0）。

## 5. 未能裁决的缺口（需 owner/集成 owner 裁决，不自行补全）
1. **RepairOnce「批准」由谁、按何判据授予**（I-04/I-05）：SG-05 表仅出现该词；mechanicalize WSC-09 未写；决策 §1.4 要求其「以前置授权、合法 provider 与剩余机会为条件」尚无落点。
2. **WSC-21 触发域对 generate 段耗尽的适用性**（I-02）：WSC-21 归 quota-pool，本 change 无权改；需 owner 确认口径后再定 28E 限定句。
3. **attempt1 沿用旧键 `single_candidate_author:{id}:{n}` 还是新键，及占位 `started` 取值**（I-09）。
4. **`work_item_plan_policy/types.rs`、`models/workspace.rs`、`lifecycle_store/workspace.rs`、`run/single_candidate.rs` 等共享文件的最终串行顺序**（I-07）：各 design 互相登记不一致，需集成 owner 统一。
5. **I-03 归档后超 500 字符**：压缩 SG-05（及同类 close/effort）还是接受 `--strict` spec 校验变红，属流程取舍，需 controller 决定。
6. **未核实**：真实 provider 行为/并发 CAS 实际表现（本审查只读规格与静态代码）；effort/WIC/X3 三个 change 的内部语义正确性（不在本报告范围，仅核了编号、MODIFIED 集合、共享面与归档后 strict 校验）；archive 模拟仅在 /tmp 副本执行，已清理。
