# provider run 爆栈根修 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在不改变 WS provider run 业务、取消、接力与持久化合同的前提下，消除深 inbound 栈构造巨型 future 的风险，并建立未装箱体积、固定 2 MiB worker、真实 WS 与双 profile 制品四层回归。

**Architecture:** 保留一个独立 run task；入口只构造小 capture 外壳，在其首次 poll 内构造 boxed task body，再以 boxed drive、逐臂与 followups 间接边界截断状态机。engine锁按值归取消域，idle guard与实体lease保留各自外层生命周期；ArmExit仅显式传递原非局部return。根修后独立提交手写runtime Builder，以编译期常量分档debug/dev 8 MiB、release/prod 16 MiB，无运行期配置；生产仅发布release，两档都不豁免2 MiB回归。

**Tech Stack:** Rust 2024、Tokio（Cargo.lock 锁定 1.52.3）、axum 0.8、tokio-tungstenite 0.29、现有 StreamingProviderAdapter/LifecycleStore、libtest 子进程隔离、tempfile、Clippy。

**Spec:** cadence/designs/2026-11-06_技术方案_provider_run爆栈_拆分与体积预算_v1.0.md

## Global Constraints

- 本 Plan 是获批方案的展开，不是新架构提案；实施顺序为 **S0→S2→S3→S4→S5，不含 S1 抬栈止血**。main Builder 在根修之后独立提交，不提前用它把红基线变绿。
- 工作树固定为 `/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`；实施者开始前读取本 Plan、Spec 与其中 D1/D2/D3。此文档编写阶段不修改产品代码、spec/tasks，不运行 cargo，不暂存；以下命令均是未来实施者的步骤。
- `src/web/workspace_ws_handler/run/provider_run.rs` 当前 **1676** 行、cap **1678**，仅余 **2** 行；不得先追加 helper。`src/web/workspace_ws_handler/tests/provider_run_events.rs` 当前 **1257** 行、cap **1257**，零余量；新测试放新文件，先移动 r16 用例与 adapter 腾注册空间。
- **任何新文件最终 ≤1200 行（硬），目标 ≤1000 行（软）**；提取后落在 1000～1200 行也必须继续拆，不可将行数留作后续优化；禁止上调 cap、禁止给新文件登记祖父项。代码行数按 `content.lines().count()`，空行/注释也算；每个新建文件任务都执行 `wc -l` 实测并以 large_file_guard 为准。旧文件一旦降到 ≤1200，**同提交删除登记项**；不能推迟到整轮结束。
- 首个原子切片仅机械提取，行为零变化、不改业务逻辑、不引入新业务抽象；`ArmExit` 仅编码原 `return` 的作用域。先提基础+SC author 一族；**仅当**所得首个 task-body 文件仍 >1000 行，才把 P6 SC manual 臂作为条件前置提取。不得把约1360行原块整搬为一个超限新文件。
- 三种守卫不可混用：engine `MutexGuard` 按值属于 drive/当前臂，drop 取消 drive 必须释放它，且先于兜底重取锁；idle `provider_drive_guard` 是外层 `Option<WorkspaceProviderDriveGuard>`，臂借 `&mut`，drop drive **不等于**释放 idle；`entity_run_lease` 是独立外层 binding，覆盖 drive、取消兜底与最终 store 写，接替 run 自行另 acquire 新 lease。正常路径 idle `.take()` 在 finish 前；强制取消路径保留到兜底/finish 后随外壳退出释放。保留注释 **“r16 现场死锁根修，缺陷#10家族”**，不得用“provider_drive_guard 随 drive 一律释放”的歧义说法代替三者合同。
- 保留 acquire entity lease→enrollment→attachment supersede/idle claim→选 provider→注册 token/唯一 command_rx→begin idle→mark started 的准入顺序；先取消旧 run、再等 engine 锁。保留250ms settle窗口与1s接替预算；不改锁粒度，不换 OwnedMutexGuard，不把每臂另 spawn，不用 drop JoinHandle 冒充取消。
- 外壳未装箱 future **≤32768 B**；task body、drive、每个臂、followups、指定重量 child 的**未装箱** future **≤131072 B**，优先≤65536 B。按实测冻结逐对象更小阈值与有限余量；不得把所有对象机械放宽到128 KiB。入口/settle helper记录实测和变化；A层不得测 `Pin<Box<_>>` 的指针大小形成假门禁。
- `exit 0` 不算动态通过。每场景同时证明：①服务无 abort 且后续 HTTP/WS 可用；②可信 provider-start 探针或真实 adapter 日志证明实际启动（locked/登记 run 不算）；③完整 JSON 帧被解析且满足该场景因果偏序；④ledger/timeline/message/turn中本场景对应 durable 状态按预期进展；另须取消后能再生成。客户端 close/timeout、unsupported 拒绝、只收到初始 session_state 均判失败。
- SIGABRT 无法进程内捕获；所有易 abort 的 B/C 与构造压力回归由父进程隔离，父持续收 stdout/stderr、退出状态与墙钟超时；任何 signal、异常退出、超时均失败。子模式防递归，精确单例名称从 `--list` 产出后确认；不能用 catch_unwind、JoinError 或同进程 timeout 兜底。
- 保留 `.cargo/config.toml` 的 `RUST_MIN_STACK={value="33554432",force=true}`。B/C 在同步子测试内显式 `Builder::new_multi_thread().worker_threads(1).thread_stack_size(2*1024*1024).enable_all()`，入口/accept/connection在 `rt.spawn` worker执行，block_on根只等待；记录线程 identity，不在 tokio test中嵌套runtime。显式值覆盖环境变量。
- 手写Builder的编译期常量 `WORKER_STACK_SIZE:usize`：`#[cfg(debug_assertions)]`为 **8 MiB（8388608 B）**，`#[cfg(not(debug_assertions))]`为 **16 MiB（16777216 B）**，不新增运行期配置。保留enable_all、原worker数与CLI退出/stdout/错误语义；仅 `aria web`启动时额外打印一行 `[aria-runtime] profile=debug|release worker_stack_bytes=<生效字节>`到stderr，非web CLI保持原stderr逐字不变。**prod只跑release制品**，D核指纹/profile/生效栈防误用dev制品；release降8另案，任何档位不豁免C层2 MiB，不改panic。
- 不改panic策略、Cargo现有dev/release优化参数，不新建 `ARIA_*` 配置、不保留旧巨型驱动flag；不纳入inbound/socket减栈、锁/actor/专用runtime改造。若完整方案仍不能过2 MiB真实WS，带测量证据回owner定范围，不放宽测试。
- Clippy先受影响模块启用，默认 **16384 B**扫描基准；A硬断言独立存在。同次**全仓只读盘点**按Tokio debug2048/release16384阈值报告，整改另案；不发布patched依赖，不为过lint全局allow或抬阈值。
- 方案§6.4既有断言可移动，不可删除/缩窄；尤其r16、ReviewOnly、SC revise/relay、Generate/Evaluate、completion/旧token scratch、idle和实体删除保护。typed waiting/AlreadyFinished/Superseded与普通失败不可统一处理，原提前返回不补通用finish/错误帧。
- 所有cargo命令必须串行：`flock /tmp/aria-cargo.lock cargo ...`；遵守仓库jobs配置，不加 `-j 1`。并行agent只编辑独立文件，交付后主控统一执行本任务所列红/绿步骤，禁止多人同时占target或检查半成品。
- 临时根使用 `tempfile::tempdir()`，自有空闲端口、自有进程组、持续读日志并清理自有进程；不读写 `/tmp/aria-verify`，不碰 `/tmp/aria-deploy`，不得重启共享服务。
- 中文注释/文档；每任务只显式 `git add <列出的文件>`，禁止 `git add -A`，不暂存用户其他改动。末尾提交命令仅供未来实施；本次写Plan不执行。
- 根修回退只能到**已验证不abort**的制品。无S1意味着不能默认旧版可用；发布前先验证回退候选，必要时隔离证明旧行为+16 MiB能用，不能回旧行为+默认2 MiB。不跨版本移交半个内存driver。

## Review Focus

1. **空 Author content 与重 plan/SC 输入**：都真正跨provider-start并产生durable进展；不能特判Story/空串，wrapper/child测未装箱值（任务1、2、9、10）。
2. **取消发生于provider start裸await、250ms窗口两侧**：先drop drive解engine，旧turn最多一次Failed回显，新run≤1s启动，idle/实体lease按各自边界释放（任务2、7、9）。
3. **typed waiting、fresh冻结反馈与RetryOnce/attempt2拒收**：零额外start、不改变turn/gate/预算，Stop不跑followups/不重复finish（任务3～7）。
4. **ReviewOnly重复relay与author→review→outline/draft接力**：单一在途run/receiver，token核验换channel，Generate让位而Evaluate驱动，旧finish不误删新scratch（任务8、9）。
5. **真实WS客户端close/timeout及异常子进程退出**：完整帧偏序、实际start、durable、HTTP/重连与取消后再生成缺一即失败；绝不因客户端exit0放行（任务1、10、13）。

## 执行约定与写集地图

### 阶段、切片与依赖

| 任务 | 方案追溯 | 独立交付/依赖 | 上限 |
|---|---|---|---|
| 1 | S0 + P1机械部分/P2，条件P6 | 可测构造点、机械拆分、隔离夹具；非可部署修复 | 4h |
| 2 | S2 / P1 | 小外壳、boxed body/drive；依赖1 | 3h |
| 3 | S3 / P2余量 | 基础/SC author边界与分支完整性；依赖2 | 3h |
| 4 | S3 / P3 | legacy outline三族；依赖3 | 3h |
| 5 | S3 / P4 | draft/batch；依赖4 | 3h |
| 6 | S3 / P5 | plan revision宏迁移；依赖5 | 3h |
| 7 | S3 / P6 | SC manual全合同；依赖6（已前置则完成边界/测试，不重提取） | 4h |
| 8 | S3 / P7 | followups、宏与登记回收；依赖7 | 3h |
| 9 | S4 / P8 | A/B完整门禁与生命周期；接口在2冻结，可与10的新文件编辑并行 | 4h |
| 10 | S4 / P9测试部分 | C/D真实WS及进程夹具；接口在2冻结，可与9新文件编辑并行 | 4h |
| 11 | S4 / P10 lint + P9 CI | lint核实/模块启用、双profile CI；依赖8～10 | 3h |
| 12 | owner只读盘点 | 仅报告全仓大future；依赖11工具语义，整改不在本Plan | 3h |
| 13 | S5 / P10 Builder | main长期配置独立提交；依赖8～11根修验收 | 2h |
| 14 | S5 | 双制品/发布回滚证据、运维说明；依赖12、13 | 4h |

共享文件 `provider_run.rs`、`task_body.rs`、`followups.rs`、`provider_run_events.rs`、`run.rs`、`large_file_guard.rs`由**同一集成owner串行修改**，任务1～8顺序提交。任务9与10仅其独立新文件可并行编辑；9用任务1已注册的unit文件、10用任务1已注册的integration文件，不允许各自争改注册点。CI/lint与main不混提交；所有cargo执行由主控串行，不把并行编辑误作并行验收授权。

新路径沿用现有 `#[path] mod`/`include!`，不是新调度层。源码行号均是本Plan读取基线，移动后按符号找，不用旧行号盲改。下列简称只用于文字：`R=src/web/workspace_ws_handler/run`，`U=src/web/workspace_ws_handler/tests/provider_run_events`，`I=tests/it_core/workspace_ws_integration`；Files和提交命令仍写精确路径。

### 固定接口（均为待实施新增，不冒称当前存在）

原入口保持：`spawn_provider_run_from_handler(ProviderRunContext, ProviderRunKind, mpsc::Sender<OutboundControl>) -> impl Future<Output=Result<(), String>>`、`spawn_provider_run_from_event(ProviderRunContext, ProviderRunKind, Option<String>, mpsc::Sender<OutboundControl>) -> impl Future<Output=Result<(), String>>`、`spawn_provider_run_claiming_idle(ProviderRunContext, ProviderRunKind, mpsc::Sender<OutboundControl>) -> impl Future<Output=Result<bool, String>>`。

为避免重复长签名，下表的**签名模板是精确展开规则，不新增Rust类型别名或参数包**。`G<'e>`仅表示 `tokio::sync::MutexGuard<'e, WorkspaceEngine>`；`P`表示 `Arc<dyn StreamingProviderAdapter>`；`Tx`表示 `mpsc::Sender<OutboundControl>`；`Rx`表示 `mpsc::Receiver<ProviderCommand>`；`Idle`表示 `WorkspaceProviderDriveGuard`。实现不得把这些记号写成新的万能上下文。

- `ArmExit<'e>`：仅 `Continue(tokio::sync::MutexGuard<'e, WorkspaceEngine>)` 与 `Stop`；Continue才运行下一段，Stop父drive立即返回。
- **Owned任务参数T（按此顺序）**：`run_context: ProviderRunContext, run_kind: ProviderRunKind, provider: P, outbound_tx: Tx, run_label: String, run_token: u64, run_cancel: CancellationToken, command_rx: Rx, provider_drive_guard: Idle, entity_run_lease: Option<crate::product::entity_operation_store::EntityRunLease>`。
- `provider_run_task(T) -> impl Future<Output=()> + Send + 'static`：普通fn返回未box外壳，内部首次poll才 `Box::pin(provider_run_task_body(T)).await`。
- `async fn provider_run_task_body(T) -> ()`：持lease/idle并取engine锁；构造boxed drive后调用既有取消helper。
- **臂公共参数A（按此顺序）**：`engine: G<'e>, provider: &'a P, command_rx: Rx, run_context: &'a ProviderRunContext, run_cancel: &'a CancellationToken, run_label: &'a str, run_token: u64, provider_drive_guard: &'a mut Option<Idle>, outbound_tx: &'a Tx`。
- 每个臂为 `async fn NAME<'e,'a>(A, 专属载荷) -> ArmExit<'e>`，只向实际选中臂移动receiver与variant载荷；共享上下文借用，禁止每臂clone整份 `ProviderRunContext`。
- `async fn provider_run_drive_body<'e,'a>(A, run_kind: ProviderRunKind) -> ()`：按值接engine guard；boxed分发→boxed followups→原正常尾部。
- `async fn drive_provider_run_followups<'e,'a>(engine: G<'e>, run_context: &'a ProviderRunContext, run_cancel: &'a CancellationToken, run_label: &'a str, run_token: u64, provider_drive_guard: &'a mut Option<Idle>, outbound_tx: &'a Tx) -> ArmExit<'e>`：不接entity lease，不把已消费receiver复制一份，继续在manager按token替换阶段channel。
- 既有 `drive_until_cancelled_settles<F>(drive:F, run_cancel:&CancellationToken, cancelled_revision_turn_id:Option<String>, engine_for_run:&Arc<Mutex<WorkspaceEngine>>, outbound_tx:&Tx, session_id:&str, run_label:&str, manager:&Arc<WorkspaceSessionManager>, run_token:u64) where F: Future<Output=()>`合同保持；任务2传入 `Pin<Box<impl Future<Output=()>>>`，其pin/select不重新内嵌raw drive。

`G`生命周期与Send可编译性**未验证**；必须通过真实编译证据解决签名标注，不得以换锁类型/取锁时点绕过合同。共享参数只在实际需要的臂使用；基础薄臂可保留同一A展开以冻结接口，后续若删未用参数须同提交迁移全部测试/调用者。

全部拟新增产品函数/ArmExit采用 `pub(crate)`可见性，不新增对外库API；测试通过 `#[cfg(test)]` 在provider_run.rs与run.rs逐级窄再导出这组真实raw构造函数，沿现run.rs:39-43先例供events测试访问。每次新增arm的任务同提交补对应两级再导出；不能写一个只返回Box的“测试专用假构造器”。这是所有任务Files中模块接线区的共同精确写集（`provider_run.rs:4-9`、`run.rs:39-43`），由同一owner串行编辑；每项提交在git commit之前额外显式 `git add src/web/workspace_ws_handler/run.rs`。测试调用的仍是产品实际构造函数。

### 通用测试命令与证据合同

下文测试名为**本Plan指定待创建名称**，不是已存在测试。非 `--exact` 的cargo过滤使用唯一叶名；先用 `flock /tmp/aria-cargo.lock cargo test --locked --lib -- --list` 或 `flock /tmp/aria-cargo.lock cargo test --locked --test it_core -- --list`取得实际全路径。父helper只运行current_exe的 `--list`并唯一匹配目标后调用 `--exact`，不得从测试中再启动cargo递归争锁；匹配0或多于1都失败。raw构造函数缺失会令整个lib测试编译失败，因此每次红测之后先补出本任务的真实机械接口再继续其余红/绿步骤，不把同一编译错误重复算成多个独立失败证据。

所有新文件任务在实现之后、提交之前增加同一行数步骤：`wc -l <本任务每个新文件>`，逐文件核对≤1200并争取≤1000；1000～1200继续拆成有职责边界子文件，新增路径同步写入测试/暂存清单；有差异时以Rust `lines().count()`门禁为准。随后运行 `flock /tmp/aria-cargo.lock cargo test --locked --test it_core large_file_guard -- --nocapture`；旧文件降到≤1200就本任务同提交删除对应登记（严格排序/唯一），不得留待P7。每任务提交以通过本任务可执行阶段门禁为前提；S0只允许记录原有失败，不能伪造“修前必红/阶段已治愈”。

## 任务序列

### Task 1：S0机械具名提取、原始体积基线与隔离测试入口

**Files:**
- Modify: `src/web/workspace_ws_handler/run/provider_run.rs:291-1676`（先CUT原体再注册新模块，不先追加）；`src/web/workspace_ws_handler/tests/provider_run_events.rs:1120-1257`（整段r16移出）；`tests/it_core/workspace_ws_integration.rs:1-12`（只注册新文件）。
- Modify: `src/web/workspace_ws_handler/run.rs:39-43`（仅测试raw构造函数窄再导出）；移动后task_body对既有legacy inc的相对路径由 `provider_run/work_item_plan_legacy_author.inc.rs`改为同目录 `work_item_plan_legacy_author.inc.rs`，内容不变，任务4再删除该文本路径。
- Modify: `tests/it_core/large_file_guard.rs:265,284-287`（仅删除已经降至≤1200的旧登记）。
- Create: `src/web/workspace_ws_handler/run/provider_run/task_body.rs`、`src/web/workspace_ws_handler/run/provider_run/basic_arms.rs`、`src/web/workspace_ws_handler/run/provider_run/single_candidate_arm.rs`；**条件Create**：`src/web/workspace_ws_handler/run/provider_run/sc_manual_revision.rs`。
- Create/Test: `src/web/workspace_ws_handler/tests/provider_run_events/stack_budget.rs`、`src/web/workspace_ws_handler/tests/provider_run_events/small_stack_worker.rs`、`src/web/workspace_ws_handler/tests/provider_run_events/supersede_reclaim.rs`。
- Create/Test: `tests/it_core/workspace_ws_integration/provider_run_small_stack.rs`、`tests/it_core/workspace_ws_integration/provider_run_process_smoke.rs`（本任务有真实父进程失效测试，不创建空文件占位）。
- Create: `cadence/reports/2026-11-06_验收报告_provider_run根修证据_v1.0.md`（持续增补A～D及U编号；不是新方案）。

**Interfaces:**
- Consumes: 现有三个spawn入口、`ProviderRunContext::test_fixture(...) -> Self`、`drive_until_cancelled_settles<F>(...)`、r16原测试和 `HangingStartProvider`。
- Produces: 上文 `provider_run_task_body(T) -> ()`（此步在原深caller直接构造，尚不加trampoline）；`ArmExit<'e>`；`drive_author_arm(A, content:String)`、`drive_author_choice_followup_arm(A, content:String)`、`drive_revision_arm(A)`、`drive_review_only_arm(A)`、`drive_single_candidate_author_arm(A)`，均async返回 `ArmExit<'e>`。条件前置则还产生 `drive_human_gate_sc_revision_arm(A, turn_id:String, prompt:String) -> ArmExit<'e>`。
- Produces（仅测试）: `run_isolated_provider_case(case_suffix:&str, timeout:std::time::Duration) -> Result<(), String>`位于small_stack_worker；父以current_exe+精确名+测试专用子模式启动。integration两个文件各自私有同合同helper，不跨测试二进制耦合。

- [ ] **Step 1：编写raw体积红断言。** `raw_task_body_baseline`构造未box `provider_run_task_body`，断言阈值131072并打印实际字节/profile/target/toolchain；同时量基础/SC author原始child，记录入口和settle helper大小。只构造不poll；重构前不可命名导致 `E0425 cannot find function provider_run_task_body` 是可接受第一红，不把它说成栈崩溃。
- [ ] **Step 2：运行红断言一次。** `flock /tmp/aria-cargo.lock cargo test --locked --lib raw_task_body_baseline -- --nocapture`；预期缺构造符号或 `raw future provider_run_task_body ... exceeds 131072`，记录真实结果。D1的243880B/487928B为原制品历史事实，机械提取后可能不同；若预算未红，如实记录并以C层事故证据作根因基线，禁止收紧到拍脑袋数值造红。
- [ ] **Step 3：只机械提取基础+SC author族与具名body。** 使用固定接口、把原正常穿过match映射Continue、原drive级return映射Stop；保留每分支原收口顺序，暂不改变Box布局/业务条件。为何先做：这是把1360行匿名块变成可独立测量对象的必要步骤；以后每拆一族才可沿用同一raw门禁逐对象比较。
- [ ] **Step 4：执行条件行数分支。** `wc -l src/web/workspace_ws_handler/run/provider_run/task_body.rs src/web/workspace_ws_handler/run/provider_run/basic_arms.rs src/web/workspace_ws_handler/run/provider_run/single_candidate_arm.rs`。**仅当**基础+SC author提取后task_body仍>1000，按同一ArmExit合同机械提取SC manual到上述条件文件；否则P6留任务7。任何新文件1000～1200继续在本原子切片内按已有职责拆分，仍禁止业务改动；不能提交一个>1200中间体。
- [ ] **Step 5：移动r16整段并挂载测试子文件。** 从events原1120～1257整体移入supersede_reclaim；保留原测试名、HangingStartProvider、注释、1s/starts断言。用释放的行数注册stack_budget/small_stack_worker/supersede_reclaim，events不得超1257。
- [ ] **Step 6：写父进程失败分类红测试。** unit写 `isolated_case_rejects_assertion_signal_and_timeout`，integration两文件分别写 `ws_isolated_case_rejects_assertion_signal_and_timeout`与 `process_isolated_case_rejects_assertion_signal_and_timeout`：仅测试专用子模式分别产生断言失败、Unix SIGABRT及超时，父必须得到Err并附exit/signal/stderr/case，普通成功子例得到Ok。两个integration文件各用具名内联子模块包裹，导入 `super::*`复用父fixture，避免现include!同一模块造成helper重名；每个新文件已有真实测试，不创建占位。
- [ ] **Step 7：运行隔离器红测。** `flock /tmp/aria-cargo.lock cargo test --locked --lib isolated_case_rejects_assertion_signal_and_timeout -- --nocapture`，再 `flock /tmp/aria-cargo.lock cargo test --locked --test it_core isolated_case_rejects_assertion_signal_and_timeout -- --nocapture`同时运行两个不同前缀integration测试；预期 `isolated child failure must be rejected`或未实现helper编译失败，不能把任意系统失败视为预期红。
- [ ] **Step 8：实现测试隔离器。** 同步父持续排空两条输出管道、墙钟超时kill+wait自有进程组；明确child模式防递归，测试列表唯一匹配后生成exact。子模式建显式2MiB runtime，真正入口rt.spawn，根只等待；不在父poll被测future。Unix signal6及所有其他signal同等失败，正常场景永不要求“必须abort才通过”。
- [ ] **Step 9：运行阶段绿并固化基线。** 串行运行隔离器测试与 `flock /tmp/aria-cargo.lock cargo test --locked --lib provider_run_events -- --nocapture`，既有行为套件全绿；raw基线不poll。若131072断言仍红，由同步父 `raw_task_body_baseline`隔离执行专用raw-budget子模式，父验其真实测量输出与预算失败分类并保存S0红值，仅作为characterization通过；不能把它冒充预算已通过。以后每族迁移的新增硬断言逐项转绿，任务9启用最终raw硬assert并移除这项历史预算失败的通过逻辑，不留ignore/should_panic。
- [ ] **Step 10：核对所有新文件行数并回收旧登记。** `wc -l`实测本任务Files全部新Rust文件；逐个≤1200，1000～1200继续拆；root provider_run和events降≤1200则立即删除其祖父登记，运行通用large_file_guard命令。文档记录原始与机械后字节/profile、真实WS红证据来源；不要求新B必能复现D1。
- [ ] **Step 11：提交机械原子单元。** 显式 `git add src/web/workspace_ws_handler/run/provider_run.rs src/web/workspace_ws_handler/run/provider_run/task_body.rs src/web/workspace_ws_handler/run/provider_run/basic_arms.rs src/web/workspace_ws_handler/run/provider_run/single_candidate_arm.rs src/web/workspace_ws_handler/tests/provider_run_events.rs src/web/workspace_ws_handler/tests/provider_run_events/stack_budget.rs src/web/workspace_ws_handler/tests/provider_run_events/small_stack_worker.rs src/web/workspace_ws_handler/tests/provider_run_events/supersede_reclaim.rs tests/it_core/workspace_ws_integration.rs tests/it_core/workspace_ws_integration/provider_run_small_stack.rs tests/it_core/workspace_ws_integration/provider_run_process_smoke.rs tests/it_core/large_file_guard.rs cadence/reports/2026-11-06_验收报告_provider_run根修证据_v1.0.md`；若触发条件前置，再单独 `git add src/web/workspace_ws_handler/run/provider_run/sc_manual_revision.rs`；`git commit -m "refactor: 机械提取provider运行构造点与隔离回归入口"`。

### Task 2：S2小capture外壳、boxed body/drive与取消所有权

**Files:**
- Modify: `src/web/workspace_ws_handler/run/provider_run.rs:291-310`对应spawn收口；`src/web/workspace_ws_handler/run/provider_run/task_body.rs`（任务1产生的构造/engine锁/分发/尾部区）；`src/web/workspace_ws_handler/run/provider_run/cancel_settle.inc.rs:17-60`（仅边界注释/必要类型适配）。
- Test: `src/web/workspace_ws_handler/tests/provider_run_events/stack_budget.rs`、`src/web/workspace_ws_handler/tests/provider_run_events/small_stack_worker.rs`、`src/web/workspace_ws_handler/tests/provider_run_events/supersede_reclaim.rs`。

**Interfaces:**
- Consumes: 任务1 `provider_run_task_body(T)`、`ArmExit`、五个臂签名与既有取消helper。
- Produces: `provider_run_task(T) -> impl Future<Output=()> + Send + 'static`与 `provider_run_drive_body<'e,'a>(A, run_kind:ProviderRunKind) -> ()`；冻结后续9/10可依赖的入口和取消域。task_body(T)签名不变。

- [ ] **Step 1：写外壳与浅构造红测试。** `raw_wrapper_is_at_most_32768_bytes`直接量provider_run_task的未box返回值；`body_is_not_constructed_before_wrapper_first_poll`用仅测试构造探针断言spawn调用侧count=0、task首次poll后count=1；`cancel_settle_grace_and_forced_drop_release_engine`以barrier分别覆盖250ms内自收口与超时drop，不用sleep猜时序。
- [ ] **Step 2：运行红测试。** 依次 `flock /tmp/aria-cargo.lock cargo test --locked --lib raw_wrapper_is_at_most_32768_bytes -- --nocapture`、同命令过滤 `body_is_not_constructed_before_wrapper_first_poll` 与 `cancel_settle_grace_and_forced_drop_release_engine`；预期缺wrapper符号、count在poll前非0或engine超时，记录实际红而非要求三项都必红。
- [ ] **Step 3：实现小外壳。** provider_run.rs只将T交给provider_run_task再spawn；provider_run_task的async块首次poll内才构造并box task_body，禁止spawn前构造body/直接body.await。外壳不额外clone上下文/enum。
- [ ] **Step 4：实现boxed取消驱动。** task_body持实体lease与外层idle Option、按现时点取engine并安装token；run_kind move前提取cancelled turn id和取消Sender clone；boxed drive按值拥有engine guard，再交既有settle helper。drive接收Continue才followups，Stop立即返回；保留r16中文注释与三种guard正常/强制取消不同顺序。
- [ ] **Step 5：运行阶段绿。** 重跑Step2三项与原r16用例过滤 `supersede_reclaims_engine_lock_from_run_hanging_outside_select`；assert旧start Drop已发生、接替≤1s、取消兜底能重新取engine锁、两条settle分支各有真实start/durable/帧与再生成。A记录wrapper/body/drive/entry/helper真实字节，尚未全拆的body超线不能宣布根修完成。
- [ ] **Step 6：运行行数门禁。** 本任务无新增文件；`wc -l src/web/workspace_ws_handler/run/provider_run.rs src/web/workspace_ws_handler/run/provider_run/task_body.rs`并执行通用large_file_guard；若旧文件首次降≤1200，本提交删除登记后再运行门禁。
- [ ] **Step 7：提交独立边界。** `git add src/web/workspace_ws_handler/run/provider_run.rs src/web/workspace_ws_handler/run/provider_run/task_body.rs src/web/workspace_ws_handler/run/provider_run/cancel_settle.inc.rs src/web/workspace_ws_handler/tests/provider_run_events/stack_budget.rs src/web/workspace_ws_handler/tests/provider_run_events/small_stack_worker.rs src/web/workspace_ws_handler/tests/provider_run_events/supersede_reclaim.rs`；必要登记回收显式加 `tests/it_core/large_file_guard.rs`；`git commit -m "fix: 在provider任务首次poll内构造boxed驱动"`。

### Task 3：P2基础族与SC author的真实Box边界和Stop合同

**Files:**
- Modify: `src/web/workspace_ws_handler/run/provider_run/basic_arms.rs`、`src/web/workspace_ws_handler/run/provider_run/single_candidate_arm.rs`（任务1迁自provider_run.rs:333-401、407-572）；`src/web/workspace_ws_handler/run/provider_run/task_body.rs`（对应分发）；`src/web/workspace_ws_handler/tests/provider_run_events/small_stack_worker.rs`（注册新测试）。
- Create/Test: `src/web/workspace_ws_handler/tests/provider_run_events/basic_sc_arm_cases.rs`。
- Test: `src/web/workspace_ws_handler/tests/provider_run_events/stack_budget.rs`。

**Interfaces:**
- Consumes: 任务1五个具名臂 `drive_author_arm(A, content:String)`、`drive_author_choice_followup_arm(A, content:String)`、`drive_revision_arm(A)`、`drive_review_only_arm(A)`、`drive_single_candidate_author_arm(A)`，async均返回 `ArmExit<'e>`；任务2 `provider_run_drive_body(A, run_kind:ProviderRunKind) -> ()`。
- Produces: 以上签名不变，drive的五个选择点改为实际选中臂 `Box::pin(...).await`；Continue/Stop合同被冻结，后续不得另造错误枚举。

- [ ] **Step 1：写本族体积/行为测试。** `raw_basic_sc_children_respect_budget`逐个量未box五臂≤131072；`sc_author_stop_does_not_start_followups`覆盖AlreadyFinished、Superseded、Message、AdmissionWaiting、NativeSupersededWaiting：原分支start数量、phase/reservation、Error/指导帧、finish次序保持；ReviewOnly两路durable Completed之后才发本turn Completed，迟到已完turn不重复。空content Author真正start且有输出/durable消息；Revision receiver只移动一次。
- [ ] **Step 2：运行红反馈。** `flock /tmp/aria-cargo.lock cargo test --locked --lib raw_basic_sc_children_respect_budget -- --nocapture`与过滤 `sc_author_stop_does_not_start_followups`；体积预期失败格式 `raw future <symbol> <bytes> exceeds 131072`，不是承诺每个提取后都超线。若行为锚已绿，保留为迁移保护；若尺寸也绿，以测试中具名raw构造接口尚未开放的编译红及任务1原始预算红为TDD证据，不人为破坏业务造红。
- [ ] **Step 3：加本族实际分发Box。** 只给被选中的arm分配；保持SC child `single_candidate::run_single_candidate_author`合同，先测该child，超131072才在其调用点继续box；无依据不Box每个小await。引入的测试可见性仅 `pub(super)`/`#[cfg(test)]`窄导出，不做公共API。
- [ ] **Step 4：审计本族提前出口。** 保留原Message分支failed node→drop engine→idle take→finish→Error，AlreadyFinished/Superseded不新发错误、不改durable；Admission/Native等待不generic Failed、不重置reservation；Stop父层不再finish/followups。仅按值传回engine guard，不能让外层永久持锁。
- [ ] **Step 5：运行绿。** 重跑Step2两项与 `flock /tmp/aria-cargo.lock cargo test --locked --lib provider_run_events -- --nocapture`；两个ReviewOnly路径/空Author/choice/revision均有真实start或该typed拒绝规定的零start证据，取消后另一个有效run可生成；raw child全部≤131072并记录更小的实测预算。
- [ ] **Step 6：核行数并同步回收登记。** `wc -l src/web/workspace_ws_handler/tests/provider_run_events/basic_sc_arm_cases.rs`，≤1200且>1000继续拆；也复测task_body/basic/single_candidate三文件，执行通用large_file_guard，任何旧登记达标本提交删除。
- [ ] **Step 7：提交。** `git add src/web/workspace_ws_handler/run/provider_run/basic_arms.rs src/web/workspace_ws_handler/run/provider_run/single_candidate_arm.rs src/web/workspace_ws_handler/run/provider_run/task_body.rs src/web/workspace_ws_handler/tests/provider_run_events/basic_sc_arm_cases.rs src/web/workspace_ws_handler/tests/provider_run_events/small_stack_worker.rs src/web/workspace_ws_handler/tests/provider_run_events/stack_budget.rs`；`git commit -m "fix: 截断基础与SC author臂future并保留停止语义"`。

### Task 4：P3 legacy outline三族与Box重建上下文

**Files:**
- Create: `src/web/workspace_ws_handler/run/provider_run/legacy_outline.rs`。
- Modify: `src/web/workspace_ws_handler/run/provider_run.rs:4-9`（子模块声明）；`src/web/workspace_ws_handler/run/provider_run/task_body.rs`（原provider_run.rs:295-302、319-322、402-406）；`src/web/workspace_ws_handler/tests/provider_run_events/small_stack_worker.rs`。
- Remove: `src/web/workspace_ws_handler/run/provider_run/work_item_plan_legacy_author.inc.rs:1-545`（迁完无调用后删除）。
- Create/Test: `src/web/workspace_ws_handler/tests/provider_run_events/legacy_outline_cases.rs`；Test: `src/web/workspace_ws_handler/tests/provider_run_events/stack_budget.rs`。

**Interfaces:**
- Consumes: `ArmExit<'e>`、A、任务2 drive；现有 `commit_rebuilt_snapshot_after_provider_start(&ProductAppPaths,&ResolvedPlanningContext,&Result<ProviderSession,ProviderAdapterError>) -> bool`、`complete_work_item_plan_outline_author_from_output(&mut WorkspaceEngine,&str) -> impl Future<Output=Result<WorkItemPlanAuthorOutcome,String>>`。
- Produces: `async fn drive_legacy_outline_arm<'e,'a>(A, outline_revision_feedback:Option<String>, is_outline_revision_run:bool, rebuild_context:Option<Box<ResolvedPlanningContext>>) -> ArmExit<'e>`；三个variant移入精确载荷：Legacy=(None,false,None)，Revision=(feedback,true,None)，Rebuild=(None,false,Some(rebuilt))。

- [ ] **Step 1：写失败测试。** `raw_legacy_outline_future_respects_budget`量未box新arm及outline启动/输出最大child≤131072；`legacy_outline_stop_and_rebuilt_context_contract`覆盖三种variant、原include:271-278输入失败、AutoRevision超上限、typed fresh等待及正常完成：Stop后start计数不增加、错误帧/mark/finish仍按原分支；Rebuild不clone裸ResolvedPlanningContext、以 `as_deref()`借现Box，provider start失败不commit snapshot，成功才commit。
- [ ] **Step 2：运行红。** 两个叶名分别用 `flock /tmp/aria-cargo.lock cargo test --locked --lib <叶名> -- --nocapture`执行；预期 `cannot find function drive_legacy_outline_arm`，随后真实raw超131072时保留字节红。普通行为已绿不冒称失败。
- [ ] **Step 3：迁移真实函数并接Box。** 从旧include移动完整内容与背景，原drive级return→Stop、穿出loop后→Continue，原break仍只退出相应loop；删除共通裸 `(**rebuilt).clone()`和feedback clone，按variant移动载荷。保留原五次AutoRevision上限及每轮新split handle，不顺手调整retry。
- [ ] **Step 4：删除旧文本路径。** task_body改boxed具名调用并移除include；旧inc整个删除，不保留别名、兼容宏或flag。嵌套child只有实测超预算才继续细拆，改变的是间接边界不是算法。
- [ ] **Step 5：运行绿。** 重跑Step2两项与既有provider_run_events套件；拒收场景零后续start、正常三族真实start，durable snapshot/outline/message符合预期，取消后可再生成。
- [ ] **Step 6：执行行数门禁。** `wc -l src/web/workspace_ws_handler/run/provider_run/legacy_outline.rs src/web/workspace_ws_handler/tests/provider_run_events/legacy_outline_cases.rs`；新文件≤1200且>1000继续拆，旧provider_run若首次≤1200本提交删其登记；运行通用large_file_guard。
- [ ] **Step 7：提交。** `git add src/web/workspace_ws_handler/run/provider_run.rs src/web/workspace_ws_handler/run/provider_run/task_body.rs src/web/workspace_ws_handler/run/provider_run/legacy_outline.rs src/web/workspace_ws_handler/run/provider_run/work_item_plan_legacy_author.inc.rs src/web/workspace_ws_handler/tests/provider_run_events/small_stack_worker.rs src/web/workspace_ws_handler/tests/provider_run_events/legacy_outline_cases.rs src/web/workspace_ws_handler/tests/provider_run_events/stack_budget.rs`；必要登记回收显式加guard文件；`git commit -m "refactor: 将legacy outline文本臂迁为boxed函数"`。

### Task 5：P4 draft/batch循环、fresh冻结载荷与RetryOnce

**Files:**
- Create: `src/web/workspace_ws_handler/run/provider_run/plan_draft.rs`、`src/web/workspace_ws_handler/run/provider_run/plan_batch.rs`。
- Modify: `src/web/workspace_ws_handler/run/provider_run.rs:4-9`、`src/web/workspace_ws_handler/run/provider_run/task_body.rs`（原provider_run.rs:573-1004）、`src/web/workspace_ws_handler/tests/provider_run_events/small_stack_worker.rs`。
- Create/Test: `src/web/workspace_ws_handler/tests/provider_run_events/draft_batch_cases.rs`；Test: `src/web/workspace_ws_handler/tests/provider_run_events/stack_budget.rs`。

**Interfaces:**
- Consumes: A、ArmExit、任务2 drive、既有 `lc_fresh_recovery_record_if_running(&ProductAppPaths,&str) -> Result<Option<LcResumeRecoveryRecord>,String>`；既有 `start_work_item_plan_author(launch:PlanAuthorLaunch,provider:Arc<dyn StreamingProviderAdapter>,input:StreamingProviderInput,cancel:CancellationToken,plan_split_run:Option<&PlanSplitRunContext>) -> impl Future<Output=Result<ProviderSession,ProviderAdapterError>>`原接口不变。
- Produces: `async fn drive_plan_draft_arm<'e,'a>(A, feedback:Option<String>) -> ArmExit<'e>`、`async fn drive_plan_batch_arm<'e,'a>(A) -> ArmExit<'e>`；两者仅Continue交回engine，原return映射Stop。

- [ ] **Step 1：写失败测试。** `raw_draft_batch_children_respect_budget`分别量两个raw arm、provider start与output child≤131072；`fresh_draft_batch_payload_and_retry_once_are_preserved`用durable冻结feedback与刻意不同的内存pending构造输入，断言使用冻结值、resume=None，RetryOnce按原预算循环且每轮新split handle，不新建业务generation/batch；`draft_batch_early_stop_does_not_finish_twice`覆盖load/build/start/output/parse失败与AwaitConfirmation，逐分支核原tail是否可达，不能统一补finish。
- [ ] **Step 2：运行红。** 三个叶名各用 `flock /tmp/aria-cargo.lock cargo test --locked --lib <叶名> -- --nocapture`；预期新函数未定义或raw超131072，记录实际失败；行为对照迁移前先绿作为保护。
- [ ] **Step 3：机械迁移draft并加boxed调用。** feedback/receiver只移动一次；保留 `Option<Option<String>>` 区分“fresh冻结为空”与“非fresh”的语义；原:`730-736`只mark后return的差异原样映射Stop，不借拆分修注册/错误协议。
- [ ] **Step 4：机械迁移batch并加boxed调用。** 同batch fresh不新建batch/不重置repair预算；AwaitConfirmation仍feedback=None继续原loop，与draft break的差异不统一。测量超线的start/output child才在对应调用点设额外Box。
- [ ] **Step 5：运行绿。** 重跑Step2三项、provider_run_events既有suite；成功路径start/完整帧/durable进展成立，typed拒绝零start不改预算，取消后恢复运行；分别记录两个arm和指定child字节。
- [ ] **Step 6：执行行数门禁。** `wc -l src/web/workspace_ws_handler/run/provider_run/plan_draft.rs src/web/workspace_ws_handler/run/provider_run/plan_batch.rs src/web/workspace_ws_handler/tests/provider_run_events/draft_batch_cases.rs`，≤1200、>1000继续拆；达标旧登记同提交删除并跑通用guard。
- [ ] **Step 7：提交。** `git add src/web/workspace_ws_handler/run/provider_run.rs src/web/workspace_ws_handler/run/provider_run/task_body.rs src/web/workspace_ws_handler/run/provider_run/plan_draft.rs src/web/workspace_ws_handler/run/provider_run/plan_batch.rs src/web/workspace_ws_handler/tests/provider_run_events/small_stack_worker.rs src/web/workspace_ws_handler/tests/provider_run_events/draft_batch_cases.rs src/web/workspace_ws_handler/tests/provider_run_events/stack_budget.rs`；`git commit -m "refactor: 拆分draft与batch异步驱动并保留fresh循环"`。

### Task 6：P5 plan revision宏迁移与非局部return

**Files:**
- Create: `src/web/workspace_ws_handler/run/provider_run/plan_revision.rs`。
- Modify: `src/web/workspace_ws_handler/run/followups.rs:103-684`（删已迁宏）；`src/web/workspace_ws_handler/run/provider_run.rs:4-9`；`src/web/workspace_ws_handler/run/provider_run/task_body.rs`（原provider_run.rs:1005-1019）；`src/web/workspace_ws_handler/tests/provider_run_events/small_stack_worker.rs`。
- Create/Test: `src/web/workspace_ws_handler/tests/provider_run_events/plan_revision_cases.rs`；Test: `src/web/workspace_ws_handler/tests/provider_run_events/stack_budget.rs`。

**Interfaces:**
- Consumes: A、ArmExit、任务2 drive；现有 `legacy_plan_revision_payload_matches_recompute(payload_retained:&[LcRetainedPlanItemRef],payload_redo:&[LcRecoveryRedoSpec],retained:&[LifecycleWorkItemRecord],redo_specs:&[RedoSpec]) -> Result<(),String>` 保留位置与合同。
- Produces: `async fn drive_plan_revision_arm<'e,'a>(A, feedback:Option<String>) -> ArmExit<'e>`；不再存在 `workspace_ws_work_item_plan_revision_arm!`定义/调用，其他followups宏在任务8迁完前仍可用。

- [ ] **Step 1：写失败测试。** `raw_plan_revision_future_respects_budget`量raw arm/最大启动child≤131072；`plan_revision_fresh_mismatch_stops_without_rebuild`断言冻结retained序列、redo old_id/feedback任一漂移typed stale、零provider start、不转OutlineRebuild、turn/gate/预算保持；`plan_revision_stop_keeps_original_tail_semantics`覆盖普通失败、typed等待、AutoRevision、正常Confirm的后续start与finish/错误帧计数。
- [ ] **Step 2：运行红。** 三个叶名逐个 `flock /tmp/aria-cargo.lock cargo test --locked --lib <叶名> -- --nocapture`；预期 `cannot find function drive_plan_revision_arm`或实际字节超131072；不把原已有typed拒收失败当新根因。
- [ ] **Step 3：迁移整个revision宏。** 按现模板建立具名函数，Box调用，原宏return逐个对应Stop；保留原循环上限与fresh冻结payload消费，不能重算反馈归一化，不能新增retry/恢复fallback。
- [ ] **Step 4：清除旧宏定义和调用。** followups保留仍用的helper/另一个宏；不保留宏别名，避免两套实现漂移。按实际新文件行数决定是否把纯prepare段拆入独立子模块，仍须具名async/Box才构成状态边界。
- [ ] **Step 5：运行绿。** Step2全部与既有suite通过，raw arm/child≤131072，typed mismatch零start，成功/失败各原durable与完整帧因果保持，取消后可再生成。
- [ ] **Step 6：执行行数门禁。** `wc -l src/web/workspace_ws_handler/run/provider_run/plan_revision.rs src/web/workspace_ws_handler/tests/provider_run_events/plan_revision_cases.rs`，≤1200、>1000继续拆；followups若有旧祖父项且降≤1200同提交删项（当前读到1113行，无需新增项），跑通用guard。
- [ ] **Step 7：提交。** `git add src/web/workspace_ws_handler/run/followups.rs src/web/workspace_ws_handler/run/provider_run.rs src/web/workspace_ws_handler/run/provider_run/task_body.rs src/web/workspace_ws_handler/run/provider_run/plan_revision.rs src/web/workspace_ws_handler/tests/provider_run_events/small_stack_worker.rs src/web/workspace_ws_handler/tests/provider_run_events/plan_revision_cases.rs src/web/workspace_ws_handler/tests/provider_run_events/stack_budget.rs`；`git commit -m "refactor: 以显式Stop合同替代plan revision宏"`。

### Task 7：P6 SC manual同turn attempt、冻结claim与取消回显

**Files:**
- Create（任务1未触发条件前置时）/Modify（已前置时）: `src/web/workspace_ws_handler/run/provider_run/sc_manual_revision.rs`（原provider_run.rs:1020-1639）。
- Modify: `src/web/workspace_ws_handler/run/provider_run.rs:4-9,77-127`（必要窄导出不改变常量/helper合同）；`src/web/workspace_ws_handler/run/provider_run/task_body.rs`；`src/web/workspace_ws_handler/tests/provider_run_events/small_stack_worker.rs`。
- Create/Test: `src/web/workspace_ws_handler/tests/provider_run_events/sc_manual_cases.rs`；Test: `src/web/workspace_ws_handler/tests/provider_run_events/stack_budget.rs`、`src/web/workspace_ws_handler/tests/provider_run_events/supersede_reclaim.rs`。

**Interfaces:**
- Consumes: A/ArmExit；现有 `deliver_attempt2_frozen_prompt(&mut WorkspaceEngine,&HumanGateTurn,&str,&ScRevisionClaimCarry) -> Result<String,String>`与 `fail_cancelled_human_gate_revision_turn(&Arc<Mutex<WorkspaceEngine>>,&Tx,&str,&str) -> impl Future<Output=bool>`；任务2取消helper。
- Produces: `async fn drive_human_gate_sc_revision_arm<'e,'a>(A, turn_id:String, prompt:String) -> ArmExit<'e>`；已前置时签名不变，本步补Box/最大child预算和完整行为而非重复迁移。

- [ ] **Step 1：写体积/attempt红测试。** `raw_sc_manual_future_respects_budget`测arm及最大prompt/启动/输出/turn完成child≤131072；`sc_manual_attempt2_claim_is_consumed_once`断言最多2次真实start、同turn、一次claim、attempt2使用冻结prompt、每次start ledger与实际探针对应；二次拒收/重放不新claim、不第三start、不错误提升候选。
- [ ] **Step 2：写typed拒绝与取消红测试。** `sc_manual_waiting_and_delivery_rejection_preserve_turn`覆盖Native mismatch、Attempt2Claimed重入、join拒收/登记失败：零额外start，turn/gate/预算保留，原错误码不变，失败中间artifact基线还原；`sc_manual_cancel_at_start_and_followup_is_durable_once`在provider start和进入review前barrier取消，两种settle分支都durable Failed(cancelled)、失败回显≤1、后续可新turn/生成。
- [ ] **Step 3：运行红反馈。** 四个叶名逐个 `flock /tmp/aria-cargo.lock cargo test --locked --lib <叶名> -- --nocapture`；未前置预期符号缺失，已前置预期预算或新增探针合同不满足。若原行为本就通过，记录保护绿，不虚称必红；真正TDD红承接raw预算/新增边界接口。
- [ ] **Step 4：迁移或补完真实Box边界。** 当前variant载荷只移动一次；保留普通prompt/fresh同turn重装、Running先于start、grant/claim与真实start分账、failure/Completed回显顺序；原loop最多2 attempts不变。实测超线child才继续Box；不能用Box<String>宣称省掉prompt全文体积。
- [ ] **Step 5：审计所有Stop与取消外层。** turn_id的取消兜底副本在run_kind移动前保留；entity lease不进入arm；`provider_drive_guard`仅借Option在原take点释放；原provider_err先fail/drop/send或break差异保留，不统一成一个通用cleanup。
- [ ] **Step 6：运行绿。** 重跑Step3、r16及provider_run_events既有suite；检查实际2MiB worker、start探针/同turn durable ledger/完整帧/取消后再生成，raw全部≤131072，容量不以32MiB环境混淆。
- [ ] **Step 7：执行行数门禁。** `wc -l src/web/workspace_ws_handler/run/provider_run/sc_manual_revision.rs src/web/workspace_ws_handler/tests/provider_run_events/sc_manual_cases.rs`；≤1200且1000～1200继续拆，旧文件达标同提交删登记，跑通用guard。
- [ ] **Step 8：提交。** `git add src/web/workspace_ws_handler/run/provider_run.rs src/web/workspace_ws_handler/run/provider_run/task_body.rs src/web/workspace_ws_handler/run/provider_run/sc_manual_revision.rs src/web/workspace_ws_handler/tests/provider_run_events/small_stack_worker.rs src/web/workspace_ws_handler/tests/provider_run_events/sc_manual_cases.rs src/web/workspace_ws_handler/tests/provider_run_events/stack_budget.rs src/web/workspace_ws_handler/tests/provider_run_events/supersede_reclaim.rs`；`git commit -m "fix: 截断SC人工修订future并保持同turn取消合同"`。

### Task 8：P7 followups接力、正常尾部与旧宏清退

**Files:**
- Create: `src/web/workspace_ws_handler/run/provider_run/run_followups.rs`。
- Modify: `src/web/workspace_ws_handler/run/provider_run.rs:4-9`；`src/web/workspace_ws_handler/run/provider_run/task_body.rs`（原1641-1655）；`src/web/workspace_ws_handler/run/followups.rs:686-993`（原宏整段删除，具名outline函数995-1113保留）；`src/web/workspace_ws_handler/run.rs:7-14`（删失效macro_use，保留真实helper导入）；`src/web/workspace_ws_handler/tests/provider_run_events/small_stack_worker.rs`。
- Modify: `tests/it_core/large_file_guard.rs:265,284-287`（最后确认两旧登记已在达标提交回收，不能等本步才补历史漏项）。
- Create/Test: `src/web/workspace_ws_handler/tests/provider_run_events/followup_cases.rs`；Test: `src/web/workspace_ws_handler/tests/provider_run_events/stack_budget.rs`。

**Interfaces:**
- Consumes: 所有前序臂→ArmExit与任务2 drive；既有 `drive_current_work_item_plan_outline_run(engine:&mut WorkspaceEngine,provider:P,run_cancel:CancellationToken,command_rx:&mut Rx,app_paths:&ProductAppPaths,session_record:&WorkspaceSessionRecord) -> impl Future<Output=Result<WorkItemPlanAuthorOutcome,String>>`。
- Produces: 固定接口 `drive_provider_run_followups(engine:G<'e>,run_context:&'a ProviderRunContext,run_cancel:&'a CancellationToken,run_label:&'a str,run_token:u64,provider_drive_guard:&'a mut Option<Idle>,outbound_tx:&'a Tx) -> impl Future<Output=ArmExit<'e>>`；drive boxed调用，Continue后执行原正常mark→drop engine→idle take→finish，Stop不执行尾部。

- [ ] **Step 1：写接力红测试。** `raw_followups_and_nested_children_respect_budget`量raw followups、reviewer gateway/传统两路、具名outline、draft start/output≤131072；`followups_keep_one_token_and_abort_current_receiver`分别在author→review、review→outline、outline→draft barrier发Abort，唯一当前provider收到、manager token未偷换；同节点relay不额外start。
- [ ] **Step 2：写委托/早退红测试。** `followups_generate_yields_evaluate_drives_once`覆盖SC Generate让位、Evaluate恰一次review、legacy不受SC守卫影响；`followups_stop_preserves_original_failure_tail`针对原925-931只mark后return及其它带finish出口分别断言，不统一补finish/失败帧。保留ReviewOnly不顶杀在途followups、SC revise仅一次review、SC author relay仍supersede三组既有assert。
- [ ] **Step 3：运行红。** 四个叶名各 `flock /tmp/aria-cargo.lock cargo test --locked --lib <叶名> -- --nocapture`；预期新followups符号缺失或raw预算超线；已有行为绿作为对照保留。
- [ ] **Step 4：迁移宏为boxed函数。** 只有实际followup阶段才构造对应child；manager `replace_command_tx_if_token(run_token, tx)`原位保留，新Rx仅一个消费者。调用既有outline具名函数时显式Box；reviewer/draft最大child若超预算在该点继续设边界。全部仍在drive同一取消范围，不另spawn发帧。
- [ ] **Step 5：删除无效宏与接线。** 清除 `workspace_ws_provider_run_followups!`及失效macro_use；run.rs不是新造的run/run.rs；保留followups.rs实际函数/导入，不留旧文本别名/双实现flag，原正常尾部只一份。
- [ ] **Step 6：运行绿。** Step3四项、provider_run_events与现manager completion/旧token scratch套件串行通过，完整接力有真实start、durable/帧进展，取消后再生成；A层所有指定对象均≤预算。
- [ ] **Step 7：行数与登记收口。** `wc -l src/web/workspace_ws_handler/run/provider_run/run_followups.rs src/web/workspace_ws_handler/tests/provider_run_events/followup_cases.rs`，≤1200且>1000继续拆；核provider_run/events旧登记不存在（若文件≤1200），新文件无祖父登记，跑通用large_file_guard。文件体积达标不代表future体积达标，两种门禁分开记录。
- [ ] **Step 8：提交。** `git add src/web/workspace_ws_handler/run/provider_run.rs src/web/workspace_ws_handler/run/provider_run/task_body.rs src/web/workspace_ws_handler/run/provider_run/run_followups.rs src/web/workspace_ws_handler/run/followups.rs src/web/workspace_ws_handler/run.rs tests/it_core/large_file_guard.rs src/web/workspace_ws_handler/tests/provider_run_events/small_stack_worker.rs src/web/workspace_ws_handler/tests/provider_run_events/followup_cases.rs src/web/workspace_ws_handler/tests/provider_run_events/stack_budget.rs`；`git commit -m "refactor: 将followups纳入boxed取消域并回收旧宏"`。

### Task 9：P8完整A/B门禁、三守卫与旧新run隔离

**Files:**
- Modify/Test: `src/web/workspace_ws_handler/tests/provider_run_events/stack_budget.rs`、`src/web/workspace_ws_handler/tests/provider_run_events/small_stack_worker.rs`、`src/web/workspace_ws_handler/tests/provider_run_events/supersede_reclaim.rs`（任务1已注册；不得并行改events父文件）。
- Create/Test: `src/web/workspace_ws_handler/tests/provider_run_events/lifecycle_guards.rs`（由small_stack_worker挂载）；必要只测可见性在任务2冻结时预留，若需改产品共享文件由集成owner串行接入，不交两个worker争改。
- Modify: `cadence/reports/2026-11-06_验收报告_provider_run根修证据_v1.0.md`的A/B、U4/U5/U8/U9/U12/U13证据区。

**Interfaces:**
- Consumes: 任务2 `provider_run_task(T)`、`provider_run_task_body(T)`、`provider_run_drive_body(A,ProviderRunKind)`，任务3～8全部raw臂/followups；任务1隔离helper `run_isolated_provider_case(&str,Duration)->Result<(),String>`。
- Produces: `raw_provider_run_futures_respect_frozen_budgets`、`provider_run_2mib_worker_matrix`、`real_run_guards_keep_depth_lease_and_new_token`、`supersede_reclaims_engine_lock_from_run_hanging_outside_select`（原名称保留）作为A/B常规门禁；不产新产品接口。

- [ ] **Step 1：将A层完整断言写为失败测试。** 构造未box wrapper/body/drive、10个具名arm（基础4、legacy1、SC author1、draft1、batch1、revision1、SC manual1）、followups以及指定重量child；wrapper≤32768、其余≤131072，记录入口/helper。断言消息包含符号/实际字节/profile/target/toolchain并以S0实测表对比，不测Box指针。legacy三个variant由同一arm类型覆盖，不能因为载荷字符串不同虚造三种体积。
- [ ] **Step 2：运行A红对照。** `flock /tmp/aria-cargo.lock cargo test --locked --lib raw_provider_run_futures_respect_frozen_budgets -- --nocapture`；最终根修已达标时应直接绿，如有超线预期 `raw future <symbol> ... exceeds <budget>`。用S0保存的raw基线结果验断言分类器会拒绝超限值（测试层纯数值输入，不污染产品）；不回退真实产品、不改阈值造假红。
- [ ] **Step 3：冻结逐对象预算。** 在32768/131072硬线上按debug/release较大实测保留有限余量，记下每个值及依据，不统一放宽。每个测试先 `size_of_val(&raw_future)`再可Box；A不poll，大构造本身仍可能有风险时也走任务1隔离helper。移除S0 characterization替代性放行，最终硬assert不得ignore/should_panic。
- [ ] **Step 4：写B矩阵与生命周期红测试。** `provider_run_2mib_worker_matrix`覆盖Author、choice、Revision、ReviewOnly两路、legacy三variant、draft/batch/revision、SC author/manual、event relay、claim busy/idle、全部followup；真实start探针验证worker identity≠根线程。`real_run_guards_keep_depth_lease_and_new_token`用真实owner实体、barrier控制双run overlap深度1→2→1→0；取消收口store barrier前删除entity_busy，completion不替代lease释放；旧finish后新token/command/scratch仍在，新run能继续/被Abort，全部写完并lease释放后才可删除。
- [ ] **Step 5：运行B红与隔离自检。** 逐个 `flock /tmp/aria-cargo.lock cargo test --locked --lib provider_run_2mib_worker_matrix -- --nocapture`、过滤 `real_run_guards_keep_depth_lease_and_new_token`、`isolated_case_rejects_assertion_signal_and_timeout`；预期新增probe/barrier未接全时出现 `provider start not observed on explicit 2 MiB worker` / `entity must stay busy before final writes`，不是要求旧代码必abort。产品已满足则记录迁移保护绿，不将异常close/超时视为通过。
- [ ] **Step 6：完成真实生命周期夹具。** HangingStartProvider加旧start-future Drop探针；两条settle窗口分支分别barrier验证，r16原1s与starts断言原样保留；SC取消turn实际Failed和回显≤1。删除使用真实实体store，不用None-owner；不得添加生产逻辑以迎合测试，若发现合同偏差交集成owner定位。
- [ ] **Step 7：运行debug/release A/B绿。** 依次 `flock /tmp/aria-cargo.lock cargo test --locked --lib raw_provider_run_futures_respect_frozen_budgets -- --nocapture`、同命令 `--release`；再分别debug/release过滤 `provider_run_2mib_worker_matrix`、`real_run_guards_keep_depth_lease_and_new_token`、`supersede_reclaims_engine_lock_from_run_hanging_outside_select`。覆盖方案§6.4原锚：events:248-374、748、1003、1088-1113；runs.rs:523/550/916-935；state.rs:1059-1081；anchor_and_trust_cases.inc.rs:57-132原assert保持。A/B不能替代C。
- [ ] **Step 8：执行新文件行数门禁。** `wc -l src/web/workspace_ws_handler/tests/provider_run_events/lifecycle_guards.rs`并复测修改的三个unit文件，≤1200、>1000继续拆；若任何旧文件达标同提交删登记，通用large_file_guard通过；完整记录A/B真实输出和剩余未验证。
- [ ] **Step 9：提交。** `git add src/web/workspace_ws_handler/tests/provider_run_events/stack_budget.rs src/web/workspace_ws_handler/tests/provider_run_events/small_stack_worker.rs src/web/workspace_ws_handler/tests/provider_run_events/supersede_reclaim.rs src/web/workspace_ws_handler/tests/provider_run_events/lifecycle_guards.rs cadence/reports/2026-11-06_验收报告_provider_run根修证据_v1.0.md`；`git commit -m "test: 固化provider未装箱预算与2MiB生命周期门禁"`。

### Task 10：P9真实WS C层与制品D层可执行场景

**Files:**
- Modify/Test: `tests/it_core/workspace_ws_integration/provider_run_small_stack.rs`、`tests/it_core/workspace_ws_integration/provider_run_process_smoke.rs`（任务1已注册，两个独立文件可与任务9编辑并行）。
- Read-only/复用: `tests/it_core/workspace_ws_integration/part_02.rs:209-283`、`part_04.rs:718-730`、`part_05.rs:640-650,914-925`、`part_05b.rs:31-77`；`tests/it_web/web_lc_gateway_multi_provider/harness.rs:12506-12531`只借合同，**不向其追加**。
- Modify: `cadence/reports/2026-11-06_验收报告_provider_run根修证据_v1.0.md`的C/D测试合同区（报告由集成owner串行汇入，避免与任务9抢写）。

**Interfaces:**
- Consumes: 原WS Hello/StartGeneration/Abort；现有 `recv_json(ws:&mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>) -> impl Future<Output=WsOutMessage>`与 `send_json(ws:&mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,message:&WsInMessage) -> impl Future<Output=()>`；`create_workspace_session_fixture_with_providers(root:&tempfile::TempDir,author_provider:&str,reviewer_provider:&str,review_rounds:u32) -> impl Future<Output=std::path::PathBuf>`、`with_available_provider_health(state:WebAppState,root:&std::path::Path)->WebAppState`，ScriptedStreamingProvider原接口不变。
- Produces: `provider_run_2mib_real_ws_matrix`同步父测试、`provider_run_2mib_real_ws_child`子模式；`provider_run_process_smoke_matrix`制品父测试。D由 `env!("CARGO_BIN_EXE_aria")`选当前profile真正aria制品，不以test binary代替aria；内联模块分别为 `provider_run_small_stack`与 `provider_run_process_smoke`。后者私有 `async fn run_provider_process_smoke(binary:&std::path::Path,workspace:&std::path::Path) -> Result<(),String>`负责自有服务、日志与证据，父线程只等待并读退出状态，避免同步helper中嵌套runtime。

- [ ] **Step 1：写C场景失败测试。** `provider_run_2mib_real_ws_matrix`分别Story空Author、实际A测量最重的plan路径、SC author/manual及review/outline/draft接力；显式2MiB、worker内accept/connection，客户端只驱真实WS深链。发送可信同源Origin与Hello Driver（last_seen_node_id=None、after_event_seq=None），等driver准入再StartGeneration；ClaudeCode等支持名注册Scripted替身，走真实validated start，不用Fake被unsupported拒绝冒充成功。
- [ ] **Step 2：写动态证据拒绝测试。** `ws_smoke_rejects_close_timeout_and_missing_durable_progress`在夹具侧提供close、timeout、仅locked、无provider-start、无durable五种证据，必须全部Err；真实成功序列必须解析完整JSON并验证locked先于本run输出、durable Completed先于其completed回显、取消Failed≤1，按session/turn/node/run关联，不断言不同Sender全局相邻总序。
- [ ] **Step 3：运行红并采S0对照。** `flock /tmp/aria-cargo.lock cargo test --locked --test it_core provider_run_2mib_real_ws_matrix -- --nocapture`与过滤 `ws_smoke_rejects_close_timeout_and_missing_durable_progress`；未完成夹具预期 `trusted provider start missing`/`durable progress missing`/`child signal`。原始真实WS事故红承接D1；新增C若在机械提取后已绿，如实记录，不虚构必SIGABRT。
- [ ] **Step 4：完成C真实场景。** 持续读完整帧、保存场景前后durable；provider至少一次start且预期输出终态，随后同连接Abort/新StartGeneration，再访问 `/api/health`并重连WS确认可用；所有close/timeout立即失败并保留子stderr。fixture使用新临时根和真实LC/owner准备，缺trust/health即修测试准备，不放宽产品拒绝。
- [ ] **Step 5：写D进程失败测试。** `provider_run_process_smoke_matrix`按真实CLI启动 `aria web --workspace <tempdir> --host 127.0.0.1 --port <自有空闲端口>`，独立进程组持续排日志。debug/release各真实制品执行同一C合同；D真实adapter日志或可信可控provider入口证明start，**不能**只依赖locked/registration。真实adapter环境缺CLI/鉴权/信任时明确未验证/阻塞，不fake成功，不擅自增加产品测试后门。
- [ ] **Step 6：运行D红分类与实现进程收口。** `flock /tmp/aria-cargo.lock cargo test --locked --test it_core provider_run_process_smoke_matrix -- --nocapture`；预期未实现harness时 `process evidence incomplete`、异常signal或缺真实adapter前置被明确报告。父持续排管道、超时终止并wait全自有进程组、记录binary sha256/profile/toolchain/flags/env/stack/stderr/完整帧/durable差异；无进程残留才完成。
- [ ] **Step 7：运行C/D双profile绿。** 串行以debug及 `--release`执行上面C和D两个叶名过滤；C固定2MiB，D此时尚未改main时记录实际启动栈，任务13后必须再跑debug8/release16并核对启动日志与binary指纹。四条件与取消再生成全部满足才通过；缺真实D前置则明确D未验证，任务14不得声称发布通过。
- [ ] **Step 8：执行行数门禁。** `wc -l tests/it_core/workspace_ws_integration/provider_run_small_stack.rs tests/it_core/workspace_ws_integration/provider_run_process_smoke.rs`（虽任务1已建，本步完整实现后重测）；≤1200、>1000立即按C/D职责细拆新文件并由单一owner注册/精确暂存，不向旧大harness塞内容；跑通用guard。
- [ ] **Step 9：提交。** `git add tests/it_core/workspace_ws_integration/provider_run_small_stack.rs tests/it_core/workspace_ws_integration/provider_run_process_smoke.rs cadence/reports/2026-11-06_验收报告_provider_run根修证据_v1.0.md`；`git commit -m "test: 增加2MiB真实WS与双制品进程回归"`。

### Task 11：P10模块级Clippy核实与P9双profile常规CI

**Files:**
- Modify: `src/web/workspace_ws_handler/run/provider_run.rs:1-9`（或等价模块属性位置，仅受影响模块启用 `clippy::large_futures`）；`src/web/workspace_ws_handler/run/followups.rs:1-2`（仅剩具名child纳入范围）；`.github/workflows/ci.yml:42-49`。
- Create: `clippy.toml`（`future-size-threshold = 16384`，不改Cargo profiles/panic/force环境）。
- Create/Test: `tests/it_core/provider_run_gate_configuration.rs`；Modify: `tests/it_core.rs:20-24`（现有path模块模式注册）。
- Modify: `cadence/reports/2026-11-06_验收报告_provider_run根修证据_v1.0.md`的U10与CI证据区。

**Interfaces:**
- Consumes: 任务9三个A/B门禁叶名、任务10 `provider_run_2mib_real_ws_matrix`、现ci fmt/clippy/check/test命令。
- Produces: `provider_run_gate_configuration_is_explicit`静态配置测试；CI在原debug全suite之外串行跑release A/B/C指定门禁。D是发布制品验收，不将真实CLI鉴权强塞到普通CI后静默skip。

- [ ] **Step 1：写配置失败测试。** `provider_run_gate_configuration_is_explicit`读取模块/CI配置，断言受影响模块明确启用large_futures、阈值16384、CI含debug/release A/B/C，所有cargo执行前有flock；同时断言 `.cargo/config.toml` force仍33554432、Cargo未改panic/opt-level、C/B代码显式2097152而非读env。只核配置，不能以此代替动态门禁。
- [ ] **Step 2：运行红。** `flock /tmp/aria-cargo.lock cargo test --locked --test it_core provider_run_gate_configuration_is_explicit -- --nocapture`；预期 `provider-run large_futures lint not enabled` / `release A/B/C gate missing` /缺clippy.toml，记录真实结果。
- [ ] **Step 3：验证本地Clippy语义。** 记录 `rustc --version --verbose`、`flock /tmp/aria-cargo.lock cargo clippy --version`；在tempdir独立最小crate做两个受控输入：未box且跨await存活的>16384B child必须触发 `clippy::large_futures`，boxed同child记录是否只看指针。命令统一flock且依赖复用锁定Tokio；不永久留下临时脚本/patch。若当前stable不可用，报告确切工具缺口，不谎称建立lint；A/C不得撤。
- [ ] **Step 4：启用模块lint与16KiB配置。** 不全仓deny、不全局allow、不把阈值抬到128KiB；受影响模块的裸大await按实测热点在已有授权文件继续边界处理，若发现需inbound/锁/actor改造停止向owner报证据。A仍在装箱前量raw类型，boxed lint恒绿不等于安全。
- [ ] **Step 5：接CI串行命令。** 现frontend build前置保持；把原fmt/clippy/check/test命令加 `flock /tmp/aria-cargo.lock`。保留 `cargo clippy --all-targets --all-features --locked -- -D warnings`（模块启用确保lint实际执行）；原 `cargo test --locked`覆盖debug A/B/C，另显式依次 `cargo test --locked --release --lib raw_provider_run_futures_respect_frozen_budgets`、`... --lib provider_run_2mib_worker_matrix`、`... --lib real_run_guards_keep_depth_lease_and_new_token`、`... --test it_core provider_run_2mib_real_ws_matrix`，每条均以flock前缀执行，不matrix并发同target。
- [ ] **Step 6：运行绿。** 重跑配置test；主控串行执行新增CI命令，lint负对照能红、实际模块lint绿、两profile A/B/C完整证据绿；既有debug全suite不删除/缩窄。D仍由任务14真实制品验证，不因CI绿声称D已过。
- [ ] **Step 7：核新文件行数。** `wc -l clippy.toml tests/it_core/provider_run_gate_configuration.rs`，≤1200、>1000继续拆；旧文件达标同提交删登记，运行通用large_file_guard。
- [ ] **Step 8：提交。** `git add src/web/workspace_ws_handler/run/provider_run.rs src/web/workspace_ws_handler/run/followups.rs .github/workflows/ci.yml clippy.toml tests/it_core.rs tests/it_core/provider_run_gate_configuration.rs cadence/reports/2026-11-06_验收报告_provider_run根修证据_v1.0.md`；`git commit -m "ci: 启用provider体积扫描与双profile小栈门禁"`。

### Task 12：同次全仓大future只读盘点，整改另案

**Files:**
- Read-only: `src/**`、`tests/**`、`Cargo.lock`、`Cargo.toml:53-67`、`.cargo/config.toml:1-16`及所用Tokio/Clippy精确源码；本任务不得修改任何产品/测试/配置文件。
- Create: `cadence/analysis-docs/2026-11-06_分析报告_全仓大future只读盘点_v1.0.md`（遵循文档存储规则的分析报告分类）；临时编译输出只在tempdir，正式制品不使用实验flags/依赖。

**Interfaces:**
- Consumes: 任务11已核实的lint测量位置、dev2048/release16384 Tokio阈值、A raw类型测量清单。
- Produces（报告schema，不新增产品类型）: 每条 `{symbol, source_path, line, profile, raw_bytes, tokio_threshold, boundary_kind, evidence_source, boxed_blind_spot, confidence}`；完整列出已发现超阈值类型、未覆盖路径、工具限制与另案整改建议，不把盘点称全仓整改。

- [ ] **Step 1：写盘点验收失败清单。** 报告列明必须有debug/release各一张表，按2048/16384分别筛；包含spawn参数与非spawn await child，区分raw类型和Box包装、给出source定位。当前缺这两组真实输出即报告验收红 `inventory missing profile/raw-size evidence`；这是只读任务的文档契约，不为满足TDD而创建产品测试。
- [ ] **Step 2：执行工具负对照。** 用任务11临时受控大future检查盘点确实能找出debug>2048但≤16384与release>16384两类，boxed包装不混成底层值；若只跑默认16KiB Clippy会漏debug2KiB～16KiB，必须明确报红，不能出“已扫全仓”结论。
- [ ] **Step 3：产出全仓编译器类型尺寸证据。** 使用方案允许的nightly `-Zprint-type-sizes`，在已有nightly可用时，于tempdir独立 `CARGO_TARGET_DIR`执行 `flock /tmp/aria-cargo.lock cargo +nightly rustc --locked --lib -- -Zprint-type-sizes`，再对 `--bin aria`、debug/release分别执行；测试专有future按 `flock /tmp/aria-cargo.lock cargo metadata --no-deps --format-version 1`列出的实际test target逐个编译盘点，不能猜target名。保留每条实际命令与toolchain，nightly数字不冒充stable产品数字；A与稳定Clippy诊断交叉标注。没有nightly则不擅自安装/切正式工具链，改用当前stable Clippy临时阈值2048和16384的全target扫描并明确Box/spawn/未执行分支缺口，完整盘点仍标未验证，不伪造字节数。
- [ ] **Step 4：补全稳定工具链边界核对。** 在tempdir只读材料/独立target做 `flock /tmp/aria-cargo.lock cargo clippy --locked --all-targets --all-features -- -W clippy::large_futures`及release对应命令，扫描阈值通过临时 `CLIPPY_CONF_DIR`指向tempdir配置，不改仓库clippy.toml。若用临时Tokio阈值诊断patch，只能在tempdir独立副本/依赖副本，绝不写回Cargo.lock/产品源码，不把其制品放入D；运行式扫描只覆盖实际执行路径，须列未执行区域，不能冒称静态穷尽。
- [ ] **Step 5：完成报告的绿验收。** 对照失败清单逐项填真实结果，所有超阈值发现都有profile/raw_bytes/符号来源；无法覆盖项明确未验证及补证命令，不能凭Box指针标安全。报告验收绿表示“证据与局限如实盘点”，不是“全仓未来均≤阈值”；没有可获得工具时状态DONE_WITH_CONCERNS，不阻止已完成根修，也不隐瞒owner要求的全仓证据缺口。
- [ ] **Step 6：核报告行数与只读边界。** `wc -l cadence/analysis-docs/2026-11-06_分析报告_全仓大future只读盘点_v1.0.md`，报告也保持≤1000便于阅读（代码门禁硬线针对代码）；只允许该报告入提交，删除自有临时材料，不修改/恢复用户文件，不以清理之名运行破坏性git。
- [ ] **Step 7：提交报告。** `git add cadence/analysis-docs/2026-11-06_分析报告_全仓大future只读盘点_v1.0.md`；`git commit -m "docs: 盘点全仓超Tokio阈值future并标注测量盲区"`。

### Task 13：根修后独立提交手写runtime Builder与编译期8/16MiB分档

**Files:**
- Modify: `src/main.rs:1-11`（唯一产品写集，与根修/Clippy/Cargo配置不混提交）。
- Create/Test: `tests/it_core/provider_run_runtime_entry.rs`；Modify: `tests/it_core.rs:20-24`（现有path模块注册）。
- Test: `tests/it_core/workspace_ws_integration/provider_run_process_smoke.rs`（复用，不改C层2MiB）。

**Interfaces:**
- Consumes: 现有 `cadence_aria::cli::run_cli_async<I,S>(args:I) -> impl Future<Output=Result<CliOutput,CliError>> where I:IntoIterator<Item=S>, S:Into<String>`（src/cli.rs:100-103）的原match行为；任务10D测试。
- Produces: 同步 `fn main() -> ()`、私有 `fn build_runtime() -> std::io::Result<tokio::runtime::Runtime>`与编译期 `WORKER_STACK_SIZE:usize`（debug_assertions=8388608，否则16777216）；new_multi_thread+thread_stack_size(WORKER_STACK_SIZE)+enable_all，不改worker数/线程名/blocking上限，沿用现CliOutput/CliError。仅web子命令启动日志输出profile与生效字节，无运行期配置面。

- [ ] **Step 1：写入口失败测试。** `provider_run_runtime_entry_keeps_cli_output_and_exit`以真正CARGO_BIN_EXE_aria运行clean temp workspace的daemon status，assert exit0/stdout精确 `daemon_not_found\n`、stderr与原基线相同；repl --no-start assert exit1/stdout空/stderr原错误逐字不变；空Text不打印。`provider_run_runtime_builder_is_explicit`检查无tokio::main、两项cfg常量8388608/16777216、enable_all及不改worker数。`provider_run_web_reports_profile_and_effective_stack`检查仅web启动有且仅一行栈日志，debug为8388608，release为16777216。
- [ ] **Step 2：运行红。** `flock /tmp/aria-cargo.lock cargo test --locked --test it_core provider_run_runtime_builder_is_explicit -- --nocapture`与过滤 `provider_run_web_reports_profile_and_effective_stack`；预期 `compile-time worker stack profile missing` / `effective runtime stack log missing`。CLI对照原行为先绿，不允许stdout/退出码/错误文本变化。
- [ ] **Step 3：实现私有Builder、同步main与web启动日志。** 保留原match输出/错误exit1；按编译期cfg选择WORKER_STACK_SIZE，build失败不fallback。将原env::args().skip(1)保留为可peek迭代器，仅首个子命令为web时打印 `[aria-runtime] profile=debug|release worker_stack_bytes=<字节>`，然后同一迭代器交原run_cli_async，不丢参数、不额外复制全argv、不改变其他CLI stderr。保留enable_all，不set_var、不加ARIA变量、不改panic/优化；prod只能用release制品。
- [ ] **Step 4：运行入口绿。** 配置test、CLI逐字对照与web栈日志test在debug/release均通过；再跑任务10 D，记录指纹/profile/debug8388608或release16777216且与启动行一致。C仍显式2097152，不能因配置分档改测试；prod候选若日志是debug或8MiB即NOT_READY，不把debug二进制部署生产。
- [ ] **Step 5：核新文件行数。** `wc -l tests/it_core/provider_run_runtime_entry.rs`，≤1200且>1000继续拆；跑通用guard。`.cargo/config.toml`/Cargo.toml不在暂存清单，保留force32MiB和panic/profile基线。
- [ ] **Step 6：提交独立配置单元。** `git add src/main.rs tests/it_core.rs tests/it_core/provider_run_runtime_entry.rs`；`git commit -m "fix: 手写Tokio runtime并按编译期分档8与16MiB栈"`。仅在前序根修门禁通过后执行，不是S1止血。

### Task 14：S5双制品发布证据、容量观察与可执行回退说明

**Files:**
- Modify: `cadence/reports/2026-11-06_验收报告_provider_run根修证据_v1.0.md`（A～D总结、二进制指纹、U1～U15）；`CHANGELOG.md:10-12`（待发布修复说明，不自行提升版本、不宣称未验收平台）。
- Create: `cadence/docs/2026-11-06_运维文档_provider_run根修发布与回滚_v1.0.md`。
- Read-only: `.github/workflows/release.yml:12-32,55-67`发布目标矩阵、Cargo.lock、当前制品与自有运行证据。正式发版若更新版本面板另按既有发布流程同步 `web/src/whats-new/changelog.ts`，本任务不借bug根修擅自发版或扩大前端写集。

**Interfaces:**
- Consumes: 任务9A/B、任务10C/D、任务11CI结果、任务12盘点报告、任务13编译期debug8/release16 Builder与web启动栈日志；无新增产品API。
- Produces: 发布检查单 `{binary_sha256, profile, toolchain, target, dependency_fingerprint, flags, startup_environment, effective_stack_bytes, A/B/C/D_result, durable_diff, abort_or_zombie, threads_vm_rss, rollback_artifact}`及有序排空→换制品→独立验证→小范围→放量流程；所有未验证明确标注。

- [ ] **Step 1：写发布失败检查单。** 缺任一profile A/B/C/D、C非2MiB、D无真实start/完整帧/durable/再生成、指纹/profile/生效栈日志不一致、prod候选不是release16、回退制品未证明不abort、缺lease/idle/token收敛均判NOT_READY，不能只看exit0。这是文档验收红，不虚构产品测试。
- [ ] **Step 2：主控构建固定两制品。** 先确认现有frontend制品前置；执行 `flock /tmp/aria-cargo.lock cargo build --locked --bin aria`与 `flock /tmp/aria-cargo.lock cargo build --locked --release --bin aria`，正式分发目标按release.yml已有矩阵与 `--target`构建。记录sha256、toolchain/target/Cargo.lock、profile/flags/启动env，不能从目录名猜实际安装制品。
- [ ] **Step 3：运行最终A/B/C与行为矩阵。** 主控按任务9/10/11命令串行跑debug/release；保留事故dev opt-level基线，C在显式2MiB，原§6.4行为assert全部保留，line guard最终过线。不是再次寻找D1根因，也不重跑owner已报告的旧故障确认。
- [ ] **Step 4：运行D分档两制品。** 在独立tempdir/端口/进程组、真实adapter条件下执行D：debug日志必须profile=debug、worker_stack_bytes=8388608，release必须profile=release、worker_stack_bytes=16777216，并与二进制sha256/构建flags匹配；prod只允许后者。四条件/取消再生成/relay/重连/删除/idle/token/scratch全部成立。缺CLI/鉴权/平台前置明确未验证，跳过不算过，不碰共享实例。
- [ ] **Step 5：采容量与资源证据。** 在构造caller/Box/spawn/poll代表点分别采样栈峰值及叠加关系，debug/release分栏；记录实际worker+已创建blocking线程数、虚拟映射/VM/RSS与并发、取消延迟。可可靠测Q时以 `H=S-Q`，16MiB要求Q_release≤8MiB为50%工程余量；Q不能可靠测则仅报固定栈成功/失败边界，绝不伪造精确高水位或将VmStk当全部线程总峰值。8MiB候选未来需release Q≤4MiB及同配置D，本次不降栈。
- [ ] **Step 6：验证并写明回退候选。** 无S1不得默认旧版安全；预先在自有副本对选定回退制品与实际启动栈执行D，必须未abort/无僵死并满足同动态合同。回退先取消/排空在途run、等其store写与lease收口，再换完整制品/启动配置，不交接半个driver；不保留旧驱动flag，不回已知旧2MiB崩溃制品。没有合格回退制品保持NOT_READY，不以git revert自动等同服务恢复。
- [ ] **Step 7：将真实证据填入文档与变更说明。** 写根修边界、16MiB暂定工程选择、32MiB测试force保留、2MiB门禁永不豁免、release/debug各值、盘点债务另案；复用现日志观察handler_run_supersede后start、provider_run_cancelled_dropped_drive、completion timeout、token/scratch、idle/entity_busy，无新增遥测项目。仅已实际支持/验收的平台写通过，不把Linux外推Windows/macOS。
- [ ] **Step 8：执行文档验收绿。** 逐项对照Step1，已满足填READY及证据链接，未满足保留NOT_READY与确切缺口；报告不因文档完成而自动宣告缺陷关闭。`wc -l cadence/docs/2026-11-06_运维文档_provider_run根修发布与回滚_v1.0.md`，目标≤1000；本任务不新建代码文件、不新增祖父项。全部可达工作完成后，只在owner批准的正式发布控制面做小范围/放量；本Plan不授权重启共享实例。
- [ ] **Step 9：提交说明与证据。** `git add cadence/reports/2026-11-06_验收报告_provider_run根修证据_v1.0.md cadence/docs/2026-11-06_运维文档_provider_run根修发布与回滚_v1.0.md CHANGELOG.md`；`git commit -m "docs: 记录provider根修双制品验收与发布回退合同"`。

## 自检

### 方案§1～§11覆盖

| Spec章节 | 本Plan承载任务 | 核对结果/无任务承载缺口 |
|---|---|---|
| §1机制/证据/profile | 1、2、9、10、14 | 沿用D1不重做根因；自动Box时机、raw布局与深栈构造分开；debug/release独立。没有用空串/Story特判。 |
| §2体积账/预算 | 1、2、3～9、14 | 32768/131072硬线、65536目标、raw非Box、入口/helper记录；U+M+C+H≤S和旧k≈2限制保留，不称128KiB是安全证明。 |
| §3结构/接口/守卫 | 1～8、9 | 小外壳首次poll、body/drive/各臂/followupsBox；ArmExit、单一receiver、Box rebuilt、取消前置数据；engine/idle/lease按真实生命周期分别验证。 |
| §4候选矩阵 | Global Constraints、2、12、14 | 仅执行获批组合；整体调用点Box对照不是修复前置，未做则U3保留未验证；不采dyn/OS线程/stacker/inbound/actor。 |
| §5 S0～S5/P1～P10 | 1～14 | S0起步、明确不做S1；1机械P1/P2及条件P6解决首文件行数可行性；2～8串行，9/10仅独立文件并行；13独立长期Builder不是止血部署。 |
| §6四层/隔离/矩阵/lint | 1、3～11、14 | A/B/C常规，D发布；子进程exact、防递归/持续排输出/超时清理；所有既有assert保留。任务9/10必须真正执行验证，注册/exit0不算通过。 |
| §7栈取值/容量资源 | 2、9、10、13、14 | 编译期debug8/release16、prod只跑release、web启动日志与D指纹防误用，C2永留、force32保留；release降8另案，U11证据待14。 |
| §8上游依据映射 | 全局、11、12、14 | 使用方案已核来源，当前Clippy语义在11核实；不把社区8MiB案例当本项目release证明，不新增依赖。 |
| §9六条复核修正 | 1、2、8～14 | 六条均进门禁：真边界、Box净收益不绝对化、显式2MiB/worker、正确余量、三守卫、四条件动态证据。 |
| §10 owner决策 | 全局、11～14 | 本次决策全部按已批准结论固化；无pending确认。全仓只读盘点另任务，不带整改；无panic/profile/force撤销/双实现flag。 |
| §11未验证/行数/接地 | 各任务行数步骤、本节、下节 | U1～U15逐项标责任；任何旧文件≤1200当次删除登记，禁止等最终收口；新文件1000～1200也继续拆，首原子切片≤4h。 |

**无必需范围的静默缺口。** 以下是显式边界而非漏任务：S1止血不执行；U3整体Box单独候选净收益不是获批根修必须实验，未跑就保留未验证；U15 opt-level/panic实验不执行；生产降8MiB不在本次；正式平台实机/真实provider凭据不可由文档创造，任务14必须标出未验证，不得宣告发布完成。

### 步骤、接口和审查焦点核对

- 每任务均列精确写集、输入/输出签名、红反馈/最小变更/绿命令/显式暂存提交；只读盘点与发布文档使用证据验收清单，不为TDD制造产品代码或假红。
- 机械提取可能改变布局，行为已绿和预算未必红均如实记录；最终A硬断言不保留should_panic/ignore/Box指针替代。根修完成的阻断条件仍是A/B/C/D与行为矩阵，不以新增接口编译绿结案。
- 后续任务使用同一A/T展开、ArmExit与函数名；lifetime/Send列为实施编译未验证，既有MutexGuard按值合同不允许替换。没有未定义的业务结果包/第二套错误抽象。
- Review Focus 1由1/2/9/10测；2由2/7/9测；3由3～7测；4由8/9测；5由1/10/13测；完整JSON偏序不与跨sender全局总序混淆。
- `provider_run.rs`、task_body、followups、events共享写集全部串行；P8/P9只有独立新文件编辑可并行，报告/注册/祖父表由集成owner收口。测试执行由主控串行；任务步骤中所有cargo均要求flock前缀，省略号只表示文字引用，不是可直接执行的命令。

### 本Plan的接地证据

已按顺序先读技术方案402行与三份支撑报告，再以CodeGraph核对指定驱动/入口/上下文/守卫，定向读取provider_run前奏、各臂、followups两宏与具名函数、provider_run下四个既有inc/entry文件、events夹具/r16、large_file_guard、run.rs、main、Cargo/force配置、真实WS与进程夹具、CI/发布/CLI基线。`ast-grep outline`在本环境返回 `outline is not supported`（exit2），已报告工具问题，降级CodeGraph+已知行号定向读取；不是假称执行过结构化outline。额外Context7只确认Builder、enable_all与block_on根非worker合同，不作为替换获批技术方案的新依据。

此文档遵循writing-plans，未改产品源码、spec/tasks、Cargo/CI配置或原报告；未跑cargo/构建/测试/gdb/复现服务，未访问共享验收/部署数据，未git add/commit。Plan拟定的新增符号/文件/测试均明确是未来接口，**未编译、未运行**；当前行号仅为只读基线。

## 未验证项

下表逐项承接Spec§11.1，实施报告不得把“已写Plan”填成“已验证”。

| 编号 | 当前未验证内容 | 负责产证任务/交付形式 |
|---|---|---|
| U1 | release原F大小、是否复现、构造/搬移/poll峰值 | 1保存可取得基线、9双profile raw、10/14制品；事故debug/release分栏，不改opt-level先消红。 |
| U2 | 机械提取是否仍243880B与原崩溃形态 | 1提取前证据承接D1，提取后实测raw与C失败/存活；不声称必红或尺寸相同。 |
| U3 | 整体调用点Box在本项目双profile净收益 | 本次主线不依赖此候选；14报告明确“未验证/未执行独立对照”，除非未来另获实验授权，不拿spawn小帧冒充全链净收益。 |
| U4 | wrapper捕获/浅栈body真实构造与峰值 | 2首次poll探针、9 raw wrapper/body/entry/helper、10/14线程落点与构造/poll证据。 |
| U5 | 每臂/followups最大raw与child热点 | 3～9逐对象尺寸与有限预算表，12编译器尺寸盘点补广度。 |
| U6 | 上游U精确高水位、新k/C与总Q | 14各测量点和叠加关系；高水位方法不可靠时仅报告固定栈边界，不编造上界。 |
| U7 | 双profile固定2MiB真实WS | 10 C、11常规CI、14最终制品前门禁；包含实际start/完整帧/durable/健康与再生成。 |
| U8 | 新签名lifetime/Send、guard drop、ArmExit可编译性 | 1～8真实编译、2/9 barrier；原MutexGuard类型/锁时点保持，不换Owned guard绕过。 |
| U9 | 三守卫/token/completion/scratch/receiver时序 | 2、7～9双settle分支、depth1→2→1→0、entity_busy及真实新runAbort/再生成。 |
| U10 | stable Clippy可用性/裸future或指针/存量告警 | 11双受控对照、12阈值分档扫描报告；A独立不被lint替代。 |
| U11 | dev8/prod16（未来prod8）容量与并发资源成本 | 13按profile配置/日志，14两profile D、线程数/VM/RSS/Q；本次不降prod8，无法可靠量Q则明确未验证。 |
| U12 | 新文件最终行数、旧祖父回收 | 每个新文件任务的wc+large_file_guard、8最终表；1200硬线/1000继续拆，旧达标当次删项。 |
| U13 | 精确子测试名、防递归/异常/超时清理 | 1从实际list唯一解析exact及父分类自检，9/10真实场景子进程信号/日志/清理记录。 |
| U14 | 平台差异、env/profile覆盖、实际安装制品 | 14指纹/配置和既有发布矩阵；未运行平台明确未验证，Cargo force不代替直跑二进制环境。 |
| U15 | opt-level=1/panic=abort效果与行为成本 | 按owner“不改panic/保持profile”不执行；14报告显式“不适用本次，未验证”，另案批准才可实验。 |

**Plan交付状态：DONE_WITH_CONCERNS。** 计划文档已完成；concerns仅为上述真实实现/测量/环境证据尚待实施，不是代码已修复或测试已通过。停止条件：本Plan落盘即止，不执行其中实施步骤。
