# 审查报告：work-item-delete-cancels-inflight-run（bug1 修复）

- **审查者**：k3-reviewer（只读独立审查，tasks.md §5）
- **审查日期**：2026-11-07
- **审查对象**：worktree `bugfix-b-11-02`，提交区间 `43eb39bc^..fabbb101`（代码提交：`4ba3d5d9` 取消块 / `3fb6558e` 三态化＋短退避 / `32afdd96` 前端值域 / `b91a0c72` coding 视图中止态 / `71b576f5` 护栏用例 / `64261990` 具名引用清理 / `fabbb101` 中止态落盘与 stage 归一）
- **审查方式**：静态只读——`git diff`/`git diff --numstat`/`git show` 逐提交核对、全仓 grep 调用点穷举、逐文件行号级阅读；**未跑任何 cargo/npm 命令、未改任何产品文件、未 git add/commit**

---

## 1. 总判

**可交付**（带 P2 回修建议）。

一句话理由：三条新增契约（REQ-WIGA-15/16/17＋MODIFIED REQ-DEL-06）的实现与测试逐条对得上——取消严格按 `entity_id` 收集面复用既有取消入口、三态返回值被具名 match、尾隙以有界（≤10×50ms）探测退避补足并已回写契约、中止态前后端值域逐字对齐且既有断言/文案零削弱；**P0/P1 为零**，P2 共 4 条（死代码谓词、用例②对日志面谎称的捕获力、TimedOut 时 409 details 的占用者粒度、前端三处次级映射未给 aborted 专属分支），均可回修不阻塞。

---

## 2. 八条审查要点逐条结论

### 要点 1：取消范围是否严格限定 `entity_id` 匹配的 session —— **通过**

- 取消块只遍历 ⓪ 收集的 `session_ids`（`src/web/handlers/lifecycle/deletion.rs:133-135` → `workspace_session_ids_for_entities`，`deletion.rs:915-928`，`filter(|session| entity_ids.contains(&session.entity_id))` 按 `entity_id` 精确过滤）；循环体内仅 `state.workspace_sessions.get(session_id)`（`deletion.rs:171-172`，registry `get` 为无副作用查找，不创建）。
- 测试护栏：`tests/it_web/web_coding_attempt_api/part_14.rs:837` `delete_work_item_plan_cancel_does_not_touch_other_session_run` —— 另一实体（设计稿）session 的 run 未被取消（`!other_fake.cancel_token.is_cancelled()`）、run 仍在位（`active_run_incarnation().is_some()`）、其实体保留；同时断言被删实体的 run **确实被取消**（防「没越界只是没取消」的假象，part_14.rs:1010 前后）。

### 要点 2：是否引入第二套终止语义 —— **通过（未引入）**

- 删除路径唯一取消入口是 `manager.abort_active_run_and_wait_completion()`（`deletion.rs:177`），即人工终止路径同一 API（`src/web/workspace_session/manager/mod.rs:790`）；底层取消原语仍是私有 `cancel_run`（`runs.rs:524-531`：`ProviderCommand::Abort` + `cancel.cancel()`），本 patch 未复制任何取消逻辑、未新增取消 API。
- 新增面只有：返回类型升格为三态枚举（`runs.rs:16-25`）＋兜底时长可注入（`mod.rs:144-147/560-568`）＋删除侧短退避探测（`deletion.rs:193-213`，属等待逻辑不是终止语义）。

### 要点 3：是否弱化/删除既有断言或改动既有文案 —— **通过（零削弱）**

核对方法见 §5；结论：`part_14.rs` numstat **489 增 0 删**、`part_25.rs` 为全新文件、两处 `entity_busy` 文案逐字未动（`deletion.rs:616`、`deletion.rs:954`）、时间线中止 summary「运行已中止」为既有字符串逐字保留（`timeline.rs:139`，diff 显示改动前即此文案）、前端既有参数化用例仅重排缩进（内容逐字相同）。

### 要点 4：「中止」表达词汇自洽、stage/status 自洽、前后端值域对齐 —— **通过**

- 后端 `TimelineNodeStatus` 新增 `Aborted`，serde `rename_all = "snake_case"` ⇒ 线上值 `"aborted"`（`src/web/workspace_ws_types/timeline.rs:46-56`）。
- 前端三处镜像逐字一致补 `"aborted"`：`web/src/api/types/workspace.ts:620`、`web/src/state/workspace-ws-store-types.ts:111`、`web/src/api/types/coding.ts:119`。
- 词汇自洽：既有 `ProviderStatus`/`ExecutionEventStatus` 已用 `"aborted"`（`workspace.ts:627/:635`），未另造词汇（满足 REQ-WIGA-16「不另造冲突词汇」）。
- stage/status：`finish_aborted_run` 落 `Aborted`＋completed_at（`timeline.rs:132-144`）；`update_timeline_node` 终态判定纳入 `Aborted`（`timeline.rs:409-415`）且仅对「agent 节点 + stage==Running + 终态」归一 stage（`timeline.rs:437-442`）。主文案面：`ChatCockpitPageParts.tsx:92-98` aborted→「生成已中止」（不落「生成已跳过」）；`plan-repair-session.ts:764-765` `codingTimelineStatus` 补 `case "aborted"`。次级映射面有三处未补（见 finding P2-4，不构成失败表述）。

### 要点 5：兜底超时/取消失败时是否如实报占用者、是否可能在 run 仍在跑时破坏性清理 —— **通过**

- `TimedOut` 分支只打事实性打点「did not settle within completion wait timeout」（`deletion.rs:185-189`），无「已取消/已停止」字样，且**不 return**——继续走既有 ② 取独占循环。
- 破坏性清理只能发生在取得删除独占 lease 之后（`deletion.rs:223-239` 的 loop，之后的 pending/墓碑/purge 全在 lease 之内）；run 仍持共享 flock 时 `try_acquire_delete` 恒 `Busy`（`src/product/entity_operation_store.rs:174-193`）→ 既有 `deletion_busy_error`（`deletion.rs:603-619`，文案逐字未动，details 带占用者 enrollment id）。
- 测试实证：`part_25.rs:160` `delete_work_item_plan_does_not_claim_cancel_when_completion_times_out` —— 409＋`entity_busy`＋逐字文案、body 不含「已取消/已停止」、5s 内返回、plan 仍在、墓碑未立（`part_25.rs:253-303`）。
- 粒度缺口：无 enrollment 的现场（如该用例），409 details 不含在途 run/session 身份，仅 eprintln 有 session_id（finding P2-3，不阻塞）。

### 要点 6：取消返回值是否被检查 —— **通过（未照抄丢弃写法）**

- `deletion.rs:177-190` 对三态**具名 match**：`NoActiveRun` 静默、`Completed` 事实打点＋置 `run_cancel_settled`、`TimedOut` 只打「未收口」事实。「未真正收口」时不谎称（响应体无任何取消宣称——删除成功路径响应体只有 `{"status":"deleted"}`，失败路径是既有 409 文案），也不因取消失败阻塞删除。
- 人工终止路径的既有丢弃语义保留且仅显式化为 `let _ =`（`mod.rs:788-790`，行为/顺序/文案不变）——两处对比正是「不照抄」的证据。
- 历史：`3fb6558e` 曾用谓词分支，`64261990` 改具名 match 并去掉 `#[allow(unused_imports)]`（git show 已核）。

### 要点 7：取消与取锁的时序尾隙 —— **通过（已补有界退避并回写契约）**

- 实现不依赖「completion ⇒ 锁已释放」：`Completed` 后进入探测退避 `for _ in 0..10`，每轮 `try_acquire_delete` 成功即 `drop(probe); break`（不保留 lease），`Busy|Deleting` 睡 50ms 重试，`Deleted|Store` 交还下方既有循环（`deletion.rs:193-213`）。合计 ≤10 轮、≤500ms，有界。
- 探测安全性：`try_acquire_delete` 只取 flock＋读墓碑、零写盘（`entity_operation_store.rs:174-193`），Drop 即释放——探测不产生墓碑/pending/破坏性动作（要点 11 详述竞态）。
- 契约回写：delta `openspec/changes/work-item-delete-cancels-inflight-run/specs/work-item-group-autopilot/spec.md:78`（REQ-WIGA-17 场景①）逐字写明「每轮一次『取删除独占并立即释放』的探测（探测不保留 lease、不触碰任何破坏性清理），Busy/Deleting 才重试，至多 10 轮、每轮间隔 50ms（合计 ≤500ms），SHALL NOT 无限等待」——与实现一致。
- 确定性用例：`part_25.rs:15-150` `delete_work_item_plan_succeeds_when_scope_lock_releases_after_completion_signal`（completion 先发、250ms 后才放锁的倒序尾隙）断言 200＋deleted＋锁已释放。

### 要点 8：有无越界改动 —— **通过（无未授权越界）**

- enrollment 写入：取消块只调 manager API，不碰 enrollment 存储（diff 内无 `IssueAutomationStore` 写调用）。
- 人工终止路径：仅 `let _ =` 适配返回类型＋注释（`mod.rs:788-790`）。
- provider 网关：无改动（`src/web/workspace_ws_handler/**` 不在 diff）。
- REQ-DEL-04 确认门禁：无改动（`DeletionConfirmation` 相关代码不在 diff；护栏用例②断言 coding 门禁 `coding_workspace_exists` 稳定码与文案逐字不变，`part_25.rs:646-657`）。
- 唯一超清单改动：`src/product/workspace_engine/mod.rs` 把 `ProviderRunKind` 从 `pub(crate)` 放宽为 `pub`（diff :167-176）——为集成测试 `start_run(ProviderRunKind::ReviewOnly, …)` 可见性所需，**controller 已授权**（计划提交 `692b8940`「Task 1 独占清单补入 ProviderRunKind 可见性放宽（controller 授权）」），纯可见性无行为变化。

---

## 3. 额外必查项 9–15 逐条结论

### 9. `RunAbortOutcome` 三态判定与实际行为一致性 —— **一致**

`src/web/workspace_session/manager/runs.rs:272-331`：

- `NoActiveRun`（:300-302）：锁内 `active_run.take()` 为 None——零取消、零副作用，与既有 `return false` 语义等价。
- `Completed`（:330）：兜底时长内观察到 completion watch 为 true——真实收口证据（`finish_run` 按 token 发信号，`runs.rs:241-245`）。
- 「无 completion 登记（旧 fixture/已被收口）⇒ Completed」（:303-306）：**既有语义逐字保留**（diff 显示原为 `return true`）。其「谎称」风险评估：该分支 active_run 在位但 `run_completions` 无登记（生产中 start_run 会同时登记两者；此分支主要服务旧夹具/收口后残留）。即使误判为 Completed，后果仅是（a）eprintln 打「cancelled」（取消确实已发出——`cancel_run` 在锁内已调，:292，非虚言）＋（b）多跑 ≤500ms 探测；若该 run 实际仍持共享锁，② 循环仍 `Busy` → 409 报占用者——**不会**出现「谎称已取消＋删除成功＋run 还在写」的组合，破坏性清理仍被独占锁挡住。
- `TimedOut`（:324-328）：超时 eprintln 后返回，调用方不阻塞（用例② 5s 上界覆盖：注入 200ms 兜底，实测断言 elapsed < 5s，`part_25.rs:236-280`）。

### 10. 返回类型变更的调用点波及 ＋ `set_run_completion_wait_timeout` 误用面 —— **全部处理；误用面为已知权衡（P3）**

全仓 grep `abort_active_run_and_wait_completion`（src+tests）穷举调用点，逐一核对：

| 调用点 | 处理 |
|---|---|
| `src/web/handlers/lifecycle/deletion.rs:177` | 具名 match 三态（本 change 消费点）✓ |
| `src/web/workspace_session/manager/mod.rs:790` | 人工终止路径，`let _ =` 显式丢弃＋注释（既有语义）✓ |
| `src/web/workspace_session/manager/runs.rs:572`（内部测 `termination_tests`） | 用于 select/超时证明，:619 断言升格为 `assert_eq!(had_run, RunAbortOutcome::Completed)`（bool `true` 的忠实升格）✓ |
| `runs.rs:608`（内部测） | 包在 spawn 任务里 await 后断言 ✓ |
| `tests/it_web/part_14.rs` / `part_25.rs` | 经 HTTP 删除路径间接触发，无直接调用 ✓ |

`set_run_completion_wait_timeout`（`mod.rs:560-568`，`#[doc(hidden)] pub`）：**无编译期约束**，任何持 `Arc<WorkspaceSessionManager>` 的生产代码都能缩短兜底。全仓唯一调用是 `part_25.rs:210`（测试注入 200ms）。文档标注「仅供测试注入，生产代码不调用」＋集成测试在库外无法用 `#[cfg(test)]` 是其存在理由——记 P3/未决，不阻塞。构造点核对：`run_completion_wait_timeout_millis` 字段在 **4 处**构造点全部初始化默认 30_000（`mod.rs:208/:263/:551/:725`；计划写「三处」，实际四处均覆盖，无遗漏）。

### 11. 短退避探测的竞态安全性与上界 —— **安全；上界对收口级尾隙足够，超界退化为既有 409**

- 探测动作 = `try_acquire_delete` 成功 → `drop(probe)`（`deletion.rs:202-206`）。`try_acquire_delete` 仅 `try_acquire_exclusive` flock＋读墓碑 JSON（`entity_operation_store.rs:174-193`），**零写盘**；Drop 释放 flock。不立墓碑、不写 pending、不触任何清理——探测不是破坏性动作。
- 「探测成功→释放→真正删除再取锁被别人抢走」的竞态：窗口内若他人取得共享锁（如 WS 用户在同 session 再起 run——注意 enrollment 已在 ①.5 终止，autopilot 不会再来），② 主循环 `Busy` → 既有重试（terminated_automation 为 Some 时 10×100ms）→ 仍 Busy 则 409 `entity_busy` 报占用者——即**最坏退化为既有语义**，无损坏、无谎称、无长挂起。这是「删除赢得准入前共享写者可随时出现」的固有属性，非本修法引入。
- 上界充分性：10 轮×50ms=500ms 针对的是「run 收口完成→释锁」的毫秒级窗口（同进程 drop lease）；用例① 以 250ms 确定性尾隙覆盖。若生产释锁延迟 >500ms（理论上存在），退化为 409 重试——有界、可诊断、不破坏。判：足够。

### 12. 时间线 `stage` 归一是否误伤 —— **不误伤；复用 `Completed` 是显式决策**

- 触发条件三合一：`is_terminal_status && node.agent.is_some() && node.stage == Running`（`timeline.rs:437-442`）。只归一「provider run 节点」。
- 无 agent 的工作流节点（如 `WorkItemPlanCompile`，`compile.rs:748-756` 证实 `agent: None`）不受影响——「compile 节点收口后仍记 running」的既有断言保持（代码注释亦明示此收敛边界，`timeline.rs:432-436`）。
- failed 节点同样被归一——这不是误伤：REQ-WIGA-16 delta 场景② 明文「WHEN 某个 run 节点进入终态（**中止 / 失败 / 完成**）THEN stage SHALL NOT 仍以 running 表述」（delta spec :56-60）。
- `WorkspaceStage` 值域确无失败/中止终态取值（`src/product/workspace_engine/types.rs:7-16`、`ws stage.rs:5-13`），复用唯一终态 `Completed` 且注释声明「不新增对外取值」。展示层面以 `status` 为主判（failed/aborted 均有专属 status 与文案），`stage=completed` 表「已收尾」不覆盖失败/中止语义——可接受；潜在观感问题（stage=completed+status=failed）归入 P2-4 一并提示。

### 13. `Aborted` 枚举新增后穷尽匹配补齐 —— **Rust 编译器保证；前端主面已补，次级三处未补（P2-4）**

- Rust：`TimelineNodeStatus` 的 match 无遗漏风险由编译器穷尽性检查兜底（本次 diff 未引入通配 match）；serde 值 `"aborted"` 与前端镜像逐字一致（见要点 4）。
- 前端逐点核对：
  - ✅ `web/src/state/plan-repair-session.ts:753-767` `codingTimelineStatus` 补 `case "aborted"`（无 default 的穷尽 switch，TS 强制）；
  - ✅ `web/src/pages/ChatCockpitPageParts.tsx:26-31` `TERMINAL_NODE_STATUSES` 补 `aborted: true`、`TerminalNodeContext.status` 联合补 `"aborted"`、`:92-98` 文案分支；
  - ✅ `web/src/pages/useCockpitNodeDetailHydration.ts:16-20` 水合集合用**排除法**（`!== "active" && !== "paused"`），aborted 天然入集合——REQ-NDR-02「水合集合覆盖全部终态节点（至少 failed/aborted/…）」继续满足（其测试 `useCockpitNodeDetailHydration.test.ts:97-99` 本就以 `status: "aborted"` 为夹具）；
  - ✅ `web/src/state/coding-workspace-store.ts:610-612` `isActiveNodeStatus` 谓词式（aborted→非活跃，正确）；
  - ⚠ 未补三处：`web/src/state/workspace-cockpit-flow.ts:28-41`（switch 有 default，aborted→`"done"`→执行流窄栏显示「已完成」）、`web/src/state/plan-repair-session.ts:640-642` `isTerminalTimelineStatus` 不含 aborted（终态快照保护弱一档）、`web/src/components/chat-workspace/TimelineNodeList.tsx:149-152`（legacy 列表非 completed 回退显示原始串 "aborted"）。均非「表述为失败」，不违 REQ-WIGA-16 字面；归 P2-4。

### 14. 用例② 的变异捕获力 —— **用户可见面强捕获；日志面不捕获（P2-2）**

变异推演（不真改代码）：

| 变异 | 用例② 是否见红 | 捕获断言 |
|---|---|---|
| `TimedOut` 被当 `Completed`（回退旧 bool 语义：超时照常返回「成功」） | **不红**（见下分析） | — |
| 响应体写入「已取消/已停止」类宣称 | 红 | `part_25.rs:266-274` `!contains("已取消"/"已停止")` |
| 改 409 文案/错误码 | 红 | `:253-263` `code=="entity_busy"`＋message 逐字 `assert_eq!` |
| 删除被取消失败长阻塞（兜底固定 30s 不可注入） | 红 | `:239-250` 5s timeout panic＋`:277-280` elapsed<5s |
| 未取得独占就开始破坏性清理 | 红 | `:288-303` plan 仍在＋无墓碑 |
| 取消根本没发出 | 红 | `:282-286` `cancel_token.is_cancelled()` |

关键行「`TimedOut` 当 `Completed`」不红的推演：该变异下 `run_cancel_settled=true` → 探测 10×50ms（测试持有共享 lease ⇒ 全 Busy）→ ② 主循环 Busy → 409＋逐字文案＋body 无「已取消」＋耗时≈200ms+500ms+重试窗口 <5s＋无墓碑——全部断言仍绿；唯一变化是 eprintln 从「did not settle」变成「cancelled」，**用例不观察 stderr**。即：契约 REQ-WIGA-17 场景② 的「SHALL NOT 记录任何与之矛盾的『已停止』事实」若按字面含日志面，则用例② 捕获力有洞（P2-2）；用户可见面（响应体/错误码/时长/零破坏）捕获力完整。

### 15. 契约 ↔ 实现逐条对应 —— **逐条对应（对照表见 §6）**

REQ-WIGA-15 五个场景、REQ-WIGA-16 三个场景、REQ-WIGA-17 两个场景、MODIFIED REQ-DEL-06 例外支均有实现落点与测试落点；REQ-WIGA-17 场景① 的退避形态（10 轮×50ms、探测即释放、Busy/Deleting 才重试、≤500ms、不无限等待）已逐字回写进 delta（spec.md:78）。唯一粒度争议：REQ-WIGA-15 场景④「明确指出占用者」在「无 enrollment、只有失控 run」现场只落到 eprintln 的 session_id（P2-3）。

---

## 4. Finding 分级清单

**P0（必须回修并重审）：无。P1（应回修）：无。**

### P2-1：`RunAbortOutcome::is_completed`/`is_timed_out` 成死代码，doc 注释已过时

- **问题**：`64261990` 把删除路径从谓词分支改为具名 match 后，两个谓词（`src/web/workspace_session/manager/runs.rs:32-39`）全仓零调用；`runs.rs:29-31` 的注释仍声称「删除路径经谓词按三态如实分支（枚举名无法进入删除路径）」——与现状（`deletion.rs:11` 直接 `use`＋`:177-190` 具名 match）矛盾，误导维护者。
- **证据**：`runs.rs:28-40`；全仓 grep `is_completed|is_timed_out` 于 `RunAbortOutcome` 上零命中；`git show 64261990` 显示谓词用法被移除。
- **影响**：公共 API 面新增无用导出＋注释失真；不影响行为、不触发 clippy（经 `pub mod web`（lib.rs:10）→`pub mod workspace_session`（web/mod.rs:36）→双层 `pub use` 的公开可达链，dead_code 不报）。
- **建议修法**：删除 `impl RunAbortOutcome` 的两个谓词与过时注释（保留枚举 doc）；或保留谓词则改写注释为真实理由。
- **建议落点**：`src/web/workspace_session/manager/runs.rs:28-40`。

### P2-2：用例② 无法捕获「日志面谎称」（变异捕获力缺口）

- **问题**：REQ-WIGA-17 场景② 措辞「SHALL NOT 记录任何与之矛盾的『已停止』事实」含记录面；用例② 的不谎称断言只检查 HTTP 响应体（`part_25.rs:266-274`），`TimedOut` 分支的 eprintln 文案不被观察。变异「TimedOut 日志改成『已取消』」用例仍绿（§3-14 推演）。
- **证据**：`part_25.rs:265-274`；`deletion.rs:185-189`（eprintln 为唯一 TimedOut 落点）；delta spec:80-85。
- **影响**：诊断日志若回归谎称，护栏不设防；用户可见面不受影响。
- **建议修法**：三选一——(a) 用例② 捕获子进程 stderr 断言不含「已取消/已停止」；(b) TimedOut 打点改走可断言的结构化通道；(c) owner 裁决契约「记录」仅指用户可见面，维持现状并在 delta 注明。
- **建议落点**：`tests/it_web/web_coding_attempt_api/part_25.rs:265-274`（或 delta spec 措辞）。

### P2-3：`TimedOut` 时 409 details 未带在途 run/session 占用者身份

- **问题**：`deletion_busy_error` 的 occupant 只认 automation enrollment（`deletion.rs:603-619`＋`automation_enrollment_bound_to_entity` :625-652）；「无 enrollment、只有失控 run」的现场 details 仅 `entity_kind/entity_id`，可诊断的 session 级占用者身份只在 eprintln（`deletion.rs:186-188`）。
- **证据**：`deletion.rs:603-619`、`:185-189`；用例②（未播种 enrollment）的 409 body 即此形态；delta REQ-WIGA-15 场景④「返回明确指出占用者的可诊断错误」、MODIFIED REQ-DEL-06「取消未收口时 MUST 明确报出占用者」。
- **影响**：真机排查「到底谁占着」需翻服务日志；不谎称、不误报（既有 F3 口径「不可证明占用不误报」未被破坏）。
- **建议修法**：`TimedOut` 时把该 session_id 并入 `deletion_busy_error` 的 details（如 `busy_run_session_ids`），不改动既有文案与稳定码。
- **建议落点**：`src/web/handlers/lifecycle/deletion.rs:603-619`（消费点 `:185-189` 与 `:233-235`）。

### P2-4：前端三处次级映射未给 `aborted` 专属分支/保护

- **问题**：(1) `web/src/state/workspace-cockpit-flow.ts:28-41` `cockpitFlowState` default→`"done"`，执行流窄栏的 aborted 瓦片标签显示「已完成」（`TimelineNodeList.tsx:105-112` `FLOW_STATE_LABELS`），与主状态行「生成已中止」轻度不一致；(2) `web/src/state/plan-repair-session.ts:640-642` `isTerminalTimelineStatus` 不含 `"aborted"`，快照 reconcile 对 aborted 节点缺「终态不被回改」保护；(3) `web/src/components/chat-workspace/TimelineNodeList.tsx:151` legacy 列表对非 completed 状态回退显示原始串（将显示英文 "aborted"）。
- **证据**：上述行号；对照已补齐的 `ChatCockpitPageParts.tsx:26-31/92-98`、`plan-repair-session.ts:764-765`。
- **影响**：展示层观感与保护面弱一档；均不违反 REQ-WIGA-16 字面（未表述为失败、词汇一致）。
- **建议修法**：`cockpitFlowState` 加 `case "aborted"` 映射到合适终态（或给 CockpitFlowState 增加 aborted 态与标签）；`isTerminalTimelineStatus` 补 `|| status === "aborted"`；legacy 列表加中文标签映射。
- **建议落点**：`web/src/state/workspace-cockpit-flow.ts:29-41`、`web/src/state/plan-repair-session.ts:640-642`、`web/src/components/chat-workspace/TimelineNodeList.tsx:149-152`。

### P3-1：`set_run_completion_wait_timeout` 无编译期防误用

- **问题/证据/影响**：见 §3-10。**建议**：维持现状（集成测试库外约束所致）＋未决问题登记；如后续收紧，可加 `#[cfg(debug_assertions)]` 断言或迁往测试专用构造入口。落点 `src/web/workspace_session/manager/mod.rs:560-568`。

---

## 5. 既有断言与文案未被削弱/改动的核对结论

核对方法（四道）：

1. **增删行面**：`git diff --numstat 43eb39bc^..fabbb101`——`part_14.rs` **489/0**（纯追加，既有断言零删改）、`part_25.rs` **738/0**（新文件）、`web_coding_attempt_api.rs` **1/0**（include 追加）；全区间 src+tests+web 合计删 55 行，逐条过目：全部是签名/文档替换（`bool`→`RunAbortOutcome`）、`use` 列表重排（ProviderRunKind 挪位）、`assert!(had_run)` 升格（见下）与前端两行缩进重排。
2. **文案面**：两处 `entity_busy` 文案「实体存在进行中的运行或并发写入，请等待收口后重试删除」在改后文件 `deletion.rs:616`（`deletion_busy_error`）与 `:954`（`cascade_delete_api_error`）逐字仍在，且均不在 diff 修改区；时间线 summary「运行已中止」为改动前已有字符串（diff 上下文可见旧代码即 `Some("运行已中止".to_string())`，本 patch 只把 `Failed` 换成 `Aborted`，summary 未动）；`[aria-termination]` 超时打点原文保留（`runs.rs:325-327`）。护栏用例反向锁死：`part_25.rs:259-263`（entity_busy 逐字）、`:652-657`（coding_workspace_exists 稳定码＋文案逐字）、`:582-584`（既有账本条目逐字未变）。
3. **前端文案面**：`git diff` 的 `-` 行仅 5 行——`ChatCockpitPage.status.test.tsx` 两行（缩进变化，字符串内容逐字相同：`["completed", "生成完成"], ["skipped", "生成已跳过"]` 仍在新 it.each 中）、`ChatCockpitPageParts.tsx` 两行（类型联合扩宽＋三元链尾部「生成已跳过」仍保留为兜底）、`coding.ts`/`workspace.ts`/`workspace-ws-store-types.ts` 各一行（联合类型追加 `"aborted"`，旧值全保留）。
4. **断言升格而非削弱**：`runs.rs:619` `assert!(had_run)` → `assert_eq!(had_run, super::RunAbortOutcome::Completed)`——`true` 语义的忠实升格（该用例构造的是真实 completion 收口），强度不减；新增强断言（`timeline.rs` 测试对 aborted 的 status/summary/stage/completed_at 四连断言＋failed 回归断言）。

结论：**既有断言零删除/零缩窄；既有文案零改动。**

---

## 6. 契约 ↔ 实现一致性对照表

| 契约条目（delta） | 实现落点 | 测试落点 | 判定 |
|---|---|---|---|
| REQ-WIGA-15 场景1：先取消在途 run 再取独占、不得 entity_busy 长期阻塞、取消后无 provider 输出 | `deletion.rs:170-192`（①.5 终止 CAS 之后、② 取独占之前）；取消经 `cancel_run` 发 `ProviderCommand::Abort`+token（`runs.rs:524-531`） | `part_14.rs:690` 用例1（200+deleted、token 已触发、无 actual-launch、锁已放、plan 已删） | ✅ |
| REQ-WIGA-15 场景2：取消不越界 | `session_ids` 按 `entity_id` 精确收集（`deletion.rs:133-135`→`:915-928`）；registry `get` 无副作用 | `part_14.rs:837` 用例2 | ✅ |
| REQ-WIGA-15 场景3：无在途 run 语义不变 | `NoActiveRun` 零副作用零等待（`deletion.rs:178`、`runs.rs:300-302`） | `part_14.rs:1010` 用例3（200+deleted、既有清理断言逐字复用、10s 上界、无新增副作用） | ✅ |
| REQ-WIGA-15 场景4：取消未收口如实报占用者、不得在 run 跑时破坏性清理 | `TimedOut` 事实打点不阻塞（`deletion.rs:185-189`）；破坏性清理仅在独占 lease 后（`:223-239`） | `part_25.rs:160` 用例②（409+逐字文案+无墓碑+实体仍在+5s 上界） | ✅（占用者粒度 P2-3） |
| REQ-WIGA-15 场景5：不弱化既有终止/前置语义（不写 enrollment、不物删记录、不动人工路径、不放宽门禁） | 取消块零 enrollment 写入；`mod.rs:788-790` 仅 `let _ =`；确认门禁未动 | `part_25.rs:434` 用例①（终止恰好一次/作废恰好一次/revision+2/账本旧条目逐字未变/记录未物删）；`:595` 用例②（门禁拒绝零破坏+越界登记未动） | ✅ |
| REQ-WIGA-16：中止不以失败表述、summary「已中止」、completed_at 落盘、stage/status 一致、真实失败仍失败、词汇沿用 aborted | `timeline.rs:132-144`（Aborted+「运行已中止」）、`:409-420`（终态集+completed_at）、`:437-442`（stage 归一，仅 agent 节点）；值域 `ws_types/timeline.rs:53-55` | `timeline.rs` lib 测 `aborted_run_node_is_not_failed_and_stage_matches_status`（四连断言+failed 回归） | ✅ |
| REQ-WIGA-17 场景①：取消后独占最终取得成功；尾隙以有界探测退避补足（≤10 轮×50ms、探测即释放、Busy/Deleting 才重试、不无限等待） | `deletion.rs:193-213`（逐字对应）＋`entity_operation_store.rs:174-193`（探测零写盘） | `part_25.rs:54` 用例①（250ms 倒序尾隙下 200+deleted+锁已释放） | ✅ |
| REQ-WIGA-17 场景②：未收口不得谎称、不得阻塞、NoActiveRun 属正常 | 三态具名 match（`deletion.rs:177-190`）；可注入兜底（`mod.rs:560-568`＋`runs.rs:307-312`） | `part_25.rs:160` 用例② | ✅（日志面 P2-2） |
| MODIFIED REQ-DEL-06 例外支：实体自身自动化 run 不长期阻塞删除；run 仍在跑时不得破坏性清理 | 同 REQ-WIGA-15 实现（同一取消块） | 同上用例①②③ | ✅（主 spec 未动，符合 tasks 7.5「delta 先行」） |
| 既有契约不冲突：REQ-WIGA-09（终止统一 Abort 语义、terminated 后零副作用）、REQ-WIGA-14（终止回执身份，位于未 archive 的 sibling change）、REQ-DRG-18（销账不物删 enrollment）、REQ-NDR-01/02/03（detail 水合） | 取消复用 WIGA-09 同源 API；取消块零 enrollment 写（WIGA-09 零副作用清单）；enrollment 未物删（用例①断言）；水合排除法天然含 aborted（NDR-02），NDR-01/03 机制未动 | 用例①（enrollment 留存）；`useCockpitNodeDetailHydration.test.ts:97-99` | ✅ |

---

## 7. 未决问题（需 owner / 设计角色裁决）

1. **「不得谎称」是否约束日志面**（P2-2）：REQ-WIGA-17 场景②「SHALL NOT 记录任何与之矛盾的『已停止』事实」按字面含 eprintln；当前用例只护响应体。裁决：(a) 补 stderr 捕获断言 / (b) 契约措辞收敛为用户可见面。
2. **「明确指出占用者」的粒度**（P2-3）：TimedOut 的 409 details 是否必须携带在途 run/session 身份（当前仅日志有）。
3. **前后端同批发布纪律**：`TimelineNodeStatus` 值域扩张是 WS＋REST 双契约面；旧前端拿到 `"aborted"` 落在 union 外（TS 运行时不炸、显示退化）。Task 9/发布流程需确认 web/dist 重建与 4317 二进制同批（plan Concern 3 已列，owner 验收时落实）。
4. **`cockpitFlowState` 把 aborted 归为 `done`（显示「已完成」）是否有意**（P2-4）：若有意（窄栏只区分成败），建议在 delta 或代码注释固化；若无意，按 P2-4 修法补分支。
5. **`set_run_completion_wait_timeout` 防误用**（P3-1）：接受 doc-only 约束还是后续加编译期/debug 期护栏。

---

## 8. 诚实边界

**代码级确证**（本人静态核对，行号均为当前工作树实测）：

- 本报告所有 文件:行号 证据；`git diff`/`--numstat`/`git show` 的增删行与文案逐字比对（§2、§5）；全仓 grep 的调用点穷举（§3-10）；四构造点默认值、`try_acquire_delete` 零写盘语义、compile 节点 `agent: None`、serde snake_case 与前端字面量一致、REQ-WIGA-16 delta 对「失败也须 stage 归一」的明文要求。
- 变异捕获力（§3-14）为**逻辑推演**：基于用例断言集合与控制流推演各变异的成败，未真实注入变异。

**推断 / 未执行**（门禁与实测由 controller 另行补入）：

- 一切 cargo（fmt/clippy/--all-targets/it_web/it_core large_file_guard）、`pnpm -C web test`、`tsc -b`、`openspec validate --strict` 均未运行（审查契约禁令）；「用例红→绿」过程与当前全绿状态取自提交信息（`b547aa8c`「失败测试」、后续 fix 提交），非本人执行结果。
- `part_14.rs` 现 1111 行、`part_25.rs` 738 行：按 1200 行门禁推断在限内，**未跑 large_file_guard 实证**。
- 「用例② 5s 上界」「用例① 250ms 尾隙」的时序结论是对断言语义＋tokio 调度合理假设的推断，未实测；probe 500ms 上界对生产释锁窗口的充分性同理（§3-11）。
- 真机验收（tasks §6 的 6.1–6.7）不在本审查范围，状态未知。

---

*审查报告完。P0/P1 为零；P2×4＋P3×1 建议回修（不阻塞归档流程，按 tasks §5「P0/P1 必须回修并重审」口径，本报告不触发重审门槛）。*
