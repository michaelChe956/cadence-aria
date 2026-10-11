# Design: fix-planning-session-anchor-recovery

## Context

动机/范围见proposal；架构全文以owner已确认`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/cadence/designs/2026-10-11_方案设计_规划阶段会话锚点固定与漂移恢复闭环_v1.1.md`（v1.1，以下“获批设计”）为准，本文件持久化该方案的实施边界与权衡，不把双审发现留给worker猜。双审：OracleAnchorFix C1–C3、GroundAnchorFix B1–B4及五建议已注入；OracleAnchorFixR2=ADOPT、无未闭合项；owner随后确认v1.1。

真实基线：workspace_repository返回值既供native launch，也供SC language.md/仓结构/IR target；manager durable fallback取出生旧record；完整projection在adapter start阶段才造且trust域空；Plan stale自动rebuild、manual turn另有durable身份。实际落点及原文核验见获批设计§2/§5.3，不把opaque gateway状态或原native指纹从当前环境倒造。

## Goals / Non-Goals

目标：E固定native锚＋A–D恢复闭环同时成立；同resolver分离business/compile；fullmap/tombstone/operation durable；latestrecord纯投影/typed失败；全Plan运行族原上下文、SC同turn恢复；60秒TTL版本观测仅压probe爆发。

边界：Coding/direct、trust登记、确认CAS、SCgate/turn/预算/compile合同不变。raw/unsupported/未知run不fresh fallback；pure snapshot不恢复写；不宣称OS隔离、跨JSON事务或外部非协作写方强原子保护。

## Decisions

### D1. 一个resolver，两种用途（获批设计§3；接地B1）

在现workspace_repository核心读同manifest/selection/实体，产native_launch_anchor与business_repository。旧workspace_repository_for_session保留business语义，nativecaller选root输出；原业务helper仍用，不能按v1.0删。业务成员language/IR physical ID不改，aggregate view不写IR。公共selection失效/focus校验前移，native-only合法草稿不因business选仓歧义阻断；业务confirm/compile仍守原gate。

选此而非全局RepositoryRecord改root：后者把SC材料迁到root且IR work_item_target_missing。两个字段getter不等于第二resolver，不能复制选仓算法。manager repository_path仍业务路径，LC envelope/root独立冻结。

### D2. durable安全事实与完整指纹（获批设计§4.1–4.3）

WorkspaceSessionRecord三个默认字段：full fingerprint记录（provider/role/run/native/snapshot binding）、多代superseded tombstones、LcResumeRecoveryRecord（tagged run_context、waiting/reserved/running/resolved及fresh_operations）。spawn前prepared持久、handshake/同run audit后active绑定；active旧指纹只比不改。缺map/旧basis/未知历史ID进入waiting，owner批准一次升级显式fresh、不回填。

先保存拒旧/拒旁路的record，再幂等终态化旧provider节点，再snapshot；跨JSON不承诺原子事务。SC业务turn/gate/预算保留，mismatch分类独立于genericFailed/rearm。direct不加LC门。A/B共同发布或A保持后端不可启动，不能只allowed_actions=[]掩旁路。

### D3. 最新record纯投影（获批设计§4.4；接地B2）

A前置manager durable_projection每次session_id读最新record并核manager绑定身份；Result错误贯穿router/attachment/socket，读失败ProtocolError workspace_durable_projection_unavailable，不旧record/defaultOpen/null假态。共用持久加载核心ProjectionOnly模式：无journal/turn/timeline恢复写入，无run/spawn；live mismatch/真实recover_on_creation才reconcile，优先阻住关联turn/outline自动接力。driver/observer同SessionState/event_seq，持engine锁也立即durable广播/attach。

不采用缓存manager.record作为正文，也不让一次性投影器变第二写源。缺基线可degraded/resync，不能拿旧可启动snapshot补。

### D4. 显式fresh与Replay（获批设计§5.1；接地B4）

StartGeneration增optional resume_recovery{recovery_id,intent:fresh,operation_id}；已有出生/directNone兼容。固定顺序auth/lease/scope→recovery身份/已受理operation→同IDtypedReplay（零spawn）→新ID才核waiting/无活run/非终态/配置/context/hash并CASclaim。普通stage守卫不能先挡running/resolvedReplay，恢复Replay直接durable读取不等engine锁；新dispatch仍engine/CAS/run-registry取权。

typed LcResumeRecoveryOperationResult（accepted/replayed、operation_state、必要reason/node）及SessionState.last_operation共同对账；prepare失败回waiting不删ID、resolved同ID回放，旧recovery替换后Stale。running/未知不自动新ID重发。前端仅waiting/reserved/running卡，resolved消卡恢复后续正常动作。

### D5. fullcandidate/readonlytrust与版本TTL（获批设计§5.4）

B0a1先只读trustfacts/digest，再B0a2公共asyncfacts/opaque candidate。真实adapter供当前有效exact-version、provider MCP/boundary/approval与纯projector，gateway私有装配/admission/既有_with_projection，spawn前同facts重验/audit，不拿空version注册projector或constantbasis冒full。defaultunsupported仅非目标/测试adapter，真实四家与LCfake正例必须可观察facts；Pi extension不写于mismatch前。

owner2026-10-11允许有界只读--version，不算会话进程，LC_VERSION_PROBE_TTL=60s（成功完成单调时刻，age<60有效、>=60过期，非滑动、进程内）。key隔离provider/canonical executable identity/参数/supplier域；samekeymiss/到期single-flight一条，differentkey独立；失败/超时/取消不缓存、不用过期成功。缓存仅version，不trust/policy/projection/admission；direct无过期helper语义原样。版本变化可最多60秒滞后，这是owner接受权衡，TTL变长会更钝，调整须新裁定。

三计数分别记录probe/adapter session-start/session child；mismatch后两项0，已spawn握手错IDkill/reap，不伪零child。Codex整config/Kimirootrecorddigest范围原样，不ensure/revoke或写trustaudit。

### D6. Plan全运行族与原身份（获批设计§5.3；接地B3）

RecoveryRunContext明确列LegacyAuthor、OutlineRevision、带origin的Rebuild、Draft、Batch、PlanRevision、SCauthor、HumanGateScManualRevision(turn_id)；feedback在.take前持久，source/artifact/ref/hash/businessidentity复验。不序列化runtimeProviderRunKind/Arc或只存prompt冒权威。fresh全输入旧IDNone、fulloutline包含原稿/feedback/review（现Rebuild出生full不足），Draft/Batch同generation/outline/batch，PlanRevisionretained/redo同原候选；SCauthor同reservation/phase而物理native新run，manual同turn/command/source_hash/attempt/budget/gate完成器。

provider_run/include/followups/single_candidate/prompts实际caller串行接线；manual active recovery阻止自动ResumeSameTurn/increment/spawn。未知tag拒context_unsupported，不fallbackplan_author/legacy；ReviewOnly、Coding/recipe/init/缺native同步split原合同不变。

### D7. 前端与最终能力门（获批设计§6）

driver和observer全量recovery/lastoperation映射，typed结果不从Error推权，null/reset收口。Inbox明确新native、保留workspace，Pagelease/connection/provider/终态守卫、不bulkfresh。resolved卡消失且generic后续动作可用。A/B全部真实coveredhandler接齐才allowedStartGeneration；C/D不能先宣传fresh。

## Risks / Trade-offs

- [升级代价] 旧map无可靠迁移材料 → owner已批准一次waiting＋显式fresh，不特殊回填，不静默更换native。
- [TTL] 当前version有效观测最多60秒滞后 → 固定owner参数、samekeysingle-flight、身份变化失效，clock/barrier边界测试；其它域不缓存。
- [TOCTOU] home/config外部非协作写者不遵Aria锁 → 只读重验/原锁合同failclosed，不能宣称消除全部外部窗口。
- [跨文件断点] session/timeline/turn/audit非事务 → durable拒旧先行、真实入口幂等reconcile、unknown不自动replay，故障注入证明不并发spawn。
- [工程体积] full四provider＋全Plancaller扩大B波 → 预拆<=4h，B等trustT7，不承诺后天，流程/验收不裁。

## Migration Plan

E∥A→B0a1/a2→四provider并行→B0f→B1/B2a–d∥C→D。严格同文件串行：types/lifecycle等trustT4，PageT5.2，SC/provider_run/gateway等T6.1并最终T7整体移交，provider_trust/adapters等T7。controller每个依赖波次brief附SDDledger任务号＋commit证据，worker不从源码状态自判完成。

实施需要本change及获批Plan；每包TDD红绿/独立commit/独立review，Main统一最终验证与真实owner链。中间无falsefresh能力；回退不自动删除用户workspace/产物/audit/trust，旧native资格不能通过回退复活。完成后核未归档源spec次序再sync/archive。

## 设计确认与双审映射

| 裁决 | 本change合同 | 工作包 |
|---|---|---|
| E与businesstarget分离/B1 | PLN08/PLN07、D1 | 1.1/E、B1nativecaller、B2b/c/d |
| durablemap/等待/升级代价 | recovery“持久superseded等待”“原生冻结记录”，D2 | A1/A2、B1 |
| 最新record/observer/B2 | recovery“最新耐久事实的纯投影”，D3 | A1b、C1 |
| Replay/B4 | recovery“耐久operation Replay与typed结果”，D4 | A2store/B1b/C2 |
| fullprojection/60s版本owner裁决 | gateway真实标题＋版本ADDED，D5 | B0a1/a2/b–f |
| Planrun/SCturn/B3 | recovery“Plan原运行上下文与SCturn保留”，D6 | B2a–d |
| C1–C3措辞修正 | gatewayPlanningReadOnly具体化、AC2、readonlytrust源场景 | 全包映射/终验 |

暂无会改变方向/验收的开放问题；exactRust字段/签名命名、测试seam由实施Plan冻结，不能让worker重裁已确认行为。历史现场WS缺口不要求再重现用户失败；新链留证。
