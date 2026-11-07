# 删除工作项组时必须取消该实体绑定的在途 run（REQ-WIGA-15 / REQ-WIGA-17）

> **本 change 记录 2026-11-06 owner 真机撞到的缺陷 + 已核实的根因 + 已选定的修法（修法 A）。**
> 缺陷编号：本仓无独立缺陷单，本 change 即缺陷单。

## Why

### 真机现象（owner 2026-11-06，开发期 5173 / 后端 4317）

```
① 在 issue_0001 上启用自动化（生成一个工作项组 WIG）
② 点该 WIG 的「删除」
③ ⇒ 删除失败，界面报：「实体存在进行中的运行或并发写入，请等待收口后重试删除」（HTTP 409 entity_busy）
④ ★同一时刻，页面上的「终止整个自动化」按钮消失（说明自动化登记已被终止）
⑤ ★provider 仍在继续输出
⑥ 之后仍然删不掉这个 WIG
⑦ 页面右下角还有一个「终止」按钮；点它之后 provider 不再输出，★这时才可以删掉 WIG
```

### 根因（三层，均已核实）

**① 判定层：删除路径只做了「终止」的一半**

```
src/web/handlers/lifecycle/deletion.rs:111  async fn cascade_delete_entity(
  :132-134  let mut session_ids = workspace_session_ids_for_entities(&store, project_id, issue_id, &entity_ids)?
            （按 entity_id 精确收集绑定本实体的 workspace session；:861-875）
  :159      let terminated_automation = terminate_automation_bound_to_entity(&app_paths, &store, &scope)?
            → IssueAutomationStore::terminate_plan
              （src/product/issue_automation_store_parts/command_writes_impl.inc.rs:9-77）
              ★该 CAS 只写 enrollment 文件★（enabled=false / terminated_at / policy_revision+1 / 账本追加）
              ⇒ 不碰任何 run / session / provider 进程
  :161-163  注释自述：「刚终止过自动化时给在途短写一个有限排空窗口（tick 写是毫秒级；长 run 不在此等）」
  :164      let mut delete_attempts = if terminated_automation.is_some() { 10 } else { 1 };   ← 约 1 秒排空窗口
  :169-185  operations.try_acquire_delete(&scope) 取删除独占（LOCK_EX）
  :549 / :894 两处各写一遍「实体存在进行中的运行或并发写入，请等待收口后重试删除」字面量
            错误码 entity_busy → HTTP 409（src/web/error.rs:198-201）
```

**② 占用层：真正挡住删除的是 in-flight run 持的 scope 共享 flock**

```
实测（lslocks + /proc/<pid>/fdinfo）：
aria 进程持 FLOCK ADVISORY READ（= LOCK_SH）于
  <issue_root>/entity-operations/scope-locks/plan/<plan_id>.lock（inode 580242025）
⇒ 与删除所需的 LOCK_EX 冲突 ⇒ Busy
已排除：资源锁争用（无 resource-locks/ 目录）、墓碑重入。
锁语义来源：src/product/entity_operation_store.rs:154 acquire_run（try-shared）/
            :174 try_acquire_delete（独占）
```

**③ 对照：人工终止路径本来就会取消 in-flight run**

```
REST post_automation_enrollment_terminate（src/web/handlers/automation_enrollment.rs:710）
  → terminate_plan_engine_side（src/web/workspace_session/manager/mod.rs:732）
  → :755-757  manager.abort_active_run_and_wait_completion().await   ← ★真实取消 + 等真实 completion★
⇒ 两处「终止」语义不一致：人工终止会取消 run，删除路径只复用了 terminate 的 CAS。
```

### 证据摘要（服务日志 `/tmp/aria-deploy/session.log` + 磁盘）

| 时刻 | 事件 |
|---|---|
|22:14:43|终止 c8d522f9 ⇒ 日志 `[aria-cancellation] workspace ws_abort_message` → `abort_workspace_run cancelling runner token run_id=run-1 run_token=1` → `work_item_plan_drive select_cancelled … role=author agent=ClaudeCode` ⇒ run 立即停|
|22:14:56|删除成功（账本 `void-enrollment-issue_work_item_plan_auto_c8d522f9-…`）|
|22:15:44|**本次由删除路径发的终止**（`terminated_at=2026-11-06T14:15:44.354080465+00:00`；账本 `entity-delete-terminate-9df5de30-…-r12`）⇒ **此后日志零 `[aria-cancellation]`** ⇒ run 未被取消 ⇒ provider 继续输出|
|22:15:50 / 22:16:11 / 22:16:58|driver 连接持续 `current_run_id=1, current_run_token=2, provider_drive_depth=1`|
|22:22:27|owner 点右下角「终止」⇒ lease `epoch 7 hold`|
|22:22:30|run 收口（`timeline_nodes.json`: `status=failed`、`summary=运行已中止`、`completed_at=2026-11-06T14:22:30.077821016+00:00`）；scope 锁释放 ⇒ 此后才可删除|

### 附带缺陷（owner 授权一并修）

被终止的 run 在时间线里被记成 `status=failed` + `summary=运行已中止`，而同一节点 `stage` 仍为 `running`
⇒ **「中止」被表述成「失败」，且 stage 与 status 不一致**（owner 验收要看时间线状态，会误导判读）。

```
实测样本（timeline_nodes.json）：
{"node_type":"author_run","agent":"claude_code","stage":"running","status":"failed",
 "title":"Work Item Plan 生成","summary":"运行已中止","started_at":"…","completed_at":"…"}
落点：finish_aborted_run()
      src/product/workspace_engine/session_state/timeline.rs:132-141
      ⇒ 写 TimelineNodeStatus::Failed（:136）+ summary「运行已中止」（:137）
```

## What Changes

```
① 删除路径补齐「终止」的另一半（★修法 A★，不新增任何 API）
   落点：src/web/handlers/lifecycle/deletion.rs:160
        （紧接 :159 的 terminate 调用之后、:161 的「② try 删除独占」注释之前）
   内容：对已按 entity_id 精确收集到的 session_ids（:132-134）
        逐个取 state.workspace_sessions.get(session_id)（src/web/workspace_session/registry.rs:94）
        调 manager.abort_active_run_and_wait_completion().await
          （src/web/workspace_session/manager/runs.rs:238；等**真实 completion**，内部 30s 兜底超时）
        ⇒ 被删除实体自身的在途 run 先收口 ⇒ 删除独占（LOCK_EX）随后可获得
        ⇒ 不再以 entity_busy 长期阻塞
   ★:161-163 注释与 :164 delete_attempts 赋值保持原样不动★
   ★取消的 bool 返回值必须被检查★（不得照抄 manager/mod.rs:756 的丢弃写法）：
     false（无活跃 run）属正常；true 但未真正收口（30s 兜底）时不得谎称已取消，
     也不得因此阻塞删除（详见「未决风险」②）
   ★取消与取锁之间的锁释放时序必须被验证★；若存在尾隙，补有界的「等待 scope
     锁释放」短退避，并把该退避回写进 REQ-WIGA-17（详见「未决风险」①）

② 时间线中止态表达修正（附带缺陷，owner 授权一并修）
   被取消的 run 节点 SHALL NOT 以 failed 表述「中止」；节点的 stage 与 status SHALL 相互一致

③ 契约改动
   work-item-group-autopilot：ADDED REQ-WIGA-15（删除必须取消该实体绑定的在途 run）
                              ADDED REQ-WIGA-17（取消与取锁的时序保证；取消结果不得谎称）
                              ADDED REQ-WIGA-16（被中止 run 的时间线表达）
   lifecycle-entity-deletion：MODIFIED REQ-DEL-06（同 scope writer 场景写明「实体自身的自动化 run」这一支的例外）
```

### 修法 A 的上下文齐备性（三条独立证据）

```
1. session_ids 已在删除前按 entity_id 精确收集（deletion.rs:132-134 → :861-875
   filter: entity_ids.contains(&session.entity_id)）⇒ 无需新查；
   plan 的 workspace session 建时 `entity_id: plan.id.clone()`
   （src/web/handlers/lifecycle/plan_preparation.rs:201 与 :226）⇒ 过滤必然命中
2. cascade_delete_entity 已持有 state: &WebAppState（:111-112）；
   state.workspace_sessions 即 WorkspaceSessionRegistry（src/web/state.rs:146）
3. 取消 API 现成且已有非 WS 调用先例：
   WorkspaceSessionManager::abort_active_run_and_wait_completion（runs.rs:238）
   先例：src/web/workspace_session/manager/mod.rs:755-757（由 REST 终止 handler 间接调用）
```

### 取消链的真实符号（写规格/实施时不得用错名字）

```
· abort_workspace_run 是★日志 trigger 字符串★（runs.rs:485），真实函数是私有的
  WorkspaceSessionManager::cancel_run(run: &ActiveRun)（runs.rs:483）
  （被 abort_active_run:292 / abort_active_run_and_wait_completion:238 / abort_active_run_from_attachment:63 调用）
· WS 入口：src/web/workspace_ws_handler/decisions/inbound.rs:489-511
· 引擎收口：src/product/workspace_engine/provider_drive.rs:399-411
  → cancel.cancelled() 臂 → flush_stream_buffer → finish_aborted_run()
· ⚠ abort_active_run_from_attachment（runs.rs:63）要求 attachment lease 持有者身份
  ⇒ 只适合 WS socket 路径，★不适用于删除路径★
```

## 不做（anti-goals）

```
✗ 不改变 owner 既有裁决「执行中禁删保持现状」：run 还在跑时不得删；
  修法 A 是让终止★先把 run 停掉★，run 不再在跑 ⇒ 不违反该裁决
✗ 不放宽任何删除前置条件（不新增、不放松 REQ-DEL-06 的互斥与 REQ-DEL-04 的确认门禁）
✗ 不物理删除 enrollment 记录（REQ-DRG-18 / REQ-WIGA-VOID-01 的「销账 MUST NOT 删除」保持）
✗ 不改动人工终止路径（REST terminate / WS abort）的既有行为、顺序与文案
✗ 不新增任何 API、不改 enrollment 模型、不改 provider 网关与授权面
✗ 不给删除路径造第二套终止语义（复用既有 abort_active_run_and_wait_completion）
✗ 不为「中止」新造一套与既有词汇冲突的语义（既有 ProviderStatus/ExecutionEventStatus 已用 aborted）
```

## 被否的备选（如实记录权衡）

> **A′ 已在 2026-11-06 复核（有硬证据），方向不变：仍是修法 A。**

```
✗ A′（字面）：把 deletion.rs:159 的 terminate_automation_bound_to_entity
  换成人工终止的完整函数 terminate_plan_engine_side（manager/mod.rs:732-860）
  —— ★不可行，是回退而非修复★：
    · terminate_plan_engine_side ★不终止 enrollment★：该函数全范围内无任何
      enrollment 写入（manager/mod.rs 内 enrollment 只出现在只读投影
      project_automation_ownership（:327-330）与文档注释（:725-731，
      自述「T3 统一终止服务在 enrollment CAS 落盘后调用」））
    ⇒ 若用它替换 :159 的 CAS，autopilot 仍读到 enabled=true
      ⇒ 下个 tick 继续 ensure/reconcile/生成写该实体 ⇒ 共享锁继续被拿
      ⇒ 缺陷未修，且现象退化（「终止按钮消失」会变成「按钮不消失、provider 还在跑」）

✗ A′-1：保留 :159 的 CAS + 追加一次引擎侧收口
  —— 可行但★净代价为正★，覆盖面严格小于修法 A：
    · 只能取消【一个】session（enrollment.session_id）；kind==Plan 的匹配路径
      （deletion.rs:585-587）不要求 enrollment 带 session ⇒ 可能为 None
      ⇒ manager/mod.rs:754 整段跳过、一个 run 都不取消
      ⇒ 而修法 A 遍历磁盘按 entity_id 收集到的全部 session_ids（deletion.rs:132-134）
    · 额外引入 3 类删除路径不该做的写入面：CodingPlanRun phase=Invalidated、
      workspace session status=Terminated、coding attempt 全套收口
      （关 reviewer 窗口 / 回收 code_delivery / 释放 shared-worktree 锁）
    · 破坏删除路径自述的「零落盘」承诺（deletion.rs:157-158、:664-680）
    · HTTP 最长阻塞从 ≈1s 抬到 ≤30s（runs.rs:277 的 30s 兜底）
    · ⛔ 实质冲突：attempt 收口后 ⑤ 编码门禁★仍拒删★
      （deletion.rs:358-372：group 与所属 WI 任一 attempt 记录存在即拒，
       「无论是否运行中」）⇒ 落盘了但删除没执行
      ⇒ 破坏「门禁拒绝时墓碑尚未立起、零破坏」的纪律（deletion.rs:339-341）

✗ B 只把删除独占的排空窗口拉长（把 :164 的 10 次 × 100ms 拉到分钟级）
  —— 长 run 不会自行收口，只是把「删不掉」变成「等更久还是删不掉」，且让删除请求长期挂住。
✗ C 删除前先杀 provider 进程 / 直接释放 scope 锁
  —— 绕过 manager 的收口与 completion 语义，留下不一致的 durable 事实。
✗ D 把「取消」放到 terminate CAS 之前
  —— 终止仍可能失败而 run 已被停（语义倒置）；CAS 之后（=A）才是「先停 run，再取独占」。
```

## Impact（受影响的既有契约与能力）

| 契约 / 能力 | 关系 |
|---|---|
| `work-item-group-autopilot` REQ-WIGA-09（整 plan 终止与统一 Abort 语义） | 不冲突：本 change 新增的动作是「取消已派发的在途 run」，不写 enrollment、不建 plan/session、不启动 provider、不复活 plan ⇒ 仍满足「绑定实体删除流程再次运行 ⇒ 零副作用」的清单 |
| `work-item-group-autopilot` REQ-WIGA-VOID-01（删除时登记作废；同胞 change，未 archive） | 不冲突：作废仍发生在删除成功之后（deletion.rs:319 / 重入分支 :145），取消步骤在删除之前且不动 enrollment 记录 |
| `work-item-group-autopilot` REQ-WIGA-14（终止回执身份限定） | 不涉及：只动后端删除路径与时间线状态表达，不动前端终止回执 |
| `dangling-reference-governance` REQ-DRG-18（销账 MUST NOT 删除 enrollment 记录） | 不变 |
| `lifecycle-entity-deletion` REQ-DEL-06（删除与生成/修订/compile 互斥且无复活） | ★需 MODIFIED★：既有场景「同 scope writer 运行时拒删 ⇒ entity_busy」与本修法在「writer 就是该实体自身的自动化 run」这一支上正面冲突，须写明例外；互斥本身、独占准入顺序、无部分删除、不复活均不变 |
| `work-item-plan-single-instance` / REQ-DEL-05（coding attempt 存在 ⇒ 拒删） | 不变：本 change 只处理 provider run，不碰 coding attempt 门禁 |

代码面（实施范围）：

- `src/web/handlers/lifecycle/deletion.rs`（新增一段取消循环）
- `src/product/workspace_engine/session_state/timeline.rs`（中止态 status / stage 表达）
- `src/web/workspace_ws_types/timeline.rs`（时间线节点状态值域，如需新增「中止」态）
- `web/src/api/types/workspace.ts`（前端时间线状态值域镜像）
- `web/src/pages/ChatCockpitPageParts.tsx`（终态文案映射：当前 failed→「生成失败」、其余→「生成已跳过」）

风险：

- 删除请求时延会增加（等真实 completion，30s 兜底）⇒ 30s 兜底后必须**明确报占用者**，不得假装已停。
- 时间线状态值域若新增「中止」态 ⇒ WS / 前端是公共契约面，前端必须同步，否则会落到「生成已跳过」的错误文案。

### ★未决风险（实施时必须验证，不得当成已解决）★

```
① ★取消与取锁之间的时序尾隙★
   abort_active_run_and_wait_completion（runs.rs:238-292）只等到 completion 信号；
   而被取消的 run 释放 scope 共享 flock（acquire_run 的 try-shared lease，
   run task 全生命周期持有 ⇒ provider_run/task_body.rs:194-196）与发出 completion 信号
   的【先后顺序未验证】⇒ 删除路径 :170 的首次 try_acquire_delete 仍可能拿到 Busy
   ⇒ 现有 10×100ms 退避（≈1s，deletion.rs:164/:173-178）能否兜住【未知】。
   ⇒ 实施必须：① 用确定性测试验证「取消完成后立即取锁能成功」；
     ② 若验证发现存在尾隙，则在取消与取锁之间补一个「等待 scope 锁释放」的短退避，
       并把该退避写进 REQ-WIGA-15 的对应 scenario（契约随之更新，不得只写在实现里）。

② ★「run 实际上没停下来」目前无法表达★
   abort_active_run_and_wait_completion 的 bool 返回值在既有人工路径
   （manager/mod.rs:756）★被丢弃★；30s 兜底超时只 eprintln! 后★照常返回 true★
   （runs.rs:277-292）⇒ 调用方无法区分「真取消了」与「超时后放弃」。
   ⇒ 修法 A ★必须检查这个返回值★（不得照抄既有人工路径的丢弃写法）：
     · 返回 false（无活跃 run）属正常，不视为失败；
     · 返回 true 但未真正收口（兜底超时）时，删除【不得谎称已取消 run】，
       也【不得因此阻塞删除】（删除仍按既有独占判定走：拿到独占就删，
       拿不到就按既有语义报 entity_busy 并指出占用者）。
```
