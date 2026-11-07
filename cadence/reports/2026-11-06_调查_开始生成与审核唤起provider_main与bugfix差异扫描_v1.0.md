# 调查报告：「开始生成爆栈」与「审核唤起 provider 失败」· main(827a9901) → bugfix-b-11-02 差异扫描

- 日期：2026-11-06
- 范围：`git diff main(827a9901) → bugfix-b-11-02`（main..bugfix 共 116 个提交）；只读调查，未改产品代码、未提交、未跑 cargo
- 取证来源：前任三份切片报告（`diffscan_slice_A_web / B_product / C_engine_providers`，已读到并复核）+ 两份 gdb 诊断报告 + 本轮对 `/tmp/aria-baseline`（main 的 debug 二进制）与 bugfix 工作树 debug 二进制的 **objdump 序言栈帧静态扫描**（脚本 `/tmp/dsfr_scan.py`，结果 `/tmp/dsfr_main.txt`、`/tmp/dsfr_bugfix.txt`，已拷副本 `/tmp/dsfr_bugfix_debug` 避免读到被并行构建改写的文件）
- 核实标记：**【核实】** = 本轮我自己读码/反汇编确认；**【转述】** = 沿用切片报告，本轮未逐行复核；**【推断】** = 静态推理，无运行证据
- 写盘位置说明：诊断报告与本报告同在 bugfix 工作树 `cadence/reports/`（主仓根为 bare，无该目录内容）

---

## 1. 一句话结论

1. **症状一（开始生成/确认送审爆栈，SIGABRT/exit 134）**：根因 = debug 构建下 tokio worker 默认 2 MiB 栈被「WS inbound 深链 + provider run 任务的 poll 帧预留」叠穿；bugfix 把外壳修好后，爆点从 spawn 搬运帧后移到**送审轮任务内部**（`provider_run_drive_body` poll 帧 724,992 B + 评审引擎嵌套 async 链），叶子 `read_json::<WorkspaceSessionRecord>`。
2. **症状二（审核 reviewer 拉不起 provider）**：**是症状一的下游表现，不是独立缺陷。** 爆点在 `build_review_input` 读盘段，早于 reviewer 的 provider 启动与 ledger 登记；`RUST_MIN_STACK=32MiB` 对照实验下同二进制同驱动 reviewer 正常登记（ledger=2）、评审轮跑完。我核实的两个「同类隐患候选」均**不构成症状二成因**（见 §4）。
3. **「main 没事、分支有事」的因果链——本轮对该前提的复核结论（必读）**：
   - 前任/诊断2 的说法是「main 单块 243 KB + 深链 ≈1.74 MiB < 2 MiB ⇒ main 不越界」。**我的静态证据不支持这一前提**：
     - main 的 `provider_run.rs` 与崩溃二进制（c7e821f8）几乎一致（`git diff main c7e821f8 -- src/web/workspace_ws_handler/run/` 仅 `single_candidate.rs` 7+/9−），`tokio::spawn` 点同在 `provider_run.rs:310`、同一个未装箱 async block。
     - 对 main 的 debug 二进制（`/tmp/aria-baseline/target/debug/aria`，2026-11-03 11:40 构建）反汇编：`tokio::task::spawn::spawn::<…spawn_provider_run_with_start_mode::{closure#1}>` 序言 `sub $0x77000,%r11`（+0x38）= **487,480 B**，与第一轮 gdb 崩溃帧（487,928 B）**同量级同形态**；future memcpy 立即数 0x3b7d8 = 243,672 B（崩溃二进制 243,880 B）。
     - main 的 inbound 链帧：`handle_workspace_socket` 279,448 + `handle_workspace_inbound_message_inner` 946,968 + `…_message` 70,264 + `spawn_provider_run_with_start_mode::{closure#0}` 247,592 + `spawn::<F>` 487,480 = **2,031,752 B（已达 2 MiB=2,097,152 的 96.9%，尚未计 tokio harness 与各小于 16 KiB 的中间帧）**。⇒ **main 在 debug+2 MiB 下 StartGeneration 同样应当爆栈（静态预测，未做运行验证）**。
     - 诊断2 引文里的「≈1.74 MiB」把 spawn 帧按 sizeof(F)=243 KB 单份计，而实测 debug 下该帧是 ≈2×sizeof(F)（487 KB），口径不一致。
   - 因此「main 能跑」更可能由**环境差异**造成而非代码结构更优：`.cargo/config.toml [env] RUST_MIN_STACK=32MiB(force)` 使 `cargo run/test` 的所有线程栈为 32 MiB（诊断2 §2.1 已实证）；owner 现场是 `script -qfc <binary>` 直启、2 MiB。**我没有任何 main 二进制在 2 MiB 栈下实跑 StartGeneration 的证据**（无兼容数据副本/不得跑 cargo/不得碰共享实例）⇒ 此点标 **【推断，未验证】**，验证配方见 §7。
   - 可成立的因果链（有实证部分）：49a39400（11-01，已在 main 内）把 `provider_run.rs` 驱动体内联到单 spawn 块（698→1676 行）⇒ 栈需求已逼近/越过 2 MiB ⇒ 只在无 `RUST_MIN_STACK`+debug+多线程 runtime 时暴露。bugfix 侧 Task 1~4/9~10 的拆分把爆点逐步后移：c8223fd4/c7e821f8 → spawn 搬运帧（第一步崩）；985ef4f6/3580f3bf → 作者轮活、**送审轮崩**（实测 4/4）。**拆分没有「把好的改坏」，是把崩点从第一步推到了第二步**。帧值对照（debug 序言）：main 的任务体 poll 帧 `spawn_provider_run_with_start_mode::{closure#0}::{closure#1}::{closure#0}` = **1,330,208 B**；bugfix 拆后 `task_body` 118,464 + `drive_body` 726,024 + 臂（`drive_review_only_arm` 163,432）= 约 1.0 MiB——即**拆分本身把任务体栈占用从 1.27 MiB 降到 ≈1.0 MiB，是真改善**，但送审臂下游的 `drive_review_session_via_gateway_with_window::{closure#0}` 768,440 B 与 `drive_review_session_with_window` 710,648 B 两个大帧在 main/bugfix 完全相同（未被任何提交触及），叠上去仍越界。**【推断】main 的 ReviewOnly 链（1,330,208 + 768,440 + 193,016 ≈ 2.29 MB）静态上也已越界，只是 main 上无人在 2 MiB 栈下走到。**
   - HEAD 现状：bugfix 已有 `4b7a390b fix: 手写Tokio runtime并将worker栈固定为8MiB`（`src/main.rs`），止血已落地；本报告第 5 章治本部分在此基础上写。

---

## 2. 差异清单（main(827a9901) → bugfix 的 provider run 相关语义改动）

置信度栏为**我的核实状态**，不照抄前任。类别：A=栈/future 形态，B=准入，C=注册/绑定，D=trust/capability，E=错误分类/重试，F=取消/收尾，G=冷启动聚合（与 Story run 面无直接关系）。

| # | commit | 位置（bugfix） | 前 → 后 | 类 | 与症状关联 | 状态 |
|---|---|---|---|---|---|---|
| D1 | c8223fd4/0b2025d4/985ef4f6/511299dd/376076b8/aaf13cb4 | `web/workspace_ws_handler/run/provider_run.rs`（main 1676 行 → bugfix 359 行）+ `provider_run/{basic_arms,legacy_outline,sc_manual_revision,single_candidate_arm,task_body,cancel_settle}` | 单 `tokio::spawn(async move{…match run_kind…})` → `provider_run_task`(小外壳)+`provider_run_task_body`(poll 帧 118,464)+`provider_run_drive_body`(poll 帧 726,024)+各臂具名 boxed 函数 | A | **症状一主因所在**：外壳修复有效；drive_body poll 帧反成新大帧 | 【核实】objdump 帧值；构造机制取自诊断2 |
| D2 | 同上 | `provider_run.rs:344` spawn 点 | 未装箱 243 KB future 搬运 → `tokio::spawn(provider_run_task(..))` 小外壳 | A | 消除 spawn 搬运帧崩（第一步崩） | 【核实】bugfix spawn 帧已不在 ≥16 KiB 列表 |
| D3 | 4b7a390b | `src/main.rs` | `#[tokio::main]` → 手写 Builder，`thread_stack_size(8 MiB)` | A | 止血；诊断2 §2.1 的 32 MiB 对照实验同类 | 【核实】读到源码；运行效果未验证 |
| D4 | fd0b9a84 | `product/coding_workspace_engine/provider_stream/launch.rs:121-131` `provider_adapter_error_from_gateway` | gateway `Adapter(inner)` 由「拍平为 ProviderUnavailable+Display」→ **保留 inner 结构化字段**；其余映射不变 | E | **与症状二无关**（仅 Coding 引擎 provider_stream 路径；Story/Design 的 reviewer 走 `review/drive.rs:1507/1568` 自己的映射，该文件非测试部分未改） | 【核实】见 §4 候选2 |
| D5 | fd0b9a84 | `product/lifecycle_store/workspace.rs:622-630` `is_logical_codebase_session` | `matches!(resolve(..), Ok(Some(_)))` → `.is_ok()`（issue 归属字段必填后语义等价于「issue 记录可读」） | C | **与症状二无关**（候选1 被推翻，见 §4） | 【核实】 |
| D6 | fd0b9a84（块3） | `product/models/project.rs:143-144`、`issue_store.rs:184-203` | `IssueRecord.logical_codebase_id: Option<String>` → 必填 `String`；旧记录缺失/空串 ⇒ `InvalidRecord(issue_not_bound_to_logical_codebase)`，零迁移 | B/C | 存量单仓 issue 读取即报错；**与本次症状无关**（Author 已成功启动说明该 issue 带归属），但是独立的存量数据风险（§6.D） | 【核实】 |
| D7 | fd0b9a84（块3） | `coding_attempt_store/admission.rs:572-592` | `None => Legacy{..}` → `None => RepositoryRouting::unbound_issue()`；删 `legacy_snapshot_digest` 放行臂 | B | 无冻结快照的 coding attempt 一律 fail-closed；**不在 Story ReviewOnly 链上** | 【核实】 |
| D8 | fd0b9a84（块3） | `web/handlers/support.rs:224-246`、`web/coding_start.rs:813-832`、`web/advance_plan.rs:214-288`、`workspace_engine/advance.rs:1064-1096` 等 | `Legacy`/`SingleRepository` 臂整体删除 → 无归属 400/`coding_start_enrollment_mismatch`/`enrolled advance carrier drift` | B | 存量无 LC 绑定数据的阶段性 fail-closed（§6.D）；与症状无关 | 【转述】（抽查 admission/issue_store 属实，其余未逐行读） |
| D9 | 677945c0/8eda119e/9b236078/667053d3/58b2e88f | `logical_codebase/provider_capability_store.rs:718-846`、`web/gateway_factory.rs`、`production_policy_resolvers.rs` | capability 记录 per-project/per-LC（`.aria/projects/<pid>/logical-codebase/capabilities.json`）→ **机器级** `.aria/provider-capabilities/capabilities.json`；**无旧记录迁移** | D | 升级后旧 Confirmed 不可见 ⇒ launch cell Unknown ⇒ gateway fail-closed。**对 Author/Reviewer 对称**；本次现场 Author 已起、reviewer 在 gateway 之前就崩，故**非本次成因**；是独立的升级风险（§6.C） | 【核实】store 构造/路径；后果【推断】 |
| D10 | fd0b9a84+ff473d93 | `web/handlers/aggregate_initialization/handlers.inc.rs:97-196,749-772` | 聚合初始化 provider 写死 claude → 选择器（L1 health + L2 机器级 capability，候选 Pi>Claude>Codex）；`select_aggregate_provider_blocking` 在同步入口内 `std::thread::spawn`+新建 current-thread runtime+`join` | G/A | 冷启动面；`std::thread::spawn` 子线程栈为 Rust 默认 2 MiB（受 `RUST_MIN_STACK` 影响，不受 tokio Builder 影响）；帧值未测（§6.A） | 【转述】+源码行定位 |
| D11 | fd0b9a84/77640c99 | `aggregate_initialization/root_recipe_audit_windows.inc.rs`、`logical_codebase/root_recipe_*` | 零变更 turn 判失败；审计预算改按变更清单计费 | G | 冷启动面，与 Story run 无关 | 【转述】 |
| D12 | fd0b9a84 | `workspace_session/manager/mod.rs:347-366`、`attachment.rs:111`、`router.rs:381,629,647,677,697` | SessionState 帧新增同步现读 3 个 JSON 注入 `plan_continue_available` | A（IO） | 无新 `.await`、不增 future；仅增广播同步 IO | 【转述】 |
| D13 | 82658d58 等 | `cross_cutting/{codex,pi,claude_code}_provider/projection.rs`、`streaming_provider/bootstrap_marker.rs` | marker digest 抽共享实现，加 `root_recipe_phase` 门 | D | 共享实现的字段序（provider、action、operation id、step、credential root、marker root、receipt context）与 main 的 claude 私有实现逐项一致，schema 前缀同 `lc-claude-bootstrap-marker-v1`；**仅 root-recipe 相位使用，Reviewer 意图不带 marker**；未实算 digest 值 | 【核实】字段序；digest 值【未验证】 |
| D14 | fd0b9a84 | `product/workspace_engine/lifecycle.rs:826-836,989-999` | `issue_lc_id: None` 比较改为冻结 legacy 别名再比 | B | 放宽一处 fail-closed | 【转述】 |
| D15 | fd0b9a84 | `coding_workspace_engine/gates_parts/worktree_lock.inc.rs`、`handoffs.rs`、`gates.rs` | 删 `IssueSharedWorktreeRoute::Legacy` 6 臂；handoffs group 作用域改读仓维记录 | C/F | 失败收尾现在真的会释放他人锁（旧缺陷修复）；Coding 路径 | 【转述】 |
| D16 | 全程 | `workspace_engine/provider_drive.rs`、`review/drive.rs`（非测试）、`lifecycle_store/{workspace,human_gate}.rs` 的 ledger/指纹/binding 读写点 | **零语义变更**（`logical_launch_fingerprints`、`work_item_runtime_binding`、`provider_start_ledger`、`superseded_native_sessions` 读写点与 main 同文件同逻辑） | C | 排除「run 注册/指纹/ledger 语义变化致 reviewer 未登记」 | 【核实】`git diff --stat` 无这些文件；三份切片一致 |

> 工作树现状（未提交，仅供上下文，不属于 diff 对象）：`git status` 显示 `provider_run.rs`、各臂、`review/drive.rs`、`stack_budget.rs` 有未提交修改，新增 `plan_batch.rs/plan_draft.rs/plan_revision.rs`——并行实施 agent 正在做治本拆分（`TaskRootFixBoxedArms` 等）。本报告的帧值取自 13:20 构建的 debug 二进制副本。

---

## 3. 症状一根因（爆栈）

### 3.1 证据链

1. **崩溃形态**（诊断2 §1，gdb，4/4 复现）：线程 `tokio-rt-worker` SIGSEGV，叶子 `read_json::<WorkspaceSessionRecord>`（`json_store.rs:36`，序言 `sub $0xe78` 仅 3,704 B），RSP 低于 2 MiB 线程栈底。调用链：`read_json ← read_workspace_session_record ← find_workspace_session_path ← get_workspace_session ← list_human_dispositions ← human_dispositions_section ← build_review_input ← drive_review_session_via_gateway_with_window::{closure#0}::{closure#0}（review/drive.rs:431）← …（:373）← drive_review_only_arm（basic_arms.rs:87）← provider_run_task_body（task_body.rs:339/219）← drive_until_cancelled_settles ← provider_run_task ← tokio harness`。
2. **崩溃线程是已 spawn 的 review 任务**（帧 #23 以下全是 tokio harness），栈是全新 worker 栈，**不叠加 WS inbound 链**。⇒ 我更正诊断2 §0 与任务简述里「+ WS inbound 深链 ~1.5 MiB 在同一 worker 栈」的说法：**送审轮崩溃的栈上没有 inbound 链**（inbound 链只存在于第一步 `StartGeneration` 的 spawn 帧崩溃里）。送审轮单靠任务自身 poll 链就超限。
3. **帧量（本轮 objdump 序言静态量，bugfix debug 二进制）**：

| 帧（poll 闭包） | 预留 B | 来源 |
|---|---:|---|
| `provider_run_task_body::{closure#0}` | 118,464 | `/tmp/dsfr_bugfix.txt` |
| `provider_run_drive_body::{closure#0}` | 726,024 | 同上（诊断2 实测 724,992，13:20 构建微差） |
| `drive_review_only_arm::{closure#0}` | 163,432 | 同上 |
| `drive_review_session_via_gateway::{closure#0}` | 193,016 | 同上 |
| `drive_review_session_via_gateway_with_window::{closure#0}` | 768,440 | 同上，**main 同值，未被任何提交修改** |
| `drive_reviewer_provider_session_once::{closure#0}` | 64,408 | 同上（在 `build_review_input` 之后才进入） |
| `ensure_review_invocation_scope::{closure#0}` | 35,352 | 同上 |

   到 `build_review_input` 为止的上界和 ≈ 118,464+726,024+163,432+193,016+768,440 = **1,969,376 B ≈ 1.88 MiB（占 2 MiB 的 94%）**，再加 `build_review_input`/`human_dispositions_section`/`list_human_dispositions`/`get_workspace_session`/`find_*`/`read_*` 的同步中间帧、tokio harness 与叶子 3.7 KiB，越过 2,097,152。与 gdb 崩点（读盘叶子，早于 reviewer ledger 落盘）吻合。该和是**上界**：`…_via_gateway` 与 `…_with_window` 是否在栈上同时存活，gdb 栈里 review/drive.rs:373 与 :431 两层均可见，故成立；两个 `drive_review_session*` 兄弟路径（`drive_review_session_with_window` 710,648 / `drive_review_session` 190,616）是否也被叠加【未验证，符号名嵌套不等于调用嵌套】。
4. **对照实验**（诊断2 §2.1，已实测）：同二进制同驱动，`RUST_MIN_STACK=33554432` ⇒ 评审轮 103 s 跑完、进程存活、ledger=2；不设变量 ⇒ 同点位 4/4 崩。⇒ 因果定性为栈深而非逻辑错误。
5. **止血已进工作树**：`src/main.rs`（commit `4b7a390b`）手写 Builder `thread_stack_size(8 MiB)`；注释自述「实测链 ≈1.9 MiB，8 MiB ≈ 4 倍余量」。运行效果本轮**未验证**（不得跑 cargo/起服务）。

### 3.2 为什么作者轮活、送审轮崩

同一 `drive_body` poll 帧（726 KB）、同一外壳；差别在臂下的引擎链深度：
- Author 臂 → `handle_user_message_from_run`（43,544）→ `handle_author_message_with_prompt_mode`（85,080）≈ 118,464+726,024+41,208(author arm)+43,544+85,080 ≈ **1.01 MB**，余量 ≈1 MB。
- ReviewOnly 臂 → `drive_review_only_arm` 163,432 + `via_gateway` 193,016 + **`with_window` 768,440** ≈ **1.97 MB**，越线。
关键增量不是 drive_body，而是 **`drive_review_session_via_gateway_with_window` 自身 768 KB 的 poll 帧**（review/drive.rs:373→431 嵌套 async 闭包 + `and_then`，debug 下把所有 `match` 臂的临时区摊在同一帧）。这个帧在 main 里同样存在——它是**潜伏的第二个越界点**，不是 bugfix 引入。

### 3.3 为什么 main「没事」——结论与保留意见

- **可确证的**：main 的 StartGeneration 链与崩溃二进制结构一致（§1），静态帧和 ≈2.03 MB，已贴 2 MiB；main 的任务体 poll 帧 1,330,208 B + `with_window` 768,440 B + `via_gateway` 192,952 B ≈ 2.29 MB，**ReviewOnly 在 main 上也不应放得下**。
- **无法确证的**：main 在 2 MiB 栈下是否真的崩（缺实跑）。「main 没事」的观察来源很可能是：①`cargo run/test` 下 `.cargo/config.toml` 的 `RUST_MIN_STACK=32MiB(force)`；②从未在 debug+裸二进制+真实多线程 runtime 上完整走过「开始生成→确认送审」；③release 帧小得多（未测）。
- **owner 现场进程**（`pid 2507207`，`script -qfc …/target/debug/aria`，exe 已被删除，说明是旧构建）的 `/proc/<pid>/environ` 无 `RUST_MIN_STACK`、profile=debug，与上述解释一致。（仅读 `/proc`，未触碰 `/tmp/aria-verify|deploy` 数据。）
- ⇒ 对 Main 的建议：**不要在对外结论里写「main 不越界」**；应写「main 同样有越界风险，只是被 `RUST_MIN_STACK=32MiB` 与测试环境掩盖；bugfix 的拆分把崩点后移，并未引入新的越界机制」。若 owner 坚持「main 在 owner 真机上能走完全流程」，需补一次 main 二进制裸跑实测（§7 V1），这会直接改变本节结论的措辞但不改变修法。

---

## 4. 症状二根因（reviewer 起不来）

### 4.1 明确回答：**是症状一的下游表现，不是独立缺陷**

依据（强→弱）：
1. **时序**：爆点 `build_review_input`（`prompts/review.rs:186`）发生在 `drive_reviewer_provider_session_once`、`gateway.start_streaming`、reviewer `provider_start_ledger` 登记**之前**。provider 根本没被拉起，自然「reviewer 拉不起 provider」；owner 现场 ledger 只有 author 条目、`logical_launch_fingerprints=[]`、`work_item_runtime_binding=None`，与「崩在 reviewer 登记之前」一致（崩溃会留下 Running/CrossReview 状态，下次重启必打 `stale zombie run recovered`，是症状而非原因，诊断2 §3.2）。
2. **对照实验**：32 MiB 栈下同序列 reviewer 正常登记（ledger=2）并跑完评审。
3. **ledger 三事实不构成差分信号**：
   - `work_item_runtime_binding=None`：该字段仅在 `compile/finalizer.rs:518` 为 **WorkItem** 子会话写入；Story/Design 会话本来恒 None，不是异常。【核实】
   - `logical_launch_fingerprints=[]`：写入点（`gateway_start.rs:297-307 persist_prepared_launch_identity`、`provider_drive.rs:179`、`review/drive.rs:930`）均 main/bugfix 同文件同逻辑（`git diff --stat` 无这些文件）。【核实】
   - `provider_start_ledger` 只有 author：reviewer 登记在崩点之后，不是语义变化。
4. **产品层 run 状态机零变更**（三份切片一致 + 我抽查）：`logical_launch_fingerprints / work_item_runtime_binding / provider_start_ledger / superseded_native_sessions / run_history` 的读写点在 main/bugfix 同文件同行；`review/drive.rs` 非测试部分 diff 仅 3 处测试夹具（`@@ -2071/-2182/-2381`）。

### 4.2 候选1（`is_logical_codebase_session` 判据放宽 ⇒ 补写 review_identity_id ⇒ 改变 reviewer policy digest ⇒ window id 漂移）——**核实后推翻**

读 `lifecycle_store/workspace.rs:585-630` 与 `workspace_engine/review/drive.rs:1876-1887`、`prompts.rs:460`：
- 判据改动属实：`matches!(…, Ok(Some(_)))` → `.is_ok()`。因 `IssueRecord.logical_codebase_id` 已必填（`issue_store.rs:184-203`），`Ok(Some)` 与 `Ok(_)` 语义等价；**唯一的行为差**是「issue 为 legacy 别名 LC」的**存量**会话（`review_identity_id` 为空，即新建会话之前的旧 JSON）读取时会被补写一次（写盘 + 刷新 `updated_at`）。
- **但 `reviewer_verify_policy()` 的门不是 `review_identity_id`，也不是 issue 记录**：第 ③ 分支门是 `is_logical_codebase_issue()` = `logical_provider_gateway().is_some()`（`prompts.rs:460`，Web 接入 task 注入 gateway 即为 LC）。单仓路径未注入 gateway ⇒ policy 仍为 `None`。补写身份**不会**使单仓会话的 policy 从 None 变 Some。前任报告把「身份被补写」误读为「policy 被启用」。
- 新建会话 `create_workspace_session` 恒写非空 `review_identity_id`（`workspace.rs:355`），补写分支本就走不到；本次现场会话为新建，**不受影响**。
- 唯一残留影响：旧 legacy-别名会话首读多一次写盘；代码里的测试 `legacy_single_repo_session_read_does_not_write_back_identity` 用「无 issue 记录」的夹具，**不覆盖**「有 legacy 别名 issue 记录」这一新命中面，且 doc 注释（"issue 记录缺失或不可判定一律按单仓"）与实现不再一致。**严重度：低**（写放大/注释失实，不影响 reviewer 启动）。
- 结论：**候选1 不是症状二成因**；降级为「低严重度残留」，建议补一条测试 + 修注释（§6.D-4）。

### 4.3 候选2（`provider_adapter_error_from_gateway` 保留 adapter 字段 ⇒ transport 失败变 retryable ⇒ 多起 run ⇒ 加深栈）——**核实后限定范围，不成立于症状二**

- 属实部分：`launch.rs:121-131` 现在 `Adapter(inner) => inner.clone()`，`adapter_error_message`（`provider_retry.rs:1295`）把 `stdout/stderr` 拼进 message，`classify_transport_message`（`:1322`）对 `connection/broken pipe/process exited/timeout/eof/stream ended/…` 关键词判 retryable。
- **范围**：该函数只被 `coding_workspace_engine/provider_stream/launch.rs:76` 调用，`launch_provider_session` 的调用方全在 `provider_stream.rs`（Coder / CodeReviewer / InternalReviewer 的 **Coding** 路径）。**Story/Design/WorkItemPlan 的 reviewer 走 `workspace_engine/review/drive.rs:1507/1568 map_gateway_error_to_adapter`**——该文件非测试部分 main→bugfix 无改动。⇒ **症状二所在路径不经过候选2**。
- **深栈放大**：重试发生在**同一任务内的 for 循环**（`provider_retry.rs:329/518`，`MAX_PROVIDER_INVOCATIONS_PER_CYCLE = 3`），不是新任务；循环里每次 attempt 的 future 若是同一个 async fn 的多次 await，**不叠加栈帧**（栈只随同时存活的嵌套 poll 深度增长）。⇒「多起 run ⇒ 间接加深栈」**不成立**。
- 残留影响：Coding 路径下 stderr 型 transport 失败从 non_retryable 变 retryable，**最多多试 2 次**；属行为变更（修复了旧注释自陈的误判），**严重度：低**，与症状无关。

### 4.4 也已排除的症状二候选

| 候选 | 判断 | 依据 |
|---|---|---|
| capability 机器级化零迁移 ⇒ launch cell Unknown ⇒ gateway 拒 | 对 Author/Reviewer 对称；**Author 已成功**；reviewer 崩点在 gateway 之前 | §4.1-1；store 改动【核实】 |
| `evidence_mediator` ④b 守卫（库目录缺失 fail-closed） | 不影响 `read_json` 叶子；是否 reviewer 与 author 对称【未验证】 | 切片 B |
| 准入/指纹/trust 语义变更（块1/块3） | 影响的是**存量/无 LC**数据，本现场 Story 会话 Author 已启动，说明 issue 已绑 LC | §6.D |
| `Box::pin` 递归环（compile.rs:599↔compile_failure_surface.rs:662↔review/routing.rs:696） | 非本次崩溃路径；另开缺陷 | 诊断1 Q2 |

---

## 5. 治标 + 治本方案

> 并行实施现状（读 `git status`/`git diff`，不属于本调查产出）：工作树未提交改动已包含 **Task 14b**——`review/drive.rs` 把 `drive_review_session_via_gateway_with_window` 与「开窗+采集材料」装箱（`Box::pin`），`provider_run/*` 各臂改动 + 新增 `plan_batch/plan_draft/plan_revision.rs`，`stack_budget.rs` +151 行。下列方案与之**对齐而不重复**；「预期效果」是我据 §3 帧表的**静态预估，未实测**。

### 5.1 治标（止血）

| 项 | 内容 | 状态/效果 | 风险 | 验证 |
|---|---|---|---|---|
| S1 | `src/main.rs` 手写 `Builder::new_multi_thread().thread_stack_size(8 MiB)`（commit 4b7a390b，已在 HEAD） | 实测链 ≈1.9 MiB ⇒ 4× 余量；32 MiB 对照实验已证同类做法有效 | 只加大 **tokio worker** 栈：①`std::thread::spawn` 线程（`select_aggregate_provider_blocking`、cli_adapter 读线程等）仍是 2 MiB（受 `RUST_MIN_STACK` 影响，不受 Builder 影响）；②`spawn_blocking` 池线程**受 Builder 影响**（同一 Builder 设置）；③future 继续膨胀仍会再爆（每 8 MiB 只是推迟） | 起 debug 二进制，裸跑（不设 `RUST_MIN_STACK`）→ 开始生成 → 送审，进程存活；`[aria-runtime] profile=debug worker_stack_bytes=8388608` 日志行 |
| S2 | 部署侧兜底：服务启动环境注入 `RUST_MIN_STACK=33554432`（对 `std::thread` 线程也生效） | 诊断2 §2.1 已实测 | 全进程线程虚拟栈上限抬高，RSS 不变；与 S1 叠加无冲突 | 同上 |
| S3 | **用 release 二进制做 owner 验收**（帧远小于 debug） | 【推断，未测】release 帧小一个量级；不能当修复 | 掩盖结构问题 | 无 |

能否本轮做完：S1 已在工作树；S2 为部署配置。**未验证**项：S1 在 owner 现场二进制（旧构建，exe 已被删）上无效，必须**重新构建并重启**服务才生效。

### 5.2 治本（结构收口，把栈占用从「摊在 poll 帧」改成「落在堆」）

| 项 | 改哪里 | 预期效果（静态预估） | 风险 | 验证 | 本轮可做？ |
|---|---|---|---|---|---|
| R1 | `provider_run_drive_body`：各臂改为**返回 `Pin<Box<dyn Future>>` 的具名构造函数**（`fn make_review_only_arm(..)->Pin<Box<..>>`），分发点只持有胖指针（诊断2 §5.1 首选）。`task_body.rs`/`basic_arms.rs`/`legacy_outline.rs`/`sc_manual_revision.rs`/`single_candidate_arm.rs`、新增 `plan_*` 臂文件 | drive_body poll 帧 726,024 → 目标 ≤64 KiB；ReviewOnly 链少 ≈660 KB | 构造点函数本身若被内联进父帧会失效（需 `#[inline(never)]`）；`Box::pin(arm(args))` 只挪存储不挪构造临时区（诊断2 已证） | ① `stack_budget.rs` 把门禁量改为 **poll 帧预留**（见 R4）② objdump 重扫 `provider_run_drive_body::{closure#0}` 序言 ③ gdb/2 MiB 真实 WS 送审轮 | **进行中**（TaskRootFixBoxedArms 在做，未提交） |
| R2 | `review/drive.rs`：`drive_review_session_via_gateway_with_window` 与 `build_review_input` 链装箱（Task 14b，未提交已见 diff）；进一步：**把 `with_window` 的 768 KB 帧本身拆开**（其 `match`/`and_then` 臂各抽成 `#[inline(never)] async fn` 或装箱） | `Box::pin(...)` 把 `with_window` 的**存储**挪堆，但其 poll 帧在 debug 下仍可能预留整份构造区——**需 objdump 复测其序言是否下降**；若不降，须拆函数 | `Box::pin` 单独对 debug 帧帮助有限（同诊断1 Q3 结论）；拆函数改动面大于装箱 | objdump 比对 `drive_review_session_via_gateway_with_window::{closure#0}` 序言前/后（现 768,440） | 装箱已在工作树；拆函数需下一轮 |
| R3 | `review/drive.rs` 中 `drive_review_session_with_window`（710,648）/ `route_legacy_review`（197,160）/ `apply_policy_route`（230,232）/ `complete_silent_output_retry`（220,040）/ `complete_review`（159,000）等同族大帧同法处理 | 为 SC/WorkItemPlan 送审路径（§6.A 的 HIGH 项）预留余量 | 同 R2 | 同 R2 | 建议排下一轮 |
| R4 | **门禁修正**：`stack_budget.rs` 现门禁量的是 **future 未装箱 size**；诊断2 已证真正的栈成本是 **poll 帧预留**（序言 `sub $imm`）。改为：①CI 对 debug 二进制扫序言（可复用 `/tmp/dsfr_scan.py` 思路）对 `provider_run*`、`drive_review_*`、`handle_workspace_inbound_message_inner` 等白名单符号设上限；②保留 2 MiB 真实 WS 送审轮冒烟（75056bb4 的 C 层门禁，仅在**无 `RUST_MIN_STACK`** 下有意义：`.cargo/config.toml` 的 32 MiB `force=true` 会让 `cargo test` 永不复现，需用独立子进程/`env_remove` 或二进制冒烟） | 防回归；让「帧涨 → CI 红」发生在 PR 前 | 序言扫描对编译器版本/优化敏感，阈值需留 20% 余量 | 红-绿：当前（未修复）二进制必红、修复后必绿 | 可与 R1 同轮 |
| R5 | `handle_workspace_inbound_message_inner`（947,192）自身：inbound 各臂（StartGeneration/Confirm/HumanGate 终止…）拆成具名 `async fn`，分发处装箱 | 第一步（StartGeneration）链 ≈2.03 MB → ≈1.2 MB；给「inbound 深链 + spawn」留 0.8 MB 余量 | 改动面中等；`handle_workspace_inbound_message` 公共入口签名不变 | objdump 序言 + 2 MiB WS 冒烟 | **建议本轮并入**（它是诊断1 第一步崩的另一半成因，目前只靠外壳变小避开） |
| R6 | 去掉「`std::thread::spawn` + 新建 runtime + join」在 tokio worker 上同步阻塞的写法（`handlers.inc.rs:749-772`）：改 `spawn_blocking` 或把调用方改 async | 避免 worker 被阻塞、且线程栈受 Builder 统一控制 | 调用方改签名牵涉 `ensure_lc_initialization_started` 链 | 聚合初始化/注册后自动推进回归 | 非本次症状，建议登记缺陷 |

**推荐顺序**：S1（已在）→ R1+R2（进行中）→ R4（门禁量对）→ R5/R3（余量）→ R6（独立）。**本轮能否做完**：S1、R1、R2 装箱部分已在工作树；R4 与 R5 取决于并行 agent 现行范围，**我无法判断其是否已纳入**——需 Main 对照 `TaskRootFixBoxedArms`/`Task10bCLayerReviewRound` 的交付确认。

---

## 6. 同类问题扫描清单

方法：对 bugfix debug 二进制副本（13:20 构建，含未提交装箱前后的混合状态，见 §2 末注）与 main 的 debug 二进制（11-03 构建）做序言栈帧静态扫描（`sub $imm,%r11`（probe 循环）+ `sub $imm,%rsp`，≥16 KiB 的函数共 main 379 / bugfix 365 条），再据源码调用关系估链。**所有「链和」均为上界/静态估计，不等于实测栈深**（符号名嵌套不等于调用嵌套；未计 tokio harness、<16 KiB 中间帧与同步叶子帧）；每条后标【核实】/【推断】。2 MiB = 2,097,152 B。

### 6.A 大栈帧 / 深 poll 链

| # | 位置（任务栈） | 帧链（B） | 占 2 MiB | 现象/风险 | 已被本次症状覆盖？ | 严重度 | 建议 |
|---|---|---|---|---|---|---|---|
| A1 | ReviewOnly 任务：`provider_run_task_body`→`drive_body`→`drive_review_only_arm`→`…via_gateway`→**`…via_gateway_with_window`** | 118,464+726,024+163,432+193,016+768,440 = **1,969,376**（到 build_review_input 前） | 94% | 实测 4/4 崩 | **是（症状一/二）** | 致命 | R1+R2+R3 |
| A2 | SC 人工修订臂：`drive_human_gate_sc_revision_arm`(344,496)→`run_sc_manual_revision_turn`(324,928)→`apply_policy_route`(230,232)→`complete_silent_output_retry`(220,040) 叠 task_body+drive_body | 118,464+726,024+344,496+324,928+230,232+220,040 ≈ **1,964,184**（若嵌套成立） | ≈94% | **与 A1 同量级**；臂在 R1 之后 drive_body 降到 ≈64 KB 时降为 ≈1.3 MB，否则同样越界。**嵌套关系为符号名推断，未验证** | **否** | **高（待 R1 后复测）** | R1 后重扫序言；若 `run_sc_manual_revision_turn` 仍 >300 KB，同法拆/装箱 |
| A3 | 单候选 author 臂：`run_single_candidate_author`(271,528)+`complete_single_candidate_work_item_plan_author`(221,352)+`apply_policy_route`(230,232) 叠 task_body+drive_body+`drive_single_candidate_author_arm`(113,288) | 118,464+726,024+113,288+271,528+221,352+230,232 ≈ **1,680,888** | 80% | WorkItemPlan 单候选路径；余量仅 ≈0.4 MB | 否 | 中高 | 同 A2 |
| A4 | 第一步 `StartGeneration`（WS 任务栈）：`handle_workspace_socket`(279,448)+`handle_workspace_inbound_message_inner`(947,192)+`…_message`(70,264)+`spawn_provider_run_from_handler`(34,456)+外壳 | bugfix ≈1,331,360 + spawn 外壳（<16 KiB） | ≈63% | 已无 243 KB 搬运帧；**main：2,031,752（97%）** | **是（第一步崩，已被外壳修复）** | 中（余量 0.7 MB，被 inbound 其它臂吃掉的风险见 A5） | R5 |
| A5 | WS inbound **其它臂**：`handle_human_gate_termination_from_handler`(90,248)→`apply_human_gate_termination`(78,920)、`handle_confirm_from_handler`(59,064)、`refresh_workspace_context_for_session`(66,696)、`handle_human_gate_feedback_from_handler`(43,048)、`dispatch_lc_fresh_recovery_run`(94,120) 叠在 1.30 MB 之上 | 1,296,904 + 各臂（<0.25 MB 级）≈ **1.4~1.55 MB**（各臂**下游引擎调用深度未量**） | ≈67~74% | 有 0.5~0.7 MB 余量；`handle_confirm_from_handler` 下游进 engine（`begin_review_after_author_confirm`）链深度未测 | 否 | 中 | R5 后复测；序言门禁（R4）覆盖 |
| A6 | Coding runner（独立 spawn 任务栈）：`execute_start_coding_flow::{closure#0}` | **807,936**（main 803,088，+0.6%）；其下 `execute_coding_with_commands_outcome` 190,936、`run_structured_provider_stream_to_completion_with_partial_output` 183,896、`execute_coder_fix_from_review_outcome` 141,120、`execute_code_review_with_commands` 127,488、`run_code_reviewer_with_retry_cycle` 126,296 …；spawn 搬运：`Cell::new<spawn_coding_runner_task::{closure#0}>` 271,616 | 链 ≈1.3 MB（主干）至 ≈1.5~1.7 MB（叠评审臂，嵌套未验证）；**spawn 点搬运 ≈ 2×135 KB = 271 KB 叠在调用方（HTTP/WS 栈）** | **同类隐患：单帧 808 KB 与 drive_body 同量级**；与 bugfix 变动无关（main 同值） | 否 | **中高** | 纳入 R4 序言门禁；`execute_start_coding_flow` 臂拆分/装箱（与 R1 同法）；先实测一次 coding 全链 |
| A7 | HTTP human-action / confirm / continue-plan 处理器（hyper 连接任务栈，全新） | `post_automation_continue_plan` 291,640、`post_workspace_failed_sc_run_retry` 142,760、`post_workspace_human_action` 147,448（+`handle_request` 148,616）、`workspace_session_confirm` 125,464；入口经 `spawn_provider_run_from_handler`/`request_provider_run` 事件接力，**provider run 本体不在这条栈上**（router.rs:198 独立 spawn） | 各 ≈14%~22% | 低；是 `ProviderRunRequested` 事件 → relay 任务（router.rs:198、mapping.rs:380）→ 外壳，**不是同栈叠加**（诊断2 §3.3 已核） | 否 | 低 | 保持；R4 门禁覆盖 |
| A8 | 自动驾驶编排器：`serve_web::{closure#3}` 的 `Cell::new` 216,704；`AutopilotOrchestrator::reconcile` 214,568、`reconcile_all_once` 75,560、`run` 73,112、`coding_chain_stage` 77,352、`human_stop_point` 76,200 | spawn 搬运 217 KB + 任务体 ≈0.5 MB | ≈25% | 低（独立 spawn，链不深）；注意 `human_stop_point` 内 `get_or_create` 63,640 | 否 | 低 | 纳入 R4 |
| A9 | `select_aggregate_provider_blocking`（`handlers.inc.rs:749-772`）：`std::thread::spawn`（**2 MiB 默认栈，不吃 tokio Builder**）+ current_thread `block_on(select_aggregate_provider)` + 外层阻塞 tokio worker 等 `.join()` | 子线程内 selector 帧未单独测（≥16 KiB 列表中 `select_aggregate_provider` 未出现，即 <16 KiB） | 低 | 同步阻塞 worker（切片 A §2.1）；DEBUG 下 health/capability 串行检查会拉长占用；`RUST_MIN_STACK` 能覆盖其栈 | 否 | 低-中（阻塞而非栈） | R6 |
| A10 | 聚合初始化 turn driver：`GatewayBackedAggregateProviderTurnDriver` 帧 43,432（bugfix）/ 42,072（main） | 小 | <3% | 低 | 否 | 低 | 无 |
| A11 | `drive_review_session_with_window`（710,648）/ `route_legacy_review`（197,160）/ `complete_review`（159,000）：legacy 单仓评审臂 | 与 A1 同族；legacy 臂仍在（`is_logical_codebase_issue()==false` 才走） | 高 | 同 A1 但不在 LC 现场路径上 | 否 | 中 | R3 |

另：`/tmp/frames3.txt`（他人对 bugfix debug 二进制的 ≥128 KiB 立即数粗扫）把 `18446744073709551615` 等无关立即数混入，**不可用**，本报告未采用。

### 6.B 依赖 run 注册 / 指纹 / ledger 的判定点（`logical_launch_fingerprints` 空、`work_item_runtime_binding=None` 时的行为）

全部判定点 main/bugfix 同文件同逻辑（【核实】`git diff --stat` 对这些文件为空）。逐点后果：

| 判定点 | 位置 | 空/None 时行为 | 是否误判 | 严重度 |
|---|---|---|---|---|
| native resume 候选 | `workspace_engine/lifecycle.rs:1305-1336` | `map_records` 为空 ⇒ 落入「fullmap 纪元之前的旧历史」⇒ 维持既有恢复（不 fail-closed） | 不误判；但**在 LC 会话上放行旧历史 resume**，是既有设计 | 低 |
| revision 重驱 | `review/drive.rs:1141-1196` | `(Some(_), None)`：指纹缺失 ⇒ **fail-closed 新会话**（`resume_allowed:false`）；`(None,_)` ⇒ 新会话允许 resume | 保守 | 低 |
| 内存 `logical_launch_fingerprints: HashMap` | `lifecycle.rs:336/616` 构造空；`provider_drive.rs:179`、`review/drive.rs:930` 写 | **进程重启即清空**（仅内存），重启后首次修订走 `(Some,None)` 新会话 | 保守，但会让「崩溃 → 重启 → 重试」多一次新会话（不是 bug） | 低 |
| 恢复前置摘要 | `conversational_gate_recovery.rs:584`、`single_candidate.rs:323` | `previous_fingerprint_digest: None` 传下游 | 取决于下游对 None 的处理（未逐行读）【未验证】 | 低 |
| durable prepared 身份 | `web/workspace_ws_handler/run/gateway_start.rs:283-307 persist_prepared_launch_identity`（唯一调用点 `:663`，role 硬编码 `Author`） | 幂等写 `(Author, provider, role_run_seq)` 的 prepared 指纹 | 只有 **Author** gateway 起点写 durable 指纹；`review/drive.rs:930`、`provider_drive.rs:179` 只写**内存** HashMap。现场「ledger 仅 author、`logical_launch_fingerprints=[]`」中**指纹为空**与「Author 经 gateway_start 成功」略不一致：可能 Author 走了不经 `gateway_start.rs:663` 的启动路径（如 `provider_drive` 直驱），或该会话为 `FreshRunning` 以外形态——**未取得会话 JSON，无法判定，标【未验证】**；该项在 main 与 bugfix 逻辑相同，故不构成差分成因 | 低 |
| coding 起点绑定匹配 | `web/handlers/coding.rs:365-380` | `work_item_runtime_binding` 六元组不匹配（含 None）⇒ 落「仓默认」回退 ⇒ LC 合成仓 `default_provider_mode` 空 ⇒ `500 default_provider_not_configured`（r41 注释自述） | **会误判**：WorkItem 会话未被 finalizer 写 binding 时（旧会话/中断编译）整条 coding 起点 500 | 中；main 同，非回归 |
| stale zombie 恢复 | `workspace_session/manager/durable_projection.rs:547-570 recover_stale_run_if_zombie` | 重启时内存 run 注册表空 + stage∈{Running,CrossReview,Revision} ⇒ 回 prepare_context | 爆栈 ⇒ zombie（症状），非误判 | — |
| entity run lease / enrollment 终态核验 | `provider_run.rs` spawn 前（:212/:234） | 与 Task 1 同序、全在 spawn 前（诊断2 §3.1 逐行对照） | 不误判 | — |

### 6.C 其它信任 / 预算 / 取消语义改动（可能同样影响 reviewer）

| # | 位置 | 改动 | 对 reviewer 的影响 | 已覆盖？ | 严重度 | 建议 |
|---|---|---|---|---|---|---|
| C1 | `provider_capability_store.rs` + `gateway_factory.rs` | capability 机器级、**无迁移**，旧项目级 `capabilities.json` 不再读 | 若真机只在旧路径核验过，Author/Reviewer **都**会在 gateway launch cell 处 fail-closed；本次现场 Author 已起 ⇒ 说明机器级 claude/pi 已 Confirmed。**reviewer provider 若与 author 不同（如 codex）且未核验，会在本次爆栈修复后暴露为新的「reviewer 起不来」** | 否 | **中高（升级门槛）** | 【未验证】读真机 `.aria/provider-capabilities/capabilities.json` 各 provider 三态；UI 提示「需重新核验」；评估一次性迁移 |
| C2 | web `app.rs:181-200` | per-project/LC capability 路由整条删除（无 shim）→ 旧前端 404 | 与 provider run 无关；影响核验入口 | 否 | 低-中 | 前后端发布需同步 |
| C3 | `evidence_mediator.rs:166-186` ④b | durable 索引在册但 `.codegraph` 目录缺失 ⇒ `evidence_index_unavailable` fail-closed（此前误报 200 空） | reviewer/author 查证据面；是否两段不对称【未验证】 | 否 | 低-中 | 查 reviewer prompt 是否调用 evidence 查询 |
| C4 | `coordinator_lifecycle.inc.rs:342,391,857` | 聚合冷启动新增硬前置（codegraph CLI 探测、聚合根可写、skills 命令链接） | 仅聚合初始化；若缺失，Story 因 `planning_ready` 不达而无法开始（但本现场 Story 已能开始） | 否 | 低 | 无 |
| C5 | `provider_run/cancel_settle.inc.rs` | `drive_until_cancelled_settles`：取消后 250 ms 有界收口窗口，超时则兜底 `fail_cancelled_human_gate_revision_turn` + `finish_run`（文件头自述「纯移动 + 取消收口窗口收敛，无其它行为变化」） | main 的 `drive_until_cancelled_settles::<…>` 符号已在（帧 133,032），属**收敛而非新增**；supersede 时序可能与旧版有毫秒级差异【未验证】 | 否 | 低 | 保持；2 MiB 冒烟覆盖 |
| C6 | `coding_workspace_engine/gates*`、`handoffs.rs`（块3） | Legacy 锁臂删除；失败收尾改读仓维 active 指针（**修复了** 此前 clean 门被跳过、收尾释放他人锁的旧缺陷） | 现在失败收尾会真的释放**其他** attempt 的锁（诊断为旧缺陷）；旧 issue 维锁无人释放 → 悬挂（`worktree_lock.inc.rs` 6 臂删） | 否 | 中 | 存量悬挂锁清理脚本；与 supersede 时序联测 |
| C7 | `root_recipe_audit_windows.inc.rs`/`root_recipe_*` | 零变更 turn 判失败；预算按变更清单计费；旧 receipt 判 `aggregate_policy_publication_legacy_digest`（`retryable:false`） | 仅聚合初始化；旧 operation 恢复口径收紧 | 否 | 低 | 无 |
| C8 | `issue_automation_store.rs`/`automation_enrollment_reenable.rs`（块1） | terminated 重开一律走 reenable；payload 漂移分 9 类 409 | 授权换代；不改变运行中启动 | 否 | 低 | 无 |
| C9 | `provider_adapter_error_from_gateway`（候选2） | 见 §4.3 | 仅 Coding 引擎；重试最多 3 次/周期 | — | 低 | 无 |
| C10 | `workspace_session` 广播面（plan_continue_available） | 每次 SessionState 注入同步读 ≤3 个 JSON | 持 engine 锁期间的降级帧会现读盘；无新 `.await` | 否 | 低 | 无 |

### 6.D 块3「单仓 fail-closed」轴对**存量数据**的影响面

事实（【核实】`issue_store.rs:184-203/45-71`、`models/project.rs:143-144`、`admission.rs:572-592`、`repository_routing_types.inc.rs:59-67`；其余切片转述）：

1. **D-1 issue 记录**：`logical_codebase_id` 必填，缺失/空串 ⇒ `InvalidRecord(issue_not_bound_to_logical_codebase)`，**零迁移、无回退**。**关键放大点**：`IssueStore::list` 对每条记录用 `?` 传播（`:67-69`），**项目内只要有一条未绑定 issue，整个 issues 列表读取即整体失败**，而不是仅跳过该条。严重度：**高（若存在存量单仓 issue）**；owner 2026-11-03 裁定（`openspec/…/logical-codebase-issue-entry` REQ-MRE-01）「存量单仓 issue 默认不存在」，**该假设是否在 owner 真机成立【未验证】**。建议：①发布前对所有真机 `.aria` 做一次 `issue.json` 的 `logical_codebase_id` 巡检脚本（只读）；②`list` 改为逐条容错 + 汇总告警（会改变 fail-closed 语义，需 owner 裁定）。
2. **D-2 coding attempt**：无冻结 `target_snapshot` 的 attempt 一律 `TARGET_SNAPSHOT_MISSING_FOR_LOGICAL`/`TARGET_SNAPSHOT_IDENTITY_DRIFTED`（admission.rs）；`classify_worktree_lease_for_attempt` 落 `unknown(...)`（可能表现为「卡住」）。影响：升级前创建、未带快照的 attempt。
3. **D-3 enrolled 单仓 plan**：`advance_plan` 单仓分支整段删除 ⇒ `enrolled advance carrier drift`；`coding_start` 单仓自动启动恒 `coding_start_enrollment_mismatch`；`resolve_automation_carrier` 无 LC ⇒ 400 `issue_not_bound_to_logical_codebase`。影响：任何**没绑 LC 的存量 enrollment**逐阶段 fail-closed，无「降级单仓」路径。【转述】
4. **D-4 候选1 的残留**（§4.2）：legacy 别名 LC + 空 `review_identity_id` 的旧会话首读补写一次；doc 注释与测试夹具未同步。低。
5. **D-5 本次现场的数据形态**：本次诊断会话的 issue/LC 都是新建的 LC 形态（聚合根 `test-center-java`，3 成员），故**块3 轴不是本次症状成因**；但它是**升级到真机存量**时最大的未知。

---

## 7. 验证方式 + 未验证清单

### 7.1 每条结论怎么验

| 编号 | 结论 | 验证方式 | 谁/何时可做 |
|---|---|---|---|
| V1 | main 在 debug + 2 MiB 栈下 `StartGeneration` 同样爆栈（§1.3、§3.3） | 用 `/tmp/aria-baseline/target/debug/aria`（main 构建）+ **数据副本**（非 `/tmp/aria-verify`；需 main 兼容的数据形态，即 issue 带 `logical_codebase_id`、capability 在项目级路径）+ 不设 `RUST_MIN_STACK` + `script -qfc` 直启；复用 `/tmp/aria-drv4c.ts` 驱动；gdb 取 `spawn::<…>` 序言 `sub $0x77000`。对照组 `RUST_MIN_STACK=33554432` | 需 Main 授权起 main 实例；**我未做**（无兼容副本、禁碰共享实例、禁 cargo） |
| V2 | 送审轮栈帧链 ≈1.97 MB（§3.1） | 对修复后二进制重跑 `/tmp/dsfr_scan.py <bin> <out>`，核对 `provider_run_drive_body::{closure#0}`、`drive_review_session_via_gateway_with_window::{closure#0}` 序言；gdb 崩溃现场 `info frame` 逐帧取 RSP（诊断2 §6 记录其脚本有缺陷，需修复） | 实施 agent 构建后 |
| V3 | 症状二 = 症状一下游（§4.1） | 同二进制 `RUST_MIN_STACK=32MiB` 对照（诊断2 已做）；修复后 2 MiB 裸跑送审轮，期望 ledger=2、`/api/health` ok | 实施后 Main |
| V4 | 候选1 推翻（§4.2） | ①读码已核（`is_logical_codebase_issue()` 门）；②补测试：「legacy 别名 issue 记录 + 空 `review_identity_id`」的 `get_workspace_session` 是否补写、`reviewer_verify_policy()` 在无 gateway 时仍为 None | 单测，可由实施 agent 补 |
| V5 | 候选2 限 Coding 且不加深栈（§4.3） | 读码已核（唯一调用点 `launch.rs:76`，重试为同任务内 for 循环）；补测试：adapter stderr 含 `connection` 时 `classify_adapter_error` 判 retryable 且最多 3 次 | 单测 |
| V6 | 治本 R1/R2 效果（§5.2） | objdump 序言前后对比 + 2 MiB WS 送审轮冒烟（无 `RUST_MIN_STACK`） | 实施后 |
| V7 | A2/A3/A6 同类大链（§6.A） | 对每条用 gdb 在 2 MiB 栈下跑真实场景（SC 人工修订 turn、单候选 author、coding 全链）取最深 RSP；或扫序言后在 CI 设阈值 | 建议排期 |
| V8 | 存量数据 D-1（§6.D） | 只读巡检脚本：遍历真机 `.aria/projects/*/issues/*/issue.json` 检查 `logical_codebase_id` 非空；**不得访问 `/tmp/aria-verify` / `/tmp/aria-deploy`**，需 owner 提供拷贝 | Main 授权 |
| V9 | C1 capability 零迁移（§6.C） | 读真机 `.aria/provider-capabilities/capabilities.json`（同样需拷贝）各 provider 三态；对照 reviewer provider | Main 授权 |

### 7.2 未验证清单（共 14 项）

1. **main 在 2 MiB 栈下是否实际爆栈**（V1）——仅静态帧和 ≈2.03 MB 预测；「owner 说 main 好」的运行环境（`cargo run` 32 MiB？release？）未取证。
2. **owner 现场进程 `pid 2507207` 的 `/proc/<pid>/environ` 无 `RUST_MIN_STACK`**：我只读了 `/proc`，该文件我只用于核对「无该变量」；profile=debug 来自命令行路径 `target/debug/aria`（推断）。
3. 帧值取自 **13:20 构建**的 bugfix debug 二进制副本，工作树随后有未提交改动（Task 14b 等）与新提交 `4b7a390b`——**最新代码的帧值未测**。
4. A1 链和是**符号名上界**，`via_gateway` 与 `with_window` 的同栈存活由 gdb 栈帧（drive.rs:373/431）支持，其余兄弟路径（`drive_review_session_with_window` 等）是否叠加**未验证**。
5. A2（SC 修订臂 ≈1.96 MB）、A3（单候选 ≈1.68 MB）、A5（inbound 其它臂下游引擎深度）、A6（coding runner 评审臂叠加）的**嵌套关系与实测栈深均未验证**。
6. 8 MiB worker 栈（`4b7a390b`）在 owner 现场的实际效果未验证；`std::thread::spawn` 线程（`select_aggregate_provider_blocking` 等）栈未被 Builder 覆盖。
7. release 构建是否爆栈：未测。
8. `drive_review_session_via_gateway_with_window` 装箱（Task 14b，未提交）后其 poll 帧是否真正下降：未测（`Box::pin` 单独对 debug 帧有限，诊断1 Q3）。
9. bugfix 与 main `claude marker digest` 的**数值**是否逐字相同：字段序/schema 逐项一致已读，**未实算**；且不在 Reviewer 路径。
10. `evidence_mediator` ④b 守卫在 author/reviewer 两段是否不对称。
11. `conversational_gate_recovery.rs:584`、`single_candidate.rs:323` 的 `previous_fingerprint_digest=None` 下游处理。
12. 症状二现场会话 JSON 的 `logical_launch_fingerprints=[]` 与 Author 成功的关系（§6.B 第 5 行）——**未取得会话 JSON**。
13. 真机 capability 记录位置/三态（C1）与存量 issue 的 `logical_codebase_id`（D-1）。
14. 切片 A/B/C 中我**未逐行复核**的转述项（§2 标【转述】的 D8/D10-D12/D14/D15）：web 层块3 各臂、`provider_installation`、聚合初始化等；它们与两症状无直接因果，未做 diff 级复读。

### 7.3 本轮的方法边界

- 只读：未改产品代码，未提交，未 `git add`；写入仅限 `/tmp` 的扫描脚本/副本（`/tmp/dsfr_scan.py`、`/tmp/dsfr_bugfix_debug`、`/tmp/dsfr_main.txt`、`/tmp/dsfr_bugfix.txt`、`/tmp/ds_spawn_syms.txt`）与本报告文件。
- 未跑 `cargo`；未触碰 `/tmp/aria-verify`、`/tmp/aria-deploy` 的数据（仅 `ps`/`/proc/<pid>/{environ,exe,cwd}` 读元信息）。
- 未派 subagent。结构化检索：CodeGraph 工具本会话不可用 ⇒ 降级为 `git show/diff` + `grep` 工具 + `objdump`。
