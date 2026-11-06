# Tasks: 删除工作项组时必须取消该实体绑定的在途 run（REQ-WIGA-15 / REQ-WIGA-17 / REQ-WIGA-16 / MODIFIED REQ-DEL-06）

## 0. 接手前必读

- `proposal.md`（现象 / 三层根因 / 证据摘要 / 修法 A / 被否备选 A′·B·C·D）
- 本 change 的两份契约 delta：
  - `openspec/changes/work-item-delete-cancels-inflight-run/specs/work-item-group-autopilot/spec.md`
    （ADDED `REQ-WIGA-15` 删除时终止必须取消在途 run；ADDED `REQ-WIGA-17` 取消与取锁的时序保证及取消结果不得谎称；ADDED `REQ-WIGA-16` 中止态的时间线表达）
  - `openspec/changes/work-item-delete-cancels-inflight-run/specs/lifecycle-entity-deletion/spec.md`
    （MODIFIED `REQ-DEL-06`：同 scope writer 场景写明「实体自身的自动化 run」这一支的例外）
- 既有契约（不得冲突）：`REQ-WIGA-09`（整 plan 终止与统一 Abort 语义）、`REQ-WIGA-VOID-01`（删除时登记作废，同胞 change，未 archive）、`REQ-WIGA-14`（终止回执身份）、`REQ-DRG-18`（销账 MUST NOT 删除 enrollment 记录）。
- ★代码落点（勿改错行）★：
  - 删除路径：`src/web/handlers/lifecycle/deletion.rs:111` `cascade_delete_entity`；`:132-134` 按 `entity_id` 收集 `session_ids`（实现 `:861-875`）；`:159` 终止 CAS；★**`:160` 插入取消**★；`:161-163` 注释与 `:164` `delete_attempts` **保持原样不动**。
  - 取消 API：`src/web/workspace_session/manager/runs.rs:238` `abort_active_run_and_wait_completion`（先例 `src/web/workspace_session/manager/mod.rs:755-757`）；`:483` 私有 `cancel_run`（`:485` 的 `abort_workspace_run` 只是日志 trigger 字符串）。
  - registry：`src/web/workspace_session/registry.rs:94` `get`；`src/web/state.rs:146` `workspace_sessions`。
  - 锁语义：`src/product/entity_operation_store.rs:154` `acquire_run`（try-shared）/ `:174` `try_acquire_delete`（独占）；`src/product/entity_deletion_store.rs:57` `EntityScope::new`。
  - 时间线：`src/product/workspace_engine/session_state/timeline.rs:132-141` `finish_aborted_run`；`:401-455` `update_timeline_node`（`:407-412` 的 `completed_at` 判定只认 Completed/Failed/Skipped）。
  - 状态值域：`src/web/workspace_ws_types/timeline.rs:47-53`（`TimelineNodeStatus` 现只有 Active/Paused/Completed/Failed/Skipped）、`:70-90`（`TimelineNode` 含 `stage: WorkspaceStage`）。

---

## 1. ★第一步：先写失败测试（TDD，先见红）★

> **纪律：只写测试并执行，确认「红」且红的原因正是缺本修法；未见到预期红之前不得进入 §2。**
> 既有断言**一条不许删/缩窄**；既有文案逐字不变。

### 1.1 删除路径：存在 in-flight run 时先取消再删（REQ-WIGA-15 场景 1）

- **落点**：`tests/it_web/web_coding_attempt_api/part_14.rs`（同目录已有 `delete_work_item_plan_*` 用例与 `group_plan_uri()` 夹具 `:16`；plan 夹具 `bootstrap_confirmed_work_item_plan_group` 在同模块 `part_02.rs:442`）
- **用例名**：`delete_work_item_plan_cancels_inflight_run_before_delete_exclusive`
- **构造**：① `bootstrap_confirmed_work_item_plan_group` 建出 `project_0001/issue_0001/work_item_plan_0001`；② 用 `LifecycleStore::create_workspace_session_bound`（`src/product/lifecycle_store/workspace.rs:370`，输入 `CreateWorkspaceSessionInput.entity_id` 见 `src/product/lifecycle_store/inputs.rs:244-247`）建一个 `entity_id == "work_item_plan_0001"` 的 WorkItemPlan 类型 workspace session；③ 把 manager 挂进 registry：`state.workspace_sessions.get_or_create(&session_id, || WorkspaceSessionManager::create(&state, &session_id).await)`（先例 `tests/it_web/web_lifecycle_api/part_05.rs:373-383`）；④ 按 **§1.5** 起一个「假 in-flight run」（持 scope 共享 flock）；⑤ `DELETE group_plan_uri()`。
- **断言**：
  - 删除返回 **200**、`body["status"] == "deleted"`（★修复前必须是 **409 + 稳定码 `entity_busy`**，先见红★）
  - 假 run 的 `CancellationToken` 已被触发（取消真的发生了），且共享 flock 已释放（删除后再取独占成功）
  - 删除后 `lifecycle.get_issue_work_item_plan("project_0001","issue_0001","work_item_plan_0001")` 不再是 `Ok`（既有删除语义未被削弱）
  - durable session 的 `provider_start_ledger` **无新增 actual-launch 条目**（字段见 `src/product/models/workspace.rs`、`src/product/lifecycle_store/workspace.rs:908-930`）⇒ 取消没有偷偷再起一次 provider
  - enrollment / 命令账本无自相矛盾条目：终止只写一次（`terminated_at` 非空、`policy_revision` 恰好 +1）、不得出现「未终止」或「重复终止」两条矛盾记录；`command_ledger` 追加一条且旧条目逐字未变

### 1.2 取消范围不越界（REQ-WIGA-15 场景 2）

- **落点**：同文件 `tests/it_web/web_coding_attempt_api/part_14.rs`
- **用例名**：`delete_work_item_plan_cancel_does_not_touch_other_session_run`
- **构造**：同 1.1，但**另建一个** `entity_id != "work_item_plan_0001"` 的 session 并在其 manager 上起一个活跃 run（不持被删实体的 scope 锁）。
- **断言**：删除过程中那个「别的 session」的 run **未被取消**（其 `CancellationToken` 未触发、其 run 仍在位）；删除本身仍按 1.1 成功。

### 1.3 没有 in-flight run 时语义不变（REQ-WIGA-15 场景 3）

- **落点**：同文件
- **用例名**：`delete_work_item_plan_without_inflight_run_keeps_existing_semantics`
- **断言**：无在途 run 时删除行为与改动前**完全一致**（200 + `status=deleted`、既有清理断言逐字未变、不产生额外等待与额外副作用）；既有 `delete_work_item_plan_removes_all_artifacts_and_preserves_issue`（`part_14.rs:356`）等用例**继续通过**。

### 1.4 时间线中止态表达（REQ-WIGA-16）

- **落点**：`src/product/workspace_engine/session_state/timeline.rs` 既有 `#[cfg(test)]` 模块（既有例 `finish_aborted_run_preserves_durable_terminated_session` 在 `:616-617`）
- **用例名**：`aborted_run_node_is_not_failed_and_stage_matches_status`
- **断言**：`engine.finish_aborted_run()` 之后，落盘时间线节点
  - `status` **不是** `failed`（须表达「中止」）
  - `summary` 表达「已中止」而不是失败原因
  - `stage` 归一只适用于**带 provider agent 的 provider run 节点**：终态时其 `stage` SHALL 归一为终态阶段 `completed`；**无 provider agent 的工作流节点**（compile / 门卡 / 上下文类）**不归一**，`stage` 仍表示其所属阶段（既有断言逐字不变）
  - `completed_at` 已落盘
- **回归**：真实失败路径（`finish_failed_run` / provider 失败 / 权限超时）仍为 `failed`，既有断言逐字不变。

### 1.5 ★如何确定性构造 in-flight run（测试支撑，必要时先加）★

> **不依赖真实 provider、不依赖 sleep 撞竞速。** 用「假 run 任务」精确复刻生产语义。

- **生产语义**：provider run 任务在跑期间持 scope **共享** flock（`src/product/entity_operation_store.rs:154` `acquire_run` → try-shared），直到 run 收口才释放；删除要的是**独占**（`:174` `try_acquire_delete`）⇒ 冲突 ⇒ `entity_busy`。
- **测试构造（全部用既有公开 API，不需要改生产代码）**：
  1. `manager.start_run(ProviderRunKind::ReviewOnly, None)`（`src/web/workspace_session/manager/runs.rs:44`；登记 `active_run` 与 `run_completions` 于 `:127-131`；返回 `(run_id, token, cancel, cmd_rx, node_id)`）
  2. `tokio::spawn` 一个**假 run 任务**：
     - 先 `EntityOperationStore::new(app_paths).acquire_run(&EntityScope::new("project_0001","issue_0001",DeletionEntityKind::Plan,"work_item_plan_0001"))` 持住共享 flock（lease 必须**保持存活**才持锁）
     - 再 `cancel.cancelled().await`
     - 然后 `drop(lease)` 释放 flock
     - 最后 `manager.finish_run(token)`（`src/web/workspace_session/manager/runs.rs:196`）发**真实 completion** 信号
  3. 删除路径的 `abort_active_run_and_wait_completion`（`runs.rs:238`）会：摘 `active_run` → `cancel_run`（`:483`，取消上面那个 token）→ 等 `run_completions[token]`（`:257`）⇒ 假 run 任务被唤醒 ⇒ 释放 flock ⇒ 发 completion ⇒ 等待解除 ⇒ 删除独占可获得。
- **若既有夹具下不可行**（例如 manager 创建需要特定 durable 前置），**先加测试支撑再写断言**，允许形态（按优先级）：
  - a) **测试侧自建「可控假 run」辅助函数**（推荐：不动生产代码）
  - b) 生产侧加 **`#[cfg(test)]` 专用 seam**（先例：`src/web/workspace_session/manager/runs.rs:11-20` 的 `RUN_SCRATCH_PREPARE_SEAM` 测试专用钩子；seam 只在测试构建存在，生产构建整体不存在）
  - c) **禁止的凑法**：靠 `sleep` 撞竞速、靠真实 provider、靠「先删后断言」、靠放宽断言
- ⚠ **不得以「不确定能否构造」搪塞**：若 a)/b) 都不通，把阻塞点写进 `cadence/reports/` 并回报，**不得**把用例标成通过。

### 1.6 ★取消与取锁的时序尾隙 + 取消返回值语义（失败测试，REQ-WIGA-17）★

> **本步排在 §1.1～§1.5 之后、§2 之前**：它与 §1.1～§1.3 共用 §1.5 的「假 run 任务」，但**验的是不同的东西**——§1.1 验「取消 ⇒ 能删」，本步验「取消**之后**取锁是否必然成功」与「取消结果不得被谎称」。

- **落点**：`tests/it_web/web_coding_attempt_api/part_14.rs`（同 §1.1）
- **用例名**：`delete_work_item_plan_succeeds_when_scope_lock_releases_after_completion_signal`
- **构造（确定性制造「不幸顺序」）**：复用 §1.5 的假 run 任务，但把释放顺序**倒过来**——
  1. 假 run 任务被取消后**先**发 completion 信号（`manager.finish_run(token)`）
  2. **再**延迟释放共享 flock（`drop(lease)`）
  ⇒ 制造出「completion 已到、锁还没放」的**时序尾隙**（生产中二者的先后顺序未验证：`abort_active_run_and_wait_completion` 只等 completion 信号；run 释放 scope 共享 flock 由 run task 全生命周期持有 ⇒ `src/web/workspace_ws_handler/run/provider_run/task_body.rs:194-196`）
- **断言**：
  - 删除 SHALL 仍然成功（200 + `status=deleted`），SHALL NOT 以 `entity_busy` 失败
    ⇒ ★若实现只靠「取消完就立刻取锁」且不补退避，本用例必须先红★
  - 若实现选择补「等待 scope 锁释放」的短退避：退避 SHALL 有界（不得改成无限等待/长挂起），且退避后取锁成功
  - 删除 SHALL NOT 在锁仍未释放时开始破坏性清理
- **用例名**：`delete_work_item_plan_does_not_claim_cancel_when_completion_times_out`
  - **构造**：让假 run 任务**不发** completion 信号（模拟收口失联）⇒ `abort_active_run_and_wait_completion` 走 30s 兜底（`src/web/workspace_session/manager/runs.rs:277-292`：超时只 `eprintln!` 后**照常返回 `true`**）
  - **断言**：删除 SHALL NOT 记录/宣称「已取消该 run」这类与事实相反的事实；删除 SHALL NOT 因该取消失败而被额外阻塞（仍按既有独占判定推进；拿不到独占时按既有语义报出占用者）
  - ⚠ 该用例需把 30s 兜底缩到可测时长：允许做法是把兜底时长做成**可注入/可配置**（生产默认仍 30s），或在测试里改用「完成但未收口」的等价状态；**不得**为了跑得快而删掉兜底语义
- **返回值语义（实现期硬约束，写进 §2.1）**：`abort_active_run_and_wait_completion` 的 `bool` 返回值**不得照抄既有人工路径的丢弃写法**（`src/web/workspace_session/manager/mod.rs:756` 丢弃了它）——
  - `false`（无活跃 run）⇒ 正常，不视为失败、不记错误
  - `true` 但未真正收口（兜底超时）⇒ 删除**不得谎称已取消**，但也**不得因此阻塞删除**

---

## 2. 实施

### 2.1 删除路径取消在途 run（REQ-WIGA-15 / REQ-WIGA-17）

- **文件**：`src/web/handlers/lifecycle/deletion.rs`，**落点 `:160`**（`:159` 终止 CAS 之后、`:161`「② try 删除独占」注释之前）
- **做什么**：对 `session_ids`（`:132-134` 已按 `entity_id` 精确收集）逐个 `state.workspace_sessions.get(session_id).await` → 命中则 `manager.abort_active_run_and_wait_completion().await`
- **硬边界**：
  - `:161-163` 注释与 `:164` `delete_attempts` 赋值**保持原样不动**
  - 不新增 API、不改 enrollment 写入、不动人工终止路径
  - 取消不得越界到别的 `entity_id` 的 session
  - 无活跃 run 时 `abort_active_run_and_wait_completion` 返回 `false` 且零副作用（`runs.rs:238`）
- **★返回值必须被检查★**（REQ-WIGA-17；不得照抄既有人工路径的丢弃写法 `manager/mod.rs:756`）：
  - `false`（无活跃 run）⇒ 正常，不视为失败、不记错误
  - `true` 但未真正收口（30s 兜底超时，`runs.rs:277-292` 照常返回 `true`）⇒ 删除**不得谎称已取消该 run**（不写「已停止/已取消」这类与事实相反的事实），但也**不得因此阻塞删除**（仍按既有独占判定推进；拿不到独占时按既有语义报出占用者）
- **验收**：§1.1 / §1.2 / §1.3 转绿

### 2.2 ★取消与取锁之间补「等待 scope 锁释放」的短退避（REQ-WIGA-17，条件触发）★

- **触发条件**：§1.6 的时序尾隙用例**先红**（即：只靠「取消完就立刻取锁」不足以保证成功）
- **做什么**：在「取消完成」与「取删除独占」之间加一段**有界的**短退避，等待 scope 锁真正释放后再取独占；退避时长与次数**有界**（不得改成无限等待、不得让删除请求长期挂起）
- **契约同步**：★该退避一旦落地，SHALL 回写进 `REQ-WIGA-17` 的「取消后删除独占必须最终取得成功」scenario（退避是契约的一部分，不得只存在于实现里）★
- **若 §1.6 未红**（实现已满足）：也要在报告里写明「时序尾隙已验证为不发生/已被覆盖」，**不得**默认跳过
- **验收**：§1.6 两个用例转绿

### 2.3 时间线中止态表达（REQ-WIGA-16）

- **文件**：`src/product/workspace_engine/session_state/timeline.rs`（`:132-141` `finish_aborted_run`、`:401-455` `update_timeline_node`、`:407-412` `completed_at` 判定）+ 状态值域 `src/web/workspace_ws_types/timeline.rs:47-53`
- **做什么**：被取消的 run 节点 `status` 表达「中止」而非 `failed`；`stage` 与 `status` 一致；`completed_at` 落盘
- **词汇约束**：沿用既有 `aborted` 词汇（`web/src/api/types/workspace.ts:627` `ProviderStatus`、`:635` `ExecutionEventStatus` 已有 `aborted`），**不另造冲突词汇**
- **硬边界**：真实失败仍是 `failed`；既有失败/跳过/完成三态判定与文案逐字不变；不改 run 收口顺序与完成度信号语义
- ⚠ 状态值域是对外契约面（WS 消息 + 前端类型镜像）⇒ 新增取值必须同步 §2.4

### 2.4 前端同步（状态值域 + 终态文案）

- `web/src/api/types/workspace.ts:620`：`TimelineNodeStatus` 值域镜像补「中止」态
- `web/src/pages/ChatCockpitPageParts.tsx:74-96` `generationStatusText` 的终态映射（现：failed→「生成失败」、completed→「生成完成」、其余→「生成已跳过」）⇒ 中止态 **MUST NOT** 落到「生成已跳过」
- **测试**：`web/src/pages/ChatCockpitPage.status.test.tsx`（既有参数化用例 `:304-310` 已覆盖 completed/skipped 分支）补「中止」分支
- **验收**：`pnpm -C web exec vitest run src/pages/ChatCockpitPage.status.test.tsx` + `pnpm -C web tsc -b` 全绿；既有文案逐字不变

---

## 3. 定向验证（逐条记录退出码与 passed/failed 计数）

```bash
flock /tmp/aria-cargo.lock cargo test --locked --test it_web delete_work_item_plan_cancels_inflight_run -- --nocapture
flock /tmp/aria-cargo.lock cargo test --locked --test it_web delete_work_item_plan_cancel_does_not_touch_other_session_run -- --nocapture
flock /tmp/aria-cargo.lock cargo test --locked --test it_web delete_work_item_plan_without_inflight_run_keeps_existing_semantics -- --nocapture
flock /tmp/aria-cargo.lock cargo test --locked --test it_web delete_work_item_plan_succeeds_when_scope_lock_releases_after_completion_signal -- --nocapture
flock /tmp/aria-cargo.lock cargo test --locked --test it_web delete_work_item_plan_does_not_claim_cancel_when_completion_times_out -- --nocapture
flock /tmp/aria-cargo.lock cargo test --locked --lib aborted_run_node -- --nocapture
flock /tmp/aria-cargo.lock cargo test --locked --lib workspace_engine -- --nocapture      # finish_aborted_run 既有回归
pnpm -C web exec vitest run src/pages/ChatCockpitPage.status.test.tsx
```

- 必查：删除用例跑完后，磁盘 `entity-operations/scope-locks/plan/<plan_id>.lock` 无残留独占占用；`timeline_nodes.json` 里被取消的节点不再是 `status=failed`。

## 4. 全量门禁（全部带 `flock` 串行；⚠ 禁 `-j 1`、禁 `--test-threads=1`）

```bash
flock /tmp/aria-cargo.lock cargo fmt --check
flock /tmp/aria-cargo.lock cargo clippy --all-targets --all-features --locked -- -D warnings
flock /tmp/aria-cargo.lock cargo test --locked --all-targets
flock /tmp/aria-cargo.lock cargo test --locked --test it_core large_file_guard   # 行数门禁：改动文件不得超限
pnpm -C web test
pnpm -C web tsc -b
openspec validate work-item-delete-cancels-inflight-run --strict
```

- 门禁判据：**退出码 0 且无失败用例**；`openspec validate --strict` 需 `is valid`（无 WARNING/ERROR）。

## 5. 独立审查（k3-reviewer，只读）

- 派 `k3-reviewer` 对本次改动做**只读独立审查**，范围：`src/web/handlers/lifecycle/deletion.rs`、`src/product/workspace_engine/session_state/timeline.rs`、`src/web/workspace_ws_types/timeline.rs`、前端两个文件、本 change 的两份 delta。
- 审查要点（逐条给结论）：
  1. 取消范围是否**严格限定** `entity_id` 匹配的 session；有无越界取消别的 session 的 run
  2. 是否引入了**第二套终止语义**（应复用 `abort_active_run_and_wait_completion`）
  3. 是否**弱化/删除**既有断言或改动既有文案
  4. 「中止」状态表达是否与既有词汇自洽、`stage`/`status` 语义是否自洽、前后端值域是否对齐
  5. 兜底超时/取消失败时是否**如实报占用者**，是否可能在 run 仍在跑时开始破坏性清理
  6. ★取消返回值是否被检查★（不得照抄 `manager/mod.rs:756` 的丢弃写法）：「未真正收口」时是否**谎称已取消**，是否反而因此阻塞了删除
  7. ★取消与取锁之间的时序尾隙★：实现是否依赖「completion 信号 ⇒ 锁已释放」这一**未验证**假设；若补了退避，退避是否**有界**、是否已回写进 `REQ-WIGA-17` 的 scenario
  8. 有无越界改动（enrollment 写入、人工终止路径、provider 网关、REQ-DEL-04 确认门禁）
- 结论落 `cadence/reports/`；**P0/P1 必须回修并重审**，不得带 P0 归档。

## 6. owner 真机验收（★这次必须执行，不留未勾★）

> owner 侧操作（复现路径与 2026-11-06 真机一致：开发期 5173 / 后端 4317）

- [ ] 6.1 启用自动化（生成 WIG）⇒ provider 开始输出
- [ ] 6.2 **不点右下角「终止」**，直接点该 WIG 的「删除」⇒ 不再报「实体存在进行中的运行或并发写入，请等待收口后重试删除」（HTTP 409 `entity_busy`），删除成功
- [ ] 6.3 删除后 provider **立即停止输出**（不再有新 chunk）
- [ ] 6.4 被取消的 provider run 节点：`status` 显示为「中止」而不是「失败」，`summary` 为「运行已中止」，`completed_at` 已落盘，且 `stage` 不再是 `running`；compile、门卡、上下文类节点的 `stage` 保持其所属阶段，不属于本项判据
- [ ] 6.5 回归：无 in-flight run 的 WIG 删除行为与改动前一致；人工「终止整个自动化」路径行为与文案不变
- [ ] 6.6 证据：`/tmp/aria-deploy/session.log` 在删除时刻出现 `[aria-cancellation]` 打点（取消确实发生过），且其后无新 provider 输出
- [ ] 6.7 日志/返回体中**不得出现**与事实相反的「已取消/已停止」表述（取消未真正收口时不得谎称）；删除本身仍按既有独占判定给出结果

## 7. 验收纪律

- 7.1 **TDD**：先写失败测试并见到预期红，再实现；红的原因必须正是缺本修法（不是编译错、不是夹具错）
- 7.2 既有断言**一条不许删/缩窄**；既有文案逐字不变
- 7.3 每条 cargo 命令都带 `flock /tmp/aria-cargo.lock`（串行）；⚠ **禁 `-j 1` / `--test-threads=1`**
- 7.4 报告落 `cadence/reports/`：红→绿证据、命令与计数、审查结论、真机验收勾选状态
- 7.5 本 change 未 archive 前，★`openspec/specs/**` 主 spec 不得直接改★；契约只经本 change 的 delta 落地
