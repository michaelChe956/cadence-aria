# Tasks

所有包仍未实施，不因本次strict通过勾选。架构权威为v1.2与2026-10-13 owner全批；当前精确独占写面/接口/TDD/独立commit以 `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/cadence/plans/2026-10-13_实施计划_确定性结构治理_v1.1.md` 同编号Task及附录A为准，v1.0保历史。接地P1×1/P2×3仅补owner/接口闭包，oracle ADOPT零漂移不重跑，修后Main定向复核P1；本次不commit/sync/archive/实施。

每个实施包均须 TDD 红→绿、独立审查通过、显式Files/Test独立commit后才移交。Main统一调度检查，不在worker/上游半成品时并行测试。超过4h派工禁止；Plan已拆SG5/SG6a/SG11，不能把拆包视为减验收。

## 0. 排程与合同冻结

- [ ] 0.1 Plan接地审查∥一致性裁决，注入发现并owner审阅；controller提供trust/anchor task+commit+独立审查+最终写面移交，不从checkbox/旧ledger推完工。共享模型、gateway/SC/provider_run/prompts/wire/Page按Plan窗口串行。
- [ ] 0.2 同capability来源组合按design：root→gateway→admission/trust→较新anchor→本change；PLN本change只ADDED09/10，CG02/03/04/05完整MODIFIED、coding首真实标题最小L1豁免；不改源change/主spec、不引回自动fresh。

## 1. typed基础（继承v1.1，不假设已实现）

- [ ] F1.1 typed失败与资格纯核心（REQ-SG-01/02/05）。依赖：无；mod无上游活动写面移交后。验收：sg_f1_1_typed_disposition：InvalidWorkItemHeading有原行/field/example；同kind稳定code/category；含Unknown/Operational或缺活动授权整批不自动；多解DependencyCycle事实完整可L1；没有业务矛盾证据不造L3。；红绿与独立commit/移交按Plan同号，2–3h。
- [ ] F1.2 compiler源typed及split可选kind旧兼容（REQ-SG-01/02/02A）。依赖F1.1。保compiler细分/line0/未知JSON；utils::error/warning维持旧三参、split literal kind=None/Unknown，不在本包切显式kind工厂让F2b追补caller；同Plan附录兼容闭包TDD/独立commit后utils串行交F2b，3–4h。
- [ ] F2a canonical引用供需typed findings（REQ-SG-01/02/REQ-PLN-09）。依赖：F1.2；planning_failure F1→F2a。验收：sg_f2a_canonical_payload：dependency_cycle/self/unknown/provider/required capability缺口不丢ID，sort/dedup集合不标有向witness；真实Input引用不删除；旧JSONUnknown不自动。；红绿与独立commit/移交按Plan同号，2–3h。
- [ ] F2b split/preflight typed工厂与全部caller清切（REQ-SG-01/02/02A）。F2a后从F1.2接utils，outline/draft/plan/mod及tests同提交切显式kind工厂/全caller，无旧签名shim；原options/intent/trusted-command权限不变、missing不统一免费L1，mixed operational停；按Plan同号独占红绿/独立review/commit，2–3h。
- [ ] F2c projection typed继承与系统故障（REQ-SG-01/02/03）。依赖：F2b；planning_failure F2b→F2c。验收：sg_f2c_projection_origin：同dependency保IDs/kind，projection内部错不驱author，Unknown旧record只折叠；逻辑/物理投影不一致拒绝，不从Display拆码。；红绿与独立commit/移交按Plan同号，2–3h。

## 2. 同源图、机器指令及前后置系统门

- [ ] SG1 单源有效图/来源边/真实witness（REQ-PLN-09/10、REQ-SG-02）。依赖：F2c；compile_support独占。验收：sg_01_dual_source_and_real_witness：gateway3/frontend4 R1两方向边及来源；一端点两个来源都保，删一来源仍有效；Kahn残余下游节点不误环；每witness跳确邻接，稳定顺序；自依赖拒；未知provider拒；same-repo无auto边，单仓空顺序不变；logical/physical映射同构。；红绿与独立commit/移交按Plan同号，3–4h。
- [ ] SG2 精确机器指令与autorepair清切（REQ-SG-02/02A/04A、REQ-PLN-10）。依赖：SG1；autorepair由F1.2移交。验收：sg_02_no_fake_edge_removal：R1 required_action含两来源/仓序/Inputs真实闭包；多个安全候选不给唯一标签；仅局部固定grammar/历史核验给唯一证据；删必需Input/AC或只改Dependencies拒；内容autorepair没有source mutation/8×AI循环；未知key大码heading不删行。；红绿与独立commit/移交按Plan同号，3–4h。
- [ ] SG3a.0 immutable结构报告存储/全部record literal兼容（REQ-SG-02/03/04、REQ-PLN-09/10）。SG2后在既有mechanical report外层Option保存结构payload，旧None skip保证hash，source/IR/context/rule/report digest真实join、新context新record/ref；Plan附录A.2全构造同提交，旧report不补证，真实publish_freshness fixture红绿/独立review/commit，2–3h。
- [ ] SG3a.1 系统门及实际SC reviewer同代结构输入（REQ-SG-02/03、REQ-PLN-09/10）。SG3a.0后，保坏candidate零提升/评审/确认；实际single_candidate_input.inc.rs builder读同record结构报告、核当前context/join并注入边来源/身份/审计三section，旧context/缺证拒发，96KiB与原review预算不变；独占PlanFiles+真实输入TDD/独立review/commit，3–4h。
- [ ] SG3b 冻结Final Compile同次typed传播（REQ-SG-03、REQ-PLN-09、REQ-CG-04/05）。依赖SG3a.1，compile/types/close/WS/REST同提交typed，prepare前无transaction仍本次failure_id/approval/source可达，不捞旧Failed、不重lower偷换发布；types后串行交SG7a；Plan同号红绿/独立review/commit，3–4h。

## 3. 用户账本、候选交付与历史恢复

- [ ] SG4a.0 模型兼容与全构造闭包（REQ-CG-02/03/05、REQ-SG-05）。SG3b及trust/anchor最终移交后，独占Plan附录A.1模型/initializer/全部Rust literal文件；TDD旧JSON字段缺失仍可读、grant/launch Unknown无新授权、新字段roundtrip/原CAS预算不变，独立review/commit后交SG4a/4b/6/7/10，2–3h。
- [ ] SG4a hold/receipt/attempt与真实启动账本（REQ-CG-02/03/05、REQ-SG-05）。依赖SG4a.0。TDD验证用户总3−charge−held、reserve仅hold、deliver settle/未deliver结构release/非结构failure charge终态互斥且恰一次；已deliver复评不退、attempt1→2唯一/L0已用2不得3、旧started非actual；独立review/commit与Plan同号独占store，3–4h。
- [ ] SG4b 合法交付/checkpoint/整组历史恢复（REQ-CG-02/05、REQ-SG-04/04A）。依赖：SG4a；models/workspace串行接管。验收：sg_04b_checkpoint_current_context_business_and_review_join：合法refs提交settle一次/turn仍Running；checkpoint三refs/version/hash完整；纯restore0author且approval/approved_at/reservation/provenance全失效；新反馈未满足、旧context/缺IR/report/hash错禁止回退；review无法join进Reviewing，已review同refs/context才能保；旧receipt不能新计。；红绿与独立commit/移交按Plan同号，2–3h。

## 4. 完整模板与冻结机器输入

- [ ] SG5.1 同源author/revision grammar教学（REQ-CG-03、REQ-SG-02）。依赖：SG4b；prompt公共输入先落。验收：sg_05_1_shared_grammar_template：初轮/SC修订都含grammar/template/WI合法ID/真实成员仓序和auto边规则；Plan Intent保持可选不造create授权；19000author预算不扩大、当前业务target/member language不被native root替换。；红绿与独立commit/移交按Plan同号，2–3h。
- [ ] SG5.2 机器可信块与完整revision输入（REQ-CG-03、REQ-SG-02/05）。依赖：SG5.1；workspace prompts/revision等待anchor B移交。验收：sg_05_2_full_prompt_and_digest：28695字节候选完整不summary/truncate；原反馈逐字保留；机器块非typed_human_feedback，新heading诊断原line/example、grammar/template/成员/仓序/resolved都在digest；inline/硬限/完整分块缺ref拒先CAS零hold/start；UTF8中文边界。；红绿与独立commit/移交按Plan同号，2–3h。
- [ ] SG5.3 冻结修复子prompt delivery与fresh builder（REQ-SG-05、REQ-CG-05）。依赖：SG5.2；inc随后交SG6b。验收：sg_05_3_frozen_subprompt_delivery：活动grant合法中间hash不同原turn.hash允许精确join；子prompt缺ref/digest错零delivery/start；无state原builder逐字段不变；原反馈/context/fullcandidate完整，fresh None resume。；红绿与独立commit/移交按Plan同号，1–2h。

## 5. 同turn驱动、复评和安全恢复

- [ ] SG6a.1 assess与同turn claim驱动前置（REQ-CG-03、REQ-SG-02/05）。依赖：SG5.3；conversational_gate从SG3b接管。验收：sg_06a_1_assess_before_terminal：第414行typed诊断turn仍Running且current refs保，claim attempt2只一次，badsource不可review；业务事实缺少走原门、operational waiting，类别和class_hint不能许可；L1多解不用户拓扑；超限typed系统故障。；红绿与独立commit/移交按Plan同号，2–3h。
- [ ] SG6a.2 handler实际author与初轮机会融合（REQ-CG-03、REQ-SG-05）。依赖：SG6a.1；provider_run等trust/anchor最终移交。验收：sg_06a_2_heading_attempt2_gateway：414错误→attempt2含原反馈/指令/fullcandidate→合法；actual starts恰2、turn/command同、accepted增1非2、review正常不自动confirm；input/fingerprint拒start0/unknown等待；无RepairOnce初轮0额外。；红绿与独立commit/移交按Plan同号，2–3h。
- [ ] SG6a.3 交付/复评/耗尽收口与三连撞 fixture（REQ-CG-02/03/04/05、REQ-SG-04/07）。依赖：SG6a.2；inc与store从前包串行接管。验收：sg_06a_3_three_collisions_bounded：R1→R2同turn2attempt到顶故障release恰1、预算3可用/last-valid三refs保、第3start0；将R3回归风险同失败历史保存，不能为了执行第三错误开turn；跨合法业务轮次完整R1/R2/R3继承resolved/checkpoint，accepted只真实用户、refs提升前阻断；reserve/claim/refs/settle/release断点幂等。；红绿与独立commit/移交按Plan同号，2–3h。
- [ ] SG6b 复评旧路由防旁路与anchor恢复join（REQ-CG-03/05、REQ-SG-03/04A/05）。依赖：SG6a.3；inc从SG5.3接管。验收：sg_06b_reviewing_recovery_and_old_policy_guard：两attempt满后review Revise第三start0、single_candidate_author key/reservation/repairs_used增量0；L1剩attempt2同key，L2/business/unknown无新许可；Reviewing重连不Completed/不author；Pass原门；R3全谓词阻断，另一合法重组允许；BlockedByLcRecovery优先、显式fresh同attempt None-resume/子prompt、错recovery/digest/ref/closed/source漂移0start/预算改。；红绿与独立commit/移交按Plan同号，3–4h。

## 6. wire、同源前端与可选预填

- [ ] SG7 server-only DTO及可选machine_repair全literal切换（REQ-SG-01/03/05/06）。SG6b/SG5.1后接SG3b DTO、前包生产/fixture按Plan附录A.3全量回交；ReviewFinding Option同提交所有literal，server证实绑定L1才Some、Raw两parser恒None/伪许可不穿透。定义纯snapshot projection但实际SessionState field/builder归SG7a；TDD/独立review/commit，3–4h。
- [ ] SG7a durable→runtime→SessionState真实生产闭环（REQ-SG-05/06、REQ-CG-02/05，P1）。SG7后/独占types、session_state、reload、CAS镜像与全runtime/frame literal，等trust/anchor A最终移交；真实from_record/reload/build/manager纯路径五态Running-held/Reviewing-delivered/结构退款/非结构计费/终态checkpoint相等，错scope/旧None/anchor/cwd保留及纯读零副作用；TDD/独立review/commit闭合才开SG8.1，3–4h。
- [ ] SG8.1 live/rebuild/observer同构归约（REQ-SG-05/06、REQ-CG-02/05）。硬依赖SG7a真实production五态测试和独立审查/commit通过，非只DTO。归约身份/receipt/三refs/context/历史同构、duplicate/旧generation拒、Unknown不推许可、中间不红错/终态不吞/reconnect0启动；Plan同号独占红绿/独立commit，2–3h。
- [ ] SG8.2 四泄漏面统一业务/工程投影（REQ-SG-06、REQ-CG-02/04）。依赖：SG8.1；copy/cockpit独占。验收：sg_08_2_failure_surfaces：cycle/heading raw行号不主红错、系统处理不人工pending；L3只业务后果；structural未deliver工程fault显示未计交付/保版本/未发布；nonstructural失败扣次不免费；stale/blocked/phase mismatch无confirm；terminal command拒收可见。；红绿与独立commit/移交按Plan同号，1–2h。
- [ ] SG9.1 状态卡/详情/机器finding独立入口（REQ-SG-06/06A）。依赖：SG8.2；finding-list/Entry独占。验收：sg_09_1_system_governance_cards：主区处理中/已解决/业务/工程一次当前状态，主区无attempt计数；details真实证据/receipt/三计数可读；处理中已解决非需采纳，零必需结构操作；keyboard展开、polite/terminal一次alert、44px/focus/4.5:1/reduced-motion；原advisory采纳不变。；红绿与独立commit/移交按Plan同号，2–3h。
- [ ] SG9.2 Page合法editor预填与命令重放守卫（REQ-SG-06A）。依赖：SG9.1；Page等trust5.2/anchor D最终移交。验收：sg_09_2_prefill_zero_outbound：当前合法editor恰一次append保用户草稿/焦点、WS/REST/provider计数0；observer/closed/旧hash/context/ref/不存在editor拒不丢草稿；实际submit新业务turn才send；非terminal重用command、terminal拒收结果可见/真实重试新ID；confirm/bulk trust动作集合不变。；红绿与独立commit/移交按Plan同号，2–3h。

## 7. 普通/legacy载体适配

- [ ] SG10a 普通author/Story/Design适用L1（REQ-SG-01/02A/05/06）。SG7a释放lifecycle/prompts/runtime/finding fixtures后，保原父durable授权/合法artifact2否则1、不叠池、业务事实与AuthorConfirm门；Plan同号接口/Files/红绿独立review/commit，3–4h。
- [ ] SG10b legacy/draft/batch单耐久计数driver（REQ-SG-01/02A/05）。依赖SG10a；provider_run由SG6a.2→SG10a→SG10b串行。TDD总3/重启不重置/第4start0；draft/batch RepairOnce2否则1、不套池；技术结构靶向机器指令、真实Inputs/AC保留、原flow门及退役命令零复活；同Plan独占文件、红绿/独立review/commit，2–3h。

## 8. coding/review L1自动处理（保已有预算）

- [ ] SG11.1 coding/review耐久父身份与窗口（REQ-SG-01/05、coding首req）。依赖：F2c；可与SG1并行；role-run模型所有literal独占。验收：sg_11_1_coding_identity_preserves_windows：cycle1..3、prior同stage/role且失败/递增，重复cycle+attempt拒；L1reason不能伪transport；teaching一次/repair一次durable不重启重置；manual retry新cycle关联原failed不变；旧recordUnknown不补调用权；unknown launch不重claim。；红绿与独立commit/移交按Plan同号，2–3h。
- [ ] SG11.2 coder教学窗口融合结构指令（REQ-SG-01/02A/05、coding首req）。依赖：SG11.1＋SG2；provider_retry/prompts本包独占后交SG11.3。验收：sg_11_2_coder_l1_teaching：未知defect_class结构错保raw诊断/业务field，机器指令真实coder恢复合法；原一次teaching已用再错不第二teach、同cycle最多3；transport分别记reason，teaching原最大组合不统一改≤3、不叠结构池；解析失败不写合法completion/commit/push；权限等待/PlanDefect/实际越界写不L1放宽。；红绿与独立commit/移交按Plan同号，3–4h。
- [ ] SG11.3 WorkItem code-review同cycle自动schema修复（REQ-SG-01/02A/03/05、coding首req）。依赖：SG11.2；provider_retry/prompts/types串行移交。验收：sg_11_3_code_review_schema_before_gate：正常无效JSON/schema→machine instruction→同role同cycle合法report；最多原cycle3（已transport用3则额外0）；无效output不合成合法blocked finding推动业务门，最终系统故障仍可现恢复；verdict/finding/repair_target真实性保、不伪approve，普通真实业务finding原triage；不驱coder、不rework_count。；红绿与独立commit/移交按Plan同号，2–3h。
- [ ] SG11.4 group review一次repair机器化与fidelity（REQ-SG-01/02A/05、coding首req）。依赖：SG11.3；review_parser串行移交。验收：sg_11_4_group_repair_once_fidelity：保GROUP_REVIEW_VERDICT原marker，禁止repair→approve/不同verdict、finding/evidence/target非原输出可追溯拒；repair16KiB cap/finding原limit；transport3＋repair1既有组合不新增L1池，重启sameclaim不第二repair、unknown等待；invalid报告不提升可final-confirm。；红绿与独立commit/移交按Plan同号，2–3h。
- [ ] SG11.5 coding/review wire/history同源呈现（REQ-SG-01/06、coding首req）。SG11.4后且公共workspace DTO/真实生产snapshot已SG7/SG7a稳定、通用卡已SG9完成；history分原transport/teaching/repair不混计，terminal可见/reconnect0出站/原门动作不改；Plan同号独占红绿独立review/commit，2–3h。

## 9. 统一事故链与最终收口

- [ ] SG12 所有独立包完成后的controller统一验证/owner验收/终审收口（全部requirements）。检查前包红绿命中证据和完整同turn超限停点＋跨合法轮次R1/R2/R3防复发、五阶段L1、user receipt/attempt/实际start/refs/context/witness/审计及gateway/fresh/confirm/broadcast四泄漏面；按Plan第12节统一验证，实证全过才勾选/sync/archive。无新实现写面、无伪空commit，2–3h。

## 10. 串行移交约束

串行移交以Plan v1.1独占正文＋附录A.1～A.4为准：utils F1.2旧签名兼容→F2b显式kind全caller；报告SG3a.0全literal→SG3a.1实际review builder→SG4b/SG6；types SG3a.1→SG3b→SG7a；finding producers/fixtures前包→SG7全machine_repair literal；out.rs SG3b最低DTO→SG7类型→SG7a真实SessionState；runtime/门/恢复/review CAS镜像SG6→SG7a→SG10，前端SG8.1及SG10a等SG7a生产五态通过；provider_run SG6a.2→SG10a→SG10b，coding窗口旧序不变。任何同文件/fixture不双写；SG7a等trust/anchor A最终移交，不能把新的production接线当serde自动。

SG12只controller最终验证/验收，红用例及fixture由前包落地，不建新的实现“收口包”。全部行为及 named scenarios 有实证后才勾选；strict仅验证规划合同，非产品绿灯。
