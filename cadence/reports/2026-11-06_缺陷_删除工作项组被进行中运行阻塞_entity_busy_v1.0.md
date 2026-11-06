# 缺陷单 · 删除工作项组被进行中的运行阻塞（entity_busy）

| 项 | 值 |
|---|---|
| 文档类型 | 缺陷单（独立缺陷登记） |
| 版本 | v1.0 |
| 日期 | 2026-11-06 |
| 状态 | **待裁决（未实施修复）** |
| 现场 | owner 真机，开发期前端 5173 + 后端 4317 |
| 工作树 | `.worktrees/bugfix-b-11-02`（分支 `bugfix-b-11-02`，HEAD `52af734b`） |
| 涉及主体 | 工作项组（WIG）级联删除路径 `src/web/handlers/lifecycle/deletion.rs` |
| 稳定错误码 | `entity_busy`（HTTP 409） |

> **本文只呈现证据与修法选项，未做最终裁决；修复由 owner 与设计角色决策后另行实施。**
> 本文不实施任何修复、不改动任何代码。

---

## 1. 一句话摘要

owner 对一个**正在被 provider 运行占用**的工作项组点「删除」时，删除路径虽然已把该实体绑定的自动化登记置为「已终止」（页面上「终止整个自动化」按钮因此消失），但**终止只落在登记层、没有触达 workspace session 里那个仍在跑的 in-flight run**；run 继续持有 scope 的共享文件锁，导致删除侧的独占锁永远抢不到，最终以 `entity_busy`（409）拒绝删除 —— 同一时刻 provider 仍在继续输出，owner 只能额外点一次页面右下角的「终止」把 run 收口之后，这个工作项组才删得掉。

---

## 2. 现象与 owner 操作序列

端口约定：开发期前端 **5173**；后端 API **4317**。

| # | owner 操作 | 观察到的结果 |
|---|---|---|
| 1 | 在 `issue_0001` 上启用自动化（生成一个工作项组 WIG） | WIG 生成，自动化登记进入 `enabled` |
| 2 | 随后点该 WIG 的「删除」 | 删除失败 |
| 3 | 看报错 | 界面报：**「实体存在进行中的运行或并发写入，请等待收口后重试删除」**（稳定码 `entity_busy`，HTTP 409） |
| 4 | ★看页面上的「终止整个自动化」按钮 | **按钮消失了** ⇒ 说明自动化登记已被置为终止态 |
| 5 | ★看 provider | **provider 仍在继续输出**（run 没有停） |
| 6 | 之后再点几次删除 | 仍然删不掉这个 WIG（持续被拒） |
| 7 | 点页面右下角的「终止」按钮 | provider 不再输出；**这时才可以删掉该 WIG** |

其中 ★ 第 4、5 步是本缺陷的关键矛盾点：**界面已经呈现「已终止」，但底下的 run 还活着** —— 用户看到的是「已停」，实际是「已停一半」。

---

## 3. 复现条件

前置环境：后端 4317 在跑、前端 5173 可用、`issue_0001` 已有一个 issue 级自动化登记。

| 步骤 | 操作 | 期望观察（用于确认复现成功） |
|---|---|---|
| 1 | 在 `issue_0001` 上启用自动化，生成工作项组 WIG | WIG 出现，登记 `enabled=true` |
| 2 | 让该 WIG 触发一次真实运行（author run 起 provider），**不要等它跑完** | 时间线上出现 `author_run` 节点；provider 开始输出 |
| 3 | 在运行进行中点 WIG 的「删除」 | 报 `entity_busy`（409），文案「实体存在进行中的运行或并发写入，请等待收口后重试删除」 |
| 4 | 观察页面「终止整个自动化」按钮 | **按钮消失**（登记已被删除路径置为终止） |
| 5 | 观察 provider 输出 | **provider 仍在继续输出** ⇒ 终止没有传导到 run |
| 6 | 重复点删除 | 每次都被同样的 409 拒绝 |
| 7 | 点页面右下角「终止」 | provider 停止输出；随后删除成功 |

复现成立的**充分条件**（推论自 §4 根因链）：

- 该实体绑定了一个 `enabled` 的自动化 enrollment（否则删除路径不会走「删前终止」分支，也就没有 10 次重试窗口）；
- 该 enrollment 触发的 provider run 仍在 in-flight 状态并**持有 scope 共享锁**（run 收口后锁释放，删除立刻恢复可用）；
- run 的时长 **> 1 秒**（删除路径给的就绪窗口只有 `10 × 100ms`）。

---

## 4. 根因链

分三层。**第一层是「谁挡住了删除」的判定，第二层是「为什么终止没让锁释放」，第三层是「为什么这个缺口在真机上必然发作」。**

### 4.1 第一层：删除判定原语 = scope 独占锁，与 run 的共享锁互斥

| 位置 | 事实 |
|---|---|
| `src/product/entity_operation_store.rs:174-195` | `EntityOperationStore::try_acquire_delete`：先 `StableFileLock::try_acquire_exclusive(&lock_path)` 拿 **scope 锁的独占（`LOCK_EX`）**；拿不到 ⇒ 返回 `EntityOperationError::Busy`；拿到后再在锁内复核墓碑（`Deleted` 稳定拒绝、`Purging` 允许删除重入） |
| `src/product/entity_operation_store.rs:199+` | 对应的运行侧是 `with_live_write`：**try-shared**（共享锁），锁争用立即返回 `Conflict{kind:"entity_deleting"}` |
| `src/web/handlers/lifecycle/deletion.rs:165-181` | 删除编排层 `cascade_delete_entity` 的取锁循环：`Err(Busy \| Deleting)` ⇒ `delete_attempts -= 1`、每次 `sleep(100ms)` 后重试；次数耗尽 ⇒ `return Err(deletion_busy_error(&scope, occupant))` |
| `src/web/handlers/lifecycle/deletion.rs:549-565` | `fn deletion_busy_error(scope, occupant)` → `ApiError::runtime("entity_busy", "实体存在进行中的运行或并发写入，请等待收口后重试删除", details{entity_kind, entity_id, automation_enrollment_id?})` |
| `src/web/handlers/lifecycle/deletion.rs:894-902` | `pub(crate) fn cascade_delete_api_error` 的 `CascadeDeleteError::Busy` 分支：**同一句文案又写了一遍**（details 为空对象 `{}`）—— 与 `:549-565` 无共享常量 |
| `src/web/error.rs:197-201` | `entity_busy` 归入冲突族 ⇒ HTTP **409 CONFLICT** |

⇒ **删除的准入判据是「能不能拿到 scope 的独占锁」；只要还有一个 in-flight run 持有该 scope 的共享锁，删除就必然 Busy。**

### 4.2 第二层：删除路径的「删前终止」只到登记层，触不到 in-flight run

| 位置 | 事实 |
|---|---|
| `src/web/handlers/lifecycle/deletion.rs:159` | `cascade_delete_entity` 的第一步：`let terminated_automation = terminate_automation_bound_to_entity(&app_paths, &store, &scope)?;` —— **先终止，再删** |
| `src/web/handlers/lifecycle/deletion.rs:608-611+` | `fn terminate_automation_bound_to_entity(...)` 的实现：走与人工终止同源的 terminate CAS，效果是 enrollment 的 `enabled=false` + 写 `terminated_at`（+ 命令账本留痕） |
| `src/web/handlers/lifecycle/deletion.rs:155-158`（代码自带注释） | 注释写明其意图：「自动化占用预解除 —— 绑定该实体的 enabled enrollment 会让 autopilot 每 tick ensure/reconcile/生成写该实体（scope 共享锁），删除独占永远抢不到（真机 entity_busy 删不掉的根因）；且被删实体一旦消失该 enrollment 本身不可继续，终止是语义必然」 |
| **缺口** | 该终止**只改登记**。它不向 workspace session 发 abort、**不取消 in-flight run**，因此 run 持有的 scope 共享锁**不会释放** |

⇒ 登记层已经「终止」，但引擎层还在跑 ⇒ 共享锁还在 ⇒ 删除继续 Busy。
⇒ 这也解释了现象 4：「终止整个自动化」按钮消失，是因为**登记**已被置终止；provider 还在输出，是因为**run**没被取消。

### 4.3 第三层：排空窗口的量级与真实 run 的时长不匹配

| 位置 | 事实 |
|---|---|
| `src/web/handlers/lifecycle/deletion.rs:162-164` | `let mut delete_attempts = if terminated_automation.is_some() { 10 } else { 1 };` |
| `src/web/handlers/lifecycle/deletion.rs:165-181` | 每次重试 `tokio::time::sleep(Duration::from_millis(100))` ⇒ **刚终止过自动化时的总等待上限 = 10 × 100ms = 1 秒** |
| `src/web/handlers/lifecycle/deletion.rs:163-164`（代码自带注释） | 注释自述：「刚终止过自动化时给在途短写一个有限排空窗口（tick 写是毫秒级；**长 run 不在此等**，仍按 entity_busy 报占用者）」 |
| 现场实测 | 本次被阻塞的 run 从 `22:15:44` 持续到 `22:22:30`（**6 分 46 秒**）⇒ 远超 1 秒窗口 ⇒ **长 run 场景下必然 Busy，这不是偶发** |

⇒ 三层叠加的因果链一句话：
**登记层终止 ≠ run 取消（第二层缺口）** ⇒ 共享锁不释放（第一层判据） ⇒ **1 秒排空窗口必然不够用（第三层量级）** ⇒ `entity_busy` 409。

---

## 5. 证据

### 5.1 现场时间线

证据来源：`/tmp/aria-deploy/session.log`（服务日志）+ `/tmp/aria-verify` 状态根下的磁盘文件（`timeline_nodes.json` / `automation-enrollment.json` / 命令账本 / `lease-diagnostics.jsonl`）+ `lslocks` + `/proc/<pid>/fdinfo`。

| 时刻 | 事件 | 证据（取得方式与要点） |
|---|---|---|
| 22:14:43 | **上一轮**：owner 终止 `c8d522f9` 成功 | 日志：`[aria-cancellation] workspace ws_abort_message` → `abort_workspace_run cancelling runner token run_id=run-1 run_token=1` → `work_item_plan_drive select_cancelled … role=author agent=ClaudeCode`（**完整取消链**） |
| 22:14:56 | 上一轮删除成功 | 账本条目 `void-enrollment-issue_work_item_plan_auto_c8d522f9-…`，`binding_version = 4` |
| 22:14:57 | **本次**新建 session `workspace_session_auto_09be5642-…`，author run 启动 | `timeline_nodes.json`：`started_at = 2026-11-06T14:14:57.329Z`、`node_type = author_run`、`agent = claude_code`、`provider_config_snapshot = {author: claude_code, reviewer: codex, review_rounds: 1}` |
| 22:15:44 | **本次**删除路径发出「删前终止」 | ① `automation-enrollment.json`：`terminated_at = 2026-11-06T14:15:44.354080465+00:00`；② 命令账本新增条目 `entity-delete-terminate-9df5de30-ba17-4254-870a-a77086f11913-r12`（`accepted`，`binding_version = 5`）⇒ **该终止由删除路径自己发出，不是 owner 点的** |
| 22:15:44 之后 | **日志中没有任何 `[aria-cancellation]` 行** | 在 `session.log` 中检索 `[aria-cancellation]` ⇒ 22:15:44 之后零命中 ⇒ **删除路径的终止未取消 in-flight provider run** ⇒ 对应现象 5「provider 仍在继续输出」 |
| 22:15:50 / 22:16:11 / 22:16:58 | driver 连接持续带活 | 日志中的 driver 连接行持续显示 `current_run_id = 1`、`current_run_token = 2`、`provider_drive_depth = 1` ⇒ run 一直在跑 |
| 22:21:39 | **★ 锁观测：run 持 scope 共享锁** | `lslocks` + `/proc/<pid>/fdinfo`：aria 服务进程 **pid 1053156** 持有 `FLOCK ADVISORY READ`（= `LOCK_SH`），作用于 `<issue_root>/entity-operations/scope-locks/plan/issue_work_item_plan_auto_09be5642-….lock`，**inode 580242025** ⇒ 与删除所需的独占锁（`LOCK_EX`）冲突 ⇒ Busy |
| 22:22:27 | owner 点右下角「终止」 | `lease-diagnostics.jsonl`：`epoch = 7`、`event = hold`、`connection_id = 0efd6032-…`、`recorded_at = 2026-11-06T14:22:27.550907295+00:00` |
| 22:22:30 | run 收口 | `timeline_nodes.json`：`status = failed`、`summary = 运行已中止`、`completed_at = 2026-11-06T14:22:30.077821016+00:00` |
| 22:27:23 | 锁释放，provider 子进程退出 | `lslocks` 显示该 scope 锁**已不再被持有**，provider 子进程已退出 ⇒ **此后可删除** |

### 5.2 日志行（原文要点）

上一轮（**有**取消链）：

```
[aria-cancellation] workspace ws_abort_message trigger=ws_abort_message session_id=…
[aria-cancellation] workspace abort_workspace_run cancelling runner token trigger=abort_workspace_run run_id=run-1 run_token=1 node_id=…
[aria-cancellation] workspace work_item_plan_drive select_cancelled trigger=engine_cancelled_observed session_id=… role=author agent=ClaudeCode
```

本轮（**无**取消链）：`22:15:44` 之后 `session.log` 中 `[aria-cancellation]` **零命中**。

⇒ 两轮的差别**不在「删除能不能删」，而在「终止有没有传播到 run」**。

### 5.3 磁盘事实

| 检查项 | 结果 | 意义 |
|---|---|---|
| `entity-operations/` 下是否存在 `resource-locks/` 目录 | **不存在** | 已排除「资源锁争用」这条替代解释 |
| `entity-deletions/plan/` 下是否有对应墓碑 json | **无** | 已排除「墓碑态重入被拒」这条替代解释 |
| `automation-enrollment.json` | `terminated_at = 2026-11-06T14:15:44.354080465+00:00` | 登记层确实已被终止 |
| 命令账本 | 新增 `entity-delete-terminate-9df5de30-ba17-4254-870a-a77086f11913-r12`（accepted，`binding_version = 5`） | 终止动作有留痕，来源是删除路径 |

### 5.4 锁观测要点

- 观测方式：`lslocks` 全量列出内核锁表 + 读 `/proc/<pid>/fdinfo` 逐个 fd 确认锁类型与 inode。
- 关键读数：aria 服务进程（pid 1053156）持 `FLOCK ADVISORY READ` 于 `scope-locks/plan/issue_work_item_plan_auto_09be5642-….lock`（inode 580242025）。
- 语义对应：`ADVISORY READ` = `LOCK_SH` = 与删除所需的 `LOCK_EX` 互斥 ⇒ `try_acquire_delete` 返回 `Busy`（`src/product/entity_operation_store.rs:174-195`）。
- 终局：22:27:23 该锁不再被持有 ⇒ 删除立刻恢复可用 ⇒ **锁是唯一的拦截者**。

### 5.5 附带观察（非本缺陷主体，同批记录）

1. 被终止的 run 在时间线里记为 `status = failed` + `summary = 运行已中止`，而 `stage` 仍为 `running` ⇒ 「中止」被表述为「失败」，且 stage 与 status 不一致。
2. `entity_busy` 文案在 `src/web/handlers/lifecycle/deletion.rs:549-565` 与 `:894-902` **两处各写一遍字面量**，无共享常量。

---

## 6. 与「上一轮能删」的对照分析

同一份代码、同一套部署、同一类操作，**上一轮（22:14:43）终止 ⇒ 删除成功；本轮（22:15:44）终止 ⇒ 删除被拒 6 分 46 秒。**

| 维度 | 上一轮（22:14:43，能删） | 本轮（22:15:44，删不掉） |
|---|---|---|
| 终止的发出者 | owner 点「终止整个自动化」（前端 WS 通路） | **删除路径自己**发出（账本 `entity-delete-terminate-…-r12`） |
| 是否进入取消链 | ✅ `ws_abort_message` → `abort_workspace_run` → `engine_cancelled_observed` 三段齐全 | ❌ `22:15:44` 之后日志中**零** `[aria-cancellation]` 行 |
| run 是否立即停 | ✅ 立即停 | ❌ 一直跑到 22:22:30 |
| scope 共享锁 | 及时释放 ⇒ 22:14:56 删除成功 | 持续持有（22:21:39 观测到 `LOCK_SH`）⇒ 删除持续 Busy |
| 终局 | 删除成功（账本 `void-enrollment-issue_work_item_plan_auto_c8d522f9-…`，`binding_version = 4`） | owner 额外点右下角「终止」后（22:22:27）才收口（22:22:30），22:27:23 后可删 |

**结论（证据支持的强结论）**：删除功能本身没有坏 —— 它在上一轮、在 run 已收口的情况下都能正常完成删除。本轮失败的**唯一差异**是「终止是否传播到 in-flight run」。

进一步说：上一轮之所以能取消，**取消并不是「终止命令」自身做的**，而是前端 WS 的 abort 消息恰好先到了（`ws_abort_message`），取消是被 WS 层的**副作用**触发的（`src/web/workspace_ws_handler/decisions/inbound.rs:490-494` → `abort_active_run()` → `src/web/workspace_session/manager/runs.rs:484-488` 打 `ProviderCommand::Abort` → 引擎侧 `src/product/workspace_engine/provider_drive.rs:403` / `provider_drive/work_item_plan.rs:144` 打出 `engine_cancelled_observed`）。删除路径**没有任何一条通往这条链的通路**。

⇒ 这不是「删除功能坏了」，而是「**终止与取消之间的耦合是靠副作用建立的，不是靠契约建立的**」。

---

## 7. 影响面

### 7.1 直接影响

- **工作项组（WIG）删除**在「自动化正在跑」时**必然失败**，且失败信息（`entity_busy` / 「请等待收口后重试删除」）把 owner 指向「等待」，而实际上**等待不会自行解决** —— 必须人工去点右下角的「终止」。这是一个**误导性错误面**：它把「不可自动恢复」表述成「稍后重试即可」。
- 删除被阻塞 ⇒ **该 issue 的自动化登记无法进入「已作废」态**（登记作废的触发路径就是删除 WIG，见 §7.2）。
- 登记停在「已终止」态 ⇒ 后续「重新开启 / 换 provider」链路的入口状态与设计预期不一致（设计的「先删后建」换代无法走通）。

### 7.2 对既有验收条目的连带阻塞

**登记作废（REQ-WIGA-VOID-01）change 的真机验收 6.3.x —— 整条线被本缺陷卡住。**
`openspec/changes/work-item-group-delete-voids-enrollment/tasks.md:54-59` 的 6.3 真机验收（★标注「这次必须执行★」）：

| 条目 | 内容 | 被阻塞的原因 |
|---|---|---|
| 6.3.1 | 建 WIG → **删 WIG** → 同 issue 新建 WIG → 以**不同** provider 启用自动化 ⇒ 一次完成，不报 rebind 文案 | 第一步「删 WIG」在运行中必然 409 |
| 6.3.2 | 弹窗显示「已随旧工作项组删除作废」 | 前置：WIG 删不掉 ⇒ 没有「删除后重开」的弹窗 |
| 6.3.3 | 登记未被物理删除（磁盘可查 `automation-enrollment.json` 仍在 + 有 `voided_at` + 账本多一条） | 前置同上；`voided_at` 由删除成功路径写入 |
| 6.3.4 | 换完后跑一轮完整生成 ⇒ 产物由**新** provider 产出；后台日志无 reconcile 持续冲突 | 前置同上 |
| 6.3.5 | 未作废的记录语义不变（回归） | 前置同上 |

**阶段 1 全流程真机验收矩阵中，依赖「删除 / 换代」的条目同样被阻塞**
（`cadence/notes/2026-11-06_测试矩阵_阶段1全流程_v1.0.md`）：

| 条目 | 位置 | 依赖的删除动作 |
|---|---|---|
| B7 | `:256` | 「删除该 Work Item → 同 issue 新建 → ★旧横幅不得出现★」——**直接要删 WIG** |
| B9 | `:300` | 「terminated enrollment 上『启用自动化』一次操作完成换代」——前置是 B7 的删除已生效 |
| B15 | `:429` | 「按指引终止 → 回弹窗重新启用 → 走通」——依赖终止态 + 重新开启链 |
| B16 | `:450` | 「终止态换 provider → 一次操作换成功，且不出现旧 provider 的产出」——依赖 B15 的终止态与换代 |
| B18 | `:494` | 「换失败 → 旧 WIG 确实已删、新 WIG 未建、失败面文案正确」——**「先删后建」设计语义，本条会真的删掉现有工作项组** |

⇒ B7 是 B9 的前置，B15 是 B16 的前置，B16 是 B18 的前置 ⇒ **本缺陷一处不通，B7 → B9 → B15 → B16 → B18 整条验收线全部停摆**。
⇒ 且 B7/B15/B16/B18 恰好对应块1 §6.3 #2/#3/#5 与归档 tasks 9.2/9.3/9.5 —— 这些是**从未执行过**的条目（`2026-11-06_交接_新会话接手指南_爆栈根治与登记作废_v1.0.md:60`：「真机 9.2/9.3/7.8 仍未执行」）。本缺陷让它们继续无法执行。

---

## 8. 修法候选（只列选项，不实施）

> 以下四个选项**均为候选**，本文不做取舍。裁决权在 owner 与设计角色。

### 选项 A：删除路径的「删前终止」主动取消 in-flight run

**改哪层**：改「第二层」（终止语义层）—— `src/web/handlers/lifecycle/deletion.rs:159` 调用的 `terminate_automation_bound_to_entity`（`:608+`）。

**改什么**：终止登记之后，**主动沿 §5.2 已验证可用的取消链**去取消该实体关联 session 的 in-flight run：
`ws_abort_message`（`src/web/workspace_ws_handler/decisions/inbound.rs:490-494`）→ `abort_active_run()` → `abort_workspace_run`（`src/web/workspace_session/manager/runs.rs:484-488`，打 `ProviderCommand::Abort`）→ 引擎 `engine_cancelled_observed`（`src/product/workspace_engine/provider_drive.rs:403` / `provider_drive/work_item_plan.rs:144`）。
即：把「终止」从「只改登记」升级为「登记 + 取消 run」的**同一语义动作**。

**代价**：
- 需要在删除路径引入一条新的取消触发通路（可能是 REST 侧的 `abort_workspace_run` 等价入口，而非 WS 侧）；
- 取消是**异步**的 ⇒ 取消信号发出后 run 需要时间真正收口、锁才释放 ⇒ 删除侧仍需要一个**有界的等待/排空窗口**（与选项 B 天然耦合）；
- 取消可能带来「半途产物」：run 被中止时已写出的文件如何处置，需要复用既有「干净退出 / 保留文件」契约。

**风险**：
- 误伤：删除路径的终止会**主动杀掉别人的 provider run** —— 必须严格限定在「绑定该实体、且处于 in-flight」的 run，不得越界取消其它 session 的 run；
- 取消与终止的**幂等/竞态**：owner 手动终止与删除路径终止并发时，不能产生双份取消或状态覆盖；
- 触及既有契约：**REQ-WIGA-09**（整 plan 终止与统一 Abort 语义）目前把「终止」与「Abort」绑在一起，若删除路径另起一条取消触发路径，需确认是**复用**该统一语义还是**新增**一条旁路 —— 这是契约层裁决，不是实现细节。

**是否触及既有契约**：**是**（`REQ-WIGA-09` 的「统一 Abort 语义」边界需要 owner 明确：删除路径的终止是否属于该统一语义的一部分）。

---

### 选项 B：把排空窗口从 1 秒改成「等 run 收口」或可配的长窗口

**改哪层**：改「第三层」（排空窗口层）—— `src/web/handlers/lifecycle/deletion.rs:162`（`delete_attempts = 10`）与 `:165-181` 的重试循环。

**改什么**：把「刚终止过自动化」的重试次数从固定 10 次（= 1 秒）改为：① 可配的长窗口（例如数十秒 / 分钟级）；或 ② 真正「等 run 收口」—— 订阅该 session 的终态后再取锁。

**代价**：
- 删除请求可能**长时间挂起**（HTTP 请求占着连接）；需要超时、取消语义与前端的等待反馈；
- 窗口调大只是把「必然失败」变成「大概率成功」—— 若 run 本身要跑几分钟，删除仍会失败，只是失败得更晚；
- 若采用 ②，则删除路径需要**订阅/轮询 session 终态**，引入新的跨模块依赖。

**风险**：
- **不解决根因**：run 仍然没有被终止，只是给了一个更长的等待窗口；owner 依然要等（甚至更久）；
- 若窗口可配，配置值成为新的调参面，需要确定默认与上限；
- 触及既有契约：需确认「删除请求同步等待」是否与删除准入的**非阻塞**设计（`try_acquire_delete` 的 try-exclusive 语义）冲突。

**是否触及既有契约**：**可能触及**（删除准入的非阻塞语义与「等 run 收口」存在张力）；至少需要新增一条「删除可等待」的场景条款。

---

### 选项 C：删除判定排除「已终止自动化下的 session」/ 提供强制收口入口

**改哪层**：改「第一层」（判定原语层）—— `src/product/entity_operation_store.rs:174-195` `try_acquire_delete` 的占用者判定，以及 `src/web/handlers/lifecycle/deletion.rs:165-181` 的占用者识别（`automation_enrollment_bound_to_entity`，`deletion.rs:568+`）。

**改什么**：① 让「已终止自动化所辖 session」的共享锁**不再计入删除占用者**（即：登记已终止 ⇒ 该 scope 视作可删）；或 ② 在删除失败面里提供一个**「强制收口并删除」**的显式入口（由 owner 显式确认后强制取消 run 并完成删除）。

**代价**：
- ① 需要把「锁的持有者」与「登记的终止态」关联起来 ⇒ 判定原语要能识别「这把共享锁属于一个已被终止的自动化的 run」，侵入 `entity_operation_store` 的纯锁语义；
- ② 需要新增一条带二次确认的强制路径，且要定义「强制」对半途产物、时间线状态（`status=failed` / `summary=运行已中止`）的处理。

**风险**：
- ① **直接与「执行中禁删」的既有契约冲突** —— 2026-11-06 owner 已明确裁定「**『执行中禁删』保持现状**」（`openspec/changes/work-item-group-delete-voids-enrollment/tasks.md:5`）。放宽判定等于部分撤销该裁定；
- ② 强制路径会让「执行中禁删」出现例外分支，与现行契约形成两套语义；
- 无论 ① 还是 ②，**共享锁仍然在**（若不取消 run），强行删除会与 in-flight 写并发 ⇒ 可能踩到「迟到写」防御（`with_live_write` 的 `entity_deleting` 冲突面）。

**是否触及既有契约**：**是，明确触及** —— 与「执行中禁删保持现状」的 owner 裁定正面相撞，需 owner 重新裁决。

---

### 选项 D：终止后强制把 session 收口为终态

**改哪层**：改「第二层」的边界 —— session / run 的终态收口语义（`src/product/workspace_engine/provider_drive.rs` 的 `finish_aborted_run` 一族 + 时间线状态写入）。

**改什么**：登记被终止后，**不由引擎自行收口**，而是由终止动作直接把关联 session 置为终态（并按既有「中止」契约写 `status` / `summary` / `stage`），使锁随之释放。

**代价**：
- 需要定义「强制收口」与「引擎自然收口」的**竞态处理**（引擎稍后回报终态时不得覆盖已写终态）；
- 需要一次性处理 §5.5 附带观察 1 提到的状态表达问题（`status=failed` / `summary=运行已中止` / `stage` 仍为 `running` 三者不一致），否则会固化一个语义矛盾的终态；
- 需要处理强制收口时 run 已产出的文件/半成品。

**风险**：
- **状态语义变更**：把「中止」表述为「失败」并提前落终态，可能与 `REQ-WIGA-09` 的终止语义、`REQ-WIGA-14` 的终止回执身份（回执的身份源须在终止态仍可得）产生交叉影响；
- 引擎侧与终止侧的写序竞争是典型的隐性缺陷源。

**是否触及既有契约**：**是**（涉及终态语义与终止回执可读性，需 `REQ-WIGA-09` / `REQ-WIGA-14` 一并复核）。

---

### 8.5 四个选项的关系（供裁决参考）

- **A 与 B 天然耦合**：A 负责「让锁会释放」，B 负责「等锁释放」；单做 B 是治标，单做 A 而不加等待窗口仍有竞态。
- **C 与 D 都在改变「执行中禁删」或「终止语义」的既有边界**，代价与契约风险高于 A/B。
- **三者的共同前置**：都需要先回答 §10 的未决问题 1（终止与取消的契约关系）。

---

## 9. 建议的验收判据

修好后应满足下列**可观察**结果（建议作为回归判据固化）：

| 编号 | 判据 | 观察方式 |
|---|---|---|
| AC-1 | 在 WIG 的 provider run **进行中**点「删除」，删除**成功**（不是 `entity_busy` 409） | 真机操作 + HTTP 响应码 |
| AC-2 | **终止后 N 秒内删除不再被拒** —— 判据取 **N = 5 秒**（覆盖取消信号发出 → run 收口 → 锁释放的完整链路）；从点击删除起算，**≤ 5 秒**内应拿到删除成功响应 | 定时器 + 响应码；5 秒内不成功即判失败 |
| AC-3 | 终止/删除动作后，provider **立即停止输出**（不再有新 token/行出现在会话流里） | 会话页观察 + 后端日志无新增 provider 输出 |
| AC-4 | 删除路径触发的终止，**必须**在日志中留下取消链痕迹（`[aria-cancellation]` 系列：与 §5.2 上一轮同型的 `ws_abort_message` / `abort_workspace_run` / `engine_cancelled_observed` 三段，或其等价的后端触发标记） | 检索 `session.log` |
| AC-5 | 删除后该 issue 的自动化登记处于**「已作废」**（`voided_at` 有值、`void_reason` 有值、登记**未被物理删除**、命令账本多一条作废留痕） | 磁盘直查 `automation-enrollment.json` + 账本 |
| AC-6 | 删除成功后，**不存在**残留的 in-flight run 与残留的 scope 锁持有者（`lslocks` 中该 scope 锁无人持有；provider 子进程已退出） | `lslocks` + `ps` |
| AC-7 | 错误面诚实：若仍存在无法自动解决的占用，错误文案必须**区分**「等一会儿就好」与「需要人工终止」，不得把不可自动恢复的情况表述为「请等待收口后重试删除」 | 错误文案断言（后端 + 前端） |
| AC-8 | 回归：上一轮可用路径**不被削弱** —— 人工点「终止整个自动化」后，run 立即停、横幅仍渲染、随后删除成功 | 真机回归（矩阵 B6 → B7） |
| AC-9 | 回归：§7.2 列出的登记作废真机验收 **6.3.1–6.3.5** 与矩阵 **B7 / B9 / B15 / B16 / B18** 全部可执行并通过 | 真机验收逐条勾选 |
| AC-10 | 状态一致性：被中止的 run 在时间线里的 `status` / `stage` / `summary` 三者自洽（不出现 `status=failed` + `stage=running` 的组合） | 时间线 JSON 断言 |

---

## 10. 未决问题（需 owner / 设计角色裁决）

1. **「终止」与「取消」的契约关系**：删除路径的「删前终止」是否**应当**取消 in-flight run？若「是」，它是 `REQ-WIGA-09`「统一 Abort 语义」的组成部分，还是一条独立的新语义？（决定选项 A 是否需要改契约）
2. **「执行中禁删」的边界是否维持**：owner 2026-11-06 已裁定「执行中禁删**保持现状**」（`work-item-group-delete-voids-enrollment/tasks.md:5`）。选项 C 会正面触碰该裁定 —— 是否维持？若维持，则选项 C 出局，只在 A / B / D 中选。
3. **「删除」是否允许同步等待**：删除请求是否可以挂起等待 run 收口（选项 B②）？若允许，等待上限与超时后的失败面如何定义？若不允许，A 必须自带足够的排空窗口。
4. **错误面是否需要改文案**：`entity_busy` 当前统一表述为「请等待收口后重试删除」，在本缺陷语境下是**误导性**的（AC-7）。是否需要新增一个更具体的稳定码 / 文案区分「需人工终止」？
5. **`entity_busy` 文案双写**（`deletion.rs:549-565` 与 `:894-902`）：是否在本轮一并收敛为共享常量？（属清理项，不影响功能，但影响后续维护）
6. **附带观察 1 的处置**：中止态的 `status=failed` / `summary=运行已中止` / `stage=running` 三者不一致，是否随本缺陷一并修正为语义自洽的终态表达？（与选项 D 强相关）

---

## 11. 附录：本次引用的契约与既有先例

| 契约 / 裁决 | 位置 | 与本缺陷的关系 |
|---|---|---|
| `REQ-WIGA-09` —— 整 plan 终止与统一 Abort 语义 | `openspec/specs/work-item-group-autopilot/spec.md:656-658`（含 `:177-178`「已终止 enrollment MUST NOT Enable/rebind/继续」、`:222-223` terminated 记录上的人工 Enable 取代场景） | **删除路径的「删前终止」正落在这条契约的语义边界上**。本缺陷暴露：终止动作在删除路径上只实现了「登记层终止」，没有实现契约里「统一 Abort 语义」的另一半 ⇒ 选项 A / D 都需要先澄清这条契约的适用范围 |
| `REQ-WIGA-09` —— 删除保留 tombstone / 绝不删记录 | 主 spec `:658` 末段 + `openspec/specs/dangling-reference-governance/spec.md:38`（「**墓碑 MUST 永不删除**」）、`:82-85`（场景「墓碑本身永不删除」）、`:389-392`（场景「墓碑永不删除」） | 既有先例：**终止 / 删除链路一律只留痕、不删记录**。这正是「删除路径不删登记、只作废」得以成立的基础，也是本缺陷修复时必须沿用的方向（不要用「物理删除 run 记录」的方式绕过锁） |
| `REQ-WIGA-VOID-01` —— 删除时登记作废 | `openspec/changes/work-item-group-delete-voids-enrollment/specs/work-item-group-autopilot/spec.md:3-5` | 「删除工作项组时，该 issue 的自动化登记 SHALL 被标成【已作废】（`voided_at` + `void_reason`），SHALL NOT 物理删除该登记；作废 SHALL 以账本追加一条的方式留痕」 |
| `REQ-WIGA-VOID-01` —— owner 裁决「执行中禁删保持现状」 | `openspec/changes/work-item-group-delete-voids-enrollment/tasks.md:5`（「owner 裁决（2026-11-06）：登记是 issue 级 ⇒ **不删，只作废**；「执行中禁删」**保持现状**；换 provider 一次完成」） | **选项 C 的直接约束**：任何放宽删除判定的改法都与这条裁决正面相撞 |
| `REQ-WIGA-VOID-01` —— 登记作废真机验收 6.3.x | `openspec/changes/work-item-group-delete-voids-enrollment/tasks.md:54-59` | 被本缺陷整条阻塞（见 §7.2） |
| `REQ-WIGA-14` —— 终止回执身份 | `openspec/changes/work-item-termination-receipt-identity/`；`cadence/notes/2026-11-06_交付说明_阶段1_爆栈根治与可用版_v1.0.md:61-62`、`:124-130` | 相关先例：**终止回执必须带 `plan_id`、展示前做身份比对、换代后旧回执失效、缺身份 fail-closed，且「身份源须在终止态仍可得」**。选项 D 强制把 session 收口为终态时，须复核这条「终止态仍可得」不被破坏 |
| 2026-11-06 交付说明 §10.1「后端换代是对的」 | `cadence/notes/2026-11-06_交付说明_阶段1_爆栈根治与可用版_v1.0.md:214`（「『后端每次开一个新的自动化实施』= **错**；后端**换代**（同 enrollment_id）= **对**」）、`:247-249` | 既有裁决：换代（先删旧 WIG → 建新 WIG）是**对**的设计语义 ⇒ **删除必须能跑通**，本缺陷不是「设计如此」，而是**实现缺口** |
| `REQ-DRG-18` —— 悬空引用治理（含 MODIFIED 的「作废」语义） | `openspec/changes/work-item-group-delete-voids-enrollment/specs/dangling-reference-governance/spec.md:7`、`:44-48` | 相关：登记作废后放行全部漂移；本缺陷导致删除失败 ⇒ 登记无法进入作废态 ⇒ 该放行规则无从生效 |
| `REQ-DEL-06` —— 删除准入的稳定 409 族 | `src/web/handlers/lifecycle/deletion.rs:548`（注释「同 scope 仍有可落盘 writer / 删除准入争用 → 稳定 `entity_busy`（REQ-DEL-06）」）、`src/web/error.rs:197-201` | `entity_busy` 的契约归属：**错误码本身是对的**（占用者确实存在），问题在**占用者本该被终止掉却没被终止** |

---

## 12. 声明

> **本文只呈现证据与修法选项，未做最终裁决；修复由 owner 与设计角色决策后另行实施。**
>
> 本次工作**未改动任何 `src/**`、`tests/**`、`web/**` 文件**，未运行 cargo / npm / pnpm 命令，未提交任何 commit。
> 本文的现场事实（时间线、日志行、磁盘事实、`lslocks` 观测）均来自 2026-11-06 的只读勘察；代码行号已在本工作树（`.worktrees/bugfix-b-11-02`，HEAD `52af734b`）逐条复核。
