# Design: batch2-automation-entry

## Context

本 change 是批 2「自动化入口」的契约层。方案设计已完成并经双审修订（oracle 一致性五冲突 C1~C5 + 接地 C1~C6 全部按 owner 2026-10-16 05:00 裁决吸收）、oracle 复审 RESOLVED（phase-ledger 2026-10-16）。**本文件不复制设计内容**，设计与结论以下列文档为唯一权威，实施时按引用路径读取：

| 权威文档（相对仓库根） | 职责 |
|---|---|
| `cadence/designs/2026-10-15_方案设计_批量2_自动化入口_v1.0.md` | 总览：范围与不变量（§1）、主流程映射与已合入接口（§2）、16 编号包拆分（§3）、依赖波次与共享合同（§4）、与批 1 文件面重叠登记（§5）、开放假设与规格修订清单（§6） |
| `cadence/designs/2026-10-15_方案设计_批量2块1_多仓授权与自动启动_v1.0.md` | 成员集合变体与规范化、七消费点、GET/PUT 与授权集合、advance/start 准入、split 策略传递与共享驱动（R1~R4） |
| `cadence/designs/2026-10-15_方案设计_批量2块2_自动批准与plan阶段继续_v1.0.md` | approval 冻结＋AutoIfValid 六接缝、trust/compile 停点、continue-plan/control、takeover 拒绝、plan 事实写者（P1~P5） |
| `cadence/designs/2026-10-15_方案设计_批量2块3_终止接管与整链验收_v1.0.md` | terminate/统一 Abort 路由、专用收口与全门关闭、continue-chain、通知接入与失效清理、整链验收七步（N0/N1/T1~T4/E1） |
| `.superpowers/sdd/batch1-batch2-interface.md` | 批 1/批 2 契约 A1~A6（批 1 已合入真实接口）与 B 区（B2-2~B2-5、B6）、C 区批 2 文件移交表 |
| `.superpowers/sdd/phase-ledger.md` | owner 全部裁定原文（12:30~15:50、2026-10-16 04:20/05:00/05:15）与双审闭环记录 |
| `cadence/plans/2026-10-16_实施计划_批量2自动化入口_v1.0.md` | 实施计划：15 派发包×6 波顺序、红测清单、共享文件串行、统一验收命令 |

## Goals / Non-Goals

**Goals**：把 owner 已裁定的批 2 行为（多仓授权/自动批准/失败继续/plan 级终止/通知接入/整链验收）落成可验收契约（specs/ delta）与可派工任务包（tasks.md），不新增范围、不重开已裁决问题、不重做批 1 已合入接口。

**Non-Goals**：proposal「12 条不做清单」全部适用。设计层补充：不改批 1 A1/A2/A4/A5 的 handler/组件/阈值（只复用与测试）；不建第二通知模型、不递归调 GET 补写事实；不建新调度器（共享 plan-run 复用现有依赖/in_place/claim 算法）；不动 story/design 阶段；不把 plan 对话继续与 coder 门混用（owner 15:20）。

## Decisions（裁决转录，不复制论证）

### owner 产品裁决（phase-ledger，全部已确认）

- **多仓授权＝A**（15:00）：自动化覆盖该 issue 代码库选择里的全部成员仓，弹窗不逐仓勾选。
- **失败接管＝A**（15:00）：provider 不可用修好点继续；评审超限用户在窗口填新上下文点继续；两类都不转手动、不丢自动化模式。
- **对话窗口 vs 门**（15:20）：plan 阶段出错走【对话窗口】补上下文；coder 阶段出错走【门】（批 1 A4）；不得混为一谈。
- **自动批准语义**（14:00/14:40）：自动化模式 plan 自动批准，不受弹窗「子 Work Item 执行前需要确认 Plan」约束；无提问/门时 outline、draft、batch 及最终批准全部自动通过；AI 主动提问或开人工门才叫用户。
- **终止语义**（13:10）：终止可在 plan 编写阶段与 coding 过程中两阶段发生；终止＝整个自动化结束、不可恢复；并行 WI 时任意工作台终止＝终止全部（整个 plan 所有 attempt）；批 2 自动化模式下改 plan 级终止＋二次确认，单仓/手动保持原样。
- **无暂停**（13:05）：没有「暂停自动化」概念；用户只通过系统主动叫他的接入点介入；可看全过程、可终止。
- **预算耗尽＝A**（2026-10-16 04:20）：manual 预算为零时，每次用户显式补新说明并继续只开放一个新人工说明回合，非后台无限重试。
- **双审五冲突全部按推荐**（2026-10-16 05:00）：C1 服务端拒绝 takeover＋隐藏按钮；C2 Abort 路由成 plan 终止（统一二次确认）；C3 `terminated_at` 纯模型字段前移 N0；C4 包显式写通知事实（与批 1 A1 同构）；C5 终止收口关闭全部 open 门。

### 关键设计裁定（双审修订，详见三块文档变更清单）

- **方案 A：冻结 `plan_approval` 选项＋AutoIfValid**（块 2 §3）：复用引擎语义，人工门天然保留；备选「编排器代点 Interactive 门」因产生假人工批准审计被拒绝。
- **成员集合最小改法**（块 1 §3.1/§10）：新增 `logical_codebase_members` kind＋共享 plan-run；单字段数组化与新调度器两个备选被拒绝（旧数据/digest 面更大、重复依赖易竞争）。
- **N0 纯字段前移**：`terminated_at` 与 `AutomationAttentionFact`/store 接口先供 P4/P3，P4 不依赖 T 包；T1 只加终止 CAS/准入，消除波次倒置（oracle C3）。
- **事实写者固定**（oracle C4/R-复审2）：P3 写/清 trust waiting 与 compile 恢复门 `plan_question`；P4 写其余 plan 三类（含 `plan_generation.rs:99-110` NeedsHuman 派发前失败 `automation_failed`）；R2 写 advance/coding_start；T3 写/清终止事实；N0 存储、N1 只读/清失效 choice；不允许旧/别的绑定写入；同一 terminated 绑定可清旧事实/写清理失败。
- **统一 Abort 语义**（oracle C2/R-复审2）：自动绑定 plan 的门 Abort/原 abort REST/通用 gate-responses REST/WS/Inbox 一律进入同一整 plan 终止与二次确认；未确认返回 `automation_plan_termination_required` 零副作用；终止发起或已 terminated 后 registry 投递的内部 AbortAttempt 在 `runner_support.rs`/stage 门 `gates.rs` 只干净退出/发 completion，不走旧 handle_abort/clean，让位专用收口；手动单 attempt 与非托管旧 clean 保持。
- **takeover 隔离**（oracle C1）：P4 对精确绑定当前 enabled enrollment 的会话在服务端一律拒绝（既有码 `workspace_session_takeover_not_allowed`、零子 session）；P5/T4 Inbox 隐藏接管按钮；非托管既有接管保持原样。
- **终止关全部 open 门**（oracle C5）：专用收口关闭全部 open blocked/choice/stage 门、清三类旧事实；Completed 保留/其余 Aborted；跳过 clean 保留文件；脏工作树不阻断终止。
- **依赖波次**（R-复审2，总览 §4 替代旧波次）：P1→P2→N0→P3→R1→R2/P4→T1→T2/T5→T3→T4→N1→R4/E1；N0 前移 P2 后避免 P3→R1→N0 成环；R3 类型前置子提交夹在 P1 后端/前端之间，其余集合 UI 在 R1 后；store 串行 P1→N0→R1→P4→T1。
- **包数口径**（总览 §3）：登记编号实数 16（R1~R4=4、P1~P5=5、N0/N1=2、T1~T4=4、E1=1）；E1 并入 R4 同一实施/验收单元后实际派发 15 包，不得写 14。

## Risks / Trade-offs

- 总览 §6 去重 5 项开放假设为已知风险，批 2 不处理：①真实四仓/AutoIfValid/choice/服务端 Abort 与终止清门尚未实跑，R4/E1 统一验收；②旧 target/Manual 轮不热升级；③继续跨域部分写入显式停等、不建事务；④Git/锁清理异常回 `cleanup_required` 仍全停禁恢复；⑤重启丢 choice 旧题失效、通知重做不加恢复按钮。
- 块 3 A-01：completion 跨 author/review/manual revision 构造点均需测试覆盖，设计读码不是运行验证。
- 结构体/枚举加字段（`EnrollmentTarget`、`EnrollmentOptions`、`AdvanceRecord.start_run_policy`、`AutomationEnrollment.{terminated_at,attention_facts}`）的编译面经设计文档逐项落到对应包独占清单（含 R1 的 8 处 fixture 同步面）；新增 enum 必须同包改完穷尽 match，不提交不可编译中间态。
- additive 回退：旧 JSON 缺省读 manual/None/[]；未知 target kind 前端透传不改（R3）；旧 abort 手动路径行为不变。
- 真实 provider 整链不能用单测替代：块 3 §8 七步由主会话在独立 `/tmp` 夹具与真实 provider 环境验收。

## Migration Plan

按实施计划 6 波顺序合入（波 1：P1(后端)→R3(类型子提交)→P1(前端)→P2→N0→P3 串行链 → 波 2：R1→R3(集合 UI) → 波 3：R2∥P4（仅无交集面） → 波 4：T1→T2；P5 与 T1/T2 并行 → 波 5：T3→T4 → 波 6：N1→R4/E1）；共享文件按总览 §4/§5 与契约 C 移交表串行（store：P1→N0→R1→P4→T1；workspace.rs：P2→P4→T2；projection：R3→P5→T4；Inbox：P5→T4；manager/mod：T2→T3；runs/router：T2/P4→N1）。全部合入后主 agent 统一验收（四条 cargo＋web 三绿＋禁 `-j`＋strict＋单仓/手动冒烟＋块 3 §8 整链七步）。

## Open Questions

无阻塞项。owner 待产品决策＝0（总览 §6）；04:20/05:00 裁决已闭合 A-02/A-04 与五冲突；契约 B/C 签字状态由 Main 按 owner 硬门禁管理，不代签、不据此扩大范围。
