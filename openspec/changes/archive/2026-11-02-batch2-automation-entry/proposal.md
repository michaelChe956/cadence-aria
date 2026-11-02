# Proposal: batch2-automation-entry

## Why

owner 于 2026-10-15/16 裁定批 2 范围（phase-ledger 12:50/13:05/13:10/14:00/14:40/15:00/15:20 及 2026-10-16 04:20/05:00）：批 2 目标＝创建 Work Item Plan 选「自动化」后，plan 生成、批准、attempt 创建、coding 启动全部自动，进入批 1 已交付的自动 coding 主链；story/design 仍由人完成，最后仍保留批 1 的人工「确认完成」。批 2 六个行为面（总览 §1/§2）：

1. **多仓授权与自动启动**：授权覆盖 issue explicit selection 全部有效成员仓，不逐仓勾选；实际 attempt 只按有 WI 的仓桶创建（P⊆A），不能给无 WI 授权仓造空组；块 2 批准后自动 advance→冻结各仓 AutoStartOnce→同 plan-run 全图驱动，上游完成自动开下游。
2. **plan 自动批准**：enrollment 冻结 `plan_approval=auto` 并把 `require_execution_plan_confirm` 压 false；正常 outline/draft/batch/最终批准自动通过（AutoIfValid）；AI 主动提问（choice）与人工门仍停等并通知。
3. **失败继续不转手动**：失败停原点＋平台通知，处理后继续自动化——plan 阶段在对话窗口补说明点继续，coder 阶段在门内补说明（复用批 1 A4）；无暂停概念，不转手动、不丢自动化模式。
4. **plan 级终止**：终止两阶段可用、二次确认、整个 plan 全部 WI 停止且 attempt 终止不可恢复；terminated 的 enrollment 仍阻断全部自动路径，但可由人工显式 Enable 取代重新开启（2026-10-23 owner 裁定）；所有 Abort 入口（门/REST/WS/Inbox）在自动化绑定 plan 上同一终止语义；auto 单仓同样覆盖，手动单 attempt 维持。
5. **通知接入**：复用批 1 A1 durable 事实投影，追加 `plan_question/automation_failed/review_limit` 三类 kind 与显式事实写者；plan 用 workspace_session 目标，不建第二通知模型。
6. **整链验收**：新 issue→design 确认→选自动化→全程不手工→批 1 唯一最终按钮；四仓真实 provider 整链与终止负例由统一验收执行。

owner 裁定摘要（phase-ledger）：多仓授权＝A 全部成员（15:00）；失败接管＝A 接着自动化、两类失败都不转手动（15:00）；plan 对话窗口与 coder 门两通道不得混用（15:20）；自动批准不受弹窗旧确认复选框约束、AI 提问/开门才叫人（14:00/14:40）；终止＝整 plan 不可恢复、任意 WI 终止即全部、二次确认（13:10）；**2026-10-23 裁定补充：plan/attempt 终止不可恢复，但 enrollment 的 terminated 可由人工显式 Enable 取代重新开启（同 `enrollment_id` 新 revision、清空 `terminated_at`、命令账本留痕、授权 payload 不得顺便改写）；一切自动路径仍不得复活/自动重开**；无暂停（13:05）；预算耗尽＝A 每次显式补新说明只开一个新回合（04:20）；双审五冲突全部按推荐（05:00：C1 服务端拒接管、C2 Abort 统一 plan 终止、C3 terminated_at 纯字段前移、C4 包显式写事实、C5 终止关全部 open 门）。设计经双审修订并 oracle 复审 RESOLVED（phase-ledger 2026-10-16）。

设计依据（本 change 不复制其内容，只引用）：`cadence/designs/2026-10-15_方案设计_批量2_自动化入口_v1.0.md` 及块 1~3 设计文档。

## What Changes

- **多仓授权（块 1）**：`EnrollmentTarget` 新增 `logical_codebase_members` 成员集合变体（旧 wire 保留，单元素折叠旧 kind，原始 JSON/磁盘统一规范化：UUID 升序去重非空）；GET/PUT/rebind 与七消费点同包切换；授权 A 与 plan 实际桶 P 满足 ∅≠P⊆A 且无未归属 WI；advance/start 准入以集合互证替代「恰一桶」；split 传递并冻结 `start_run_policy`（旧 record 默认 Manual 不升格）；LC 自动编排改调 plan_start 提取的共享服务，首启与续驱沿 `deterministic_start_command_id`，全部自动桶（含非首桶）Enrolled origin，失效零 provider、禁止降级 Manual。
- **自动批准（块 2）**：`EnrollmentOptions` 加 `plan_approval`（缺省 manual）；binding/rebind 只放行与当前 enabled 精确绑定且冻结策略一致的 AutoIfValid；bound 创建/准入/确认投影/trust 准备六条接缝贯通；自动 compile 前复用 `ensure_confirmation_trust_ready`，非 Ready 落 typed waiting 停点＋通知；`finalizer` WI 确认快照 auto 压 false（manual 原值保留）。
- **失败继续（块 2/3）**：新 `POST …/automation-enrollment/continue-plan`（plan 对话窗口：Failed 重生成、评审超限同 session 原地重开 HumanConfirm＋说明、既有 choice/门照旧）与 `POST …/automation-enrollment/continue-chain`（advance/coding_start 停点第三类显式继续，无说明文本，重放原 claim）；`GET …/automation-enrollment/control` 只读管理投影；精确绑定 enabled enrollment 的会话服务端一律拒绝 takeover（既有码 `workspace_session_takeover_not_allowed`）＋收件箱隐藏。
- **plan 级终止（块 3）**：enrollment 加 `terminated_at` 终态标记（写入即关闭授权，仅人工重新开启可清空）；新 `POST …/automation-enrollment/terminate`（`confirmed:true` 统一二次确认）；终止顺序＝持久关闸→禁新 run/失效 plan-run→取消 run 等 completion→取消全部 runner（内部 AbortAttempt 仅干净退出，不走旧 handle_abort）→专用收口关全部 open blocked/choice/stage 门（Completed 保留/其余 Aborted、保留文件）→清三类旧事实；旧 abort REST/gate-responses REST/WS/门 Abort 全部路由同一终止，未确认返回 `automation_plan_termination_required` 零副作用；迟到 Failed/Open 不覆盖 Terminated；terminated 下 rebind/继续/新首启与一切自动路径拒绝且零副作用，人工 Enable 允许以同 `enrollment_id` 新 revision 取代重新开启（清空 `terminated_at`、解绑旧 plan/session、命令账本留痕；授权 payload 变更仍 Conflict、`stage=termination` 清理失败事实未收口时 409 `automation_plan_cleanup_required`）。
- **通知接入（块 3）**：enrollment 持久 `attention_facts`（N0 纯模型/store）；写者固定——P3 trust waiting/compile 恢复门 `plan_question`、P4 其余 plan 三类（含 `plan_generation` NeedsHuman 派发前失败 `automation_failed`）、R2 advance/coding_start、T3 终止清写；新 `automation_attention_info.rs` 只读 mapper 合并进 lifecycle `coding_attention_items`（N1），答后/run 结束清失效 choice；稳定 key、durable time、写失败上抛。
- **整链验收（E1 并入 R4）**：夹具扩四仓，零人工正例与故障/终止用例分开；terminated＋部分 Completed 不可最终确认（A2 不改）；手动单/多仓回归不变。

## Capabilities

### New Capabilities

无；全部新增行为并入既有 capability，不拆新 capability。

### Modified Capabilities

- `work-item-group-autopilot`：REQ-WIGA-01「多 target 不自动纳入」改为全集合授权、P⊆A 实际桶、Enrolled 首启（MODIFIED）；REQ-WIGA-02 Interactive-only/禁 AutoIfValid 改为冻结 `plan_approval`→AutoIfValid 正常自动批准、精确绑定校验（MODIFIED）；REQ-WIGA-03 单 target 推进改为授权集合、共享 plan-run 全图驱动（MODIFIED）；REQ-WIGA-06 范围扩为授权集合（MODIFIED）；REQ-WIGA-08 managed/terminated 投影、无接管转手动、失败通知同源（MODIFIED）；REQ-WIGA-C5-CHAIN 删去正常人工批准停点、扩全成员链与最终唯一按钮（MODIFIED）；新增整 plan 终止与统一 Abort 语义（REQ-WIGA-09，含 terminated 人工重新开启契约）、plan 阶段失败继续（REQ-WIGA-10）、链停点继续（REQ-WIGA-11）、plan 通知事实与接入（REQ-WIGA-12）、整链验收不变量（REQ-WIGA-13）。
- `multi-target-group-coding`：REQ-MTG-03「不自动跨 target 启动」改为精确授权 AutoStartOnce/Enrolled 下共享 plan-run 驱动；手动仍显式（MODIFIED）。

## 规格修订基线（原 delta 头部，archive 不携带故记于此）

- `work-item-group-autopilot`：基线 `openspec/specs/work-item-group-autopilot/spec.md`（REQ-WIGA-01~08、REQ-WIGA-C5-PREFLIGHT、REQ-WIGA-C5-CHAIN）。本 delta 修订 REQ-WIGA-01/02/03/06/08/C5-CHAIN 六项（owner 2026-10-16 05:00 双审裁决与 phase-ledger 批 2 裁定），新增 REQ-WIGA-09~13 五项；REQ-WIGA-04/05/07、REQ-WIGA-C5-PREFLIGHT 不变。规格修订登记依据总览 §6 规格修订完整登记。**2026-10-23 owner 裁定修订**：REQ-WIGA-09 的「terminated 不可恢复」限定为 plan/attempt 层——terminated 仍阻断全部自动路径，但 SHALL 可由人工显式 Enable 以同 `enrollment_id` 新 revision 取代重新开启（清空 `terminated_at`＋账本留痕）；同步修订 REQ-WIGA-02「终止后自动重开被拒、人工重新开启受控」场景。
- `multi-target-group-coding`：基线 `openspec/specs/multi-target-group-coding/spec.md` REQ-MTG-03（及 REQ-MTG-01/02/04/05 不变；REQ-MTG-06~13 归属见各 change，编号不重叠）。本 delta 按 owner 2026-10-15 15:00（多仓授权＝A）与总览 §6 规格修订登记，把 REQ-MTG-03 的「一期无自动跨 target 启动」修订为：仅 `work-item-group-autopilot` 精确授权集合（AutoStartOnce/Enrolled）下允许共享 plan-run 驱动自动启动全部实际桶；手动与无授权场景仍为人工显式。配套修订见 `work-item-group-autopilot` delta（REQ-WIGA-01/02/03/06/08/C5-CHAIN 与新增 REQ-WIGA-09~13）。

## Non-Goals

owner 已裁定的不做清单（原文依据总览 §1/§6 与三块设计，批 2 全部不做）：

1. provider 启动前检查不做：owner Q3 裁定放批 3，不在批 2（phase-ledger 12:50；总览 §1「provider启动前检查、story/design自动化不做」）。
2. story/design 自动化不做：起点＝新 issue 的 story/design 由人完成、design 已确认（总览 §1）；源 story/design 人工流程及 Final Confirm 保持人工。
3. 无暂停、不转手动：「无暂停，不转手动」（总览 §1；owner 13:05「批 2 没有『暂停自动化』这个概念」）；自动化运行中不提供用户主动暂停或手动改某步的入口；两类失败都不是转手动、不丢自动化模式（owner 15:00 A）。
4. 不逐仓勾选、不造逐仓授权/启动 API：授权覆盖全部有效成员，弹窗不逐仓勾选（owner 15:00 A）；「REST不增路由……不造逐仓授权/启动API」（块 1 §4）。
5. 无 WI 授权仓不造空组：「不能给无WI授权仓造空组」「A−P不造空组」（总览 §1；块 1 §2）。
6. 旧 target/Manual 轮不热升级：存量扩仓迁移不自动代办，旧 Manual 不补 AutoStartOnce，结束旧轮后用既有重置流程显式新建（块 1 §6.1；开放假设②）。
7. 跨域原子事务/exactly-once 不做：继续命令跨 enrollment/session/intent 部分写入显式停等，不建事务框架（块 2 §5/A-03）；「完整跨域原子事务不在主流程范围」。
8. 终止收口不做自动文件恢复：Git/锁清理异常只记 `cleanup_required` 仍全停禁恢复、保留现场（块 3 A-02；开放假设④）。
9. 重启丢 choice 不加恢复按钮：失效事实清理后通知显式重做，不纳入批 1 已否决的恢复 choice 按钮（块 3 A-03；开放假设⑤）。
10. 不建第二通知模型：复用 A1 durable 事实投影与 DTO/target，「不另建模型、不伪造attempt」（契约 A1）；不加 `focus.session_id`、不加通知页/已读/离线推送。
11. 批 1 已合入接口不重做：A1/A2/A4/A5「上述接口不重做」（总览 §2），handler/组件不改、只复用与测试；coder 阶段失败继续复用 A4 门，不与 plan 对话窗口混用（owner 15:20）。
12. 不代点最终确认：自动链只到 ready，不自动代点批 1 人工「确认完成」；terminated plan 不可出现可点确认（契约 B2-3）。

## Impact

- 模型与存储（`src/product`）：`models/automation.rs`（approval 选项、terminated_at、AutomationAttentionFact）、`issue_automation_store.rs`（集合规范化、事实 upsert/resolve、终止 CAS）、`logical_codebase/types.rs`（成员集合变体）、`advance_store.rs`/`workspace_engine/advance.rs`（start_run_policy 冻结）、`coding_plan_run.rs`（plan 级失效）、`coding_attempt_store/{admission,gate}.rs`（terminated 准入、关门）、`lifecycle_store/workspace{,_single_candidate}.rs`（AutoIfValid bound、takeover 拒绝、原地重开）、`workspace_engine`（compile trust 接线、conversational gate、session timeline 防覆盖）、`coding_workspace_engine/handoffs.rs`（专用收口）。
- Web 服务端（`src/web`）：automation_enrollment/target/gateway_preflight、plan_preparation、plan_generation、plan_confirmed_info、advance_plan、coding_start、plan_start 共享服务、autopilot_orchestrator、workspace_sc_recovery（continue-plan/control/terminate）、workspace_session manager/router/choices、coding_ws_handler（Abort 路由/干净退出）、新 `automation_attention_info.rs`。
- 前端（`web/src`）：WorkItemPlanOptionsDialog/useIssueLifecycleGeneration（auto 入口）、lifecycle.ts 集合 target 与 approval 类型、cockpit projection/Inbox（管理投影、隐藏接管）、ChatCockpitPage/ChatInputBar（恢复输入/继续）、coding-workspace 页面/控件/store/WS hook（统一终止二次确认、隐藏局部 abort）。
- 兼容性：全部新字段 additive（serde default：旧 JSON `plan_approval=manual`、`attention_facts=[]`、`terminated_at=None`、旧 advance record 默认 Manual）；旧 wire/单成员折叠保留；手动单/多仓与批 1 行为零回归。
- 批 1 接口：契约 A1~A6 全部复用不重做；A2 plan 级确认 handler/组件不改，仅负例验证。
- 实施口径：16 编号包（E1 并入 R4，实际派发 15）× 6 波，全部 glm53-task；TDD 先红后绿；同波仅无共享文件并行；统一验收命令与真实四仓整链验收见实施计划。
