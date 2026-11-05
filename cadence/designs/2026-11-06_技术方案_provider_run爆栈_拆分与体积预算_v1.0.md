# 技术方案：provider run 爆栈——拆分与体积预算

| 项目 | 内容 |
|---|---|
| 日期、版本 | 2026-11-06，v1.0 |
| 适用工作树 | `bugfix-b-11-02` |
| 文档状态 | 设计完成，实施与效果验证尚未进行；owner 决策见 §10 |
| 目标 | 恢复所有 WS provider run 的既有启动、取消、接力与落盘行为，消除深栈构造巨型 future 的风险，并建立可重复门禁 |
| 本次交付边界 | 仅本设计文档；不修改产品代码、OpenSpec/spec/tasks、编译配置；不运行 cargo/gdb；不访问共享验收、部署实例；不暂存 |

**推荐摘要：** 保留独立 run 任务和既有取消合同，采用“**小 capture 任务外壳 + 外壳首次 poll 内构造 boxed drive + 各业务臂及 followups 的具名 async fn/显式 Box 间接边界**”根修；若需要立即恢复服务，先以独立、可撤销的启动栈缓解止血；长期用未装箱体积断言、经验证的 Clippy 扫描及固定 2 MiB 真实 WS 子进程回归防复发。不能只靠改行数、切 release、Tokio 自动 boxing 或扩大测试栈宣布解决。

## 1. 问题陈述、证据与统一机制

### 1.1 已确认事实与影响面

本文承接既有取证，不重跑 gdb。内部证据索引：

- **D1：** [诊断报告](../reports/2026-11-06_诊断_stack_overflow_workspace_supersede_v1.0.md)，重点 §1.3–1.4、Q1、Q7。
- **D2：** [独立复核](../reports/2026-11-06_复核_stack_overstack修复方案_独立挑刺_v1.0.md)，重点 §2–§7。本文采纳其六条修正，见 §9。
- **D3：** `2026-11-06_调研_Rust异步大future栈溢出与栈取值_v1.0.md`，本次读取的实际落盘位置为主检出目录 `/home/michaelche/workspace/github/cadence-aria/cadence/reports/`，不是假定工作树内有副本。关键外链在 §8 展开。
- **H1：** 已成功读取 `history://DesignStackOverflowFix-2` 的可见转录，承接此前源码核对及取证上下文；本文精确数值以 D1/D2/D3 的落盘证据为准，不把历史工具调用摘要当作新增测量。

事故 debug 制品在 `src/web/workspace_ws_handler/run/provider_run.rs:310` 的 `tokio::spawn(async move { … })` 崩溃；该块延伸到 `:1670`，约 1360 行。future 实测 **243,880 B（0x3b928，约 238.16 KiB）**，该制品 `spawn::<F>` 序言预留 **487,928 B**，约为 `2×F`；上层 axum→socket→inbound→worker poll 链已占 **>1.5 MiB**，worker 使用默认 **2 MiB**。栈探测写穿 guard page，最终 SIGABRT；**spawn 体尚未 poll，不是递归**。

所有 `spawn_provider_run_from_handler`、`spawn_provider_run_from_event`、`spawn_provider_run_claiming_idle` 最终共用此构造点，包括 StartGeneration、Retry、Revision、事件 relay、auto 认领。空 `content` 是正常 Author 载荷；不能针对 Story、空串或会话数据形状做特判。REST 自有 runner 不因此自动纳入改造。D1 提到的编译返修递归环另案处理。

### 1.2 自动 boxing 与构造期栈占用并不矛盾

锁定 Tokio **1.52.3** 的 `task/spawn.rs:179–184` 会对 `size_of::<F>() > BOX_FUTURE_THRESHOLD` 无条件 `Box::pin`；`runtime/mod.rs:643` 附近阈值为 debug_assertions 开启时 **2048 B**、关闭时 **16384 B**。当前 F 远超二者，自动装箱已经发生。

但是调用 `spawn(F)` 前必须构造/传入 F。**任务最终住在堆上，不等于构造、按值传参与搬移不占调用栈**；内部装箱不能追溯消除深 inbound 链上的实参临时值。本项目 debug 实测的“构造期临时值 + spawn 搬移帧约 2×F”与自动装箱时机完全一致。约 2×是这个制品的帧事实，不是 Rust 对所有 profile/函数的 ABI 保证。

调用点 `spawn(Box::pin(future))` 会使 spawn 接到小指针包装，tokio#6057 的历史最小例确有救活 release 的证据，故它是**有上游依据的最小缓解候选**。但 `Box::pin(x)` 按值接收 x，不保证本项目编译器直接在堆上原位构造：可能消掉、减少，也可能把临时大帧移到调用者。**本项目 dev/release 的净收益均未验证**，不写“一定无效”或“一定消除栈临时值”。

结构解法则切断状态内嵌：独立 `async fn child()` 后直接 `child().await`，父状态机仍保存 child；必须明确 `Box::pin(child()).await` 等间接存储边界。互斥 match 臂通常按最大活跃布局及共享字段计账，**不是把全部臂大小相加**。`include!`、宏只做文本展开，不建立边界；followups 同样必须计入。

### 1.3 profile 与部署语境

已核对 `Cargo.toml:53–67`：本 crate dev 仅 `debug="line-tables-only"`，未显式开优化，通常仍为 opt-level=0；`[profile.dev.package."*"] opt-level=1` 只优化依赖。release 为 `opt-level="s"`、LTO、单 codegen unit、strip；两个 profile 未显式修改 panic，未被外部覆盖时沿用 unwind。实际进程环境覆盖未验证。

`.github/workflows/release.yml:55–67` 明确正式分发制品由 `cargo build --release --locked --target …` 生成；本次事故却明确是 `target/debug/aria`，不能用发布通常是 release 排除 debug 验收。上游既有 release-only，也有 debug-only 事故；`"s"+LTO` 对本项目 F 的大小方向没有测量或系统性保证。**双 profile 都是验收对象，切 release/opt-level/panic 不是默认根修。**

## 2. 体积账、预算与依据等级

### 2.1 修正余量算式

定义 S=worker 栈，U=进入构造点前的栈占用，M=该点构造/搬移峰值增量，C=其他同时存活的临时开销，H=要求保留的安全余量。安全条件是：

`U + M + C + H <= S`。

若以旧制品近似 `M≈kF`、k≈2，则 `F_max <= (S-U-C-H)/k`。**只有 U 的上界、各峰值叠加关系与 k 都经过测量，这才是可用的安全上限；现在只有 U>1.5 MiB 下界，不能据此证明某个 F 绝对安全。**

| 代入情形 | 正确计算 | 可以下的结论 |
|---|---|---|
| 当前实测 F 与 spawn 帧 | `2,097,152 - U - 487,928 < 36,360 B` | 按 U 下界计算的剩余量上界只有约 35.5 KiB；现场已实际越界，不是还有这么多可用栈 |
| 工程目标 F=128 KiB，按约 2F | `2 MiB - >1.5 MiB - 256 KiB < 256 KiB` | D1“>1.2 MiB 余量”算错；实际只能得出 **<256 KiB**，且不保证为正 |
| 理想化忽略 C/H、令 U=1.5 MiB | `F_max=256 KiB` | 只是使用下界得到的乐观天花板，既不是安全阈值也不是实测上限 |
| 假设 U=1.5 MiB、另留 H=256 KiB、C=0 | `F_max=128 KiB` | 解释 128 KiB 工程目标的来源；真实 U 更大、C 非零时应更严格，不能作为证明 |

不得把同一临时值同时计入 U 和 M，也不得仅拿 `size_of_val` 当栈高水位。新边界改变后必须重新量构造侧、spawn 侧和 poll 侧，不能继续机械套旧 k=2。

### 2.2 本次建议的工程门禁

以下是**设计预算，不是已达成的测量结果**；依据等级分别列出。

| 对象 | 初始门禁/目标 | 依据等级、后续校准 |
|---|---|---|
| 传给 spawn 的未装箱小外壳 future | **≤32 KiB** | 工程选择；避免继续把巨型 child 带回 inbound 深栈。以旧 k 粗估为≤64 KiB 增量，但仍需量测，不能声称真实留有448 KiB |
| 具名未装箱 drive body | **≤128 KiB** 硬上限 | 继承并纠正 D1/D2 的工程目标；体积先下降至少约46%，不宣称可证明安全 |
| 每个具名臂、followups、关键嵌套 child 的未装箱 future | **≤128 KiB** 硬上限，优先争取≤64 KiB | 硬上限是本次保守约束；64 KiB 是优化目标，不是源码实测预测。若仍超线，在该 child 内继续设边界 |
| 注册前入口 future、取消 settle helper | 记录真实大小与修前/修后变化，不允许无解释膨胀 | 它们也处于调用链；不能只瘦已 spawn 的 F |
| Clippy `large_futures` | 默认 `future-size-threshold=16384` B 作为扫描基准 | 上游默认，不等价于本项目安全证明；量测位置及误报/漏报见 §6 |

先记录机械提取后的真实基线，再冻结具体对象的回归阈值，留明确且有限的维护余量，不能把所有对象都放宽到128 KiB。参数/capture 变化会改变布局，**不保证具名提取后仍恰为243,880 B或必红**；真实 WS 红基线承接 D1，新增测试是否修前能复现另记。

### 2.3 不该记错的账

- `String`、`Vec`、`PathBuf` 的堆内容不按 `len()` 内联进 future；给 String 再套 Box 不会省整段 prompt 的字节数。优先量大型内联 record、最大 enum/Option 变体、跨 await 存活值及 child 状态机。
- `run_context_clone.session_record` 是应量的内联 capture；不能为了传参便利每臂再 clone 整份 `ProviderRunContext`。
- 现 `provider_run.rs:319–322` 将 `Box<ResolvedPlanningContext>` 解引用再 clone 成裸重建上下文；应去掉这份不必要裸副本，见 §3。其精确收益未验证。
- Box 边界约束的是父体存储；child 构造和 child.poll 的同步帧仍可能大。固定2 MiB真实入口与双profile是独立条件，不由体积测试替代。

## 3. 拆臂、间接边界与生命周期合同

### 3.1 推荐结构：不增加新的业务调度模型

沿用 `run.rs` 已有 `#[path] mod`、`provider_run/entry.rs` 单入口以及当前取消 settle helper。下文文件名/新符号均为**拟新增设计接口，尚不存在**；既有符号与来源行号另行注明。保持一个独立 run task，不把每个臂改成独立 task，不引入 dyn 驱动 trait、actor、专用 runtime 或第二套实现开关。

1. **入口不变：** `spawn_provider_run_with_start_mode` 继续按既有顺序 acquire entity lease→enrollment 检查→attachment supersede/claim→选 provider→注册 token/command_rx→begin idle guard→mark started。尤其不得先拿 engine 锁再取消旧 run。
2. **小外壳：** spawn 的 async 块只捕获必要 owned/Arc 数据、现有一份上下文、run_kind、token、receiver、守卫及取消兜底数据。大 body 的调用表达式必须在**新任务首次 poll 内**才执行，形成 `Box::pin(provider_run_task_body(...)).await` 边界；不能在 spawn 外先构造 body，也不能直接 `body(...).await` 使巨型 child 重新内嵌外壳。Tokio 不同步 poll spawn 的 future，但调度先后仍不作业务承诺。
3. **任务 body：** 外层持有 entity lease 与 `Option<WorkspaceProviderDriveGuard>`；按现时点取得 engine MutexGuard、安装 run token，构造取消驱动与兜底参数。在取消 helper 内 await 的 drive 使用 **`Box::pin(provider_run_drive_body(...))`**，避免 helper 的 pin/select 将完整 drive 重新放到栈上。锁获取仍在取消观察之前，保留当前行为，不顺手改锁粒度。
4. **drive：** 按值拥有 engine guard，先 boxed 分发实际臂，再 boxed followups，最后保持原正常尾部。所有 provider 启动、SC attempt循环、review/outline/draft接力都在同一取消范围内。
5. **各臂：** 每个具名 arm 调用点均 `Box::pin(arm(...)).await`，不把全部臂同时构造；只为实际选择的臂分配。重量子调用仍超预算时，在其调用点继续 box，不能仅凭拆函数行数判减肥成功。

这是“脱离深 caller 栈构造”和“缩小状态机”两个杠杆的组合。外壳捕获、body构造与poll峰值尚未实测，因此结构推荐不是效果通过声明。

### 3.2 分发与文件布局

基准目录为 `src/web/workspace_ws_handler/run/`。新文件目标≤1000行，硬线1200行（注释/空行也算），不继承旧cap。

| 既有臂/展开位置 | 拟新增具名 async fn | 拟存放文件 | 重要边界 |
|---|---|---|---|
| Author `:333–342` | `drive_author_arm` | `provider_run/basic_arms.rs` | boxed 调用；沿用 handle_user_message_from_run |
| AuthorChoiceFollowup `:343–352` | `drive_author_choice_followup_arm` | 同上 | boxed 调用；不改变 choice/session record语义 |
| Revision `:353–357` | `drive_revision_arm` | 同上 | boxed 调用；receiver仅移动一次 |
| ReviewOnly `:358–401` | `drive_review_only_arm` | 同上 | gateway/传统review两路与Completed回显都在臂内 |
| LegacyAuthor、OutlineRevision、OutlineRebuild `:402–406` + 既有545行include | `drive_legacy_outline_arm` | `provider_run/legacy_outline.rs` | boxed 调用；把旧include内容转为真正函数，移除旧include文件 |
| SingleCandidateAuthor `:407–572` | `drive_single_candidate_author_arm` | `provider_run/single_candidate_arm.rs` | boxed 调用；继续复用现有single_candidate::run_single_candidate_author |
| Draft `:573–792` | `drive_plan_draft_arm` | `provider_run/plan_draft.rs` | boxed 调用；循环、RetryOnce、fresh payload消费不变 |
| Batch `:793–1004` | `drive_plan_batch_arm` | `provider_run/plan_batch.rs` | boxed 调用；保留batch fresh与loop语义 |
| PlanRevision `:1005–1019` + followups.rs:103–684宏 | `drive_plan_revision_arm` | `provider_run/plan_revision.rs` | boxed 调用；删除旧宏定义/调用，不保留文本别名 |
| HumanGateScManualRevision `:1020–1639` | `drive_human_gate_sc_revision_arm` | `provider_run/sc_manual_revision.rs` | boxed 调用；同turn attempt、claim、frozen prompt、失败回显原顺序 |
| followups `:1641–1651` + followups.rs:686–993宏 | `drive_provider_run_followups` | `provider_run/run_followups.rs` | boxed 调用；所有接力仍在drive取消范围 |
| 共通外壳/分发/正常尾部 | `provider_run_task_body`、`provider_run_drive_body` | `provider_run/task_body.rs` | 二者边界都显式Box；provider_run.rs仅保留准入与小spawn外壳 |

已有 `followups.rs:995–1113` 的 `drive_current_work_item_plan_outline_run` 保留位置和调用合同；在新followups中调用时box。reviewer两路、draft provider start/output、legacy/SC重量嵌套child都纳入A层量测；只有实测超预算的子树继续细拆，避免无依据地Box每一个小await。删除两个迁移后无调用宏；`run.rs` 中不再需要的 `#[macro_use]` 一并清理，仍保留实际使用的函数导入。没有借拆分改变业务算法或引入新retry。

### 3.3 共享状态与控制流接口

采用既有上下文加窄参数，不创建克隆整份上下文的“万能包”。各臂内部对 engine 持锁，取消时必须能连同它一起被drop。

| 状态 | 传递与所有权合同 |
|---|---|
| `engine` | Arc留在任务外层供取消兜底重新取锁；**MutexGuard按值**进入drive，再按值进入当前arm/followups。子函数不是仅借用外层永久存活的guard |
| `run_kind` | 移入drive，匹配后只向选中arm移动该variant载荷；不clone整enum用于业务分发 |
| `run_cancel` | 任务外层拥有token，drive/arm用引用；安装engine token及现有按值provider API处才clone，保持同一取消源 |
| `outbound_tx` | 任务拥有Sender；arm/followups借引用，按既有位置发送，不另spawn发帧，不改变持锁/解锁与send的相对次序 |
| `provider_drive_guard` | 最外任务body的可变Option；drive/arm借 `&mut Option<WorkspaceProviderDriveGuard>`，仅既有分支 `.take()`。不得移入一个臂的局部变量 |
| `entity_run_lease` | 从入口移到最外任务body的独立binding，覆盖drive、取消兜底和全部最终store写；不传给arm，不转移给接替run |
| `rebuild_context` | 删除 `(**rebuilt).clone()` 裸副本；在匹配OutlineRebuild时把现有Box载荷交legacy arm，其他同族臂传None。arm以 `as_deref()` 借用；不为引用原有payload而再clone全文结构 |
| `cancelled_revision_turn_id` | run_kind移动前提取，仍由取消外层持有；不能仅留在可被drop的SC arm里 |
| `outline_revision_feedback` | 匹配OutlineRevision时将feedback移动给legacy arm；仅该臂需要，不在共通前奏复制。现有 `is_outline_revision_run` 由明确的同族入参保留判断 |
| `cancel_arm_outbound_tx` | 在run_kind/drive移动前保留既有Sender clone给取消外层，确保drop drive后仍可回显 |
| 其他共享量 | `&ProviderRunContext`、provider Arc引用、manager Arc引用、run_label/session_id字符串引用、run_token复制；`command_rx`为单一owned Receiver，进入当前arm，不clone/重建为第二消费者 |

**拟新增返回合同：** `ArmExit<'engine>` 只有 `Continue(MutexGuard<'engine, WorkspaceEngine>)` 与 `Stop` 两种控制含义，不是新的业务错误抽象。各helper按值接guard；原来正常穿过match末尾的路径返回Continue，原来 `return` 退出整个drive的路径按原顺序执行该分支收口后返回Stop；父drive遇Stop立即返回，**不继续followups、不补一次通用finish、不补错误帧**。返回Continue才把guard交给下一段。

这是为保留宏/include里的非局部return建立显式合同，不把typed waiting、AlreadyFinished、Superseded、普通失败统一为 `Ok(())`。原 `break` 仍只退出其对应loop，之后是否进入正常尾部按原逻辑。followups也返回该合同。原本只mark后return、或drop engine后send但未finish的差异原样映射；若这些差异另有缺陷，须另案授权，不能借减栈悄悄规范。接口lifetime/Send可编译性尚未验证，实施必须以现MutexGuard类型完成编译检查，不能改用OwnedMutexGuard或改锁获取时点来回避。

### 3.4 三种守卫：取消时必须逐个区分

| 守卫 | 当前合同/根修后必须保留 | 正常结束 | 取消窗口超时强制drop |
|---|---|---|---|
| **engine MutexGuard** | `provider_run.rs:315`取得，drive因按值drop/使用而拥有；取消范围对当前arm的拥有关系必须传递到此guard | 正常尾部mark→drop engine→drop idle→finish_run；提前分支维持原顺序 | `cancel_settle.inc.rs:30–42` pin作用域结束drop drive，递归drop当前boxed child并释放engine锁，**先于**取消兜底重新取锁 |
| **idle WorkspaceProviderDriveGuard** | `:284` begin；外层Option由drive借用；`state.rs:65–103` Drop只递减depth，不是engine锁 | 在既有 `.take()` 处释放；followups仍运行时不得提前解禁idle | **drop drive并不等于释放它**；没有被旧路径take时，外层Option继续持有到取消兜底/finish_run完成后、任务外壳退出时释放 |
| **实体 EntityRunLease** | `:177–195`在engine/session锁之前acquire，独立保护删除准入；旧无owner时None是既有行为 | 整个任务所有store写结束后释放 | drive drop后仍保留，取消turn落盘、回显及finish_run结束后才能释放；接替run必须另acquire自己的lease |

原 `cancel_settle.inc.rs` 的约250ms有界收口窗口保持：先允许provider自有取消臂正常收口；超时才drop drive，随后必要时 `fail_cancelled_human_gate_revision_turn`，再 `finish_run(old_token)`。engine锁在兜底前释放；idle与entity lease在此时仍可持有，之后各自按现外层作用域收口。正常尾部idle明确先于finish；强制取消尾部idle保留到finish之后，**不能把两条路统一成同一个drop顺序**。实体lease始终不早于全部写完成；末尾两种非engine守卫按其外层binding布局保持既有释放关系，不另引入提前释放点。

保留并搬到新边界旁的注释必须明确标注 **“r16 现场死锁根修，缺陷#10家族”**：旧drive悬停在select外裸await时，取消后drop整棵drive必须解锁，否则新run永久等engine，WS/HTTP冻结。不能把guard留在取消外层、只drop借用子future；不能内部spawn child然后仅drop JoinHandle（那会脱离运行继续持锁）。也不能把取消观察范围缩到provider stream而漏掉provider start、SC attempt或followups。

## 4. 候选方案对比与定案证据

表中“可独立成立”指实施后有可能满足完整验收，不代表当前已经验证有效。

| 候选 | 有效性 | 代价与副作用 | 能否独立成立 | 本项目定案所需实测 |
|---|---|---|---|---|
| **调用点整体 `Box::pin`** | 上游历史复现有效；spawn泛型变小，提前装箱可能减少搬移；无原位构造保证 | 一处分配，小代码改动；大构造帧可能转移到caller/Box | 可以作为最小缓解候选，**本项目未定案**；不能先验排除 | dev/release各测原始F、caller/Box/spawn峰值及固定2MiB真实WS；检查是否仅“spawn帧变小” |
| **只拆async fn、直接await** | 可读性改善；child仍内嵌，无结构性减肥保证 | capture/layout可能变化，可能大或小 | 不作为可靠独立方案 | 未装箱父/最大child与真实WS均过线才有事实收益；不得靠文件行数 |
| **拆臂+Box间接边界** | 父只保存child指针；包含followups才闭合 | 每实际执行阶段分配，需审计return/guard；child构造/poll仍可能大 | 根修主干；单独能否达标须实测 | 父/各臂/followups未box体积，child构造/poll峰值，A–D及行为矩阵 |
| **小外壳浅栈构造+上述拆分的组合（推荐）** | 同时减少inbound上F与后续最大child；不改任务模型 | 增加有限Box与显式控制流；capture若仍大也失败 | 推荐永久方案，效果未验证 | 外壳≤32KiB、body/child预算、显式2MiB真实WS、双profile制品与取消生命周期 |
| **加大worker栈，手写Builder** | 直接增加容量，社区8MiB有存活案例 | 影响所有runtime生成线程含blocking；线程数×栈预留，需保留I/O/time/多线程/main输出退出语义 | 可独立止血，不是根修 | 实际线程配置/环境、D层完整行为、线程数/VM/RSS；2MiB门禁不得跟着放宽 |
| **独立OS线程+大栈+Handle::block_on** | 仅在新线程里构造body才避开caller深栈 | 每run一线程；取消不能安全强杀，join/completion、shutdown与I/O/runtime归属复杂 | 技术上可行，当前资源审计未做 | 线程内部构造证据、并发资源上限、取消/join/关停/lease全链；本次不推荐 |
| **stacker::maybe_grow** | 在同步构造或每次poll前扩栈可能有效 | 新依赖与平台/栈切换复杂性；只包future构造不保护未来poll；跨await不能保留临时换栈 | 理论候选，没有本项目验证，不独立采纳 | 每次poll包装、Pin/Drop/unwind/Send、red_zone覆盖峰值、目标平台与性能；当前无Tokio官方组合示例 |
| **压inbound/socket poll链** | 减少U，直接增加构造余量 | 需找出实际重子树；拆连接任务可能改变消息串行、断线/lease语义 | 可救当前点，但热点份额未量 | socket:1829、inbound:632–637等已知边界各峰值、顺序/Abort/断线行为；不自动扩大本次范围 |

明确不推荐：删除spawn改长run内联await（阻塞同连接后续Abort/新消息，改变独立存活语义）；`spawn_blocking`冒充大栈（同样默认2MiB，已启动工作不可abort）；以current_thread绕开worker；为减栈建立dyn trait（尺寸收益来自Box，不来自dyn）；切panic=abort、dev opt-level=1或release作为唯一修复；对整个仓库统一加Box/改锁粒度。它们的仓级行为成本大于本次必要切口。

## 5. 推荐方案、分步实施与发布回滚

### 5.1 一个推荐，三个层次

- **治标：** 若owner要求先恢复，独立部署启动环境抬栈；需要产品保证时才把main改手写Builder。环境必须在新进程启动前设置；直跑二进制不会读取`.cargo/config.toml`。两种方式择一作为止血控制面，不新增未经设计的`ARIA_*`配置项。
- **治本：** §3的小外壳、浅栈boxed body、各臂与followups边界；不改业务协议/数据schema/锁粒度，不保留旧巨型驱动flag。
- **防复发：** §6四层回归；未box体积硬断言为主，Clippy补全仓扫描，固定2MiB真实WS不能被生产抬栈豁免。编译器/依赖升级仍须过双profile。

调用点整体Box可以作为低成本对照候选；只有完整验收已证明有效时才可单独作为临时止血制品。它不是拒绝后续拆分预算的理由，也不能与“自动boxing已经存在”混为一谈。

### 5.2 实施顺序、通过判据与可回滚单元

下列为未来实施建议，**本设计任务不执行**。每步使用验收者自有数据副本/独立端口；不写 `/tmp/aria-verify`、不碰 `/tmp/aria-deploy`，不重启共享服务。

| 步骤 | 变更边界 | 验证与通过判据 | 独立回滚边界 |
|---|---|---|---|
| S0 固化红与测量入口 | 新隔离测试、必要具名构造点；不顺手优化 | 保留D1实测；记录机械提取前后大小与profile。A输出真实字节，C/D收集完整失败证据；新B不必然复现，不能冒称必红 | 测试/测量提交可单撤；不是可部署修复 |
| S1 可选止血 | 启动环境；或单独main Builder提交，owner选定 | D覆盖事故debug与release；服务活、provider真启动、完整帧与durable进展四项同时成立，取消/再生成正常；记录有效栈配置 | 单撤配置或main，不混根修。撤后若回到已知崩溃制品须停止放量 |
| S2 小外壳与drive边界 | 小spawn capture，在任务首次poll才box构造body，保持取消域和三守卫 | A测外壳/body；B/C测真实入口、r16解锁；帧与durable判据同上。即便过也只算阶段验证，不宣告全部根修 | 可回到S1已验证止血制品；无S1时不能假定旧版可用 |
| S3 按族拆臂及followups | §3具名函数与Box；逐支审return、take/drop、command/token | A每个未box对象达预算；B/C覆盖最重臂及接力；原行为断言全部保留；最终新文件≤1200，回收已不需祖父项 | 按下表可独立提交；回退不得遗留一半宏/一半新调用。若该步未全绿不发布 |
| S4 门禁固化 | 新测试注册、CI双profile；lint语义核实后启用 | A/B/C常规门禁，D发布门禁；有效2MiB且真实worker执行；检测不到裸future的lint不得当唯一护栏 | 门禁变更可单撤以定位工具问题，不能因此宣告产品修复通过 |
| S5 发布与是否撤缓解 | 固定debug/release制品和启动配置指纹；隔离副本→小范围→放量 | D与§6行为矩阵通过；无abort/zombie、取消/relay/重连/删除/idle收敛，资源无不可接受增长 | 根修行为回归回到“旧行为+已验证止血栈”，不是“旧行为+2MiB”；撤缓解另做独立批准与提交 |

**退出0不算通过。** 所有动态步骤都至少要求：①服务进程未abort且后续HTTP/WS可用；②可信provider-start探针或真实adapter日志证明跨过启动，不以locked或登记run替代；③完整JSON帧被解析且满足该场景因果顺序，不能只打印前200字；④ledger/timeline/message/turn等该路径的durable状态发生预期进展。还要验证取消后能再生成。客户端close/timeout都应报该场景失败，不能沿用旧复现脚本的exit0逻辑。

### 5.3 可派工切片与独占写集

以下新路径均是拟新增；时间为工作包上限估计，不是完成承诺，均≤4h。根修共享文件有单一集成owner，**相同共享文件的切片串行**；不能为了并行让多人同时改provider_run.rs。每片附最小行为/体积测试，完整A–D由主控在全部改动落地后统一验证。

| 切片 | 独占文件集（基准路径缩写见下） | 接口与验收 | 估计 |
|---|---|---|---|
| P1 外壳与取消域 | R/provider_run.rs、R/provider_run/task_body.rs、R/provider_run/cancel_settle.inc.rs | S2；三守卫及250ms窗口不变，外壳可量测；不先塞helper把原文件顶过cap | 3h |
| P2 基础与SC author | R/provider_run.rs、R/provider_run/basic_arms.rs、R/provider_run/single_candidate_arm.rs | 按ArmExit/Box合同接入，选择/typed waiting/ReviewOnly回显保留；依赖P1 | 3h |
| P3 legacy outline | R/provider_run.rs、R/provider_run/legacy_outline.rs、R/provider_run/work_item_plan_legacy_author.inc.rs（移除） | 3种legacy同族覆盖；不再裸clone rebuilt；原return不误跑followups | 3h |
| P4 draft/batch | R/provider_run.rs、R/provider_run/plan_draft.rs、R/provider_run/plan_batch.rs | fresh冻结payload、RetryOnce/loop与失败路径原样映射，两个实际child分别量测 | 3h |
| P5 plan revision | R/provider_run.rs、R/followups.rs、R/provider_run/plan_revision.rs | 整个旧revision宏迁移/删除，所有非局部return有明确Stop，原循环上限不变 | 3h |
| P6 SC manual | R/provider_run.rs、R/provider_run/sc_manual_revision.rs | 同turn两次attempt、claim/ledger/frozen prompt/回显/取消完整；不引入新retry | 4h |
| P7 followups与门禁回收 | R/provider_run.rs、R/followups.rs、R/provider_run/run_followups.rs、R/run.rs（即实际父目录run.rs，见注）、tests/it_core/large_file_guard.rs | reviewer→outline→draft接力全部在取消域；删旧宏/失效macro_use；原文件≤1200后删旧登记，新文件不登记 | 3h |
| P8 A/B行为回归 | src/web/workspace_ws_handler/tests/provider_run_events.rs及拟新增provider_run_events/{stack_budget.rs,small_stack_worker.rs,lifecycle_guards.rs,supersede_reclaim.rs} | 移r16而不删断言，注册不超过cap；实现§6 A/B与guard矩阵 | 4h |
| P9 C/D与CI | tests/it_core/workspace_ws_integration.rs、拟新增其目录下provider_run_small_stack.rs与provider_run_process_smoke.rs、.github/workflows/ci.yml | 复用实际WS夹具补hello/Origin/真实gateway支持名；双profile、子进程完整判据 | 4h |
| P10 owner批准的独立配置 | src/main.rs（Builder方案）；或.cargo/config.toml（仓级force调整）；或Cargo.toml（优化/panic）；拟新增clippy.toml（若选择此配置方式） | 这几类**不能混一提交**；只做owner明确选中的项，各自验证与可撤销，绝不作为根修前置假设 | 每项≤2h |

R表示 `src/web/workspace_ws_handler/run`；P7表中父模块实际路径是 **`src/web/workspace_ws_handler/run.rs`**，不是目录内另有run.rs。R/provider_run.rs等重复写集表示顺序提交的同一个集成owner，不是并行授权。P8与P9在P1接口冻结后可并行，最终门禁登记由P7单一owner收口。本次不要求更高档/gated worker，常规实施链足够；若实测迫使改inbound/锁/actor等未授权边界，应回到owner重新定范围，而不是扩大这些切片。

发布时复用现有日志观察 `handler_run_supersede` 后实际启动、`provider_run_cancelled_dropped_drive`、completion wait timeout、token/scratch、idle depth与entity_busy；不为本缺陷顺手建设新遥测。先有序排空/取消在途run再换制品，不把半个内存driver交给旧版；无schema变化不等于无需生命周期回滚纪律。

## 6. 四层回归、子进程隔离与行为矩阵

本节是实施后的验证设计，**不是已运行的结果**。动态测试必须走真实被测代码；Scripted/Hanging provider只是可控测试夹具，不能以“unsupported provider被拒”替代真实启动。所有相关子进程使用自己的临时数据根、空闲端口与清理所有权，不沿用共享实例。

### 6.1 四层门禁

| 层 | 组织与测量对象 | 必须满足的判据 | 明确不能证明什么 |
|---|---|---|---|
| **A 体积门禁，不poll** | 拟新增 `provider_run_events/stack_budget.rs`；复用现fixture，分别构造未box的小外壳、task body、drive、每个arm、followups及最大嵌套child，`size_of_val(&raw_future)`；同时记录入口与取消helper | §2预算逐个断言，失败打印符号/字节/profile/target/toolchain；dev与release各留基线；同一future类型无需靠改变字符串内容伪造多个尺寸 | 不保证构造/poll栈安全；对 `Pin<Box<_>>`测到的8/16字节只是包装，不能替代raw future |
| **B 固定2 MiB worker路径** | 普通同步test创建 `Builder::new_multi_thread().worker_threads(1).thread_stack_size(2*1024*1024).enable_all()`；根future只等待，**入口流程放到rt.spawn中执行**；Author、各plan/SC重量臂、event relay/claim、followups分别覆盖 | 探针证明业务发生于worker而非libtest/block_on根；真provider start；同连接/manager的supersede在既有1s预算内接替；旧start Drop探针触发、新token/receiver有效；取消与终态落盘符合场景 | 直接helper路径没有axum深链，**修前未必红**，不能等同D1现场；只标multi_thread/1 worker也不等于2MiB |
| **C 真实WS深链** | 复用 `tests/it_core/workspace_ws_integration/part_02.rs:209–283` 的TcpListener→axum→WS→StartGeneration夹具，新独立文件注册；运行在显式2MiB runtime，accept/connection在worker；按现协议补hello(driver)、可信Origin与健康/信任准备 | 跨过唯一spawn，看到ScriptedStreamingProvider以ClaudeCode等gateway支持名真实start；完整帧、message/timeline/turn/ledger进展；继续Abort/新StartGeneration/重连后HTTP与WS活；至少Story/Author及最重plan、SC路径 | locked/start_generation节点只证明前奏；unsupported、连接关闭、超时或只收到初始session_state都不算成功 |
| **D 制品进程smoke** | 独立debug制品与正式release制品，各用独立数据副本和端口启动；记录二进制指纹、编译器/profile、启动环境、栈配置、stderr、完整帧与durable前后差异 | 服务不abort；实际provider启动并到预期终态；取消后可再生成，健康请求/重连可用；子进程退出/超时/异常信号明确归因 | 生产显式8/16MiB的D不能替代C的固定2MiB；客户端自身exit0不是服务成功 |

A/B/C进入常规门禁；D进入发布与制品验收。事故dev配置保持原样验收，不先把dev opt-level=1后再说原问题消失。release与dev的raw future大小及栈峰值分开记录，不能混用。

### 6.2 32 MiB环境掩盖与线程落点

`.cargo/config.toml:8–13` 设置 `RUST_MIN_STACK={value="33554432",force=true}`。普通libtest每个用例本来通常是std spawn的新线程（默认2MiB，不是保证8MiB主线程）；本仓Cargo执行又强制扩大到32MiB。D1关于“current_thread测试必在8MiB主线程”不能作为当前测试配置事实：current_thread只是沿用调用线程，实际栈由该线程和启动环境决定。

同时，多线程runtime的 `block_on` 根future仍在调用线程poll，不能把真正入口直接放根future里就声称覆盖worker。B/C必须显式builder设2MiB，并让被测入口在worker任务里运行，记录线程identity。显式stack_size覆盖RUST_MIN_STACK，因此不必先撤仓级force才能写可信回归。禁止在现tokio测试内嵌套runtime block_on；采用同步父测试/子测试组织。

仓级32MiB目前还有provider_matrix历史用例依赖（配置注释说明），不在本缺陷中直接撤除。若owner决定收回force，单独提交、完整验收那组既有用例；不能删它们或静默改assert。现注释“RSS不受影响”也不应当作资源保证：实际触页仍增加RSS。

### 6.3 易abort测试的隔离合同

- B/C由父测试通过 `std::process::Command` 启动当前测试二进制的**精确单个子测试**；实现时创建明确child模式标识，子模式只运行实际场景，禁止递归再启动自己。`--exact`使用实施后实际存在的全路径，不预写猜测命令。
- 子测试在自己进程内建显式2MiB runtime；父进程不poll被测巨型future，只读取子进程stdout/stderr/exit status、收集完整证据并实行墙钟超时。读取输出须持续进行，避免管道写满造成伪超时；超时后终止并等待自有子进程/其服务子进程，不能遗留监听器。
- 红基线记录信号与错误：Unix `ExitStatusExt::signal()==Some(6)`，shell常见等价码134，并记录 `has overflowed its stack` / `fatal runtime error: stack overflow`。非栈错误同样报失败，不能一律归为预期红。修复后信号/异常退出/超时任何一项都失败。
- 绿判据是子场景完整断言和服务状态，不是把“必须SIGABRT”保留为修复后通过逻辑。SIGABRT/栈溢出不能被JoinError、catch_unwind、同进程timeout安全捕获；父进程隔离是为了保留其他测试和证据，**不是扩大被测线程栈**。
- D另起真正aria二进制，使用仓内既有CARGO_BIN_EXE_aria/独立进程组模式的合同（D2 §5.2指向live harness），但不向超大harness继续塞测试。生产源码若显式设置栈，环境无法把D强行降到2MiB，固定小栈责任仍由C承担。

### 6.4 业务回归矩阵：可以移动，不能删除或缩窄

| 行为 | 必保留的既有锚点（本工作树/报告锚） | 本次增强 |
|---|---|---|
| r16取消回收engine | `provider_run_events.rs:1163` `supersede_reclaims_engine_lock_from_run_hanging_outside_select`，含HangingStartProvider与1s断言 | 移到拟新增`supersede_reclaim.rs`腾注册行；保留名称/注释/原assert，新增旧start Drop探针、取消窗口内自行完成与超时强制drop两条barrier分支，SC取消turn有durable Failed/回显 |
| ReviewOnly不顶杀在途followups | 同文件`:248–374`（D2已核） | reviewer实际start计数、同节点relay去重，不把各臂新spawn造成双驱动 |
| SC revise只一次review、SC author relay仍supersede | 同文件`:748`、`:1003`（D2已核） | Generate/Evaluate委托切换、token及command channel仍同一在途run |
| Generate/Evaluate守卫与legacy不变 | 同文件`:1088–1113` | 取消在author→review→outline/draft各接力边界可达，当前唯一receiver收到Abort |
| manager completion与旧token隔离 | `src/web/workspace_session/manager/runs.rs:523,550,916–935`（D2已核） | completion仍代表其合同内真正结束；旧finish_run不删新token/command/scratch。不得把completion当作entity lease已释放的替代指标 |
| idle depth | `src/web/state.rs:1059–1081` 已定点核对 | 在真实双run overlap按barrier观察1→2→1→0；不能只保留手工begin/drop单测 |
| entity删除保护 | `provider_workspace_runner.rs:93–106`合同 | 使用真实owner实体夹具，取消收口barrier前删除返回entity_busy；旧新run全部写完且lease释放后才可删除，不用None-owner夹具冒充 |
| early return/typed waiting/fresh/retry | legacy include、revision/followups宏及SC各分支 | 普通失败、AlreadyFinished、Superseded、AdmissionWaiting/Native等待、结构拒收、RetryOnce、正常完成分别检查后续start次数、预算/turn/gate、错误帧；Stop不得落入followups或重复finish |
| 真WS因果顺序 | `part_02.rs`真实连接fixture；D2列明的review/SC回显路径 | locked先于本run输出、durable Completed先于其completed回显、取消失败至多一次；按session/turn/node/run关联。只断合同偏序，不假设跨独立Sender/引擎广播的全局相邻总序 |

D2提到 `anchor_and_trust_cases.inc.rs:57–132` 的adapter-start与split落盘序号测试仍保留，但它不等价于socket接收顺序。因拆分需要迁移测试时，原断言/背景不删；旧完整用例不能替换为“外壳size小”这一条。

### 6.5 Clippy与体积断言的分工

上游 `clippy::large_futures` 默认阈值 **16384 B**，配置键 **`future-size-threshold`**。建议把它用作广覆盖扫描，直接未box体积断言则作为本缺陷硬门禁。不能假设当前 `cargo clippy … -D warnings` 自动启用了此lint。

实施时先在所用stable clippy核对lint可用性及启用方式，并做两个受控对照：裸大child的await必须被报出；改为Box后的代码究竟仍测底层还是只看指针，要结合当前lint实现/诊断确认。对照仅为验证工具的临时材料，不永久留无关脚本。**本任务未核实该版本lint对Box表达式的精确测量位置，标记未验证。** 若box后恒绿，仍可扫描漏网裸await，但不能据“lint全绿”替代A；A必须在装箱前量实际类型。

默认16KiB可能扫出本次范围外既有大future；推荐本次先限定provider-run受影响边界，确认无假门禁后纳入现CI，仓级推广由owner单独定案。不为过lint普遍加allow、不把阈值整体抬到128KiB掩盖所有热点，也不为达到覆盖声称已完成全仓整改。Tokio维护者临时patch阈值panic的扫描法仅是额外盘点选项，不能把patch版依赖带进正式制品，且它主要看到spawn参数，不保证覆盖已box的底层child。

## 7. dev / prod栈取值建议、依据与代价

### 7.1 两档建议与适用阶段

| 环境 | 推荐值 | 理由与限制 |
|---|---|---|
| **dev/本地验收常规启动** | **8 MiB** worker栈作为需要止血时的明确值 | std默认2MiB已在本项目失败；社区有2/4MiB崩、约8MiB活的真实对照。保持当前dev优化语义，便于暴露本地大future；8MiB是有案例依据的容量选择，不是本项目已验证安全线 |
| **prod紧急止血/初次根修发布** | **16 MiB暂定兜底档**，owner批准后使用 | release的sizeof与峰值尚缺，生产进程级abort成本高；在8MiB社区收敛值上再加一档是**工程风险选择，不是上游证实本项目需要16MiB**。不能把双profile验收省掉 |
| **prod完成根修与度量后** | 优先评估收敛到 **8 MiB**；是否保留16MiB由owner依据证据决定 | 8MiB足够的条件不是“dev过了”，而是release实际峰值、并发资源与A–D都满足；降栈另提交/另发布。若无抬栈授权，根修仍须固定2MiB C层成立，不能悄悄依赖16MiB |
| **CI事故回归（B/C）** | **显式2 MiB，永不随dev/prod放宽** | 验证结构根修脱离大栈缓解；仓级32MiB环境不能覆盖此值 |

8MiB可以作为prod第一档更低成本选择，但在release未测清前，本方案倾向临时16MiB稳妥止血；差额成本见下。若owner选择prod8MiB，需先用同配置D层验证，不把本建议表当作已通过的容量测算。

### 7.2 正确的余量表述与容量判定

旧制品局部已知需求下界 `U + 487,928 > 2,060,792 B（约1.965 MiB）`；栈保护/其他帧使现场实际溢出。仅由该下界推算，在8MiB时“余量<约6.035MiB”，16MiB时“余量<约14.035MiB”——这仍是**余量上界，不是保证有这么多空余**。不能把增加6/14MiB容量偷换成已测量的安全余量。

实施应记录各profile/代表最重场景的实际最大总栈占用Q，以及构造/poll测量位置和峰值叠加关系，再用 `H=S-Q`评估。建议容量策略为 **S至少2×已测Q**，作为50%测量余量的工程标准，而非Rust官方要求：dev8MiB需Q_dev≤4MiB，prod16MiB需Q_release≤8MiB，prod降8MiB需Q_release≤4MiB。以上Q均**未验证**。若可用高水位手段不可靠，只能报告固定栈成功/失败所给的边界，不能伪造精确Q；A–D仍必须独立通过。编译器/依赖/目标平台变化后重新采样，不能跨版本保证。

### 7.3 配置形态与资源代价

- `src/main.rs:1`目前裸 `#[tokio::main]`；宏不支持`thread_stack_size`，要代码保证必须改成手写多线程Builder，保留`enable_all`及原CLI输出、错误退出语义，不顺手改worker数。只是环境止血则在新进程启动前设置RUST_MIN_STACK，不能运行中set_var；显式Builder值优先于该变量。
- 默认runtime worker数通常随CPU核数；本次不改变它。栈配置也用于runtime共用的blocking pool生成线程，资源评估必须数**实际创建**的worker+blocking线程，而不只workers，也不能把blocking最大容量误当全部已分配。
- 对N条受影响实际线程，8MiB总虚拟栈预留约N×8MiB，16MiB约N×16MiB；从2MiB升8MiB增加N×6MiB，从8升16增加N×8MiB。例如32个worker仅worker部分分别约256/512MiB，增量192/448MiB（相对2MiB）；blocking另计，guard/对齐开销另计。
- Linux通常按需触页，虚拟预留不等于立即等量RSS，但实际使用、触页、平台提交策略会带来物理内存成本。不能承诺“RSS不受影响”，也不把/proc的单一VmStk字段当全部线程的精确总高水位；至少结合线程数、映射/进程资源观察与实际RSS。
- 独立OS线程每run扩栈的资源放大远高于共享worker抬栈；stacker动态栈则增加切栈/平台维护成本，均不是本次首选。
- dev `opt-level=1`在社区有多个有效个案，但会改变编译速度、可调试性及布局基线；panic=abort改变进程故障语义，不能只为消除一次搬移与现有unwind合同一起切换。两者保留为owner独立决策，不合入默认修复。

## 8. 上游与社区依据：结论到决策的映射

下列URL由D2/D3的一手资料核查承接；本次不重新联网取证。源码/官方合同强于社区个案，个案不得泛化成本项目效果保证。省略号链接已展开为可点击完整地址。

| 来源 | 可采信结论及证据等级 | → 对应决策 |
|---|---|---|
| [Tokio1.52.3 spawn源码](https://raw.githubusercontent.com/tokio-rs/tokio/tokio-1.52.3/tokio/src/task/spawn.rs)、[runtime阈值](https://raw.githubusercontent.com/tokio-rs/tokio/tokio-1.52.3/tokio/src/runtime/mod.rs) | 源码：按size阈值无条件box；2048/16384B。spawn不同步poll的合同另见[官方spawn文档](https://docs.rs/tokio/1.52.3/tokio/task/fn.spawn.html) | → §1不依赖自动box救构造；§3把大body构造放新任务首次poll |
| [tokio#6057](https://github.com/tokio-rs/tokio/issues/6057)、[PR#6826](https://github.com/tokio-rs/tokio/pull/6826) | 上游复现/修复：历史release-only裸spawn崩，调用点Box存活；后改为无条件阈值装箱。不是1.52.3仍缺自动box | → §4保留调用点Box最小候选，不先验判无效；§6双profile |
| [mitmproxy维护者评论](https://github.com/tokio-rs/tokio/issues/6057#issuecomment-2331098720) | 实务个案：依赖微调可令Windows release越界；临时patch Tokio阈值panic盘点大future | → §6.5及§10可选全仓扫描；升级后门禁，不发布patched Tokio |
| [Async Book状态机说明](https://rust-lang.github.io/async-book/07_workarounds/04_recursion.html)、[Rust2026 async future优化目标](https://goals.rust-lang.org/2026/async-future-memory-optimisation.html) | 官方：await child进入状态机，布局优化仍在演进；此处引用布局原理，非说本缺陷递归 | → §3必须间接存储，不能仅提async fn；§2/§6量真实类型 |
| [Swatinem：The size of Rust Futures](https://swatinem.de/blog/future-size/)、[moka#212](https://github.com/moka-rs/moka/issues/212) | 汇编/type-sizes个案：参数/嵌套future放大、Box后仍可能有约2倍临时帧 | → §2分开F/构造/poll；不把装箱当语言层原位保证 |
| [Forum109571](https://users.rust-lang.org/t/stack-overflow-in-async-main-function-due-to-excessive-stack-allocation-not-from-recursion/109571) | 社区实测：多处Box边界后main栈约280K→90K，含type-sizes教学 | → §3拆分+多边界推荐，不承诺本项目同幅收益 |
| [Forum117224](https://users.rust-lang.org/t/causes-for-thread-tokio-runtime-worker-has-overflowed-its-stack/117224)、[kornel“趁小装”](https://users.rust-lang.org/t/causes-for-thread-tokio-runtime-worker-has-overflowed-its-stack/117224/8) | 社区：Linux2/4MiB崩、约8MiB活；太晚Box可能来不及；平台崩点不同 | → §7 dev8MiB依据；§3小capture/引用传参；不推导4MiB一定能救本项目 |
| [Rust std线程栈官方文档](https://doc.rust-lang.org/std/thread/#stack-size) | 官方：Tier-1当前默认2MiB；主线程非Rust决定；显式stack_size覆盖RUST_MIN_STACK，启动后修改可能无效 | → §6显式2MiB；§7部署启动前配置、不误说主线程也随变量改变 |
| [Tokio Builder](https://docs.rs/tokio/1.52.3/tokio/runtime/struct.Builder.html#method.thread_stack_size)、[builder源码](https://raw.githubusercontent.com/tokio-rs/tokio/tokio-1.52.3/tokio/src/runtime/builder.rs)、[main宏](https://docs.rs/tokio/1.52.3/tokio/attr.main.html)、[runtime模型](https://docs.rs/tokio/1.52.3/tokio/runtime/index.html) | 官方/API源码：默认未显式栈；可手写Builder；main宏无栈选项；连接task在work-stealing池，不是每连接一线程 | → §3保持独立任务；§7是否放弃main宏及按实际线程数估资源 |
| [blocking pool源码](https://raw.githubusercontent.com/tokio-rs/tokio/tokio-1.52.3/tokio/src/runtime/blocking/pool.rs)、[spawn_blocking合同](https://docs.rs/tokio/1.52.3/tokio/task/fn.spawn_blocking.html) | 官方/源码：blocking线程消费同一栈配置，未配不自动更大；已启动工作不能abort | → §4拒绝用spawn_blocking当流式run大栈捷径 |
| [Microsoft M-ASYNC-STACK-SIZE](https://github.com/microsoft/rust-guidelines/blob/main/src/guidelines/performance/M-ASYNC-STACK-SIZE.md) | 官方工程指南：跨await内联状态、size_of_val断言模板与热路径约束 | → §2预算、§6A未box断言；不只量指针 |
| [Clippy large_futures](https://rust-lang.github.io/rust-clippy/master/index.html#large_futures)、[配置默认值](https://doc.rust-lang.org/clippy/lint_configuration.html#future-size-threshold)、[lint源码](https://github.com/rust-lang/rust-clippy/blob/master/clippy_lints/src/large_futures.rs)、[讨论#16702](https://github.com/rust-lang/rust-clippy/discussions/16702) | 上游：默认16384B/配置键；Box建议及诊断局限要按本地版本核实，不能只按最新网页认定门禁有效 | → §6.5对照实验后启用，与A互补；§10全仓推广另批 |
| [Forum125212](https://users.rust-lang.org/t/stack-overflow-in-debug-even-after-stack-increase-ok-in-release/125212) | 社区debug-only个案，large_stack_frames定位，opt-level=1救活 | → §1双profile；§10 dev优化仅独立候选 |
| [Forum129819](https://users.rust-lang.org/t/stack-overflow-with-panic-unwind-but-ok-with-panic-abort/129819)、[多处Box后2MiB够用](https://users.rust-lang.org/t/stack-overflow-with-panic-unwind-but-ok-with-panic-abort/129819/4)、[bjorn3解释](https://users.rust-lang.org/t/stack-overflow-with-panic-unwind-but-ok-with-panic-abort/129819/5) | 社区个案：unwind/abort、优化、版本可翻转；unwind1GiB不够而abort2MiB够的观察不是Rust对全部Box的保证 | → §7不默认改panic；§3多边界优先；§11本项目效果仍待测 |
| [rust#53827](https://github.com/rust-lang/rust/issues/53827)、[Box::pin签名](https://doc.rust-lang.org/std/boxed/struct.Box.html#method.pin)、[StackOverflow77934697](https://stackoverflow.com/questions/77934697/will-boxnew-make-a-copy-from-stack-to-heap) | 上游问题/API/汇编个案：按值Box可能先栈构再搬；优化不是语义保证 | → §1不承诺调用点Box一定原位；§4测caller/Box/spawn全过程 |
| [rust#107358](https://github.com/rust-lang/rust/issues/107358) | 多方复现：rustc1.66→1.67、tracing等微调可诱发；SIGABRT终止测试进程 | → §6子进程隔离与版本漂移门禁，不以catch_unwind兜底 |
| [wg-async Krustlet叙事](https://rust-lang.github.io/wg-async/vision/submitted_stories/status_quo/alan_runs_into_stack_trouble.html) | 官方工作组案例：select/pin_mut栈固定可拉回大future | → §3取消helper接boxed drive；保留select语义，不把raw巨型值重新pin在栈 |
| [测试线程/bench差异](https://stackoverflow.com/questions/42955243/cargo-test-release-causes-a-stack-overflow-why-doesnt-cargo-bench)、[Tokio test宏](https://docs.rs/tokio/1.52.3/tokio/attr.test.html) | 社区/官方宏合同：测试线程通常2MiB；block_on根不自动到worker | → §6.2纠正“所有current_thread=8MiB主线程”及32MiB假绿 |
| [stacker文档](https://docs.rs/stacker/latest/stacker/)、[nightly type-sizes](https://doc.rust-lang.org/unstable-book/compiler-flags/print-type-sizes.html)、[cargo-call-stack适用范围](https://github.com/japaric/cargo-call-stack) | API/官方不稳定flag/工具自述：动态栈、remaining_stack可用；无已核实Tokio组合示例；静态全栈工具对std/间接调用有限 | → §4不贸然引入动态切栈；§11实施可选测量工具，不把nightly当正式产品前置 |
| [Cargo Profiles](https://doc.rust-lang.org/cargo/reference/profiles.html#default-profiles)、[Linux proc手册](https://man7.org/linux/man-pages/man5/proc.5.html) | 官方构建语义；Linux资源观察的一般依据，D3未逐字核OS栈提交细节 | → §1区别默认dev与正式release；§7虚拟内存/RSS按实测，不能承诺零成本 |

以上包含D3的十个实际案例：tokio#6057、mitmproxy、商业代码生成服务、Osterman、shaw、rust#107358、rsland、rust#53827、Krustlet、Sentry/moka。**没有任何案例替代本项目未做的release测量或修后2MiB真WS验证。**

## 9. 对独立复核六条硬修正的采纳

六条全部采纳；本节不是对修复实现的复审通过声明。D2在当时未看到主设计最终稿，因此不能声称“独立评审已批准本文”。

| 硬修正 | 采纳结论与理由 | 本文落点及可检查证据 |
|---|---|---|
| ① 提成独立async fn不保证父future变小，followups宏也要量 | **采纳。** child直接await仍内嵌；互斥臂不按大小简单相加；分文件不是边界 | §1.2、§2、§3明确逐臂Box、task/drive两层及followups边界；A层测未box父/child而不是指针 |
| ② 整体Box不能绝对判无效，须核Tokio和profile | **采纳，并细化统一表述。** 1.52.3已按2048/16384自动box，时机晚于调用点构造；手动Box有上游有效案例但无原位保证 | §1.2、§1.3、§4保留候选，对本项目dev/release实际净收益都标未验证；不再沿用D1“单独无效”的绝对结论 |
| ③ force32MiB掩盖小栈，多线程根future不一定worker | **采纳。** 栈大小与实际执行线程是两条独立条件；libtest线程不是保证8MiB主线程 | §6.1–6.3显式2MiB Builder、rt.spawn真实入口、子进程隔离；仓级force单独决策；生产抬栈不影响C层 |
| ④ 128KiB余量算错，只有下界不能证明安全 | **采纳。** 正确为<256KiB且可非正；2×F只是旧debug实测，U没有上界 | §2给出通用公式、旧帧剩余上界<36,360B与128KiB预算依据等级；§7给Q/H实测判据，不称>1.2MiB |
| ⑤ 三种守卫不能混用，否则重引r16死锁 | **采纳，并纠正“drop drive即释放idle guard”的含混表述。** engine锁、idle depth与实体删除准入是不同资源 | §3.3–3.4逐个列所有权/正常/强制取消时点，保留先drop drive解engine再兜底的顺序；§6.4以barrier检查token、depth、entity_busy |
| ⑥ 复现脚本exit0不等于通过 | **采纳。** 旧脚本close/timeout都exit0且截断帧，无法证明provider启动和durable进展 | §5.2四条件同时成立；§6C/D独立核服务进程、provider-start、完整帧、durable与再生成；拒绝locked冒充实际启动 |

D3个别表述作证据等级收敛，而非照抄扩大保证：`Box::pin`“可直接堆构造”在本文改为需本项目codegen实测；panic策略的个案解释不升级为所有Rust程序的定律；“8MiB社区可活”不等于本项目release已过；“未来布局通常更小”不代替双profile数字。这些限制与六条采纳一致。

## 10. owner决策项与推荐取舍

下表都是**待owner裁决的实施/部署边界**，本文不给自己授权，也不因未裁决停止写完设计。无争议根修方向是§3，不能把可选仓级变化悄悄纳入必需项。

| 决策项 | 可选方案与代价 | 本文推荐/实施前置 |
|---|---|---|
| 是否先止血再根修 | 仅根修减少一次部署，但事故窗口延续；先独立抬栈再根修多一次发布/回退管理 | 如需立即恢复业务/验收，先止血再根修；止血独立制品/配置，不称永久修复 |
| prod栈8还是16MiB | 8有社区实际对照、成本较低；16增加每线程8MiB预留但提供额外兜底，暂无本项目必要性实测 | release峰值未知阶段临时16；Q与D证明8足够后单独降；owner若首选8先测同配置D。两者均不豁免2MiB C层 |
| dev栈与本地优化 | 8MiB容量缓解；本crate opt-level=1可减少部分codegen膨胀，但改变基线、编译/调试体验 | dev先选8MiB作为运行兜底；保持现opt-level测根修，是否opt-level=1另提交，不能以其替代体积门禁 |
| 是否放弃 `#[tokio::main]` 改手写Builder | 启动环境改动最小但依赖部署控制；Builder有代码保证、需审main语义且覆盖环境 | 紧急可先启动环境；长期若要求稳定两档栈则批准手写多线程Builder。宏无栈选项，不设计不存在的宏参数；保留enable_all与CLI退出/输出 |
| 是否动仓级 `RUST_MIN_STACK` force32MiB | 保留避免触发既有provider_matrix历史红；收回可恢复更多天然小栈暴露，但会扩大仓级验收 | 本次默认保留，用显式2MiB回归对抗；收回/按需限域另立可回滚配置单元，保留原provider_matrix测试 |
| 是否改panic策略（含dev/prod分开） | abort可能减少某些构造搬移，但改变可恢复panic/展开析构/进程失败语义；unwind维持现合同 | **本次不改panic**。若owner选择试验，先清点现有catch_unwind/线程panic恢复依赖、双profile实测，再单独批准；不能借栈缺陷顺手切换 |
| 是否顺带扫全仓大future | 临时patch Tokio在超阈值处panic可盘点spawn参数；Clippy更广但可能有现存债务/Box盲点；扩大整改会延长P0 | 先完成provider-run根修；同次可批准只读/临时盘点，整改另案。明确Tokio阈值dev2KiB/release16KiB而非安全线，不保留patched依赖 |
| Clippy启用范围和阈值 | 受影响模块先启用易控；仓级开启覆盖广但需审现存大future；仅-D warnings未必启用lint | 本次模块级先核语义，默认16KiB用于扫描；A硬断言独立保留。仓级推进与异常处理规则由owner确认，不全局allow/抬阈值 |
| 是否采用128KiB预算 | 把它当安全证明不成立；当原始future硬上限配真实WS回归可执行；更严阈值会增加拆分分配 | 外壳32KiB、body/child128KiB硬线，逐对象实测后尽量冻结更小值；预算上调必须有新栈账与owner裁决 |
| 是否连带压inbound/改锁/actor/专用runtime | 可能进一步减U/锁占用，但涉及消息串行、断线与资源归属；当前只完成已知点方案 | 本次不纳入；若§3已完整实施仍无法过2MiB真实WS，以测量证据重新申请范围，不偷偷把生产栈门槛放大 |
| 正式验收profile与平台 | 只release省事但漏事故debug；只Linux也不能证明Windows/macOS | debug+release均必须；正式支持平台按现发布矩阵验证制品。额外跨平台扩大范围由owner安排，不把Linux数字泛化 |
| 回滚控制面 | 双实现flag保留已知P0路径且业务合同会漂移；独立制品/提交回退边界更小 | 不保留旧巨型驱动，保留可单撤栈缓解；回退到已验证不立即abort的制品；撤缓解单独批准 |

若拒绝所有抬栈，仍有完整的结构根修路线，不以额外配置为强制前置。若只批准紧急抬栈，交付只能叫“止血”，A–D/拆分未完成前不能关闭根因缺陷。

## 11. 未验证清单、实施门禁与本次接地范围

### 11.1 仍需产生的证据

下列全部**未验证**，不得用本设计代替实现验收；编号便于后续报告逐项填真实结果，不是本任务遗留代码占位。

| 编号 | 未验证内容 | 完成它所需证据/责任阶段 |
|---|---|---|
| U1 | 当前release的sizeof(F)、是否复现、构造/搬移/poll峰值 | 正式release制品测量；与事故debug分栏，记录toolchain/target/依赖/flags，不能从opt-level="s"推断 |
| U2 | 机械具名提取后的红基线是否保持原243,880B与崩溃形态 | S0记录提取前后实际类型/构造路径；若布局变化如实说明，不虚称必红 |
| U3 | 整体调用点Box在本项目dev/release的净收益 | 比较caller、Box构造、spawn、poll各层及2MiB真实WS，不只看spawn帧变小 |
| U4 | 小外壳/浅栈boxed body的实际capture与构造峰值 | A测raw wrapper/body/entry/helper，B/C/D测线程落点与峰值；确认大body确实在首次任务poll才构造 |
| U5 | 拆后每臂/followups最大raw future及子树热点 | 按§2逐个size_of_val、必要时获准用nightly type-sizes；128/64KiB是预算/目标，非预测结果 |
| U6 | 上游链U的精确高水位、上界与新的k/C，及容量Q | 不重做D1根因；实施后采样构造与poll峰值并标重叠关系。现仅>1.5MiB下界，不把估计写安全证明 |
| U7 | 固定2MiB真实WS在新实现双profile的结果 | §6C子进程包含axum→socket→inbound及受支持provider真实start/完整帧/durable；B不能代替C |
| U8 | 新函数lifetime/Send与guard/drop布局、ArmExit可编译性 | 实施编译与barrier测试；保持MutexGuard按值位于取消域，不以改Owned guard/锁时点偷换方案 |
| U9 | 三守卫、旧新token、completion、scratch、receiver在取消/接力全部分支的实际时序 | §6.4完整矩阵，尤其250ms窗口内/超时、删除entity_busy、depth轨迹与新run不误伤 |
| U10 | Clippy当前stable可用性、实际量到裸future还是box指针、既有告警规模 | §6.5对照与当前源码/版本核验；未核实前不称已建立有效lint门禁 |
| U11 | dev8/prod16或8的本项目容量与并发内存成本 | 同实际启动配置D测量、线程数/VM/RSS与Q；16MiB是临时工程推荐不是实测需要 |
| U12 | 新文件最终行数与旧祖父登记回收 | 实施后按content.lines().count()验全部新文件≤1200，目标≤1000；provider_run.rs现1676/cap1678余2行，events现1257/cap1257零余量（D2与现源码锚） |
| U13 | 子进程精确测试全路径、测试模式防递归、超时与进程组清理 | 实施后生成真实--exact名称，分别演练断言失败、超时、异常signal的父报告行为；不虚构现已存在的测试命令 |
| U14 | 发布平台差异、环境profile/栈覆盖与实际安装制品 | 固定指纹和启动环境，正式支持平台按制品验证；Cargo force不自动作用于直跑二进制，显式Builder优先 |
| U15 | opt-level=1或panic=abort在本项目的效果/行为成本 | 仅owner批准后独立实验/仓级回归；当前方案不依赖这两项成立 |

### 11.2 文件与行数的实施约束

`tests/it_core/large_file_guard.rs`现合同已定点核对：默认1200行；provider_run.rs登记1678、events登记1257；新路径回落1200，不继承旧cap；登记严格排序且唯一，空行/注释计入。拆完旧文件≤1200就同一可回滚提交删除旧登记，**不加新祖父条款**。不得先把helper追加到仅余2行/零余量旧文件再计划将来拆。

§5.3的P1–P7由同一个根修集成owner顺序集成；其共享调度实现位于拟新增`provider_run/task_body.rs`。因此**P2–P7的写集也必须包含该文件**以接入新arm/followups调用，不能误把表中仅列provider_run.rs理解为“分发仍在旧大块”或并行独占授权。provider_run.rs负责子模块声明/准入外壳，task_body.rs负责drive分发；这两项归同一集成owner，不产生多写冲突。P7的父模块文件始终指实际已存在的`src/web/workspace_ws_handler/run.rs`，不是目录内拟造的`run/run.rs`。此段是切片写集的明确补充，接口与执行顺序不变。

测试新增文件使用现有path子模块/include模式，先移动r16及其adapter到新文件腾空间再注册新子模块，移动原测试不删assert。C/D新文件在已存在的`tests/it_core/workspace_ws_integration.rs`注册；不向现live超大harness追加大量内容。P8如果将events降到≤1200，其旧登记删除由P7/门禁集成owner同次最终收口，不能让两个worker争写登记表。对测试断言/注释的必要修正须说明事实依据，不以行数压力缩窄覆盖。

### 11.3 本文实际做了什么、没有做什么

- **已读：** D1、D2全文与D3全部相关章节；成功读取H1转录。D3实际在主检出目录，未复制/改写任何原报告。
- **定点接地：** 通过CodeGraph和指定范围读取核对现`spawn_provider_run_with_start_mode`前奏/分发/取消尾部、`cancel_settle.inc.rs`、legacy include、两个followups宏及具名outline函数、ProviderRunKind与ProviderRunContext、idle守卫/实体lease定义、r16 fixture、真实WS fixture、行数guard、Cargo/force环境、main、release/CI入口。没有寻找新根因，也未重新运行gdb、objdump、服务或复现驱动。
- **承接而非新增观测：** 243,880B、487,928B、>1.5MiB与2MiB事故事实来自D1；manager/更多回归锚点以D2定点证据注明。未把报告建议或本方案预算写成已测结果。
- **只交付此文档：** 未改src、spec/tasks、Cargo/CI/Clippy配置或原报告；未运行cargo/测试/构建/格式化；未访问共享验收、部署数据；未git add/commit。按要求分批落盘，后半通过bash `cat >>`追加，避免单次大输出丢失。
- **停止条件：** 设计文档落盘即完成本次委派，不执行§5实施步骤。本文状态为 **DONE_WITH_CONCERNS：设计交付完成，效果与owner可选决策待实施/裁决**；不是“缺陷已修复”，不是“测试已通过”，也不表示独立复核已审过本文最终稿。
