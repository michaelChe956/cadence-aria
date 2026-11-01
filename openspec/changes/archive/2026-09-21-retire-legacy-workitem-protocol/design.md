# Design: retire-legacy-workitem-protocol

## Context

**决议权威（不得漂移）**：`.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/stage4-tripartite-decision.md`（Q1 全口径重测+挂起问法、Q4 (b) 契约显式化+(a) 条件 defer、Q5 切分 change ③）；oracle 裁决 `stage4-oracle-verdict.md`（Q1「不放宽门、不条件放行；重测仍超标挂起等用户」+remaining risks：pi 重测超时预算前置/归档 owner；一致性核查 2 项 scope 修正：前端 legacy 消费面迁移必须入 scope、重测按全口径不止 pi 时长）；scout 详报未落盘（决议/输入包转述其要点，本 change 起草已逐项实读复核并 supersede 其数字，见下）；ter 可行性=yield 摘要未落盘（工时带 C=39-64h、D=2-4h 以决议转述为准）。本 change 是决议 change ③ 的契约化落地，不重新论证、不引入未拍板项。

**现状技术事实基线**（本 change 起草实读，行号为 worktree `feat-b-0808-add-monorepo` HEAD=`244f6c85`（2026-09-19）实读）：

| # | 事实 | 证据 |
|---|---|---|
| 1 | REQ-WSC-07 判据原文（全口径清单）：codex 与 pi 各 1 案例到达 Confirmed（2/2）、单案例时长 ≤12 分钟、初评 ≤1 且复评 ≤1（总 ≤2）、自动返修 ≤1（均从服务端持久计数读取）、14 条 classifier golden（rep2/3/4 的 9 条+rep1 round-1 的 2 条 Advisory+3 条人工标注 class_hint 变体）全部归入预期 finding 分类、仅明确属 grammar/lowering 的 reviewer finding 通过 compiler diagnostic golden、断线重连/恢复测试通过、legacy 路径回归全绿；第二段=多仓 preflight 条款（legacy fallback 仅限确定性 preflight 失败且新路径未产生副作用，已持久化/已启动 provider 后失败收敛新路径 durable 终态、不静默切 flow_kind） | `openspec/specs/work-item-plan-single-candidate/spec.md:121-135` 实读 |
| 2 | pi 时长唯一登记未过：08-31 实测 938.04s（15.63min）＞12min；用户 1c 裁决（08-31）=不放宽、不改口径、不删子项；解锁路径仅两条=pi 达标 or 后续专项裁决；重测唯一实测量早于其后全部栈变更 | oracle 继承决策核查清单（实读转述）；立项输入包面 C |
| 3 | build 版本：决议记载重测基线「现行 build（v22）」；起草时现行部署已至 v24（aria-4x，progress.md 2026-09-19 01:0x 实录）。重测以 WP 执行时现行 build 为准（≥v24），版本如实记录于证据矩阵，判据零改 | progress.md :567 实读 |
| 4 | 后端 wire 层：`WsInMessage` 28 变体全活。legacy 逐段确认族（REQ-WSC-07 点名三族+专属 DTO）：`SelectWorkItemGenerationMode`（:68，generation-mode 决策，+`WorkItemGenerationModeDto` :178）、`WorkItemDraftDecision`/`WorkItemBatchDecision`（:81/:86，逐段确认消息，+`WorkItemDraftDecisionDto`/`WorkItemBatchDecisionDto` :185/:193）、`ReviewDecisionResponse`（:61，review_decision 双选项语法）、`SelectRevisionPath`（:71，+`RevisionPath` :152）；另有 `AuthorDecision`（:65，+:168）、`RequestRevision`（:75）、`RequestOutlineRevision`（:78）、`HumanConfirm`（:105，+`HumanConfirmDecision` :160） | `src/web/workspace_ws_types/in_.rs` 实读 |
| 5 | 消费核心：`src/web/workspace_ws_handler/decisions.rs`（`handle_review_decision_from_handler`:87、`handle_author_decision_from_handler`:175、legacy confirm 判定 :324-326、`handle_human_confirm_from_handler`:367 起）；`src/product/workspace_engine/decisions.rs`（`handle_human_confirm` 决策路由 :570-726 区，Confirm/RequestChange/Terminate 三分支）；`src/product/workspace_engine/draft_batch/decisions.rs`（batch 逐段决策） | 实读 |
| 6 | **量化复核（supersede scout/输入包旧号）**：`HumanConfirmDecision` 触及 24 个 .rs 文件（非 `/tests/` 路径产码 10 个）；全 legacy 族符号触及面随符号集口径而变：六符号窄集（HumanConfirmDecision+逐段族点名+SelectRevisionPath）32 个 .rs，精确全名集（再含 AuthorDecision/RequestRevision/RequestOutlineRevision/SaveHumanPresentationRevision 及各 Dto）57 个——绝对数以实读为准，趋势结论（前端大头、测试面过半）不变；前端 `human_confirm|HumanConfirm` 触及 61 个 ts/tsx（产码 22）。决议/输入包记载「后端 44 文件/前端 57 文件」与 HEAD 不一致——以本表实读为准；结论不变（oracle scope 修正成立：删除大头在前端，且测试面占比过半） | 本 change 起草 `grep -rl` 全仓实读（排除 target/测试分栏见数） |
| 7 | 前端双轨三处（决议原文锚）：①union 双轨——`web/src/api/types/workspace.ts:267` 同一 union 含 `{type:"human_confirm",decision,payload}` 与 `human_gate_feedback`/`advance`；②useStageUI legacy 三动作——`web/src/hooks/useStageUI.ts:55-60` `human_confirm:{actions:["confirm","request_change","terminate"]}`（另 `review_decision:{actions:["select_revision_path","abort"]}`）；③plan-repair/cockpit 仍发 human_confirm——`web/src/hooks/useWorkspaceWs.ts:483-487`（request_change/terminate 经 `human_confirm` 发送）、`web/src/state/bulk-confirm-runner.ts:127`（批量 confirm 经 `human_confirm`）、`web/src/state/cockpit-action-routing.ts:139`（`legacy:human_confirm` gateId 路由）；plan-repair 动作面经该路由（`useWorkspaceWs.plan-repair-actions.test.tsx:49,80` 佐证）；`ChatWorkspacePage.tsx:3` 引 `LegacyChatWorkspacePage`（`ChatWorkspacePageLegacy.tsx` 903 行产码） | 实读 |
| 8 | SC 门关门决策现承载（重承载前提）：approve=`Confirm` 变体（:44，无 payload）；abandon 经 `HumanConfirm{decision:Terminate}`；`conversational_gate.rs:776-871` 以 `HumanConfirmDecision` 参数承载关门（:808 RequestChange 分支=legacy-only、:831 Confirm 分支）；REQ-CG-02 边界句「legacy session 保持 RequestChange 行为不变；旧枚举在 REQ-WSC-07 退役门满足前 SHALL NOT 删除」+REQ-CG-04「映射现行 HumanConfirmDecision::Confirm/Terminate」 | `src/product/workspace_engine/conversational_gate.rs`、`openspec/specs/work-item-plan-conversational-gate/spec.md:27,75` 实读 |
| 9 | DEF-4 代码事实（显式化前提=行为已在）：advance 到 Ready 严格不启动 provider（`advance_ready_only.rs:72` 测试 `advance_ready_response_does_not_start_coding_provider`）；coding 启动唯一入口=WS `StartCoding` 手动；守卫=`SC_CODING_REQUIRES_ADVANCE` fail-closed（`src/web/coding_ws_handler/socket.rs:284,302,321`）；defer-ledger DEF-4 原文=「未来 `auto_start_coding` 显式语义（advance 到 Ready 即止，不自动启动 coding provider）」owner 后续 change；DEF-6=「旧协议退役门（`HumanConfirmDecision` 等旧枚举在 REQ-WSC-07 退役门满足前不删）」owner 退役门 change | 实读+`cadence/reports/workitem-conversational-gate-advance/defer-ledger.md:23,25` |
| 10 | `flow_kind`（`WorkItemPlanFlowKind`）消费面广布：lifecycle_store（inputs/workspace/amendment_gate/workspace_policy_route:80-84/workspace_single_candidate）、models（outline/workspace）、workspace_engine（compile/prompts/review）；durable session record 持久化字段 | 实读 |
| 11 | campaign 基建在场（重测复用）：`cadence/reports/workitem-coding-campaign/`（workitem_run_campaign.mjs/coding_run_campaign.mjs/stage3 驱动器族，显式省略 `--dry-run` 才调真实 provider）；coding_run_campaign 未测区按五阶段路线「顺带核（冗余则标记退役）」 | 目录实读+路线 v2.0 |
| 12 | 3.3 设计先例（auto 通道红线源）：拒绝「advance 批量拉起全部 coding」——不另造自动启动协议；将来需要时单独显式定义 auto_start_coding；单独显式定义未被排除 | oracle 继承决策核查清单（3.3 设计实读转述） |

**本文只做规划产物**：精确文件、命令、测试与提交步骤由 `superpowers:writing-plans` 写入 `cadence/plans/`。

## Goals / Non-Goals

**Goals:**

- 退役门全口径重测在现行 build 出具完整证据矩阵，判据零改；全绿→解锁删除（REQ-RET-01）。
- legacy 决策协议删除：后端 wire 层+引擎消费核心+`flow_kind` 单路径收敛、前端消费面全量迁移、全仓残留归零（REQ-RET-02；REQ-WSC-08）。
- SC 门关门决策 typed 重承载（approve=`Confirm` 保持、abandon 显式 typed 命令），REQ-CG-02/04 同步修订。
- 多仓 preflight 条款修订：失败一律收敛新路径 durable 终态，无 legacy 回落（REQ-WSC-08；oracle 风险承接）。
- DEF-4 契约显式化销账+auto 通道红线条件 defer 登记（REQ-ADV-05）。
- coding_run_campaign 未测区顺带核，冗余则标记退役（REQ-RET-04）。

**Non-Goals（与 proposal 非目标互补）:**

- 不重开 Q1/Q4/Q5 已决事项（全口径重测不放宽、DEF-4 (b) 形态、三 change 切分与 ①→③→② 序）。
- 不在本 change 预设「重测仍超标」的处置——挂起等用户终裁是唯一分支，星化问法三选仅备呈报，不预执行任何一项。
- 不动 SC 共享基座（REQ-WSC-01..06、REQ-CG-01..07 语义、compile/policy/amendment、StartCoding 语义本身）。
- 不做前端 UI 重设计——迁移以「typed 等价替换 legacy 动作面」为限，视觉/交互不变。
- 归档/主 specs sync 只同步本 change 四 capability delta，不顺手改其他 spec。

## Decisions

### D1: 门重测口径 = REQ-WSC-07 判据原文全子项、现行 build、判据零改、超标即挂起

**决策**：重测作为本 change 第一个 WP 与解锁闸门——WP2 及以后的所有删除动作在重测证据矩阵全绿留档前不得开始。矩阵覆盖事实基线 #1 全部子项（codex+pi 各 1 案例 Confirmed 2/2、时长 ≤12min、初评/复评总 ≤2、自动返修 ≤1、服务端持久计数读取、14 条 classifier golden、grammar/lowering 才走 compiler diagnostic golden、断线重连/恢复、legacy 回归全绿、多仓 preflight 不静默回落），在 WP 执行时现行 build（≥v24）真实出具，复用 change ① 验证过的 campaign 基建（事实基线 #11）。**判据零改**（1c 钉死）：不放宽阈值、不改计数口径、不删子项；矩阵表格逐子项留档证据锚。**超时预算前置**（oracle remaining risk：pi 曾需 1800s 上限）：重测跑前先设定单案例超时上限与总预算并留档，防provider 挂死拖垮矩阵。**超标分支=挂起**：pi 任一关键子项重测仍超标→本 change 挂起等用户终裁，星化问法三选（A=维持门不删 legacy/B=授权登记例外+修订 REQ-WSC-07 门文本放行/C=限定 N 次重测取最佳）如实呈报，三方不得自行放宽（分歧预判 1 调解锚：若行使 C 类只能是「后续专项裁决」显式行使+同步修订门文本+登记例外，不得静默）。

**scope 接正声明**：决议 Q1 文「在现行 build（v22）重出」——起草时现行部署已至 v24；重测以执行时现行 build 为准（≥v24），build 版本如实记录于证据矩阵。判据与口径不受 build 版本影响，非漂移。

**替代方案与否决理由**：①沿用阶段 2 时代证据直接退役——证据陈旧且 pi 单子项登记未过，违反 1c；②重测仅测 pi 时长——oracle 一致性核查明确全口径（其余子项证据同为阶段 2 时代）；③超标后三方裁定限次取最佳——越权（1c 解锁路径仅两条）。

### D2: 删除面分层 = wire 层→引擎层→前端层，SC 门关门决策 typed 重承载先行

**决策**：删除面按三层推进，每层独立可验收：

- **L0（前置重承载）**：SC 门关门决策脱离 `HumanConfirmDecision`——approve 保持 `Confirm` 变体（REQ-CG-02 既有接受面，不动），abandon 以显式 typed 入站命令承载（新命令，形态由 writing-plans 定，契约要求=与 legacy `human_confirm` 通道零共用枚举）；`conversational_gate.rs` 关门签名收敛为门专属 typed 决策；后端此时 legacy 双轨保留（新命令落地+legacy 消息仍接受），全量门禁绿。
- **L1（前端迁移，oracle scope 修正的大头）**：前端全部 legacy 决策发送/动作面切 typed——union 收敛（`workspace.ts` 删 `human_confirm` 分支+加 abandon 命令）、useStageUI 三动作与 `review_decision` 动作迁移、`useWorkspaceWs`/`bulk-confirm-runner` 发送面、cockpit/plan-repair `legacy:human_confirm` 路由重承载、`ChatWorkspacePageLegacy` 处置（legacy 路径删除后其决策面不可达——合并/删除按消费路由实测定）；前端测试面同步收敛（61 文件中 39 个测试文件的 legacy 断言随行为删除，产码 22 文件的 legacy 分支清理）。
- **L2（后端删除，原子收口）**：`in_.rs` legacy 变体族+DTO 删除（事实基线 #4 清单：三族点名+`HumanConfirm`/`HumanConfirmDecision`+`SelectRevisionPath` 族+`AuthorDecision`+`RequestRevision`/`RequestOutlineRevision` 逐个判归属——判定规则=SC 路径是否消费，SC 不消费即删）；`workspace_ws_handler/decisions.rs`+`workspace_engine/decisions.rs`+`draft_batch`/`plan_outline` 逐段引擎面删除；`flow_kind` 单路径收敛（新会话一律 SingleCandidate；durable 存量字段只读兼容，见 D6）；收到已删除消息类型返回 protocol error 零副作用；前端残余类型同批归零（跨端原子，不留中间双轨态）。

**共享变体归属判定规则**：REQ-WSC-07 点名三族+`HumanConfirmDecision`（DEF-6 点名）为必删集；其余 legacy 家族变体（`AuthorDecision`/`SelectRevisionPath`/`RequestRevision`/`RequestOutlineRevision`/`SaveHumanPresentationRevision` 等）在 writing-plans 逐个以「SC 路径是否消费」判定——消费则保留（显式记录），不消费则删；本 change specs 只锁必删集与残留归零断言。

**替代方案与否决理由**：①后端先删前端后迁——中间态前端发已删消息全断，破坏「每 WP 独立可验收」；②长期双轨兼容期（legacy 消息保留 N 版本）——决议钉死退役即删，双轨维持成本正是本 change 要消除的；③只删后端不动前端（原输入包 scope）——oracle scope 修正否决（前端是大头）。

### D3: 退役后多仓 preflight 条款 = 失败一律收敛新路径 durable 终态（oracle 风险承接）

**决策**：REQ-WSC-07 第二段（legacy fallback 仅限确定性 preflight 失败且新路径未产生副作用）随退役失效；修订进 REQ-WSC-08：legacy 删除后**不存在** legacy fallback 路径——多仓 Issue 确定性 preflight 失败 SHALL 收敛为新路径 durable fatal/recoverable 终态（含原因），`flow_kind` 无切换目标、无静默回落；已持久化新路径状态或已启动 provider 后的失败同口径。实现面=会话创建路由处 legacy 分支删除（`workspace_policy_route` 族），preflight 判定本身保留（其输出从「可回落」变为「新路径终态原因」）。

**替代方案与否决理由**：保留条款文本不修订——条款指向已删除的 fallback，成死文本且误导（oracle 风险：spec sync 不同步=契约漂移）。

### D4: DEF-4 显式化措辞 = 三句契约+守卫+auto 通道红线条件 defer

**决策**：REQ-ADV-05 三句写入 `work-item-plan-advance`：①advance 的完成态=`Ready`（group workspace 就绪），SHALL NOT 启动任何 coding provider；②coding provider 启动唯一入口=显式 `StartCoding` 入站命令（人工触发），advance 完成/任何编排动作 SHALL NOT 隐式、批量或随附启动；③SC admission 的 attempt 在经 advance 置 Ready 前收到 StartCoding SHALL fail-closed 拒绝（错误码 `SC_CODING_REQUIRES_ADVANCE`）。**auto 通道红线条件 defer 登记**（Q4 裁决 (a) 降级形态）：显式 opt-in auto 通道不在本 change 实施；触发条件=autopilot/驾驶舱真实 auto 需求；届时另行立项且 MUST 满足全部红线五条——opt-in 持久化 run_policy、默认 off、per-attempt 单发、绝不批量、不动唯一人工门（3.3 设计先例「单独显式定义未被排除」的行权边界）。DEF-4 据此销账（defer-ledger 登记指向 REQ-ADV-05）。

**替代方案与否决理由**：①同 change 交付 auto 通道（Q4 挂起问法 B）——决议选 (b) 主裁，(a) 降为条件项；②完全跳过 DEF-4（选项 c）——路线漂移，否决。

### D5: 删除完成判定 = 残留归零断言 + legacy 回归退役留档

**决策**：删除完成的可验收口径（REQ-RET-02）=①全仓 grep 断言：legacy 必删集符号（`HumanConfirmDecision`、`SelectWorkItemGenerationMode`、`WorkItemDraftDecision`、`WorkItemBatchDecision`、`ReviewDecisionResponse` 双选项、`select_work_item_generation_mode` 等 wire 名）在全仓产码+测试归零（历史归档文档/durable 数据除外）；②全量门禁双绿（lib/it_web/it_core+fmt/clippy+前端测试）；③legacy 路径回归测试族（REQ-WSC-07「legacy 路径回归全绿」子项的证据留档后）随删除面退役——退役动作=删除测试+证据矩阵引用留档，不以「测试还在但永红」或 ignore 压制。

**替代方案与否决理由**：以「编译过+抽样测试」为完成口径——删除面残留（如前端类型、e2e fixture）不可见，违反残留归零断言的可判定性。

### D6: 存量处置 = 历史 durable 记录只读保留 + 在途 legacy 会话不承诺决策通道连续性

**决策**：历史 legacy session 记录/事件前缀只读保留（事件前缀不可变契约不破坏，不迁移不清洗）；`flow_kind` durable 字段对存量记录保持只读兼容（读侧容忍 Legacy 值、不再以其路由到 legacy 引擎——呈现为已完成历史）；部署时点在途 legacy 会话**不承诺决策通道连续性**（如实登记为迁移限制：在途 legacy 会话的后续人机决策消息将收到 protocol error，会话以现状终态留存）；新会话一律单候选流。不提供 legacy 复活开关或回退开关。

**替代方案与否决理由**：①部署前 drain（等待全部在途 legacy 会话终态）——单机自部署场景在途会话属边缘态，drain 无限期阻塞退役；②迁移存量 legacy 会话到 SC 流——跨协议状态机迁移无契约依据，发明新机器违反范围铁律。

### D7: 附录 B 决议承接对照表

见本文附录 B（Q1/Q4/Q5(③) 逐句 → WP/REQ 映射，含 oracle 裁决要点与风险登记承接）。与 C1 惯例一致，作为双审（k3+oracle）对照决议的锚。

## Risks / Trade-offs

| # | 风险 / 取舍 | 缓解 |
|---|---|---|
| R1 | pi 重测仍超标（15.63min 基线，表现不可预知） | D1 挂起分支前置（星化问法三选备呈报）；超时预算前置设定防挂死；挂起不阻塞 DEF-4 WP（独立面，可先行）|
| R2 | 重测暴露 SC 路径真实缺陷（现行 build 全口径首次复测） | 缺陷按既有契约 TDD 修复后重出证据；修复不扩权；如实登记 |
| R3 | 删除面回归（消费面广：后端 24 文件/前端 61 文件） | D2 三层推进每层全量门禁；D5 残留归零断言；L0 重承载先行保 SC 门行为零变 |
| R4 | abandon typed 新命令与既有 `Abort` 语义混淆 | L0 显式新命令+REQ-CG-02 边界句同点修订；形态留 writing-plans，契约锁「与 legacy 通道零共用」 |
| R5 | 前端迁移遗漏（cockpit/plan-repair 路由面散） | 以事实基线 #7 三处锚+全仓 grep 清单驱动；e2e 覆盖 typed 动作面 |
| R6 | wire 破坏性变更伤及未知客户端 | 单机自部署+前端同仓原子切换（L2 跨端原子）；已删消息 protocol error 零副作用可诊断 |
| R7 | 在途 legacy 会话决策通道断（D6 取舍） | 如实登记迁移限制；历史记录只读保留可审计；无静默行为变异（明确错误优于隐式回退） |
| R8 | spec sync 漏项（四个 capability 联动修订） | specs delta 一次成套（single-candidate/advance/conversational-gate/retirement）；关闸 WP 复核主 specs 无悬挂引用（REQ-WSC-07 被引点全数接正） |
| R9 | DEF-4 红线将来被静默突破 | 红线五条写入 REQ-ADV-05 条款文本（spec 级约束）+defer 登记；行权必须另行立项 |

## Migration Plan

1. **交付序**：WP1 门重测（解锁闸门；超标→挂起等用户，仅 WP4 DEF-4 可独立先行）→ WP2 L0 重承载 ∥ WP4 DEF-4 → WP3 L1 前端迁移 → WP5 L2 后端删除+跨端原子收口 → WP6 关闸。每 WP 独立验收。
2. **无部署迁移、无数据迁移**：历史 durable 记录只读兼容（D6）；wire 变更前后端同仓原子。
3. **回滚**：L0/L1 性质可普通提交回滚；L2 删除面大——回滚=git revert 整批（durable 新记录字段 additive，旧代码可读）；重测证据矩阵与删除留档不随回滚消失。
4. **挂起分支**：WP1 超标→change 挂起（WP2/3/5 冻结；WP4 可独立落地）；用户终裁后按裁决分叉（A=关 change 不删/B=修订门文本重开/C=限次重测条款补入后重跑 WP1）。

## Open Questions

均为实施细化留白（不改架构、specs 或工作包划分）：

1. abandon typed 命令的具体形态（独立 `abandon` 命令 vs 其他）——D2/L0 锁契约（与 legacy 通道零共用枚举），形态属 writing-plans。
2. `ChatWorkspacePageLegacy` 的处置形态（整体删除 vs 决策面剥离后保留只读呈现）——按其消费路由实测定，D2 锁定「legacy 决策面不可达」口径。
3. 共享变体逐个归属判定的最终清单（`AuthorDecision`/`SelectRevisionPath`/`RequestRevision`/`RequestOutlineRevision`/`SaveHumanPresentationRevision` 等）——D2 判定规则（SC 是否消费）在 writing-plans 逐个执行并留档。
4. 重测样本与超时预算的具体数值（单案例上限/总预算/失败重试次数）——D1 锁「前置设定+留档」，数值按 campaign 先例在 WP1 计划定。
5. 在途 legacy 会话的呈现细节（历史列表只读入口形态）——D6 锁红线（只读、无决策通道），呈现属 UI 实施细节。

# C1 归档时序说明

change ①（close-provider-validation）归档 sync 先于本 change WP6 关闸：其 REQ-PVR-03 的「REQ-WSC-07 门」为 Q3 划界口径的历史性提及（语义自足非悬挂），WP6.1 悬挂扫描已显式点名。

## 附录 B 决议承接对照表

（决议=`stage4-tripartite-decision.md`；oracle=`stage4-oracle-verdict.md`；scout 详报未落盘（决议/输入包转述要点，本文事实基线已实读复核并 supersede 其文件数）；ter=yield 摘要未落盘（以决议转述为准）。逐句承接，双审对照锚。）

| 决议原文 | 承接位置 | 备注 |
|---|---|---|
| Q1：退役门 pi 时长 → (a) 全口径重测，按 REQ-WSC-07 全口径在现行 build 重出完整证据矩阵，作为退役 change 第一个 WP | WP1 全部；REQ-RET-01；D1 | 判据子项全清单=本文事实基线 #1（spec 实读） |
| Q1：门全绿→退役解锁（1c「pi 达标」路径零例外） | tasks 排序总则（WP1 关闸=WP2/3/5 解锁前提）；REQ-RET-01 scenario「全绿解锁」 | 1c=08-31 用户裁决（不放宽/不改口径/不删子项） |
| Q1：重测仍超标→挂起等用户终裁（星化问法已备：A 维持门不删/B 登记例外+修订门文本/C 限 N 次取最佳）；不放宽不改口径不删子项 | REQ-RET-01（超标挂起 scenario+判据零改条款）；D1 挂起分支；proposal What Changes 1；tasks 全局边界 | oracle 分歧预判 1 调解锚（C 类只能专项裁决显式行使）在 D1 承接 |
| Q1：（决议风险登记）pi 重测超时预算前置设定（曾需 1800s 上限） | REQ-RET-01 超时预算前置条款；D1；tasks WP1.1 | oracle remaining risks 承接 |
| Q5 change ③：门重测 WP（含 coding_run_campaign 未测区顺带核） | WP1.4；REQ-RET-04 | 五阶段路线「冗余则标记退役」原文承接 |
| Q5 change ③：legacy 删除（后端 HumanConfirmDecision 消费面+in_.rs 逐段确认族+decisions.rs 消费核心） | WP2 L0+WP5 L2；REQ-RET-02；D2 | 消费核心行号=事实基线 #5（HEAD 实读）；scout「44 文件」已复核修正为 24（#6） |
| Q5 change ③：前端 legacy 消费面迁移 WP（oracle scope 修正：前端是大头——plan-repair 仍发 human_confirm/useStageUI legacy 三动作/union 双轨） | WP3 全部；REQ-RET-02 前端层；D2 L1；事实基线 #7 三处锚 | oracle 一致性核查 scope 修正 1 承接；scout「57 文件」复核为 61（#6） |
| Q4：(b) 契约显式化结项——「advance 到 Ready 即止+StartCoding 唯一入口+SC_CODING_REQUIRES_ADVANCE 守卫」写入 spec | WP4；REQ-ADV-05 三句；D4 | DEF-4 台账销账（defer-ledger.md:23） |
| Q4：(a) 显式 opt-in auto 通道降为红线条件项 defer（触发=autopilot/驾驶舱真实 auto 需求；若做须满足 opt-in 持久化 run_policy/默认 off/per-attempt 单发/绝不批量/不动唯一人工门） | REQ-ADV-05 红线条件 defer 条款；D4；tasks 全局边界 | 3.3 设计先例（单独显式定义未被排除）行权边界=红线五条 |
| oracle Q1 风险承接（决议 Q5 四项之外）：退役后 REQ-WSC-07 多仓 preflight 条款同步修订——REMOVED 后不修订即死文本 | REQ-WSC-08 多仓条款；D3；WP5 | oracle 风险（spec sync 同步）承接 |
| oracle：证据新鲜度（pi 唯一实测=08-31 早于全部栈变更）→重测按全口径不止 pi 时长 | D1 全子项矩阵；REQ-RET-01 | oracle 一致性核查 scope 修正 2 承接 |
| 决议 scope 接正：面 C 删除大头在前端 57 文件非后端 21 | D2 L1 独立 WP；事实基线 #6 复核（61/24，趋势成立） | 数字以 HEAD 实读为准，结论承接 |
| ter 工时带 C=39-64h / D=2-4h | design Risks（R3 删除面广）；WP3/WP4 对应 | DEF-4 2-4h=Q4(b) 销账定价，不虚编细分 |
| 决议下一步：四件套→validate strict→双审（k3+oracle）→计划→实施 | 本四件套即交付物；WP6.1 validate strict 关闸 | 双审输入=本 change+附录 B |
