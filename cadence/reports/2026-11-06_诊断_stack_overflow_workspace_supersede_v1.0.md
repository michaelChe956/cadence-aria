# 诊断报告：真机 stack overflow（SIGABRT / exit 134）· workspace「开始生成」supersede 路径

- 日期：2026-11-06
- 分支/工作树：`bugfix-b-11-02`（`/.worktrees/bugfix-b-11-02`）
- 崩溃实例：`aria web --workspace /tmp/aria-verify`（**debug 构建** `target/debug/aria`，2026-11-05 23:19 构建）
- 性质：**只诊断，不改代码**；报告未 git add
- 结论状态：**根因已用 gdb 在数据副本上复现并抓到崩溃栈，实锤**

---

## 0. 根因结论（一句话）

**不是递归。** 崩溃点是 `src/web/workspace_ws_handler/run/provider_run.rs:310` 的 `tokio::spawn(async move { …1360 行驱动体… })`：该 async block 的 future `size_of` = **243,880 B（≈238 KiB）**（spawn 内 memcpy 立即数实测，§1.4），debug 构建下 `tokio::spawn::<F>` 单个栈帧需预留 `0x77000 + 0x2d8` = **487,928 B ≈ 2 × sizeof(F)**，在 tokio worker 线程默认 **2 MiB** 栈上构造/搬移这个 future 时触发栈探测写穿 guard page → SIGSEGV → Rust 栈溢出处理 → `fatal runtime error: stack overflow, aborting`（exit 134）。

一句话：**11-01 提交 49a39400 把 provider_run.rs 驱动体从 698 行扩到 1676 行后，「开始生成」在 WS handler 内联构造的巨型 spawn future 超过了 tokio worker 2 MiB 栈预算，点按钮必炸；与会话数据形状、空 content、supersede 语义均无关。**

---

## 1. 证据链

### 1.1 现场（owner 三次崩溃 + 补充事实）

| # | 时间(+08) | session | 关键日志 | 证据 |
|---|---|---|---|---|
| 1 | 11-06 00:24:06 | workspace_session_0001 | `handler_run_supersede … kind=Author { content: "" }` → overflow | typescript 14 行（`cadence/reports/2026-11-06_崩溃日志_stack_overflow_workspace_supersede_v1.0.log`） |
| 2 | 11-06 00:28:10 | workspace_session_0002 | 重启后重试同操作，同款签名（Main 提供） | 崩溃前有 `[aria-recovery] stale zombie run recovered …`（重启恢复的是崩溃 #1 登记的 run） |
| 3 | 11-06 00:29:12 | workspace_session_0003 | owner 三度重试（`/tmp/aria-verify` 现场留在该形态） | 见 §1.2 durable 序列 |

- 三次崩溃唯一共同点 = 在 Story 工作台点【开始生成】按钮 → 打出 `handler_run_supersede kind=Author { content: "" }` → 立即爆栈。**确定性 100%**。

### 1.2 durable 状态证明「崩溃点在 spawn 体执行之前」

以崩溃 #3 现场（只读检查 `/tmp/aria-verify/.aria/projects/project_0001/issues/issue_0001/`）：

```
00:29:12.265  story-specs/story_spec_0003.json          ← story spec 落盘（engine.start_generation 之前）
00:29:12.357  workspace-sessions/workspace_session_0003.json（messages=[system]×1, status=open, ledger=0）
00:29:12.561  planning-context-snapshot.json            ← refresh_workspace_context（spawn 之前）完成
00:29:12.607  workspace-timelines/…/lease-diagnostics.jsonl（hello hold 事件）
≈00:29:12.6+  崩溃：此后再无任何 durable 写

### 1.3 gdb 复现与崩溃栈（决定性证据）

gdb 崩溃栈（两次复现一致）：

```
#0 spawn<…provider_run::spawn_provider_run_with_start_mode::{async_fn#0}::{async_block_env#1}>
   at tokio-1.52.3/src/task/spawn.rs:174        ← 崩在 tokio::spawn 自身
#1 {async_fn#0} at src/web/workspace_ws_handler/run/provider_run.rs:310   ← tokio::spawn(async move {…
#2 {async_fn#0} at src/web/workspace_ws_handler/run/provider_run/entry.rs:88  ← spawn_provider_run_from_handler
#3 {async_fn#0} at src/web/workspace_ws_handler/decisions/inbound.rs:637      ← StartGeneration 臂
#4 … socket.rs:1829 → socket.rs:1644 → axum ws on_upgrade → tokio worker poll 链
```

崩溃点反汇编（`x/8i $pc-16`）——教科书式**大栈帧探测循环**：

```asm
+3:  sub  $0x77000,%r11      ; 本帧总深 0x77000（487,424 B）
+10: sub  $0x1000,%rsp       ; ┐ 循环每次下探 4 KiB
+17: movq $0x0,(%rsp)        ; ┘ ★SIGSEGV：探测写命中 guard page
+25: cmp  %r11,%rsp / +28: jne +10
+30: sub  $0x2d8,%rsp        ; 再加 728 B ⇒ 帧合计 487,928 B ≈ 476.5 KiB
```

- 崩溃时 `RSP=0x7ffff5a49928`，该地址**不在任何已映射 VMA 内**（线程栈底之下）⇒ tokio worker 的 2 MiB 栈在进入此帧前已基本耗尽（其上是 axum on_upgrade → handle_workspace_socket → inbound 决策整条 async poll 链，debug 构建下每层 future 都很大），再叠加 477 KiB 单帧必然写穿。
- `#[tokio::test]`（current_thread）把测试 future 跑在 libtest 主线程（默认 8 MiB 栈）上，477 KiB 单帧装得下 ⇒ **单测/集成测全绿、真机 debug 多线程 runtime 必炸**——这就是「测试没拦住」的原因。
- 复现后副本的 session 仍是 `msgs=1, ledger=0`，与 owner 三次现场完全一致（§1.2 交叉验证）。

### 1.4 量化结论（objdump 静态分解，精确到字节）

- **`size_of::<该 async block 的 future>` = `0x3b928` = 243,880 B（≈238 KiB）**——依据：`tokio::spawn::<F>` 内部把 F 按值搬进堆任务分配的 `memcpy` 长度立即数（`objdump -d --start-address=0x1980a5d`：`mov $0x3b928,%edx; call memcpy@plt`）。
- **`tokio::spawn::<F>` 单栈帧预留 = `0x77000 + 0x2d8` = 487,928 B（≈476.5 KiB）≈ 2 × sizeof(F)**（debug codegen 下参数绑定槽 + 按值搬移缓冲双份）；崩溃点 RSP `0x7ffff5a49928` 已低于该线程栈映射底部 ⇒ 进入此帧前，上层 async poll 链（axum on_upgrade → handle_workspace_socket → inbound 决策，debug 下每层 future 都大）已耗掉 >1.5 MiB。
- **崩溃机制属「spawn 搬移 future 溢出」，非 poll 嵌套**：崩溃栈里没有任何 spawn 体的帧——future 从未被 poll。
- **worker 栈 = tokio 默认 2 MiB**：`src/main.rs:1` 为裸 `#[tokio::main]`，全仓无 `thread_stack_size`/`worker_threads` 配置。
- future 体积来源（结构性归因；逐字段占比未验证，需 nightly `-Zprint-type-sizes`）：async block（provider_run.rs:310-1656）把全部 ~13 个 `run_kind` 臂跨 await 的局部量装进**同一个** future——共通前奏的 `run_context_clone`（含 ~12KB 级 `WorkspaceSessionRecord`）、WorkItemPlanDraft 循环（plan_launch/provider_input 含完整 prompt+baseline_tree）、HumanGateSC 修订 attempt 循环（carry/frozen_prompt）、Legacy include! 臂（Box<ResolvedPlanningContext>）、ReviewOnly closed_turn 块、followups 宏调用点等。

### 1.5 回归窗口（问题 5）

- `git log -S 'handler_run_supersede'`：打点行由 **49a39400（2026-11-01「多仓 work item group 全链」）** 引入。
- 同一提交把 `src/web/workspace_ws_handler/run/provider_run.rs` 从 **698 行改到 1676 行（+1525/−547）**——`tokio::spawn` 内联的 `match run_kind` 驱动体（含 WorkItemPlan 各臂、SC 人工修订 attempt 循环、followups 宏等）一次性长胖，future 体积随之超过 2 MiB worker 栈预算。
- 最近 30 个提交（c7e821f8…39163bb0）未再碰该文件 ⇒ **是 11-01 的 49a39400 引入的回归**；11-05 真机验收才第一次在「多线程 runtime + debug 构建」下点【开始生成】。
- （未验证项）49a39400 之前的版本是否能在 2 MiB worker 栈上跑过：未构建旧二进制实测；但 7 月底~10 月底该路径在真机验收通过过，旧驱动体显著更小，推断旧 future 在预算内。

---

## 2. 逐条回答（对照任务 7 问）

### Q1 `handler_run_supersede` 完整调用链（文件:行号）

1. 前端按钮：`web/src/components/chat-workspace/ChatInputBar.tsx:256` `handleStartGeneration` → `web/src/pages/ChatCockpitPage.tsx:356` → `web/src/hooks/useWorkspaceWs.ts:421-428` 发 WS `{"type":"start_generation", provider_config, reviewer_enabled}`（单发，每点击一条；无重发逻辑）。
2. 服务端 WS 入口：`src/web/app.rs:618` `/api/workspace-sessions/{session_id}/ws` → `socket.rs` `handle_workspace_socket`（socket.rs:1644 select 循环）→ `decisions/inbound.rs:59` `handle_workspace_inbound_message_inner`。
3. `WsInMessage::StartGeneration` 臂（inbound.rs:585）：
   - `engine.start_generation(...)`（inbound.rs:602）——建 `start_generation` 时间线节点、stage→running、发 `locked` 帧（**这就是现场看到的「生成确实被启动了」**）；
   - `refresh_workspace_context_for_session`（inbound.rs:609 → 857）——重建聚合上下文消息 + planning snapshot；
   - 构造 run_kind（inbound.rs:620-631）：**非 WorkItemPlan ⇒ `ProviderRunKind::Author { content: String::new() }`（inbound.rs:627-629，设计如此）**；
   - `spawn_provider_run_from_handler`（inbound.rs:632）。
4. `run/provider_run/entry.rs:77-90` → `run/provider_run.rs:153` `spawn_provider_run_with_start_mode`：
   - run lease 准入（:177-195）→ enrollment 核验（:199）；
   - **:210 `SupersedeFromAttachment` 臂 → :213-216 eprintln 打出 `[aria-cancellation] workspace handler_run_supersede … kind=Author { content: "" }`**（这就是日志最后一行）；
   - :220 `abort_active_run_from_attachment`（无活 run 时近似 no-op）→ :267 `start_run_from_attachment`（登记 run ⇒ 之后的「stale zombie」）；
   - **:310 `tokio::spawn(async move { … })` ← ★崩溃点（spawn 体从未开始执行）**。
5. （未执行到的「真正干活」段）spawn 体内 `ProviderRunKind::Author` 臂（:333-342）→ `engine.handle_user_message_from_run`（`workspace_engine/provider_drive/author_root_launch.inc.rs:414`）→ LC root launch 解析 + `handle_author_message_with_prompt_mode`（`workspace_engine/provider_drive.rs:65`）→ provider 启动。

### Q2 「递归」在哪 —— **没有递归环**

- 爆栈机制不是无限递归，而是**单个巨型栈帧**：`provider_run.rs:310` 的 async block（310-1656 行，覆盖全部 run_kind 分支的驱动体）经 49a39400 扩写后，其 future 类型（`{async_block_env#1}`）巨大；debug 构建下 `tokio::spawn::<F>` 需要在 worker 栈上构造并按值搬移 F，单帧预留 487,928 B（§1.3 反汇编实测），叠在 axum→socket→inbound 的 poll 链之上，写穿 2 MiB worker 栈。
- 佐证（排除法）：两个 scout 对该窗口内全部同步/异步链（resolver 家族、gateway validate、prompt/压缩、聚合输出解析、manager run 生命周期、LC store）做了静态排查，**均无自调用/互调环**；且崩溃帧就是 `tokio::spawn` 本身（不是任何业务函数）。
- （附带发现，与本崩溃无关）`workspace_engine/compile.rs:599 ↔ compile_failure_surface.rs:662 ↔ review/routing.rs:696` 存在一段 `Box::pin` 的编译失败自动返修 async 递归环，其终止仅靠行为性判重（非结构性上限），建议另开缺陷跟踪。

### Q3 空 `content: ""` 的作用 —— **无关变量（设计内的正常载荷）**

- 来源：`inbound.rs:627-629`——【开始生成】对 Story/Design（非 WorkItemPlan）会话**本来就构造空串 Author**；Story 的生成指令来自 system 上下文（「Workspace 生成任务已准备…」5,980 B），不靠 user content。
- engine 侧第一行就会兜底：`prompts.rs:68-82` `normalize_generation_prompt` 把空串规范成 `“Workspace 类型: Story Spec\n开始生成 Story Spec”`——不存在「空内容触发异常分支」；空与非空走的是同一条 spawn 路径、同一个 future 类型，都会爆栈。
- 结论：日志里的 `content: ""` 只是这条 UI 路径的**常量特征签名**，不是诱因。

### Q4 与持久化数据形状的关系 —— **无关**

- 三个不同 session（0001/0002/0003）+ 我的副本复现，同一 LC（聚合根 `/home/.../test-center-java`，3 成员 busi/gateway/frontend，`layout=common_non_git_parent`）、不同 story spec 均复现；崩溃点在读取这些数据**之前**（spawn 构造阶段）。
- `.aria` 检查（只读）未发现环状引用：`lc_resume_recovery=null`、`superseded_native_sessions=[]`、`provider_conversations=[]`、selections/manifest/aggregate-index/成员记录均为扁平结构；`planning-context-snapshot.json` 正常写完。
- 「崩溃前 4 次 `git init`（root-commit 81f5a10/c364a89×3）」与崩溃无关：那是 owner 在 UI 上做的 4 次仓库注册（provider 驱动初始化的子进程 stdout 串进 typescript），注册本身成功完成；崩溃 #2/#3 无这些输出仍复现。

### Q5 近期改动相关性 —— **是回归**（见 §1.5）

引入提交：**49a39400（2026-11-01）**，`provider_run.rs` +1525/−547；之后 30 个提交未再触碰。

### Q6 最小复现（已验证）

**UI 版（owner 已三次复现）**：
1. 准备一个多仓（LC）项目 + issue + Story spec + workspace 会话（即 11-05 晚 owner 的操作序列：聚合初始化 3 成员 → 建 issue「公告删除」→ 生成 Story spec → 打开工作台会话）；
2. 用 **debug 构建**（`target/debug/aria web …`）起服务；
3. 在 Story 工作台点【开始生成】→ 秒级 abort（exit 134）。

前置条件就这四条：多线程 tokio runtime（默认配置）+ debug 二进制 + workspace 会话 + 任意 StartGeneration。**不需要**特定会话状态/lease/时间线节点；content 是否为空也无关（同一路径）。

**脚本版（本轮已验证，1 秒复现）**：
```bash
cp -a /tmp/aria-verify /tmp/aria-diag            # 数据副本（勿用原实例）
gdb -q target/debug/aria   # run web --workspace /tmp/aria-diag --host 127.0.0.1 --port 4401
# 另终端：bun 驱动（hello + start_generation，见 §1.3；脚本留在 /tmp/aria-diag-driver.ts）
```

**cargo 测试落点建议**（把「测试为什么没拦住」补上）：
- 模块：`src/web/workspace_ws_handler/tests/provider_run_events.rs`（已有 `user_message_supersedes_streaming_run`（:452）等直接调 `spawn_provider_run_from_handler` 的用例——它们跑在 `#[tokio::test]`（current_thread、libtest 线程 8 MiB 栈）所以不炸）。
- 新测试（红）形态：同 fixture，但用 `#[tokio::test(flavor = "multi_thread", worker_threads = 1)]`，并把 `spawn_provider_run_from_handler(...)` 包进 `tokio::spawn(...)` 后 await——强制它在 **2 MiB worker 栈**上执行；当前代码会让测试进程直接栈溢出（红），修复后应通过。
- **静态体积护栏（TDD 首选，修复前必红/修复后必绿）**：匿名 future 拿不到类型名 ⇒ 先做一步机械重构——把 `provider_run.rs:310` 的 async block 提取为具名构造点（如 `async fn provider_run_drive_body(...)` 或返回 `impl Future` 的具名函数，spawn 处改为 `tokio::spawn(provider_run_drive_body(...))`，行为零变化）。测试里 `let fut = provider_run_drive_body(…fixture…); assert!(std::mem::size_of_val(&fut) <= 131_072);`——`size_of_val` 对不透明 future 有效，无需点名类型。阈值依据：worker 栈 2 MiB − 实测上层 poll 链 >1.5 MiB − spawn 帧 ≈2×F ⇒ F ≤ 128 KiB 时帧 ≈256 KiB，留 >1.2 MiB 余量。当前 F=243,880 B ⇒ 断言必红；拆分后 ≤128 KiB ⇒ 必绿。

### Q7 影响面 —— **所有 WS handler 发起的 provider run，不止 Story/Author**

`provider_run.rs:310` 是唯一 spawn 收口：StartGeneration（Story/Design→Author、WorkItemPlan→WorkItemPlan*）、RetryInterruptedRun、RequestRevision 各臂、引擎事件 relay（`spawn_provider_run_from_event`）以及 auto 认领（`spawn_provider_run_claiming_idle`）**全部经同一行构造同一个巨型 future** ⇒ 在当前 debug 二进制 + 默认 runtime 下，任何一个都会爆栈。不受影响的：REST 侧流程（仓库注册/聚合初始化等自有 runner）、已 spawn 之外的纯查询面。**修复必须按「future 减肥」收口，不能只修 Story。**

---

## 3. 最小修复方向（只给方向，不写代码）

1. **首选（结构性，真缩小）**：拆 `provider_run.rs:310` spawn 体的 `match run_kind`——每个 run_kind 臂（或按族：Author 族 / ReviewOnly / WorkItemPlan 族 / HumanGateSC 修订）提取为独立 `async fn`，spawn 体只留共通前奏+分发+followups；大臂内再把巨型局部（`StreamingProviderInput`/prompt 全文/`ResolvedPlanningContext`）以 `Box`/引用传递。目标 `size_of(future)` ≤ 128 KiB（现 238 KiB；按臂拆分后每臂 future 预估几十 KiB——推断，需实测）。
2. **`Box::pin` 整个 future：单独用无效**——debug codegen 下 async block 仍先在调用方栈上构造出 243,880 B 的值再搬进堆，栈需求不变；只能作为拆分后的双保险。Send 满足（现 spawn 已要求）、取消语义（drop 取消臂/lease 释放）不受影响。
3. **止血（可并行、治标）**：`#[tokio::main]`（src/main.rs:1）改为 `tokio::runtime::Builder::new_multi_thread().thread_stack_size(8 << 20)`（或该 spawn 走 `std::thread` 大栈 + `Handle::block_on`）。不改变 task 生命周期——drop 取消臂随 task drop 释放 lease 的语义不变；风险是 future 继续膨胀，只作临时放行。
4. **防回归护栏**：§Q6 的 size_of_val 体积断言 + worker 栈冒烟测试（`#[tokio::test(flavor = "multi_thread", worker_threads = 1)]` + `tokio::spawn(spawn_provider_run_from_handler(...)).await`，修复前测试进程本身栈溢出=红）。
5. 顺带跟踪（另开缺陷）：`compile.rs` 编译失败自动返修 `Box::pin` 递归环的非结构性终止条件（§Q2 附带发现）。

## 4. 复现材料与现场清理说明

- 复现副本：`/tmp/aria-diag`（我的数据副本，可随时删除重拷）；驱动脚本 `/tmp/aria-diag-driver.ts`；gdb 命令文件 `/tmp/aria-diag-gdb*.cmds`。
- gdb 服务已全部退出；`/tmp/aria-verify`（owner 实例）与 `/tmp/aria-deploy` 全程未写未访问（后者未触碰）。
- owner 的 `aria-4317` 服务（pid 2158404/2158444）未受本轮任何操作影响。

## 5. 未验证 / 推断项（明确标注）

- 「49a39400 之前的版本在 2 MiB worker 栈上可正常运行」：**未验证**（未构建旧二进制；仅有文件行数 698→1676 与该路径历史真机通过记录作支撑）。
- ~~future 的精确 size_of 未取到~~ 已取到：`0x3b928` = 243,880 B（§1.4，spawn 内 memcpy 立即数实测）。
- 「release 构建是否也会爆」：**未验证**；release 下 future 体积与搬移次数都会收缩，可能暂时装得下，但结构风险同在。
