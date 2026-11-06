# 诊断报告 2：新二进制（HEAD 3580f3bf）Stack Overflow 仍复现 —— 根因重构

- 日期：2026-11-06（第 2 轮）
- 工作树：`/.worktrees/bugfix-b-11-02`（HEAD `3580f3bf`；崩溃二进制 = `target/debug/aria`，2026-11-06 12:16 构建）
- 方法：数据副本（/tmp/aria-diag3、/tmp/aria-diag4）+ gdb + Bun WS 真实全流程驱动（story-specs:generate → WS hello → start_generation → 自动应答 choice → author_confirm → POST confirm `with_review:true`）
- 性质：只诊断，不改代码；报告未 git add

---

## 0. 新根因（一句话）

**上一轮修复（Task 1~4 的 spawn 外壳）真实有效——新崩溃不在 spawn 搬运，而在已 spawn 的 ReviewOnly 任务内部：`provider_run_drive_body` 的 poll 帧在 debug 构建下预留 `0xb1000` = 724,992 B（≈708 KiB）**（`Box::pin` 只把臂的**存储**挪去堆，各臂 future 的**构造临时区**仍全部摊在 drive_body 的 poll 栈帧里），叠加 `provider_run_task_body` poll 帧 0x1c000 = 114,688 B、engine 评审驱动链的嵌套 async poll 帧（review/drive.rs:373→431 嵌套闭包 → build_review_input → human_dispositions_section → lifecycle store 读取），把 tokio worker 的 2 MiB 栈吃穿——最后在叶子 `read_json::<WorkspaceSessionRecord>`（json_store.rs:36，帧仅 3,704 B）的序言探测写上爆。

Author 路径不崩、ReviewOnly 崩的原因：两者共用同一 spawn 外壳与同一 drive_body poll 帧（708 KiB），但 Author 臂下的 engine 调用链浅（`handle_user_message_from_run` → `build_streaming_input`，同步浅链），ReviewOnly 臂下的 engine 评审链**多出至少 6 层嵌套 async fn poll**（含嵌套闭包 + and_then），恰好越过 2 MiB 红线。

## 1. 决定性证据：gdb 崩溃栈（本机复现，与 owner 两次现场同签名）

复现序列（owner 同款 UI 动作）：开始生成（Author supersede 日志）→ pi 作者轮完成（数分钟）→ author_confirm 门 → 点「确认并送审」（`POST /confirm {with_review:true}`，前端 client.ts:146 `withReview` 分支）→ **`kind=ReviewOnly` supersede 日志** → 数百毫秒内 SIGSEGV → exit 134。

```
Thread 13 "tokio-rt-worker" received signal SIGSEGV
#0  read_json::<WorkspaceSessionRecord>            src/product/json_store.rs:36        ← 爆点（帧 3,704 B，序言 sub $0xe78 探测写穿）
#1  read_workspace_session_record                  src/product/lifecycle_store/utils.rs:82
#2  find_workspace_session_path                    src/product/lifecycle_store/workspace.rs:1244
#3  get_workspace_session                          src/product/lifecycle_store/workspace.rs:591
#4  LifecycleStore::list_human_dispositions        src/product/lifecycle_store/human_dispositions.rs:54
#5  human_dispositions_section                     src/product/workspace_engine/prompts/review_context.rs:651
#6  build_review_input                             src/product/workspace_engine/prompts/review.rs:186
#7  drive_review_session_via_gateway_with_window::{closure#0}::{closure#0}   review/drive.rs:431   ← 嵌套 async 闭包
#9  {async_fn#0}                                   review/drive.rs:431
#10 {async_fn#0}                                   review/drive.rs:373
#11 poll<Box<drive_review_only_arm>>               basic_arms.rs:87（臂已 Box::pin ✓）
#13 {async_fn#0}                                   run/provider_run/task_body.rs:339     ← ReviewOnly 臂分发
#14-15 poll<Box<provider_run_drive_body>>          （drive body 已 Box::pin ✓）
#16-18 drive_until_cancelled_settles (select)      cancel_settle.inc.rs:37
#19 {async_fn#0}                                   run/provider_run/task_body.rs:219     ← task body
#20-22 poll<Box<provider_run_task_body>> / provider_run_task          （外壳/体均已 Box::pin ✓）
#23+ tokio task harness（**独立 spawned task，全新 worker 栈**）
```

要点：
- 崩溃线程是**已 spawn 的 review 任务**（帧 #23 以下全是 tokio harness）——**不是** spawn 搬运帧，**不是** inbound 深链，**不是**准入清理。
- `RSP=0x7ffff5a49c60`，低于该 worker 线程栈底（≈0x7ffff5a4a000，2 MiB 栈）⇒ 全新任务栈被 poll 链自身吃满。
- **三层 Box::pin 边界（task/task_body/drive_body/臂）全部在位**（帧 #11/#14/#20 可见）——Task 1~4 的结构没被绕过。

## 2. 体积实测（新二进制，objdump 直接量编码器输出）

| 对象 | 未装箱 size | poll 帧预留（栈成本） | 来源 |
|---|---|---|---|
| `provider_run_task`（外壳） | 3,704 B ✓ | spawn 帧 0xe88+memcpy 0xe60（≈3.7 KiB） | spawn_inner 符号 0x25fb550 序言+memcpy |
| `provider_run_task_body` | 15,344 B ✓ | **0x1c000 = 114,688 B**（首次 poll 构造+装箱 drive_body） | 闭包 0x311e150 序言 |
| `provider_run_drive_body` | 112,024 B ✓ | **0xb1000 = 724,992 B** ★元凶 | 闭包 0x311f190 序言 |
| 各臂 future（Box::pin 构造 memcpy 实测） | 最大 68,832 B；分布 5.8~68.8 KiB 共 20 处 ≥4 KiB | 构造临时摊在 drive_body poll 帧 | drive_body 闭包内 20 处大 memcpy |

- **体积预算门禁（Task 9/10 的 ≤32 KiB「未装箱」指标）量错了东西**：`drive_body` 未装箱 112 KiB 本身合规（若门禁是 future size）；但 debug codegen 下 **poll 帧预留 = 各臂构造临时区之和 ≈ 708 KiB**，与「未装箱 size」是两个量纲。门禁必须同时断言 **poll 帧预留（可用 prologue `sub $imm` 量）** 或改用「臂构造本身也装箱」的形态。

同行静态求和（来源=DiffScan 静态测量：objdump 序言 sub 求和，12:16 worktree 二进制，含未提交 P4 plan_draft/plan_batch 臂；上界估计，未计 tokio harness/同步叶子帧）：ReviewOnly poll 链 task_body 118K + drive_body 726,024 + drive_review_only_arm 163,432 + drive_review_session_via_gateway 193,016 + …_via_gateway_with_window 768,440 ≈ 1.97 MB（2 MiB 的 94%），其后 ensure_review_invocation_scope 35,352 / drive_reviewer_provider_session_once 64,408 越界——与本文实测（drive_body 序言 724,992 ≈ 其 726,024；崩溃点在 reviewer ledger 落盘之前的读盘段）互相印证。其另标注「ReviewOnly→complete_review→route_legacy_review→apply_policy_route 求和 2.56MB」为**符号推断嵌套（未证实调用嵌套），仅作风险待验证**。

### 2.1 RUST_MIN_STACK 环境对照（同二进制同驱动，决定性对照）

- **背景**：本仓库 `.cargo/config.toml [env]` 设 `RUST_MIN_STACK = { value = "33554432", force = true }`——**所有 `cargo run`/`cargo test`（开发/CI）的线程默认栈 32 MiB**（tokio runtime 未显式 thread_stack_size 时随 std 默认吃该变量）；owner 现场为 `script -qfc <binary>` 直启（/proc/2507207/environ 无该变量）⇒ 2 MiB ⇒ 崩。**这完整解释了「为什么开发/CI 从未复现」**（该配置注释还自述 it_web provider_matrix 曾在 8 MiB 栈溢出——本库栈深问题有前科）。
- **对照实测**（3580f3bf 二进制 + 同驱动全流程，服务端 `RUST_MIN_STACK=33554432` 直启，/tmp/aria-diag6 端口 4411）：Author 轮 76s（2 次 choice 应答）→ confirm with_review=true → **cross_review 评审轮 103s 完整跑完** → 回 author_confirm；进程存活（/api/health ok）；durable msgs=7（评审报告入流）、**ledger=2（reviewer 已登记）**。对照组（不设变量，2 MiB）同点位 4/4 崩。
- ⇒ **止血已验证**：生产注入 `RUST_MIN_STACK=33554432`（或代码内 `thread_stack_size(8<<20)`）立即可用；结构修复（§5.1/5.2）仍必须做。
- `drive_review_only_arm` 本身不是最大臂（≤68.8 KiB 级，与 legacy/SC 臂同量级）——**爆栈不取决于单臂大小，取决于 drive_body poll 帧 + 嵌套 engine 链的总深度**。

## 3. Main 四点假设的核实结论

### 3.1 「准入时序被 Task 2 推迟到首次 poll」——**排除**（逐行对照）

`git show c8223fd4` / `985ef4f6` / 当前 3580f3bf 三版的 `spawn_provider_run_with_start_mode`，准入序列**全部在 `tokio::spawn` 之前、完全同序**：

| 准入动作 | c8223fd4 (Task1) | 985ef4f6 (Task2) | 3580f3bf (现) |
|---|---|---|---|
| entity run lease acquire | :198 spawn 前 | :200 spawn 前 | :212 spawn 前 |
| enrollment 终态核验 | :219 spawn 前 | :221 spawn 前 | :234 spawn 前 |
| supersede eprintln + abort_active_run_from_attachment | :234/:241 spawn 前 | :236 spawn 前 | :248/:255 spawn 前 |
| provider 选择 + registry | spawn 前 | spawn 前 | :259-:295 spawn 前 |
| start_run_from_attachment（token+command_rx 注册） | :288 spawn 前 | :290 spawn 前 | :301 spawn 前 |
| begin_provider_drive | :304 spawn 前 | spawn 前 | :319 spawn 前 |
| mark_active_run_started | :308 spawn 前 | :310 spawn 前 | :321 spawn 前 |
| `tokio::spawn(...)` | :333 `provider_run_task_body(...)`（**未装箱**） | :336 `provider_run_task(...)`（小外壳） | :344 `provider_run_task(...)` |

唯一差异只有 spawn 目标（Task 1 直搬 body / Task 2 起小外壳）。Task 2 没有移动任何准入动作。

### 3.2 「stale zombie run = run 没注册/漏取消」——**排除，zombie 是症状**

- 打印点：`src/web/workspace_session/manager/durable_projection.rs:547-570` `recover_stale_run_if_zombie`——重启时 in-memory run 注册表为空（新进程必然）且 engine stage ∈ {Running, CrossReview, Revision} ⇒ 回 prepare_context 并打点。它是「进程在 run 进行中死亡」的**兜底恢复**。
- 本轮 gdb 证据：崩溃时 ReviewOnly run **已注册且任务已在跑**（帧 #11-#19 就是该任务的驱动链）——注册链路正常。每次崩溃必然留下 Running/CrossReview 态 ⇒ 下次重启必然打 zombie 恢复。**因果方向：爆栈 ⇒ zombie，不是时序错乱 ⇒ zombie。**

### 3.3 「ReviewOnly 是绕过新外壳的第二条路径」——**排除**

ReviewOnly 接力链：confirm 端点（workspace_session.rs:224 `http_confirm_disposition` → lifecycle.rs:2021 `begin_review_after_author_confirm` → `request_provider_run(ReviewOnly)` 事件）→ router（router.rs:198 **独立 tokio::spawn 的 relay 任务**，自身栈是全新的）→ `spawn_provider_run_from_event`（无活 run 时不 drain）→ `spawn_provider_run_from_handler` → 与 Author **同一** `spawn_provider_run_with_start_mode` → 同一 `tokio::spawn(provider_run_task(...))` 小外壳（provider_run.rs:344）。没有旁路。

### 3.4 ReviewOnly 步骤失败/爆栈的因果——**本报告根因**

同 §0：外壳修复后，剩余栈预算 = 2 MiB − task_body 帧(114 KiB) − **drive_body 帧(708 KiB)** − 臂构造/胶水(~100 KiB) ≈ 1.1 MiB；Author 臂 engine 链 ≈ 数十~数百 KiB ⇒ 存活；ReviewOnly 臂 engine 链（6+ 层嵌套 async poll，每层 debug 帧数十 KiB）⇒ 越界。

## 4. 「改动之前全流程是好的」的再校准（提交二分）

owner 记忆的「00:04 全流程好」= **聚合初始化/LC 建仓流程**（REST 自有 runner，不经 workspace provider run），不是 Story 工作台全流程。Story「开始生成」链在 49a39400（11-01，provider_run.rs 698→1676 行）后就已爆（第 1 轮诊断实锤：spawn 搬运 243,880 B future 崩）。时间线：

| 提交/二进制 | Story 全流程（开始生成→确认送审） | 崩点 | 证据 |
|---|---|---|---|
| 49a39400 之前（10 月基线） | ✅（推断，未重建实测） | — | 历史真机验收通过 |
| c7e821f8 时代二进制（11-05 23:19 构建） | ❌ 第一步就崩 | **spawn 搬运帧**（243,880 B future） | 第 1 轮 gdb（报告 1） |
| **c8223fd4（Task 1，已实测）** | ❌ 第一步就崩（点开始生成 <1s，ws 1006 断开） | **spawn 搬运帧**：`spawn<provider_run_task_body::{async_fn_env#0}>` @ tokio spawn.rs:174，调用者 provider_run.rs:333（spawn 直搬未装箱 body） | 本轮 gdb 实测（/tmp/aria-bisect-c822 独立 worktree 构建，实例 /tmp/aria-diag5，端口 4409） |
| **985ef4f6（Task 2）/3580f3bf（现，已实测）** | ❌ 作者轮 + AuthorChoiceFollowup 续跑轮均存活跑完、送审轮崩 | **任务内 drive_body poll 帧(708 KiB)+评审链** | 本轮 gdb 两次（diag3 + diag4，签名一致） |

⇒ **不存在「Task 1/Task 2 把好流程改坏」**：这两个提交只是把崩溃点从 spawn 搬运推进到了任务内部的评审驱动链；真正的破坏提交是 **49a39400**（一次性把驱动体+评审臂内联体积推过栈预算）。

## 5. 最小修复方向（不实施）

1. **把「臂构造」也移出 poll 栈**（首选）：drive_body 分发点不再 `Box::pin(arm(ArmDrive{...}))`（构造临时区仍在栈），改为每臂一个**返回 `Pin<Box<dyn Future>>` 的具名构造函数**（`fn make_review_only_arm(...) -> Pin<Box<...>>`，构造在函数内完成、只回指针），或把 `ArmDrive` 与臂参装 **`Box`/`Arc`** 后再传——目标是把 `provider_run_drive_body` 的 poll 帧从 708 KiB 压到 ≤64 KiB 量级。
2. **压浅 ReviewOnly 的 engine 嵌套链**：`drive_review_session_via_gateway_with_window`（review/drive.rs:373→431）的嵌套 async 闭包 + `and_then` 链拍平/装箱一层（`Box::pin` 该嵌套闭包），减少叠加在 708 KiB 之上的多层 debug 帧。
3. **门禁指标修正**：Task 9/10 的体积门禁从「未装箱 size_of」扩为「**poll 帧预留**」（可在测试里用 `std::mem::size_of` + 一次真实 poll 后的栈差，或 CI 对二进制符号序言 `sub $imm` 做静态断言）；并保留 2 MiB worker 栈真实场景冒烟（multi_thread + spawn 后驱动评审链的 E2E）。
4. **临时止血（✅已由对照实验验证，见 §2.1）**：生产环境注入 `RUST_MIN_STACK=33554432`，或 `#[tokio::main]` 改 `Builder::new_multi_thread().thread_stack_size(8 << 20)`——对照实测同二进制同驱动在 32 MiB 栈下全流程（含 ReviewOnly 评审轮）完整跑完；治标，体积继续增长仍会再爆，结构修复（1/2）必须做。

## 6. 复现稳定性与材料

- 本机（与 owner 同二进制、同数据副本）：**2/2 命中**——diag3 全流程驱动（Author 轮 36s 含 choice 应答 → confirm with_review → ReviewOnly supersede → 崩）；diag4（session_0006，含 Author→AuthorChoiceFollowup→ReviewOnly 三次 supersede）再次同签名崩。owner 真机两次（session_0004/0005）⇒ **合计 4/4 稳定**。
- c8223fd4 二分实测：点开始生成 <1s 崩于 spawn 搬运帧（§4 表）——干净 checkout 需先拷 `web/dist`（rust-embed 编译期嵌入）再 `cargo build --locked --bin aria`（独立 worktree /tmp/aria-bisect-c822 + 独立 flock /tmp/aria-bisect.lock，未触碰主 target 锁）。
- 已知波动项：pi 作者轮产物需过 artifact gate（open_items 必须经 choice 交互解决）——驱动已实现 choice 自动应答 + gate 失败自动重试。
- 材料：数据副本 `/tmp/aria-diag3`~`/tmp/aria-diag6`（diag6=RUST_MIN_STACK 对照）；二分 worktree `/tmp/aria-bisect-c822`（含其独立 target，可整体删除）；驱动 `/tmp/aria-drv4c.ts`、`/tmp/aria-drv5.ts`、`/tmp/aria-drv6.ts`；gdb 命令 `/tmp/aria-diag{3,4,5}-gdb.cmds`。
- 逐帧 RSP 捕获脚本有缺陷（gdb python 未逐帧 select，delta 全 0，原始数据未采到）；帧级栈成本以 objdump 序言实测为准（§2），崩溃时 RSP 低于线程栈底的总量证据见 §1。

## 7. 未验证 / 推断项

- 「49a39400 之前全流程好」：未重建旧二进制实测（构建成本高）；依据 10 月真机验收记录 + 第 1 轮的 698→1676 行增长链推断。
- c8223fd4 spawn 搬运帧的**精确字节数**未量（崩点形态已实测定性，与第 1 轮同型；如需数字可对 /tmp/aria-bisect-c822 二进制同法量 memcpy 立即数）。
- release 构建行为：未测（debug 帧布局是放大器；release 可能暂时不爆但结构风险同在）。

## 8. 决定性四格矩阵 + 其它深链帧表（追加轮）

### 8.1 main 在 2 MiB 下崩不崩——四格实测（同装置全流程：建会话→choice 应答→start_generation→confirm with_review→评审轮）

| 二进制 | 栈 | abort? | 结果 |
|---|---|---|---|
| main(827a990) | 2 MiB 默认 | **✅ +4.2s 崩** | `Author { "" }` supersede → stack overflow abort（第 1 轮同款签名）——**缺陷 main 也有** |
| main(827a990) | RUST_MIN_STACK=32 MiB | ❌ | Author 108s + **评审轮 134s 完整跑完**（msgs=10、ledger=2）——「main 能跑」= 32 MiB 环境掩盖 |
| bugfix | 2 MiB | ✅ 4/4 | ReviewOnly poll 链崩（§1） |
| bugfix | RUST_MIN_STACK=8 MiB | ❌ | 全流程存活（Author 146s + 评审 101s，ledger=2）——阈值定位 2 MiB 崩/8 MiB 活 |

（main@32MiB 首两轮因复制数据的能力行 unknown/缺 codex 被拒，LC 路由 revalidate pi+codex 后全绿；与栈无关。）

### 8.2 表 A：其它深链（bugfix 当前二进制全量扫描 31,164 符号/685 大帧，口径=序言 poll 帧预留上界）

| 链 | 关键帧 | 链和上界 | 判断 |
|---|---|---|---|
| A1 WS socket+inbound | upgrade Cell 327,680 / handle_workspace_socket 278,528 / **inbound_inner 946,176** | ≈1.6 MB | ⚠️ 接近（spawn 已搬出才幸存） |
| A2 Coding WS 启动 | **execute_start_coding_flow 806,912** / runner Cell 270,336 / task_body 94,208 | ≈1.3-1.45 MB | ⚠️ 高危存量 |
| A3 SC 人工修订 | drive_body 819,200 + arm 344,064 + turn/prompt 族 323,584×N | **≈1.8 MB+** | 🔴 预计 2 MiB 越界（未实跑） |
| A4 Revision 返修 | drive_body 819,200 + resolve_revision_root_launch_with_freshness 765,952 + drive_revision_session 114,688 | **≈1.8 MB+** | 🔴 同上 |
| A5 ReviewOnly | drive_body 819,200 + with_window 765,952/708,608 | ≈2 MB | 已证崩 4/4 |
| A6 Author | drive_body 819,200 + arm 40,960 + drive_provider_session 139,264 | ≈1.2-1.5 MB | ✅ 实测存活 |
| A9 LC fresh 恢复 | verify_lc_fresh_* 270,336×2 + lc_fresh_recovery 192,512×2（叠 A1） | ≈1.8 MB | 🔴 贴线高危 |

**结论**：除 provider run 外，A3/A4/A9 三条链静态同样逼近/越过 2 MiB；A1/A2 是一加就破的存量。8 MiB 下全部安全。

### 8.3 表 B：main vs bugfix（同口径）

- **逐字节同值的共有大帧**（≥300K）：inbound_inner 946,176、execute_start_coding_flow ≈803-807K、resolve_revision_root_launch_with_freshness 765,952、drive_review_session_via_gateway_with_window 765,952、drive_review_session_with_window 708,608、SC 族 323,584×N、socket 278,528/Cell 327,680、SC author 270,336、drive_review_session 188,416、drive_provider_session 139,264 ⇒ **高危链主体是两分支共有存量，非本分支引入**。
- **分支差异**：main 的 spawn 侧巨块（Cell 1,462,272 / closure 1,327,104 / spawn 帧 487,424 / RawTask::new 241,664）已被分支消除（外壳 3,704）；分支新形态 task_body 114,688 + drive_body 819,200（12:16 构建 724,992 → 当前 819,200，**P4 臂合入后又涨 94K**）+ 各臂（SC 臂 344,064）。
- 全量帧表：`/tmp/frames-bugfix.tsv`（685 行）、`/tmp/frames-main.tsv`（622 行）。

### 8.4 门禁清单（poll 帧预留口径）

- P0 单帧 ≤64 KiB：provider_run_task / task_body / drive_body / 全部 drive_*_arm。
- P0 spawn 侧 ≤32 KiB：start_mode 闭包族、spawn_coding_runner_task 及其 spawn 搬运。
- P1 单帧 ≤256 KiB：drive_review_session(_via_gateway)_with_window、resolve_revision_root_launch_with_freshness、run_sc_manual_revision_turn、SC prompt 族、execute_start_coding_flow、drive_provider_run_followups。
- P1 链和 ≤1.2 MiB：inbound_inner（946K 首拆）、handle_workspace_socket、upgrade Cell、scoped_coding_ws、replay_ws_event_from_journal_event、lc_fresh 族、post_automation_continue_plan。
- 链级 2 MiB E2E 补测：ReviewOnly 经 LC gateway、SC 人工修订、Revision 返修、LC fresh、coding start；**必须裸 2 MiB（剥离 .cargo/config.toml 的 RUST_MIN_STACK=32MiB——本轮全部漏网之鱼的根源）**。
