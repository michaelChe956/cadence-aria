# Design

## Context

本 change 只落方案 v1.2 §5.2 C2 的契约边界，动机与范围见 `proposal.md`；specs delta（两个新 capability ＋ 七个 modified capability）已冻结行为契约，本设计说明落点与防护。现有系统已具备 manager／runner 事件路径、attempt claim 与工作树文件锁、UnitRun 提交区间存储、rework instruction／context note 写面、coding 门六动作、role provider 快照、SC 人工返修 prompt 构造、LC evidence token／mediator 与驾驶舱只读投影；C1（`enrollment-recovery-surface`，432e3c5a／6edd0910）已落地租约三态判定与确认接管服务、CandidateRecovery variant 恢复模式、C1WaitingItem 驾驶舱投影、retry-initialization 状态机与命令账本幂等模式，全部作为本 change 的直接复用基础。

上位追溯：

- §3.1 定义“错误→通知→用户操作→原链继续”闭环；§3.2 只保留“同一时刻至多一个编码进程”这一最小并发约束；§3.3 要求展示身份／副作用并对未知状态 fail-closed。本设计全文贯穿“自动推进＋人到点处理”口径：coding 链自动推进，遇断连、冲突、配置缺失、验证不可满足时转可操作停等，人处理后续进，不做无人值守自愈。
- §4.2 的 #2／#3／#4／#5／#7／#8／#9 文案与 §4.3 的 #11 coding 侧／#13／#15／#17／#18／#19 是本 change 的故障面；§4.4 BYPASS-17／18／19 产品化。
- §5.2 是 C2 实施合同；§5.5 共享边界要求 C2 统一 manager／runner／attempt claim 的最小互斥与产物落盘、复用 C4 resolver 而不另猜路径；§7 定义 5–8 人日、验收 A06／A08／A09 coding 部分／A10 reviewer 部分／A11／A14／A15。
- §8 明确不选全局 owner／fence、统一 context evolution、服务端通用命令执行器；§9 状态为方案完成待实施。

## Goals / Non-Goals

**Goals:**

- 断连只影响观察：provider 已完成的 source／artifact／completion checkpoint 先写既有持久面再发观察事件，完成状态不明时停等用户确认。
- 同一 attempt／共享工作树至多一个编码进程：启动／重试／继续／restart 在原子临界区内重读租约三态，活跃拒绝、已死经用户确认接管、未知停等；同 `command_id` 幂等。
- abort 后同进程 restart 可用、人工继续后回到原编排阶段且不重跑已完成 review；旧 run 迟到回执被拒。
- reviewer 三值（effective／provisional／enabled）持久化与重建，根除 author／Codex 回填。
- execution 提交区间按认领前真实 HEAD 冻结，零提交＝空区间，完成时不按基线回填。
- 返修指令／context note 的“渲染完整 prompt→认领→消费→spawn”合并为一次可重放写入，实际 prompt 必含新指令。
- 验证类停等产品化：计划命令与实际命令并列证据、重跑原计划命令、独立验证处理／受限豁免（绑 finding／check／revision／scope／expiry），#2／#9 文案区分。
- blocked／rework 页面受限政策读取（消费 C4 resolver），SC 人工返修完整预算与完整有序传输。

**Non-Goals:**

- 不实现 durable runner owner、run incarnation、worktree fence／epoch、双主检测或第二套 durable operation 状态机（v1.2 §5.2 非目标）。
- 不扩展 `coding_output_human_triage` 两动作与 Code Review 分诊门四动作；不向原门塞 plan repair；不自动批准 FinalConfirm、危险命令或验证豁免。
- 不清 findings、不改旧 hash／旧 execution／旧 artifact／旧门证据；不以改 JSON／consumed／note 作为恢复手段。
- 不新增服务端通用命令执行器；不负责 #9 计划权限语义（C1）、LC authority resolver 本身（C4）、单仓 role-chain 预检（C5）。

## Decisions

### 1. 断连产物先持久化后观察（REQ-CRO-01，#3／#8）

**改什么：** manager／runner 的完成收尾路径（`src/web/workspace_session/{manager/mod.rs,router.rs}`、`src/web/workspace_ws_handler/run/provider_run/entry.rs`、`src/web/coding_ws_handler/{runner.rs,socket/resumption.rs}`）统一顺序：provider 运行输出、artifact、source、completion checkpoint 先经既有 store 写面落盘，再 `event_tx.send` 观察事件；attach／detach／supersede 路径保持 `spawn_provider_run_claiming_idle` 的“只观察不取消”语义，用户显式路径的 supersede 不改变业务结果归属。`src/web/coding_ws_handler/outbound.rs` 的 write-settlement（confirm／fail）模式扩展到返修完成事件：投递失败只标记未送达，不回滚业务事实。

**为什么根治：** `provider_run/entry.rs:33-37` 自述同节点去重不命中时无条件 supersede 会取消在途 run token（杀死握手中的 reviewer 会话）；产物收尾目前依赖连接生命周期，断连窗口内已完成的返修修订会随 supersede 丢失（#8）。

**防护：** 完成信号与持久化之间中断（无法证明 provider 是否完成）时，落地“完成状态待确认”等待项——展示最后活动时间与已知证据，用户确认前不启动新 provider 运行（MUST NOT 自动重启）。

### 2. 最小编码互斥与接管判别（REQ-CRO-02，#3／#11 coding 侧）

**改什么：** coding 启动入口（`execute_coding` 前的 admission，经 `coding_attempt_store/gate.rs:39 ensure_provider_run_allowed` 所在临界区）在原子临界区内重读工作树租约与 attempt claim：任何启动／重试／继续／restart 请求先调用租约三态判定，再决定放行、拒绝、呈现接管或停等。第二次启动对活跃持有者返回“已在运行／请等待”并通知。

**复用什么：** 直接复用 C1 已落地的 `CodingWorkspaceEngine::classify_worktree_lease`（`gates.rs:1052`，活跃→ActiveWait／终态或锁释放→DeadNeedsTakeover／证据缺失或读失败→UnknownNeedsHuman）与 `confirm_takeover`（`gates.rs:1213`，命令账本幂等→binding 校验→三态复核→文件锁内 owner CAS 清出，旧持有者迟到写入按旧身份拒绝）。

**防护：** 系统 MUST NOT 自动抢占活跃或未知持有者；接管确认前展示最后活动时间与节点证据（A09 误确认判据）；同 `command_id` 同负载返回首次 durable 结果（复用 C1 命令账本 `payload_digest` 模式），异负载 fail-closed；不引入 owner incarnation／epoch／fence。

### 3. abort 后显式 restart（REQ-CRO-03，#5）

**改什么：** `src/web/state/coding_run_registry.rs` 为 `retired_attempts`（`:50`）增加显式清除路径：restart 应用服务先确认该 attempt 无活跃编码进程（registry 无 runner＋租约非活跃），再原子移除退役标记并走既有终态重开 admission（`admit_and_transition_attempt_to_executable`）。restart 请求携带 attempt 身份与版本，迟到旧回执在 registry 既有 `retired_attempts` 检查（`:208/:262/:404`）下被拒。

**为什么根治：** 现状 `abort_attempt`（`:303`）插入 retired 集合后没有任何产品动作清除它，同进程 restart 被 `claim`／`run` 检查拒绝，只能靠重启服务清内存（#5）。

**防护：** restart 按钮仅在 Aborted／Failed 且无活跃进程时可见；“正在停止”中间态不可 restart。

### 4. 人工继续后回到原编排阶段（REQ-CRO-04＋coding-code-review-triage delta，#4）

**改什么：** `src/web/coding_ws_handler/runner.rs:282 should_resume_runner_after_gate_response` 的续跑白名单增加 `manual_continue`（及允许的质量绕过与验证处理结论）；门响应仍先写既有 gate 记录（`resolve_blocked_gate`），再唤回原 runner continuation。续跑后跳过已完成的 Code Reviewer／Internal Reviewer role run（复用已持久化结论）。

**为什么根治：** 现状白名单只含 retry 类动作，`manual_continue` 提交后 gate 关闭但 runner 不唤回（#4“断头”）；且续跑 MUST NOT 依赖提交动作的连接保持打开。

**防护：** 同 `command_id` 重复提交返回首次结果；唤回失败时落地可操作等待项，用户点击继续后从同一阶段续跑。

### 5. reviewer 配置三值完整性（REQ-CRO-05，#7）

**改什么：** 删除两处毒默认回填——`src/product/coding_models/provider_config.rs:67` 的 `unwrap_or_else(|| snapshot.author.clone())` 与 `src/product/lifecycle_store/plan.rs:273` 的 `unwrap_or(ProviderName::Codex)`；角色快照改为严格三值：effective（实际生效，可为空）／provisional（用户已选未生效）／enabled，消费 `workspace_session/manager/mod.rs:249-250` 已存在的 `provisional_reviewer_provider`／`reviewer_enabled_at_start` 字段并补齐 coding attempt 侧持久化与重建。缺失时落地“reviewer 配置缺失”等待项＋配置／重试操作。

**为什么根治：** disabled／null 在持久层被静默回填成 author／Codex，重建后误认有效配置并启动 reviewer（#7 毒默认）。

**防护：** reviewer 为空或 disabled 时 MUST NOT 回填、MUST NOT 视为自动通过；plan 会话 disabled 重启后仍 disabled。

### 6. execution 区间按事实派生（coder-owned-work-item-commit／coding-workspace-completion／group-final-review-triage deltas，#13）

**改什么：** ① `src/product/coding_attempt_store/unit_run.rs` 重试 execution 创建（`:240-270`，`prior.clone()`）重置 `start_commit=None`，不再继承旧值；② `src/product/coding_workspace_engine/coding.rs execute_coding_with_commands_outcome` 在 provider 认领前（`ensure_provider_run_allowed` 通过后、role run 创建前）记录当时真实 `HEAD` 为本 execution 的 `start_commit`；③ `src/product/coding_workspace_engine/group_completion.rs:87` 删除“execution_no==1 且缺失时按 base_branch HEAD 回填”（`backfill_coding_unit_run_start_commit` 调用），改为持久化起点缺失诊断并停等。最终 scope 由各 execution 真实区间聚合（组就绪检查沿既有聚合路径）。

**为什么根治：** 重试克隆保留前一 execution 的 `start_commit`（陈旧起点复用），完成时又按基线回填首次 execution（`group_completion.rs:87`），导致零提交重跑把人工 WIP 倒算进当前 Work Item（#13）。

**防护：** 零提交＝空区间（start==completion），不倒算父提交；重连／恢复同一 execution 不改写已记录起点；越界提交仍走既有终态写入范围检查拒绝。

### 7. 返修指令单次消费事务（REQ-GCE-C2-INSTR，#18／BYPASS-18）

**改什么：** 把 `coding.rs`（渲染 `:135-152`→消费 `:162`→spawn 三个独立写入窗口）与 `rework.rs`（`:138` 先消费、`:157` 再以 `None` 渲染上下文）统一为同一消费口径：先以待消费 instruction／note 渲染完整 prompt 与执行上下文，再以一次可重放的原子写入（journal＋CAS）完成“认领→绑定渲染结果→标记消费”，最后 spawn。渲染失败（含 rendered_context 为 None）时禁止标记消费。

**为什么根治：** 现状两路径时序相反且非原子：中断窗口内指令已 consumed 但 prompt 未含指令（或反之），用户只能直写 consumed 标志回旧 hash 脱困（BYPASS-18）。

**复用什么：** C1 命令账本幂等（同 command 同 payload 重放首次结果、异 payload fail-closed）与一次 CAS 写面模式。

**防护：** 实际 prompt 必含被消费指令；中断后用户点继续以同一认领与同一上下文 hash 启动，旧 hash 不覆盖；同一指令至多一次 role run 消费。

### 8. 独立验证处理／受限豁免（REQ-CVT-03／04＋coding-code-review-triage delta，#19／BYPASS-19）

**改什么：** 新增验证处理记录（持久面＋DTO），字段严格绑定 finding、check、plan revision、原／替代命令、cwd、执行结果、测试执行数量、环境与 scope（含 expiry）。入口放在 `gates.rs` 门呈现面旁边（`coding_output_human_triage`、`code_review_verification_incomplete` 等），MUST NOT 进入门动作集合（`gates.rs:18-40` 六动作枚举不变）。三类结论（批准计划修订经既有 amendment 链、接受可信等价证据、授予限域环境例外）均需用户明确批准。

**为什么根治：** 现状 coder 输出门只有 retry／abort，字面验证命令不可满足时用户只能直写 context note 清空 `plan_defect_findings`（BYPASS-19），无权威裁决记录。

**防护（滥用面）：** 同一 finding／check／plan revision 已有未决验证处理时拒绝重复转入；零测试执行且 check 要求非零测试的证据被拒；plan revision 过期、scope 宽于所绑定 check、证据不完整均 fail-closed；豁免只覆盖绑定 check 与 scope 并记录理由／操作者／时间；findings 不清空只标注“已由验证处理覆盖”；FinalConfirm 仍人工并可见全部验证处理记录。

### 9. 计划命令 vs 实际命令与文案（REQ-CVT-01／02／05，#2／#9）

**改什么：** 在验证类等待面上并列展示计划合同 check 命令与 coder 实际执行命令、cwd、exit、环境摘要（从已记录的工具调用与输出证据派生，缺失显示“未记录”）；提供“重跑原计划命令”操作——走既有 rework 路径、以计划合同字面命令与 cwd 作为明确返修指令纳入实际 prompt，携带 `command_id`＋gate／check 身份。错误文案三类口径：“实际执行命令与计划不一致”（指向重跑）、“计划未声明该路径／命令”、“计划路径不可执行”（指向计划反馈／修订或验证处理）。

**为什么根治：** 现状把“coder 误跑目录命令”误报为计划缺陷，文案混同升级 Node／泛化 retry／忽略 finding（#2／#9 文案责任在本 change）。

**防护：** 重跑不改写计划合同、不清 finding、不跳过 Code Review；MUST NOT 新增服务端通用命令执行器。

### 10. 受限政策读取（REQ-ENV-C2-POLICY，#17／BYPASS-17）

**改什么：** blocked／rework 等待面新增“读取政策”与“重新授权”操作：读取经唯一 LC authority resolver（C4 `logical-codebase-bootstrap-and-repair-surface` 交付）解析 attempt 冻结 envelope 的 policy_id／revision／digest 并返回同 digest 正文；重新授权在用户确认后为下一次返修 run 签发 evidence 授权。

**复用什么：** `src/product/logical_codebase/evidence_token.rs`（`issue_evidence_token`／`validate_evidence_token`）与 `evidence_mediator.rs handle_evidence_query`（token→attempt 归属→Running 校验→lc 子树解析）的既有受控读取骨架；`src/product/cadence_skills/routing_reference.rs LogicalPolicyReference` 的三值引用形态。

**防护：** 授权绑 attempt＋role＋policy digest，仅该 run 运行态有效；错 role、过期／已替换授权、错 attempt、digest 不匹配、越出 manifest 成员范围均拒绝；resolver 无法唯一解析时 fail-closed 落核验等待项，MUST NOT 回落到成员仓路径／项目级历史布局／绝对路径猜测，MUST NOT 把政策绝对路径或正文写入 context note。

### 11. SC 人工返修完整预算与传输（REQ-CG-03，#15）

**改什么：** `src/product/workspace_engine/prompts/human_gate_revision.rs` 的固定 32,000B 上限（`:10 SC_MANUAL_REVISION_PROMPT_QUALITY_BUDGET_BYTES`，超限直接拒绝）改为完整预算口径：扣回合前以 UTF-8 字节计算完整组装输入（当前候选全文＋固定合同＋feedback＋上下文）与所选 provider 真实输入预算比较——不超 inline 预算整体内联；超 inline 但在硬限内经既有 artifact 读取能力或完整有序分块传输完整候选原文并记录组装 digest；不可读／缺块／超硬限在 turn CAS 之前拒绝并通知停等，门状态与预算不变。

**为什么根治：** 固定常量只解决当前 28,695B 样本，下一个更大候选会再次死亡；截断或摘要替代会破坏候选权威（#15）。

**防护：** 候选不截断、不以“模型可自行读取本地路径”为前提、不新增第二候选权威；用户点击“分段返修／重试”后才开新回合。

### 12. C2 等待项统一投影（REQ-CRO-06）

**复用什么：** C1 `C1WaitingItemDto`（`src/web/plan_confirmed_info.rs:141`）＋`list_c1_waiting_items` 的驾驶舱只读投影模式：本 change 的全部等待项（完成状态待确认、已在运行、确认接管、活性未知、restart、reviewer 配置缺失、验证处理、政策核验、返修消费中断、大候选停等）同样只从 durable 事实派生，进入 `IssueLifecycleResponse` 既有条目通道；通知投递失败仅影响即时提醒，驾驶舱经 GET 补读。每项展示原因、attempt／unit／gate 身份、已完成步骤、可能外部副作用、操作会做与不会做的事、成功后下一阶段；每个操作携带稳定 `command_id` 与 expected 对象版本，REST 与页面动作调用同一应用服务。

## Failure Handling and Migration

- **持久化失败：** fail-closed 保留可诊断错误；观察事件不发送，等待项落驾驶舱可补读；不得回滚已完成业务事实。
- **版本／身份冲突：** restart／接管／验证处理／重跑操作核对 attempt 身份与版本，过期返回“请刷新”，不触发 provider。
- **旧 durable 数据：** 已有 `start_commit` 的历史 UnitRun 不重算、不改写；缺失起点的旧 execution 按起点缺失诊断停等，不回填、不猜测。旧 role provider 快照缺三值字段时按空 effective 解释并落“reviewer 配置缺失”等待项（用户补齐后继续），不回填 author／Codex。
- **C4 依赖边界：** 政策读取在 C4 resolver 未交付时以“resolver 不可用”停等呈现，不实现本地 fallback 路径。
- **兼容边界：** `coding_output_human_triage` 两动作与三个 Code Review 门四动作集合零变化；既有 CAS、attempt 文件锁、工作树锁、journal／DTO 与驾驶舱投影只扩展不新建；本 change 不改 C1 target／binding 写面、不碰 C5 入口。

## Contract Traceability

| C2 依据 | 本设计落点 |
|---|---|
| v1.2 §4.2 #3／#8 | 决策 1：产物先持久化后观察，supersede 只影响观察，完成不明停等 |
| v1.2 §4.2 #2／#9 文案 | 决策 9：并列证据、重跑原计划命令、三类文案口径 |
| v1.2 §4.2 #4／#5 | 决策 3／4：restart 清退役、manual_continue 续跑白名单 |
| v1.2 §4.2 #7 | 决策 5：删除 author／Codex 回填，三值持久化 |
| v1.2 §4.3 #11 coding 侧 | 决策 2：复用 classify_worktree_lease／confirm_takeover 三态与接管 |
| v1.2 §4.3 #13 | 决策 6：认领前冻结 HEAD、重试重置 start、完成不回填 |
| v1.2 §4.3 #15 | 决策 11：完整预算＋artifact／分块传输＋CAS 前停等 |
| v1.2 §4.3 #17／BYPASS-17 | 决策 10：受限政策读取与 evidence 重新授权（消费 C4 resolver） |
| v1.2 §4.3 #18／BYPASS-18 | 决策 7：渲染→认领→消费→spawn 单次可重放事务 |
| v1.2 §4.3 #19／BYPASS-19 | 决策 8：独立验证处理／受限豁免，原门动作不变 |
| v1.2 §5.2／§5.5 | 全文复用既有 manager／runner／attempt claim 边界，不建 owner／fence |
| v1.2 §7 A06／A08／A09／A10／A11／A14／A15 | 每项修法以“通知→用户操作→原链继续”为可观察验收链（见 tasks.md 映射） |

## Risks / Trade-offs

- **接管误确认（无 fence 体系）：** 已承认残留风险；以“接管前强制展示最后活动时间与节点证据”降低概率，不引入 epoch／fence（v1.2 §5.2 明确口径）。
- **先持久化后观察增加完成路径时延：** 换取断连零丢失（A06）；观察事件本就是投影，业务事实不依赖 WS。
- **三值改造触及既有快照消费方：** `From<&ProviderConfigSnapshot>` 的消费面需同步迁移为显式处理空 reviewer，迁移期旧 JSON 缺字段按空 effective 解释并停等，不静默回填。
- **验证处理面是新滥用面：** 以绑定 finding／check／revision／scope／expiry＋三类结论全部人工批准＋审计追加收敛；无批准零推进。
- **大候选分块传输的完整性：** 依赖组装 digest 与缺块检测；不可达输入在扣回合前停等，保证预算与门状态不变（A14）。
- **C4 resolver 交付节奏：** 政策读取为硬依赖路径；resolver 未就绪时该子面以停等呈现而非本地绕行，联验阶段需与 C4 一次集成。
