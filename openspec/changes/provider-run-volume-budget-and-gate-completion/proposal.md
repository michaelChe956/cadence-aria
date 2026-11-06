# provider run 体积预算与四层门禁 —— 收尾未完成项（延后实施记录）

> **本 change 只记录，不在第一阶段实施。**owner 2026-11-06 裁定：第一阶段只保证「产品可用」，剩余项后续单独实施。
> 本文件冻结**已交付部分的接口与预算**，并逐条列出**未完成项**及其验收口径，避免后续会话重新推导。

---

## 1. 为什么分两阶段

爆栈缺陷（点 Story「开始生成」打死整个服务进程，exit 134）的**结构性根因**已由第一阶段消除：

```
改前：WS inbound 链（已用 ~1.5 MiB）→ 在深栈上构造 235,088 B 未装箱 future
      → tokio::spawn 再占 ~488 KiB → 越过 2 MiB 栈 → 写穿保护页 → 进程 abort
改后：WS inbound 链 → 入口只构造 3,704 B 小 capture 外壳
      → 外壳首次 poll 内才 Box::pin(task body) ⇒ 构造落在堆上
      → 各族臂逐个具名化 + 真实 Box 边界（6/13 族已完成）
```

⇒ **深链路的栈占用已从该路径消失**，2 MiB 栈对已收敛的入口与各臂足够。
⇒ 剩余项属**纵深优化与护栏扩面**（把剩余族也拆、把门禁扩到 release/仓级/制品层），**不阻塞可用**，故延后。

## 2. 第一阶段已交付（冻结，不回退）

| 项 | 交付态 | 出处 |
|---|---|---|
入口外壳 | 未装箱 **3,704 B**（预算 ≤32,768 B） | `provider_run.rs` 的 `provider_run_task` |
task body | 未装箱 15,344 B；**构造在首次 poll 内、落堆** | `provider_run/task_body.rs` |
drive body | 未装箱 **112,024 B**（硬线 131,072 内，**高于优先线 65,536**）⇒ 后续项 1/2/3 | `provider_run/task_body.rs` |
已拆族 | author / author_choice_followup / revision / review_only / single_candidate_author / legacy outline（臂 14,800 B、最大 child 9,080 B） | `provider_run/{basic_arms,single_candidate_arm,legacy_outline}.rs` |
**未拆族** | **plan revision / draft / batch / SC manual / followups** ⇒ 后续项 1/2/3/4 | `provider_run/task_body.rs`、`followups.rs` |
三守卫合同 | engine `MutexGuard` 按值属 drive/当前臂、idle `provider_drive_guard` 为外层 `Option`（臂借 `&mut`）、`entity_run_lease` 为独立外层 binding；「r16 现场死锁根修，缺陷#10家族」注释保留 | 四个臂文件头 + `task_body.rs` |
A 层 | 未装箱体积硬断言（**量 raw 值，不量 `Pin<Box<_>>` 指针**）+ S0 基线对照 | `provider_run_events/stack_budget.rs`、`lifecycle_guards.rs` |
B 层 | 显式 **2 MiB worker** + `rt.spawn` 执行 + 线程 identity + 三守卫生命周期矩阵 | `provider_run_events/lifecycle_guards.rs` |
C 层 | 真实 axum/socket/inbound 深链 + 2 MiB worker + 四条证据 | `tests/it_core/workspace_ws_integration/provider_run_small_stack.rs` |
D 层 | 真实 `aria` 制品进程 smoke（指纹/profile/生效栈/stderr/完整帧/durable 前后差异/无进程残留） | `tests/it_core/workspace_ws_integration/provider_run_process_smoke.rs` |
隔离器 | 子进程隔离执行器（父持续排 stdout/stderr、退出状态、墙钟超时；signal/异常退出/超时均失败；防递归；`--exact` 唯一匹配） | `provider_run_events/small_stack_worker.rs` |

## 3. 第一阶段的已知未完成/未验证（**如实登记，不得当作已通过**）

| # | 事项 | 现状 | 恢复条件 |
|---|---|---|---|
U-a | drive body **112,024 B** 高于优先线 65,536（**硬线内**） | A 层只对硬线断言 | 后续项 1/2/3 拆完后逐对象冻结更小阈值 |
U-b | **release profile 的体积数字未实测**（第一阶段所有数字均为 debug：rustc 1.99.1 / x86_64-linux） | 未验证 | 后续项 5：双 profile 门禁 |
U-c | **prod 仍用 tokio 默认 2 MiB worker 栈**（原决策「prod 先 16 MiB」被延后） | 未落地 | 后续项 4：手写 Builder + 编译期分档 |
U-d | **「可发布」证据不齐**（双制品指纹 / 回退合同 / 容量观察 / CHANGELOG） | 未完成 | 后续项 6 |
U-e | **全仓同类隐患未盘点**（其他 spawn 点的超阈值 future） | 未盘点 | 后续项 7（只读盘点，整改另案） |
U-f | 仓级 `RUST_MIN_STACK` 仍 force 32 MiB（保留：避免触发既有 `provider_matrix` 历史红；小栈用例改为**显式** 2 MiB 对抗） | 已决 | 后续项 5 复核 |
U-g | 既有 minor 挂账：`cancel_settle.inc.rs:17` 可见性一词放宽（brief 清单外但为 `run.rs` 再导出链所必需）；`lc_fresh_payload_verify.inc.rs:139` 注释指向已删 inc 的悬空引用；`case_assert_no_more_provider_inputs` 的 150 ms 负锚窗口是启发式 | 已登记 | 整分支终审分诊 |

## 4. 未完成项清单（= 后续任务，逐条含验收口径）

见 `tasks.md`。摘要：

1. 拆 **plan revision** 族（宏迁移 + 非局部 return → `Stop`）
2. 拆 **draft / batch** 族（fresh 冻结载荷 `Option<Option<String>>` 语义、RetryOnce 循环、每轮新 split handle、`AwaitConfirmation` 与 draft break 的差异**不可统一**）
3. 拆 **SC manual** 族（同 turn 两次 attempt、冻结 claim、取消回显 ≤1）
4. 拆 **followups** 接力 + 旧宏清退
5. **门禁扩面**：clippy `large_futures` 仓级启用（阈值 16 KiB，**须先核实它量的是未装箱值**）+ 双 profile CI + prod/dev 栈分档
6. **发布证据**：双制品指纹、回退候选验证（**回退目标必须是已验证不 abort 的制品**）、容量与资源采样、运维文档、CHANGELOG
7. **全仓大 future 只读盘点**（按 tokio dev 2 KiB / release 16 KiB 阈值分档；整改另案）

## 5. 边界（后续实施时必须守）

- **不重开已冻结的接口**：`provider_run_task` / `provider_run_task_body` / `provider_run_drive_body` / `ArmExit` / 臂公共参数 A / 三守卫合同。
- **A 层不得测 `Pin<Box<_>>` 指针大小**（恒绿＝假门禁）。
- **`exit 0` 不算动态通过**（四条证据 + 取消后再生成；close/timeout/仅 locked/无 provider-start/无 durable 全判失败）。
- **SIGABRT 不可进程内捕获** ⇒ 易 abort 场景必须子进程隔离。
- **不改 panic 策略、不改 Cargo 现有 dev/release 优化参数、不新建 `ARIA_*` 配置、不保留旧巨型驱动 flag**。
- **不纳入** inbound/socket 减栈、锁/actor/专用 runtime 改造（若已完整实施仍过不了 2 MiB 真实 WS，带测量证据回 owner 定范围）。
- **体积数字必须标 profile/target/toolchain**；debug 数字不得冒充 release。

---

## 6. 补充登记（2026-11-06 治本审查 I-3：过渡预算与未完成项）

治本（`1d721bec`）把 poll 帧从 724,992 B 降到 4,096 B，但**留下两处未达 64 KiB 优先线**的对象 ⇒ **引入「过渡预算 128 KiB / 帧」这一档**（只作过渡，门禁可过；**不是安全证明**）：

| 对象 | 实测帧预留 | 归类 | 归属 |
|---|---|---|---|
`drive_body` | 4,096 B | 达标 | — |
`task_body` | 4,096 B | 达标 | — |
`drive_review_only_arm` | 904 B | 达标 | — |
`drive_single_candidate_author_arm` | 4,096 B | 达标 | — |
`drive_human_gate_sc_revision_arm` | 61,440 B | 达标（≤64 KiB） | — |
`drive_legacy_outline_arm` | 61,440 B | 达标（≤64 KiB） | — |
author / draft / batch / revision 臂 | ≤40,960 B | 达标 | — |
**评审链 `with_window` 两变体** | **118,784 / 102,400 B** | **过渡预算（≤128 KiB）** | 写集外（engine 侧 `complete_review` 等）⇒ 建议独立 change |
**`execute_start_coding_flow`（coding runner）** | **806,912 B** | **未覆盖（同量级大帧）** | 写集外（`src/web/coding_ws_handler/**`）⇒ 建议独立 change |
`handle_workspace_inbound_message_inner`（入口链） | 946,176 B | 未覆盖（链和 ≈1.55–1.6 MiB） | 入口链专项 |

⇒ ★**「治本后 2 MiB 安全」不能由门禁或「8 MiB 下不崩」证明**（链和 ≈1.88–1.97 MB 贴边）⇒ **必须由 C 层送审轮 2 MiB E2E 在治本提交后复跑留痕**。
⇒ ★门禁覆盖面（诊断报告 §8.4）尚有缺口：PlanRevision 与 followups 两宏体未纳入 poll 帧门禁 ⇒ 本 change 的门禁任务必须补齐。
