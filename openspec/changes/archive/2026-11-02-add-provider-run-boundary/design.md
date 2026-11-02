# Design: add-provider-run-boundary

## 决策记录（owner 已批准，2026-10-27）

1. 三件全做，**分三步独立提交**（每步可单独验收 / 回滚）：① 提示词无条件注入“本轮运行边界”块 ② per-session scratch 目录 ③ 宿主侧检测告警。
2. **D3 不换 codex sandbox**：保持 `danger-full-access` + on-request；理由是触及信任与认证，且原生 FileChange 判失败与路径无关，换 sandbox 会误杀。
3. **D4 只告警、不阻断**：阻断会误伤 provider 读自身历史（合法 resume）与读状态目录查证据，并会加重“provider 不退出”类卡死。
4. **D6 孤儿 scratch TTL = 6 小时**。
5. 残余风险明示保留：提示词是软约束（模型可忽略），故检测层是必须兜底；resume 场景禁令无效；codex 原生 FileChange 写 scratch 仍被 T13 判失败。

## 架构

````
作者侧 prompt 漏斗（初次 / 续轮 / 修订）──无条件追加──▶ 本轮运行边界块（身份 + 白名单 + 禁读 + 落盘规则 [+ scratch 路径]）
自动续写重试（整体替换 prompt）───────同样追加──────▶ 同一边界块

manager 登记 run ──▶ prepare：清空重建 <state>/run-scratch/<session_id>/（0700，经 session 存活准入）
manager finish_run ──▶ 锁内判定“已无活动 run”→ 释放 engine 后条件删除
服务启动 bind 之后 ──▶ 一次孤儿 sweep（记录不存在 / 终态 / mtime > 6h）

provider tool-call 事件 ──▶ emit_execution_event 漏斗：原事件先持久化 ──▶ 纯函数分类 ──▶ 追加 Provider 告警事件（只记录）

provider 适配层 / gateway envelope / ToolPolicy / 指纹 / StreamingProviderInput：零改动
````

## Context

动机与范围见本 change 的 proposal；已确认的详细方案是 `cadence/designs/2026-10-27_技术方案_Provider运行边界与跨run隔离_v1.0.md`，本文只固化架构边界与权衡，不重新裁定 owner 决策。

接地基线为 `.worktrees/feat-b-0808-add-monorepo`（分支 `feat-b-0808-add-monorepo`）。已读并采信的事实（file:line 已逐一核对）：

- 事故 prompt 经 `build_work_item_plan_markdown_prompt`（`src/product/work_item_split_engine/prompts.rs:300`）→ `build_work_item_plan_streaming_input_with_session`（`src/product/workspace_engine/prompts.rs:494`）发出；三个作者漏斗（`build_streaming_input` `prompts.rs:373`、plan 漏斗 `prompts.rs:505`、修订 `src/product/workspace_engine/revision.rs:69`）都在合同尾之前调用 `append_author_baseline_teaching`（`src/product/workspace_engine/prompts/baseline_teaching.rs:77`）；而 `resolve_issue_baseline_tree` 对聚合规划会话（`baseline_teaching.rs:50`）与无 `repository_path` 的会话（`:53`）恒返回 `None`，教学块因此从未注入。产物自动续写重试整体替换 prompt（`src/product/workspace_engine/provider_drive/artifact_retry.rs:244`），不经该漏斗。
- 物理写边界不存在：codex `danger-full-access` + `on-request`（`src/cross_cutting/codex_provider/session.rs:46-51`）；claude 只禁 `Edit,Write,NotebookEdit`（`src/cross_cutting/claude_code_provider/mod.rs:49-56`）；pi 只排除 `edit,write`（`src/cross_cutting/pi_provider/mod.rs:50-53`）；kimi 拒 fs 写但 terminal 可用（`src/cross_cutting/kimi_code_provider/client_services/policy.rs:181-203`）。codex T13 只在原生 `FileChange` item 上失败 turn（`codex_provider/session.rs:199`、`:577`、`:707`），与路径无关。
- 作者侧 provider 的 tool-call 事件统一经 `emit_execution_event` 漏斗落入 node detail（`src/product/workspace_engine/provider_drive/work_item_plan.rs:464`；调用点 `src/product/workspace_engine/provider_drive.rs:632`、`:646` 与 `work_item_plan.rs:313-347`）；`ProviderExecutionEvent` 已有 `Provider` kind（形状先例 `src/product/workspace_engine/conversational_gate_parts/revision_turn.inc.rs:41`）；`upsert_execution_event_json` 按 `event_id` 去重（`src/product/workspace_engine/mappings.rs:90`）。
- 同一 workspace session 同时只有一个 `ActiveRun`，登记点只有 manager 的两个函数（`src/web/workspace_session/manager/runs.rs:122`、`:344`，函数入口 `:81`、`:308`），收口点只有 `finish_run`（`runs.rs:157`）；`spawn_provider_run_with_start_mode`（`src/web/workspace_ws_handler/run/provider_run.rs:153`）在 `:266-276` 把全部 `ProviderRunKind` 收敛到这两个登记点。
- 行数红线（`tests/it_core/large_file_guard.rs`，祖父条款 `MAX_PRODUCT_CODE_LINES = 1200`）：`workspace_engine/prompts.rs`（1665/1665）、`workspace_engine/lifecycle.rs`（2167/2167）、`lifecycle_store/plan.rs`（1683/1683）、`lifecycle_store/workspace.rs`（1730/1730）、`streaming_provider/mod.rs`（2062/2062）已到上限，`web/workspace_ws_handler/run/provider_run.rs` 仅余 1 行（1677/1678）；`Cargo.toml` 无 `regex`，`tempfile` 仅 dev-dependencies。

## Goals / Non-Goals

**目标：**交付 REQ-PRB-01–12：作者侧 prompt 无条件携带「本轮运行边界」块；宿主为每个 workspace session 托管一个 0700 的 scratch 并管理其创建、注入、清理与崩溃回收；对作者侧 tool-call 做四类越界检测并只告警；全程 provider 侧零改动。三件事分三步独立提交，可单独验收与回滚。

**非目标：**不换 codex sandbox、不阻断、不改 gateway envelope / `ProviderToolPolicy` / `SessionResumeFingerprint` / `StreamingProviderInput` / daemon canonical writeback；不覆盖 `TMPDIR`、`CODEX_HOME`、`HOME`；不处理 reviewer 路径；不做产物路径引用校验；不接线 `ProviderRunResourceScope`；不清理现场既有残留。

## Decisions

### 1. 约束层级：软约束 + 检测 + 生命周期

提示词是软约束，由检测层兜底，由生命周期降低残留面。不新增 `StreamingProviderInput` 字段（字面量构造波及面大）；scratch 不进 gateway envelope：规划只读 envelope 要求零 `writable_roots`（claude/pi/kimi 的 projection 校验会报错，`src/cross_cutting/claude_code_provider/projection.rs:279-283`、`src/cross_cutting/pi_provider/projection.rs:209-214`、`src/cross_cutting/kimi_code_provider/projection.rs:242-247`），且把每 session 不同的路径纳入指纹会破坏 resume 匹配（`src/product/logical_codebase/provider_gateway.rs:567-632`）。工具层限制（codex `workspace-write`、隔离 `CODEX_HOME`/`HOME`）触及信任与认证，列为 owner D3 不做。该软约束 + 接受残余风险的取舍与既有 `baseline_teaching_block` 同构（用户 2026-09-25 裁决）。

### 2. 边界块无条件注入，作为白名单而非「/tmp 禁令」

块挂在 `append_author_baseline_teaching` 的最前（基线解析之前、无条件），自动覆盖 story / design / work_item / work_item_plan 四种作者类型和修订面；自动续写重试因整体替换 prompt 不经漏斗，需单独追加。块写成「事实来源白名单 + 禁读清单 + 默认不落盘」，不把 `/tmp` 整体列为禁区：状态根即 `<workspace_root>/.aria`，部署中可位于 `/tmp` 之下，仓库本身也可位于 `/tmp` 之下。无持久 store 的内存态 engine 整块省略，保证既有 prompt 预算测试字节级不变。块体积受固定预算断言约束。不告知计划产物的真实状态路径（`src/product/lifecycle_store/paths.rs:95` 的 `issue-work-item-plans`，由 daemon 写入，告知等于邀请 provider 读写状态树）。

### 3. scratch 放状态根，按 workspace session 稳定键

路径 `<state>/run-scratch/<workspace_session_id>/`（`<state>` = `ProductAppPaths::root()`，`src/product/app_paths.rs:13`），与 `tool-policy-run-audit/`、`work-item-split-runs/` 并列（后两者见 `src/product/lifecycle_store/plan.rs:1374-1390` 的级联清理面）。备选对比：`/tmp/aria-run-<sid>/` 仍会被通用 `find /tmp` 发现、命名可预测、孤儿回收要另写；仓库内会污染用户工作区。选状态根的代价是 provider 仍可读该目录，故必须与检测配套。键用 session 而不是 run 序号：同一原生会话 resume 时，旧上下文里的路径与新 prompt 一致；内容短命由「注册时清空 + 收口时删除」保证。新分区不 bump `PRODUCT_DATA_SCHEMA_VERSION`。

### 4. 生命周期三段式

- **注册时清空重建**：在 `ActiveRun` 登记之后的两处登记点（supersede 与 claim-if-idle）调用 prepare，尽力而为，失败只 warn；prepare 经 session 存活准入（`src/product/lifecycle_store/tool_policy_run_audit.rs:238` 的 `with_workspace_session_live_write`），purging/deleted 的 session 不会复活目录。同 run 内 provider 重启不清空。
- **收口时条件删除**：`finish_run`（`runs.rs:157`）是所有结束路径（正常 `provider_run.rs:1656`、取消收口 `provider_run/cancel_settle.inc.rs:60`、abort 迟到收口 `runs.rs:176` 注释）的唯一收口；仅在锁内判定「已无活动 run」（判定点落在 `runs.rs:166-175` 的 take/restore 之后）时才删除，旧 token 的迟到收口在新 run 已登记时必须跳过。清理发生在释放 engine 之后。
- **启动 sweep**：bind 之后一次（`src/web/app.rs:768-773`，与 `reconcile_claimed_coding_runs_once` 同侧），回收崩溃遗留。规则：session 记录不存在、session 状态为终态（`Confirmed` / `Terminated` / `Failed`，`src/product/models/workspace.rs:30-40`）、或目录 mtime 超过孤儿 TTL（6 小时）。

不做定时清理（不增常驻任务）；不做 session 结束清理（session 可长期停在 `WaitingForHuman`）；不挂在 `mark_active_run_finished`（调用点过多）；不改 `lifecycle_store/plan.rs` 的级联删除（行数红线），已删 session 的遗留由 sweep 的「session 记录不存在」规则回收。

### 5. 删除护栏：复用现成 registry 的护栏语义，不接线 registry

`ProviderRunResourceScope::cleanup_owned_scope`（`src/cross_cutting/provider_resources.rs:1052`）的护栏语义（严格子路径 `:1122`、路径漂移停等 `:1096`、共享资源不清理 `:1160`）被原样要求于本能力的所有删除动作：

- 只删 `run-scratch/` 分区下的直接子目录，分区根自身永不删；
- 目录名先过 `validate_relative_id`（`src/product/json_store.rs:109`），非法名跳过不删；
- 删除前核对目标 canonical 的父目录仍是 canonical 分区根，漂移则不删并 warn；
- 先用 `symlink_metadata` 判定目标：非真实目录（含符号链接）不删不跟随；目录内部的符号链接只删链接本身，不进入指向物；
- NotFound 视为成功；任何删除失败只 warn，不影响 run；单项失败不影响 sweep 其余项。

不接线 registry 的理由：其生产入口 `ProviderRunResourceScope::open`（`provider_resources.rs:512`）仍是 `#[cfg_attr(not(test), allow(dead_code))]`（`:511`），`with_private_path`（`:189`）仅测试调用；只有 `open_for_recovery`（`:534`）有生产调用（`src/product/coding_workspace_engine/provider_stream.rs:177`、`:287`；`code_review.rs:504`、`:549`）。其键是每次 launch 一次的 `provider_run_id` 并带 quiesce/reconcile 语义，与本能力「session 稳定路径 + prompt 显式路径」的需求不同；接线属于另一条变更的体量。生产目录操作只用 `std::fs`，沿用 `lifecycle_store` 已有的 `remove_dir_all_if_exists` 语义（`src/product/lifecycle_store/utils.rs:170`，底层 `fs::remove_dir_all` 不跟随符号链接）。

### 6. 检测：纯函数分类 + 单漏斗追加事件，只告警

扫描器是纯函数（输入边界上下文与单个执行事件，输出 findings），不 panic、不返回 `Result`；手写切词，不引 `regex`。接线只在 `emit_execution_event` 漏斗：原事件先持久化，再对作者侧节点的 `Command` 类事件扫描，finding 以 `Provider` kind 事件发出（不再触发扫描，故无递归）。四个稳定码：`run_boundary_provider_history_read`、`run_boundary_aria_state_read`、`run_boundary_tmp_draft_touch`、`run_boundary_write_outside_scratch`。扫描范围限作者侧节点类型，reviewer 节点不扫——作者侧判定按 `TimelineNodeType` 枚举（`src/web/workspace_ws_types/timeline.rs:10`）定白名单，经 `active_node_type()`（`src/product/workspace_engine/lifecycle.rs:1449`，只调用不改该文件）取当前节点类型；注意 `emit_execution_event` 的第三个参数是 `Option<ProviderName>`（provider 名）而非 `AgentRole`，不能用它判角色。有界（只扫 `command` 与 `detail` 前 4 KiB，每 node 最多 8 条告警）。

**只告警（owner D4）。**阻断会误伤 provider 读自身历史（合法 resume）与读状态目录查证据，并会加重「provider 不退出」类卡死。告警只记录，不改变 turn 成败、审批、auto_if_valid 或产物确认。数据积累后再议升级。

### 7. 文件窗口

全部新增逻辑落在新文件或未登记文件：状态路径方法、`lifecycle_store` 的新模块、`manager/runs.rs`（701 行，无红线）、`web/app.rs`（1868 行，上限 2222 有余量）、`baseline_teaching.rs`（523 行，无红线）、`provider_drive/artifact_retry.rs`、新建 `workspace_engine/run_boundary.rs`（`workspace_engine/mod.rs` 226 行，无红线），以及 `provider_drive/work_item_plan.rs` 的漏斗胶水。禁止触碰已到顶的红线文件；`lifecycle_store/tests.rs` 无法再加 `include!`，新测试用内联 `#[cfg(test)] mod`。

### 8. 实施期必须注意的两个既有机制

- **会话读 spy**：`LifecycleStore::get_workspace_session`（`src/product/lifecycle_store/workspace.rs:582`）在 `#[cfg(test)]` 下记读并可 panic（`src/product/lifecycle_store/tests/workspace_session_read_spy.rs`）。sweep 判「session 记录是否存在」必须走 `find_workspace_session_path`（`workspace.rs:1230`，`pub(super)`，`lifecycle_store` 模块内可见）而非 `get_workspace_session`，否则在武装 read spy 的既有用例里会被计为意外读取而 panic。
- **状态终态集合**：`WorkspaceSessionStatus`（`src/product/models/workspace.rs:30-40`）含 `Open` / `Running` / `WaitingForHuman` / `ChangeRequested` / `BlockedProviderUnavailable` / `StoppedNeedsHuman` 等非终态，sweep 的终态判定只认 `Confirmed` / `Terminated` / `Failed` 三项（与 owner 裁定一致）。

## Risks / Trade-offs

- 模型忽略提示词 → 检测层是必须兜底；只告警不拦截，故污染仍可能进入产物，只是可被发现。
- resume 场景下已读入原生历史的内容无法撤回，本轮禁令无效 → 在残余风险中明示，不假装治理；检测可在其再次读取历史时告警。
- codex 原生 `FileChange` 即使写 scratch 仍被 T13 判失败 → 块中如实披露「原生文件变更工具请勿使用」，不教模型绕过，也不改 T13。
- scratch 在状态根内，provider 可读 → 与禁读清单 + `run_boundary_aria_state_read` 配套；不新增污染类别（状态本就存放于此）。
- 检测启发式有误报与漏报（如名称含 `design` 的 `/tmp` 一级仓库目录误报；纯目录扫描命令召回不足）→ 只告警故代价低；稳定码便于数据积累后调整。
- 孤儿 TTL（6 小时）须大于单 run 合理最长无写入时长，否则活 run 的 scratch 可能被另一实例的 sweep 提前回收 → 该前提待实现时对照 watchdog 零活动阈值核实。
- 多实例共享同一状态根时，sweep 的「session 记录不存在 / 终态 / 超 TTL」规则是会话级加年龄判定而非全删，避免误伤同伴实例的活 run。
- 提示词增长 → 固定字节预算断言；共享的 markdown author 预算断言不因本块而被改写。
- 产物 `exclusive_scopes` 或验证命令中泄入 scratch 路径 → 当前无校验器可拦，归入后续另议，不在本 change 假装覆盖。

## Migration Plan

新增分区、事件和 prompt 块不改既有数据格式；无需数据迁移，不 bump `PRODUCT_DATA_SCHEMA_VERSION`。按三步独立提交：第一步只改 prompt（无 scratch 条目）；第二步新增分区、生命周期接线和块中的 scratch 条目；第三步新增检测与告警。每步纯增量，回滚均为 revert 该步提交；第二步回滚后残留的空 `run-scratch/` 目录无害，可由 owner 手动清理，且 sweep 的缺席不会使任何功能失效。provider 侧、envelope、指纹零改动，故回滚不涉及 provider 兼容性。