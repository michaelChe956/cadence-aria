# Design

## Context

动机见 proposal。架构权威为 owner 于 2026-10-13 已全批的 `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/cadence/designs/2026-10-13_方案设计_五阶段错误体验_确定性结构治理_v1.2.md`（含 Oracle 四问裁定、GROUND_OK、§11独占包）。本 change 持久化获批方案，不改原设计历史“待批”文本；本批复覆盖其§9.2及v1.1遗留三项。预算B新释为实质重裁，退款仅结构未交付，coding/review纳L1，主区沿默认，非将旧B冒称同意。

已核验指定 worktree：compile_support::project_work_item_plan_drafts_for_compile 从 canonical Inputs 派生depends_on，apply_change_order_cross_repo_depends_on补全部前仓→后仓边；detect_dependency_cycle的Kahn残余不是有向witness且排除自依赖。contract_prerevision仅canonical、autorepair仍直接改变内容；run_sc_manual_revision_turn失败先终态、成功先Completed后review；HumanGateTurn预约扣次且attempt固定2。故v1.1的typed producer/grant/DTO链是待实施基础，不能写成已实施前提。新增类型/接口均在Plan标“拟新增”。

coding实证：provider_retry::MAX_PROVIDER_INVOCATIONS_PER_CYCLE=3，coding::run_coding_output_teaching_reredrive为恰一次教学窗口、窗口内仍用retry cycle；code_review::execute_code_review_with_commands在report保存/门前解析；group_review_orchestrator::GROUP_REVIEW_MAX_ATTEMPTS=3，execute_repair独立一次且16KiB，validate_repair_fidelity保verdict/证据/target不伪approve。Main已裁保已有最大组合，不统一改≤3、不叠L1池；coding-provider-transport-retry首req只解除L1结构拒收禁止，transport语义不动。

## Goals / Non-Goals

目标：纯同源结构门→精确机器指令→既有授权author→全量复验/原评审→原确认；合法历史防回归；SC用户hold/receipt与结构物理attempt分立；五阶段同源typed错误面。实现必须覆盖继承producer基础和coding/review，不仅SC。

边界：不改已确认业务事实/Design仓序/成员权限，不增加provider授权/调用机会，不系统patch内容，不追溯旧预算/授权，不把Final Compile、confirm、anchor fresh当新增author入口。coding/review不统一重排预算结构。无新的人工门/workspace/stage枚举、ProviderRunKind或平行validator/harness。

## Decisions

### D1. 源typed判据，不从文本/模型许可

复用 compiler/contract/split/projection producer；PlanningFailureKind携结构化payload，category/origin/disposition由同源函数投影。旧code继续诊断兼容，未知旧record为Unknown且不补授权。L1由全事实确定检出并可生成靶向指令，多解仍author工作；L2开放技术沿既有合同，L3仅业务必须让步证据，operational/unknown先等待。非法heading单独typed，不按unknown key大码删除。provider RawReviewFinding封闭输入不能提交服务端planning IDs/资格，structural_suspect仅请求复检。

### D2. 一个纯有效图，candidate与compile同构

compile_support抽PlanningStructureContext/items→report：context冻结plan绑定Confirmed Design、成员选区、options/baseline/intent授权、refs/hash及规则digest，外层装载一次，不在review纯门写selection。E=canonical Inputs provider→consumer ∪ change_order自动边；声明Dependencies单独一致性校验。Logical WI/Logical member映射physical WI/repository同构，native aggregate不混业务target。同端点来源集合全部保留。Kahn判阻断，固定ID/邻接序DFS提实际witness；线性图检测，不穷举最小删边集。保持自依赖/未知引用/同仓与空单仓原语义。

### D3. 精确指令与单作者来源

扩contract_prerevision与autorepair既有finding→required_action模式，指令绑定rule/source/context/failure/repair/digest、affected fields/edge来源、允许闭包、保护项、expected predicates。局部完整域唯一才附证明；拓扑一般多解交author，删边须同供需已满足证据，不以无环判业务正确。范围内autorepair语义内容修改迁为producer，不保直改旁路；仅原交付前前言/固定heading/EARS空白规范化保留。所有新内容包括唯一解均author→新immutable refs→全量验证→原review。机器可信块不伪造human feedback，完整原反馈/候选/grammar/合法模板/成员仓序/已解决事实均进入UTF-8输入预算与prompt digest。

### D4. 系统门前置，compile冻结复验

author原输出保存evidence，适用全套件通过后才提升source/IR/report/current artifact、grant Reviewing；reviewer只读同代结构报告与scope，Pass不能盖系统MustFix。Final Compile重验冻结refs与当前合法context，不重lower最新artifact偷换发布物；prepare前无transaction时本次close直接携typed failure_id/approval/source context并durable diagnostic，禁止捞最近Failed transaction。compile类型、close、WS/REST共用本次typed对象，stable code保持；后续映射不是先Display再parse。晚发现缺口且无活动授权则工程故障/保合法版本，无新turn/自动confirm/continue改author。

### D5. durable grant、双预算与review守卫

engine判定/CAS，现handler驱provider。原turn.source_hash仍授权基线，grant单独绑定中间/失败candidate/context/prompt；attempt1原调用、attempt2为L0/L1共享唯一机会。用户最多3轮hold，deliver refs/current CAS成功settle一次；Reviewing仍Running/gate_busy，settled复评失败不退。未deliver结构耗尽release；取消/provider/timeout等精确nonstructural_failure_charge不退；receipt互斥幂等、旧记录不追溯。budget_reserved=1仍原预约身份，accepted不是charge，reservation不是actual start，unknown不自动重领。

review/routing在旧policy/repair reservation前同grant分流，routing_scope与dispatch二次guard，终态tombstone拒旧single_candidate_author key旁路。恢复按phase：有输出assess/commit、Reviewing只review、Attempt2Claimed同key、unknown等待；合法中间candidate不同原hash不能误漂移，见artifact不能误Completed。source/turn/session多JSON按durable证据防裂缝，不承诺多文件原子事务。

### D6. checkpoint唯一纯状态豁免

session structure_checkpoints存version/source node/三refs/hash/context/authorization/receipt/review scope/status，grant结束保留；旧ArtifactVersion没有三refs关联，禁止从版本号补IR/report。当前上下文重验且同业务请求完整证明、无发布副作用时整组回绑，CAS失效approval/approved_at/compile reservation/publication provenance并审计；不混新旧refs、不新计receipt、不自动approve。审核不能绑定则原复评。新业务未满足只能author融合旧结构。resolved predicates是合法性护栏不是永久冻结边/target；R3typed回归在current提升前阻断，指令带差异/旧refs，context变更重验，重复指令不循环。

### D7. anchor/trust冻结与逐载体预算

完整gateway admission/full fingerprint/readonly trust仍每次真实启动必经；mismatch零adapter start/child，probe单列。BlockedByLcRecovery优先；在同sessionCAS冻结recovery×turn×grant×attempt×prompt/ref/digest join，显式fresh仍同子调用且resume None，不补map、不silentfresh、不重置预算。无活动结构state时原fresh行为不改。初轮RepairOnce、普通artifact一次（provider资格不足为1）、legacy总3、draft/batch RepairOnce只并入原机会。coding保一次teaching、每cycle3；code-review在原cycle剩余机会纳L1，不变transport；group-review保原transport3与一次repair/16KiB及fidelity，各机会关联同durable父身份，不新结构池。语义finding/plan defect/权限等待不变成结构许可，review输出协议坏修原reviewer不驱coder。

### D8. 同源呈现及零出站建议

后端typed failure/current structure state/receipt→wire→live/rebuild/observer/cockpit一源；四泄漏面同failure_id去重。主区不刷attempt，中间系统处理/已解决不红错，不“需你修结构”；最终故障不隐未交付/已交付与未发布事实。机器建议独立于advisory，采纳仅props预填当前合法editor、去重保草稿focus零网络；自动链不依赖点击。保现terminal command replay拒收与replayableGateCommandId守卫，不写第二权威。UI复用MASTER中性白卡、键盘/44px/4.5:1/reduced-motion、polite/终点一次alert。

## Risks / Trade-offs

- 推荐机器指令→author→复验，相比直接patch多一次有界调用，但保持单作者provenance/批准refs；直接patch拒绝，用户必需采纳也拒绝。
- 局部唯一证明保守、拓扑多解仍author；不做指数搜索，不将未找到解冒称业务矛盾。两attempt可能无法完成三种连续错误，正确停工程故障而非偷开第三次。
- 最小退款不是所有未交付免费，UI必须分交付与非结构失败扣次；统一未交付release备选未采用。
- 旧记录没有完整join/actual launch时unknown，不追溯补造；d4c4标识未读原件，仅fixture与运行核验可证明回退。
- broad闭包采用≤4h独占子包、共享文件串行移交；Plan双审/owner审阅前不实施，controller统一红绿/最终验证，不跑半成品并行检查。

## Migration Plan

### 来源组合次序与逐标题覆盖

1. 现主规格作为基础；先sync `lc-root-initialization` 的REG/ENV/PLN来源，再sync `lc-gateway-multi-provider` 的gateway、ENV/REG/PLN来源。
2. 随后叠 `fix-admission-member-rules-scope` 对REG-12的收窄、`fix-provider-trust-confirm-register` 的REG-15（引用root REG-14，不新造来源）；这两来源的只读/确认授权约束保留。
3. 再sync较新 `fix-planning-session-anchor-recovery`：PLN-05/07/08以其最终native/business分域和显式fresh为准，gateway同真实标题的full admission/fingerprint为准，恢复delta为新权威。本change只消费，不改源delta。
4. 最后本change：PLN仅ADDED09/10，不覆盖anchor MODIFIED标题；CG02/03/04/05以本完整MODIFIED块替换旧预算/结构拒收条文，其他原场景保留；coding-provider首真实标题仅L1豁免覆盖，后四条原合同不动；新SG能力直接ADDED。
5. 如anchor先归档，先读最新主规格再逐标题重核；禁止last-write-wins或从旧PLN-03自动fresh恢复不安全合同。本次不sync/archive或更改主spec、trust/anchor来源。

### 实施与发布

先完成v1.1尚未实施typed基础F1/F2，再SG1–SG11按Plan；不建立旧B三reservation槽中间版本，所有基础直接采用本新双预算。编译typed cutover、复评守卫、模型/DTO literal迁移以同提交完整调用闭包为准。共享写面等待trust/anchor ledger任务＋commit＋独立审查＋controller移交，不从checkbox、agent空闲或代码看似已改推定结束。每包红绿→独立审查→独立commit；最终三连撞/真provider零启动负例/各stage/四泄漏面由controller一轮收口。未接齐所有授权guard前不发布可自动L1能力。

### 2026-10-13 Plan v1.1接地闭包修订（不重裁行为）

oracle ADOPT零漂移、接地GROUND_BLOCKED的P1×1/P2×3由Main授权修入Plan v1.1；v1.0保历史，不重跑oracle。F1.2保utils旧签名/可选Unknown兼容，F2b独占显式kind工厂和全caller清切。SG3a拆0/1：既有immutable mechanical-report外层Option结构报告（含source/IR/context/rule/digest与来源边/audit）及全record literal同提交兼容；实际SC reviewer builder核同代join、注入三可信section且保原96KiB预算/原review计费，旧context拒发。SG7同提交切可选machine_repair与全ReviewFinding literal，Raw输入仍封闭；SG7a独占types/from_record、CAS镜像、reload、SessionState实际生产builder及fixture五态测试，前端SG8.1与普通SG10硬等该真实链/移交。所有共享写面仍trust/anchor A/B/C最终移交；这只是补接口/owner，不改变双预算、退款、五阶段范围、单作者/三refs/Task9c/confirm。


## 交付与未验证事项

本change只规划，当前实施Plan为 `cadence/plans/2026-10-13_实施计划_确定性结构治理_v1.1.md`，v1.0保留历史。接口/字段/测试仍拟新增；只运行OpenSpec strict及规划路径/同号检查，不运行产品测试/build/lint/formatter、不commit。接地发现修订后由Main仅定向复核P1生产链；oracle既有ADOPT按Main裁定不重跑，owner审阅门仍在，strict不代表产品或定向复核已通过。
