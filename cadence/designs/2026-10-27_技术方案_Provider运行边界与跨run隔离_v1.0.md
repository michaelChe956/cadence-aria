# 技术方案: Provider 运行边界与跨 run 隔离(v1.0)

- 出具: design-task-minimax, 2026-10-27; **设计完成未实施(DONE)**
- 接地基线: worktree `.worktrees/feat-b-0808-add-monorepo`（分支 `feat-b-0808-add-monorepo`）
- 本文档性质: owner 已批准方案的固化产物,只描述设计与决策,**不含实现代码**; 精确文件与命令由后续 `cadence/plans/` 实施 Plan 展开
- 配套 OpenSpec 契约: `openspec/changes/add-provider-run-boundary/`（capability `provider-run-boundary`）

## 0. 摘要与命名理由

2026-10-27 现场出现作者侧 provider 跨 run 污染: 模型把前序 run 遗留的通用名草稿、provider 自身原生会话历史、旧状态备份与服务日志当成事实来源。owner 已批准三件全做、分三步独立提交:

| 层 | 交付物 |
|---|---|
| ① 提示词边界 | 作者侧 prompt **无条件**注入「本轮运行边界」块(软约束) |
| ② 宿主托管暂存 | per-session scratch 目录 + 三段式生命周期(创建/清理/崩溃回收) |
| ③ 可发现性 | 作者侧 tool-call 四类越界稳定码告警(**只告警不阻断**) |

**命名理由**:

- change 名 `add-provider-run-boundary` —— 沿用本仓 `add-<能力>` 前缀约定(既有 `add-multi-repo-issue-entry`、`add-kimi-code-provider`、`add-pi-provider`、`add-provider-revalidate-probe`),`add-` 表达「新增一项此前不存在的能力」。
- capability 名 `provider-run-boundary` —— 描述的对象是 **provider 的一次运行(run)的边界**,不是 provider 适配器本身的能力面。既有 `*-provider-integration` / `*-provider` 系列命名已被适配器接入类 spec 占用(`pi-provider-integration`、`kimi-code-provider-integration`、`claude-code-structured-interaction`),若复用以 `-provider` 结尾会被读成「又一个 provider 接入」。本 capability 跨 provider 通用、以「运行边界」为语义核心,故取 `provider-run-boundary`。spec 能力名在本仓不含动词前缀(`session-policy-envelope`、`provider-stream-log-placement`、`per-issue-base-branch` 均为名词短语),故不加 `add-`。

---

## 1. 背景与事故取证

### 1.1 事故现场

- 时间窗: 2026-10-27 16:22:17 起
- 会话: `workspace_session_auto_510c9278…`
- 形态: work_item_plan / codex / single_candidate / auto_if_valid
- 现象: 作者侧 provider 在推导计划时读取了外部来源,并把前序 assistant 输出当作事实来源。

### 1.2 四条前提订正(归因边界,先读这一节)

这四条是本次取证的关键,写错会把责任归到错误的 run 上。

**订正一:写 `/tmp/work-item-plan*.md` 的是前序 run,不是本次 run。**

写盘者是 codex rollout `15:30:59` / `15:58:17` / `16:06:21` 三次;本次 run(`workspace_session_auto_510c9278…`,16:22:17 起)**只读没写**——本次自己只写了 `/tmp/prior_assistant.txt` 与 `/tmp/current_assistant.txt` 两个文件。因此事故的正确表述是「本次 run 读到了前序 run 的残留」,不是「本次 run 写坏了草稿」。

**订正二:本次 run 的四类越界读取。**

| # | 类别 | 具体对象 |
|---|---|---|
| ① | `/tmp` 通用名草稿 | `find /tmp -iname '*work*item*plan*'`、`/tmp/plan/*` |
| ② | provider 自己的原生会话历史 | `~/.codex/sessions/**/rollout-*.jsonl`(用 jq 抽前序 assistant 输出) |
| ③ | 旧状态全量备份 | `/tmp/aria-restore/**`(10-26 14:04,含 `entity-deletions/plan/*.json`) |
| ④ | 服务日志与前序 run 产物 | `/tmp/aria-deploy/session-*.log`、`/tmp/prior-plan*` |

**订正三:事故 prompt 里没有任何边界文案。**

- `resolve_issue_baseline_tree` 对聚合/LC 会话恒返回 `None`(`src/product/workspace_engine/prompts/baseline_teaching.rs:50`),对无 `repository_path` 的会话同样返回 `None`(`:53`)。既有「不得访问仓库工作区之外的路径」教学块因此**从未注入**到事故这类会话。
- SC markdown prompt(`build_work_item_plan_markdown_prompt`,`src/product/work_item_split_engine/prompts.rs:300`;其 `[source_boundary]` 块在 `prompts.rs:326-372`)没有「不要落盘」条款;而 Draft prompt 有(`prompts.rs:1144`「不得创建 cadence/plans/ 或任何 workspace 文件」)。同类约束在作者族内不一致。

**订正四:物理边界不存在,T13 也拦不住这类写入。**

| provider | 实际边界 | 证据 |
|---|---|---|
| codex | `sandbox=danger-full-access` + `approvalPolicy=on-request` | `src/cross_cutting/codex_provider/session.rs:46-51` |
| claude | 只禁 `Edit,Write,NotebookEdit` | `src/cross_cutting/claude_code_provider/mod.rs:49-56`;LC allowlist 不含 Bash(`projection.rs:38`) |
| pi | 只排除 `edit,write` | `src/cross_cutting/pi_provider/mod.rs:50-53` |
| kimi | 拒 fs 写,但 terminal 可用 | `src/cross_cutting/kimi_code_provider/client_services/policy.rs:181-203` |

codex 的 T13 只在**原生 `FileChange` item** 上失败 turn(`codex_provider/session.rs:199` 定义、`:577` / `:707` 调用),**与路径无关**。前序 rollout 15:58 用 `apply_patch` 走的是 `CommandExecution`(item 统计:CommandExecution 35 / FileChange 0),故 T13 完全没被触发。换言之:现有机制只覆盖「provider 原生文件变更工具」,不覆盖「provider 用 shell 写文件」。

### 1.3 三条根因

1. **prompt 无边界文案** —— 模型没有被告知「最终响应流是唯一权威通道」「磁盘文件不是事实来源」「默认不落盘」。
2. **物理边界不存在** —— 没有任何 provider 能物理阻止作者侧通过 shell 读写任意路径。
3. **宿主无读侧记录** —— tool-call 事件会落入 node detail,但没有任何分类,越界读取一律静默通过。

第 4 条派生缺口: **无宿主托管的暂存落点与回收** —— 模型确需暂存时只能自选路径(`/tmp` 根、家目录),文件无主、无生命周期,成为下一个 run 的「发现面」。

---

## 2. 目标行为 A1–A6

| 编号 | 目标行为 | 可验收判据 |
|---|---|---|
| **A1** | 每个 workspace session 的 author/plan/revision provider 启动时,prompt **无条件**包含「本轮运行边界」块 | 含聚合/LC、无基线、无 `repository_path` 的会话;无持久 store 的内存态 engine 除外(与 `baseline_teaching` 跳过面同构)。块含:① 本轮身份 + 「产物唯一权威通道是本轮最终响应流,磁盘文件(含自己的草稿、历史运行遗留)都不是权威来源」② 事实来源白名单 + 禁读清单(provider 自身历史目录、`.aria` 状态目录、临时目录通用草稿名、其他会话/历史运行的日志与产物)③ 落盘规则(默认不写直接输出;确需暂存只许写本 session 专属 scratch,结束即删;编辑类内建工具不要使用——如实披露 T13 契约,不教模型绕过)。**不告知 `issue-work-item-plans/` 真实路径**(daemon 写入,告知等于邀请 provider 读写状态树) |
| **A2** | scratch 位于 `<state>/run-scratch/<workspace_session_id>/` | 权限 0700,**不在仓库工作区或 cwd 内**;每次 run 注册时清空并重建 |
| **A3** | 一个 session 最后一个活动 run 收口后删除其 scratch | 崩溃遗留由下次 run 注册清空 + 启动 sweep 回收 |
| **A4** | 作者侧 provider 的 tool-call 触碰四类路径时,node detail 出现稳定码告警事件 | provider 历史目录 / Aria 状态目录 / `/tmp` 下通用草稿名 / scratch 之外的写入;**原 tool-call 事件不受影响** |
| **A5** | 既有契约零改动 | 不改 gateway envelope、`ProviderToolPolicy` 的 token 与 digest、`SessionResumeFingerprint`、`StreamingProviderInput` 结构、daemon 的 canonical writeback |
| **A6** | 既有测试保持绿色 | 既有 prompt 预算测试与 in-memory engine 测试不改断言即通过 |

---

## 3. 落点清单(file:line 已逐一核对)

| # | 位置(file:line) | 改动 | 约束 |
|---|---|---|---|
| 1 | `src/product/app_paths.rs:13`(`root()`)、`:21`(`projects_root()`) | 新增 `pub const RUN_SCRATCH_PARTITION = "run-scratch"`、`run_scratch_root()`、`run_scratch_dir(session_id)` | 纯路径层不校验 id(同层既有方法,如 `issue_work_item_plans_root` 亦不校验) |
| 2 | 新文件 `src/product/lifecycle_store/run_scratch.rs` + `lifecycle_store/mod.rs` 加 `pub mod` | `impl LifecycleStore` 上新增 `prepare_run_scratch(sid)` / `cleanup_run_scratch(sid)` / `sweep_orphan_run_scratch(ttl)` | `pub fn` 避免 clippy `-D warnings` 的 dead_code;`validate_relative_id`(`src/product/json_store.rs:109`)校验 id;prepare 走 `with_workspace_session_live_write`(`src/product/lifecycle_store/tool_policy_run_audit.rs:238`),保证 purging/deleted 的 session 不复活目录 |
| 3 | `src/web/workspace_session/manager/runs.rs`:`start_run_from_attachment`(`:81`,登记点 `:122`)与 `try_start_run_if_idle`(`:308`,登记点 `:344`) | ActiveRun **仅有的两个登记点**;`spawn_provider_run_with_start_mode`(`src/web/workspace_ws_handler/run/provider_run.rs:153`)在 `:266-276` 把所有 `ProviderRunKind` 收敛到这两个函数。登记并释放锁后调用 `prepare_run_scratch`(尽力而为,失败只 warn) | 落在 `runs.rs`(701 行无红线),避开 `provider_run.rs` 的 1 行余量(1677/1678) |
| 4 | 同文件 `finish_run`(`runs.rs:157`) | 在锁内算 `no_active_run = state.active_run.is_none()`(判定落在 `:166-175` 的 take/restore 之后),锁释放后若 `no_active_run` 则 `cleanup_run_scratch` 再 `maybe_recycle` | `finish_run` 是所有路径唯一收口(正常结束 `provider_run.rs:1656`、取消收口 `provider_run/cancel_settle.inc.rs:60`、abort 迟到收口 `runs.rs:176` 注释);调用在 `drop(engine)` 之后不持 engine 锁;`no_active_run` 防 supersede 竞态(旧 token 的 finish_run 见新 run 在位即跳过清理) |
| 5 | `src/web/app.rs:768-773`(`reconcile_claimed_coding_runs_once` 之后、bind 之后) | 加一行 sweep 调用,失败只打日志 | 启动副作用必须在 bind 之后(`app.rs` 注释 B2/F3:同端口 bind 失败=零副作用);`app.rs` 1868 行 / 上限 2222 有余量 |
| 6 | `src/product/workspace_engine/prompts/baseline_teaching.rs` 的 `append_author_baseline_teaching`(`:77`) | 新增 `append_author_run_boundary(&self, &mut String)`,在解析基线**之前无条件**调用。路径用 `store.run_scratch_dir(session_id)`(纯计算)+ `std::path::absolute`;目录不存在则省略 scratch 条目;无 store 则整块省略 | 三个作者漏斗都已调此函数(`prompts.rs:373` build_streaming_input、`prompts.rs:505` plan 漏斗、`revision.rs:69` 修订)→ 覆盖 story/design/work_item/work_item_plan 四种类型;该文件无行数红线(523 行) |
| 7 | `src/product/workspace_engine/provider_drive/artifact_retry.rs:244` `build_artifact_retry_input` | clone 后整体替换 prompt 的分支同样追加边界块 | 该处不经漏斗(自动续写重试) |
| 8 | 新文件 `src/product/workspace_engine/run_boundary.rs` + `workspace_engine/mod.rs` 加 `mod` | 纯函数 `scan_event(&RunBoundary, &ProviderExecutionEvent) -> Vec<BoundaryFinding>`,`RunBoundary{ scratch, state_root, history_dirs }` | 纯函数便于表驱动测试;`mod.rs` 226 行无红线 |
| 9 | `src/product/workspace_engine/provider_drive/work_item_plan.rs:464` `emit_execution_event` | 持久化原事件后,若事件是作者侧节点的 `Command` 类,调用 `scan_event`,finding 以 `kind=Provider` 的 `ProviderExecutionEvent` 走同一漏斗发出 | author/plan/reviewer 三类驱动共用的唯一漏斗(调用点 `provider_drive.rs:632`/`:646`、`work_item_plan.rs:315`/`:328`/`:342`、`review/drive_parts/reviewer_provider_session.inc.rs:451`);告警事件 `kind` 不是 `Command` 故不递归;仿 `conversational_gate_parts/revision_turn.inc.rs:41` 的 Provider 诊断事件;`upsert_execution_event_json`(`workspace_engine/mappings.rs:90`)按 `event_id` 去重 |
| 10 | **明确不改** | `provider_gateway.rs`(envelope / `writable_roots` / 指纹)、`streaming_provider/mod.rs`、`codex_provider/session.rs`(T13)、`lifecycle_store/plan.rs`(级联清理)、`provider_context_builder.rs` | 见 §5.1 |

### 3.1 实施期必须注意的三个既有机制(已核实)

- **会话读 spy。** `LifecycleStore::get_workspace_session`(`src/product/lifecycle_store/workspace.rs:582`)在 `#[cfg(test)]` 下记读并可 panic(`src/product/lifecycle_store/tests/workspace_session_read_spy.rs`)。sweep 判「session 记录是否存在」必须走 `find_workspace_session_path`(`workspace.rs:1230`,`pub(super)`,`lifecycle_store` 模块内可见),**不得**走 `get_workspace_session`,否则在武装 read spy 的既有用例里会被计为意外读取而 panic。
- **作者侧判定按节点类型,不按 role。** `emit_execution_event` 的第三个参数是 `Option<ProviderName>`(provider 名),**不是** `AgentRole`(`src/product/models/workspace.rs:965` 只有 `Author`/`Reviewer` 两个值)。作者侧白名单必须按 `TimelineNodeType` 枚举(`src/web/workspace_ws_types/timeline.rs:10`)定,取值经 `active_node_type()`(`src/product/workspace_engine/lifecycle.rs:1449`)——该文件已到行数红线(2167/2167),**只许调用,不许改**。
- **终态集合。** `WorkspaceSessionStatus`(`src/product/models/workspace.rs:30-40`)含 `Open`/`Running`/`WaitingForHuman`/`ChangeRequested`/`BlockedProviderUnavailable`/`StoppedNeedsHuman` 等非终态;sweep 的终态判定只认 `Confirmed`/`Terminated`/`Failed` 三项。

---

## 4. 数据形状

### 4.1 目录布局

```
<workspace_root>/.aria/                     ← ProductAppPaths::root()（状态根；.gitignore 已含 .aria/）
├── schema.json                             ← PRODUCT_DATA_SCHEMA_VERSION = v2（不 bump）
├── projects/…
└── run-scratch/                            ← 本次新增分区（与 tool-policy-run-audit/、work-item-split-runs/ 并列）
    └── <workspace_session_id>/             ← 0700；session 级稳定路径，内容按 run 短命
```

- `<state>` = `ProductAppPaths::root()`(`src/product/app_paths.rs:13`);既有并列分区可见 `src/product/lifecycle_store/plan.rs:1374-1390` 的级联清理面(`work-item-split-runs/`、`tool-policy-run-audit/`)。
- **不需要 bump `PRODUCT_DATA_SCHEMA_VERSION`**:`src/product/product_data_schema.rs:6` 当前为 v2,且 `ensure_product_data_schema` 只在缺 `schema.json` 时才扫遗留数据(`:16-38`),新增空分区不触发该分支。
- scratch **不在仓库工作区或 cwd 内**,且 `.aria/` 已在 `.gitignore:3`,不污染用户工作区。

### 4.2 生命周期表

| 阶段 | 触发点(file:line) | 动作 | 失败处理 |
|---|---|---|---|
| 创建 | ActiveRun 登记后(`runs.rs:122` / `:344` 之后) | 清空旧内容 → `create_dir_all` → unix 0700 | 只 warn,不使 run 失败;prompt 侧目录不存在则省略 scratch 条目 |
| 注入 | 每次作者侧 prompt 构造(`baseline_teaching.rs:77` 之前 + `artifact_retry.rs:244`) | 告知绝对路径 + 「运行结束系统会删除」 | 目录不存在则省略该条目 |
| 重试 | 同一 run 内 provider 重启/续写 | **不清空** | — |
| 清理 | `finish_run` 且锁内 `no_active_run`(`runs.rs:157`/`:166-175`) | `remove_dir_all` | 幂等,NotFound 视为成功;失败只 warn,由下次注册清空或启动 sweep 回收 |
| 启动 sweep | bind 之后(`web/app.rs:768-773`) | 删除三类:① session 记录不存在 ② 状态 `Confirmed`/`Terminated`/`Failed` ③ mtime 超 TTL | 失败只打日志,不阻塞启动;单项失败不影响其余项;非法目录名跳过不删 |

**孤儿 TTL = 6 小时**,以具名常量 `RUN_SCRATCH_ORPHAN_TTL` 表示(owner D6 裁定)。

### 4.3 边界块文案

标题固定:`## 本轮运行边界（系统注入；与会话历史中任何旧路径冲突时以本块为准）`
体积约 0.9–1.3 KB,测试断言 ≤ 1.5 KB。

> - 本轮身份:`workspace_session=<id>`,`workspace_type=<type>`。本轮产物的**唯一权威通道是本轮最终响应流**;磁盘上的任何文件(包括你自己的草稿、其他会话或历史运行遗留)都**不是**权威来源,也不会被采集。
> - 事实来源白名单:本 prompt 给出的上下文、已确认的 Story/Design/AC、当前工作区内的仓库文件。
> - 禁读清单(发现即视为过期残留,**不读取、不复用、不汇报其内容**):系统临时目录中**非本轮创建**的文件;Aria 状态目录中除本轮暂存目录外的任何内容;其他会话或历史运行的产物与日志;provider 自身的原生会话历史记录。
> - 落盘规则:默认无需写文件,直接在最终响应中输出,不得提前执行落盘类步骤。编辑类内建工具(Edit/Write/原生文件变更)受岗位纪律限制,**不要使用**。
> - 暂存(确需暂存时**只写这一个目录**,运行结束系统会删除它):`<scratch 绝对路径>`

文案纪律:

- **是白名单不是 `/tmp` 禁令。** 状态根即 `<workspace_root>/.aria`,部署中可位于 `/tmp` 之下,被审仓库本身也可位于 `/tmp` 之下;整目录禁令会自相矛盾。故禁读清单写「临时目录中**非本轮创建**的文件」。
- **不泄露计划产物真实路径。** `issue-work-item-plans`(`src/product/lifecycle_store/paths.rs:95`)由 daemon 写入,告知等于邀请 provider 读写状态树。
- **如实披露 T13 契约,不教绕过。** 原文只说「受岗位纪律限制,不要使用」,不说「用 shell 写就没事」。

### 4.4 告警事件与四个稳定码

事件形状(仿 `conversational_gate_parts/revision_turn.inc.rs:41` 的 Provider 诊断事件):

```
ProviderExecutionEvent {
  event_id: "run_boundary_{code}_{path_hash8}",
  kind:    Provider,          // 不是 Command → 不递归触发扫描
  status:  Completed,
  title:   "运行边界告警: {code}",
  detail:  "path=… access=read|write source_event=<event_id>",
  command: None, cwd: None, output: None,
}
```

| 稳定码 | 触发规则 | 备注 |
|---|---|---|
| `run_boundary_provider_history_read` | 路径等于/位于/是 provider 历史目录的**祖先** | codex `~/.codex/sessions` 已证(`src/cross_cutting/local_usage.rs:134`);claude `~/.claude/projects` 为推断,**待实现时真机核实**;pi/kimi 未核实 → 用**可配置表**,首步真机核实。skills 目录**不在**表内,避免误报 |
| `run_boundary_aria_state_read` | 位于 `ProductAppPaths::root()` 之下**且**不在本 session scratch 之下 | |
| `run_boundary_tmp_draft_touch` | 系统临时目录(`/tmp`、`/var/tmp`)下**第一级名称**不区分大小写命中 `*plan*` `*draft*` `*prior*` `*refined*` `*work-item*` `*spec*` `*design*` `*story*` `aria-*` | 按第一级名称判,**不依赖 cwd**,故仓库在 `/tmp` 下(如 `/tmp/test-center-java`)不误报 |
| `run_boundary_write_outside_scratch` | 写形态词元(`>`、`>>`、`tee`、`cp`、`mv`、`sed -i`、`apply_patch` 的 Add/Update File、`write_text`、`open(…,'w')`、`cat <<`)指向 scratch 之外的**绝对路径** | 作者角色按契约本不应写,故不依赖 cwd |

上界:只扫 `command` + `detail` 前 4 KiB;每 node 最多 8 条告警;扫描函数不 panic、不返回 `Result`。扫描范围限作者侧节点类型(reviewer 节点不扫)。

---

## 5. 关键取舍

### 5.1 三层约束的取舍

| 层 | 做法 | 结论 | 理由 |
|---|---|---|---|
| **提示词约束(软)** | 边界块无条件注入 | ✅ **采用,主手段** | 仓库已有同构先例(`baseline_teaching.rs:3-9`、`streaming_provider/mod.rs:901-909`);owner 2026-09-25 裁决已接受残余风险 |
| **结构化注入** | 给 `StreamingProviderInput` 加字段 / 塞进 gateway envelope | ❌ **不做** | 加字段波及 25+ 处字面量构造;塞 envelope 会触发校验——PlanningReadOnly envelope 必须零 `writable_roots`,claude/pi/kimi projection 校验会直接报错(`claude_code_provider/projection.rs:279-283`、`pi_provider/projection.rs:209-214`、`kimi_code_provider/projection.rs:242-247`);且路径纳入指纹会破坏 resume 匹配(`provider_gateway.rs:567-632`) |
| **工具层限制** | 换 codex sandbox / 隔离 `CODEX_HOME`/`HOME` / bwrap | ❌ **不做(owner D3)** | codex `danger-full-access` 是 owner 已裁决基线;换 sandbox 触及信任与认证(`src/product/logical_codebase/provider_trust_adapters.rs:589-591` 是 owner home / `CODEX_HOME` 发现的单一入口,禁止多处各自解析);且换 sandbox 会连带改 envelope/指纹,放大回归面 |

**结论:软约束 + 检测 + 生命周期**,并如实标注残余风险,不假装已治理。

### 5.2 scratch 位置

| 候选 | 结论 | 理由 |
|---|---|---|
| **`<state>/run-scratch/<sid>/`** | ✅ **采用** | 复用 session 分区/校验/sweep/准入模式;`.aria/` 已在 `.gitignore:3`;不在 cwd 内不污染用户工作区 |
| `/tmp/aria-run-<sid>/` | ❌ | 共享 `/tmp`、预测性命名、仍会被 `find /tmp -iname '*plan*'` 发现、孤儿回收要另写、测试环境与 `aria-restore`/`aria-deploy` 混杂 |
| 仓库内 | ❌ | 污染用户工作区 |

代价:provider 仍可读状态根(因此必须与 `run_boundary_aria_state_read` 配套)。键用 workspace session id 而非 run 序号:resume 时旧上下文路径与新 prompt 一致;内容短命靠「注册时清空 + 收口时删除」保证。

### 5.3 清理策略

采用「**run 注册时清空** + **收口时条件删除** + **启动 sweep**」三段式。

不做定时清理(不增常驻任务);不做 session 结束清理(session 可长期停在 `WaitingForHuman`);不挂 `mark_active_run_finished`(95+ 调用点);级联删除面(`lifecycle_store/plan.rs:1374-1390`)不动——已删 session 的遗留由 sweep 的「session 记录不存在」规则回收。

### 5.4 删除护栏:语义对齐现成 registry,但不接线

评审告诫「必须用现成 registry 护栏 / 不要自写 rm -rf」。本方案**采纳其护栏语义**:

| 护栏 | `ProviderRunResourceScope::cleanup_owned_scope` 现有实现 | 本方案要求 |
|---|---|---|
| 严格子路径 | 根自身/根外永不清理(`provider_resources.rs:1122`) | 同 |
| 路径漂移停等 | 登记路径 canonical 不符则停等(`:1096`) | 同 |
| 共享资源不清理 | 精确 ID 在但标签不匹配则永不清理(`:1160`) | 同 |
| 符号链接 | — | 只删链接本身,不进入指向物;目标本身是链接则不删不跟随 |

**但不接线 registry**,理由:

- 其生产入口 `ProviderRunResourceScope::open`(`:512`)仍是 `#[cfg_attr(not(test), allow(dead_code))]`(`:511`),`with_private_path`(`:189`)仅测试调用;只有 `open_for_recovery`(`:534`)有生产调用(`coding_workspace_engine/provider_stream.rs:177`、`:287`;`code_review.rs:504`、`:549`)。
- 其键是每次 launch 一个的 `provider_run_id`,并带 quiesce/reconcile 语义,与本能力「session 稳定路径 + prompt 显式路径」的需求不同。
- 接线属于另一条变更的体量,会把三步提交撑成四步。

本次按自实现落地、护栏对齐其语义;`remove_dir_all_if_exists`(`src/product/lifecycle_store/utils.rs:170`,底层 `fs::remove_dir_all`)不跟随符号链接,provider 放链接无法诱导删外部目录。**明确禁止:不提供对任意路径的递归删除入口;不写无护栏的 `rm -rf` 式清理。**

---

## 6. 已确认决策(owner,2026-10-27)

| 编号 | 决策 | 状态 |
|---|---|---|
| **D1** | 三件(提示词边界 / per-session scratch / 检测告警)全做 | 已批准 |
| **D2** | 分三步独立提交,每步可单独验收、单独回滚 | 已批准 |
| **D3** | **不换 codex sandbox**,不做 `CODEX_HOME`/`HOME`/`TMPDIR` 覆盖 | 已批准 |
| **D4** | **只告警不阻断** | 已批准 |
| **D5** | (本次 brief 未载明具体内容 → **待实现时核对 owner 裁决原文**) | 未知 |
| **D6** | **孤儿 scratch TTL = 6 小时** | 已批准 |

D4 的理由:阻断会误伤 provider 读自身历史(合法 resume)与读状态目录查证据,并加重「provider 不退出」类卡死。

---

## 7. 残余风险(明示保留,owner 已确认接受)

1. **提示词是软约束,模型可忽略。** 第 3 层检测是唯一能把静默污染变成可观测事件的一层;检测只告警不阻断,故污染仍可能进入产物,只是可被发现。
2. **resume 场景禁令无效。** 前序内容已在 provider 原生会话历史里,本轮禁令无法撤回已读入的内容;能做的只是在再次读取时告警。
3. **codex 原生 FileChange 写 scratch 仍被 T13 判失败**(与路径无关)。这是既有契约,本方案不为其开豁免。
4. **检测启发式有误报与漏报。** 如名称含 `design` 的 `/tmp` 一级仓库目录可能误报;纯目录扫描命令召回不足。只告警故代价低,稳定码便于数据积累后调整。
5. **孤儿 TTL 前提。** 6 小时须大于单 run 合理最长无写入时长,否则活 run 的 scratch 可能被另一实例的 sweep 提前回收 —— **待实现时对照 watchdog 零活动阈值核实**。
6. **多实例共享状态根。** sweep 的「session 记录不存在 / 终态 / 超 TTL」是会话级加年龄判定而非全删,避免误伤同伴实例的活 run。
7. **产物中泄入 scratch 路径**(`exclusive_scopes`、验证命令)当前无校验器可拦,归入后续另议,本方案不假装覆盖。
8. **若三件都不做,跨 run 污染照旧。**

---

## 8. 分步落地顺序

### 第 ① 步:提示词边界块(落点 6 + 7)

- **交付**:作者侧 prompt 无条件注入边界块(无 scratch 条目)。
- **验收**:边界块在聚合/LC 会话 prompt 中出现;单测断言块字节与 ≤ 1.5 KB 预算;四种作者类型 + 修订 + 自动续写重试路径均至多一次;既有基线教学块、既有 prompt 预算与内存态 engine 测试不变。
- **回滚**:删注入调用即恢复(纯增量,无数据形态变化)。

### 第 ② 步:per-session scratch(落点 1 + 2 + 3 + 4 + 5)

- **交付**:scratch 分区、路径方法、prepare/cleanup/sweep 三能力,以及边界块中的 scratch 条目。
- **验收**:注册后目录存在且 0700;收口后删除;崩溃残留被下次注册清空;启动 sweep 回收三类孤儿;supersede 竞态不误删;已删除 session 不复活;符号链接与路径漂移护栏生效。
- **回滚**:撤下 run 注册/收口/启动 sweep 调用,残留目录由 sweep 缺席无害(空目录可由 owner 手动清理),无功能依赖。

### 第 ③ 步:检测告警(落点 8 + 9)

- **交付**:纯函数扫描器 + 事件漏斗接线 + 四个稳定码。
- **验收**:四类越界各触发对应稳定码告警事件;原 tool-call 事件仍在;reviewer 节点不触发;扫描不 panic;合法访问与位于 `/tmp` 下的仓库不误报。
- **回滚**:移除 emit 漏斗里的 scan 调用。

### 贯穿三步的验证要求

必须包含一条「provider 试图读旧 run 残留时会被发现或被阻止」的验证。参考既有惯例:`src/product/logical_codebase/*_tests.inc.rs` 与 `tests/it_web/`;新文件的单测放各自模块内(`#[cfg(test)] mod`,因 `lifecycle_store/tests.rs` 无法再加 `include!`)。

真机验收(codex author 在 `/tmp` 诱饵文件存在时重复生成)需 owner 另行授权 provider 调用,不以 fake provider 冒充。

---

## 9. 明确不做的事

- 不改 provider sandbox / tool policy token 与 digest。
- 不改 gateway envelope 与 `writable_roots`;不改 `SessionResumeFingerprint`;不改 `StreamingProviderInput` 结构。
- 不删 `/tmp` 历史残留(用户机器文件,不属宿主所有权)。
- 不动历史版本产物保留策略;不删改既有 spec 或既有 change。
- 不做定时清理常驻任务;不 bump `PRODUCT_DATA_SCHEMA_VERSION`。
- 不提供任意路径递归删除入口;不写无护栏的 `rm -rf` 式清理。
- 不引入 `regex` 依赖(`Cargo.toml` 无 `regex`;`tempfile` 仅 dev-dependencies):扫描器用手写切词,生产目录操作用 `std::fs`。
- 不改动已到行数红线的文件(`workspace_engine/prompts.rs` 1665/1665、`workspace_engine/lifecycle.rs` 2167/2167、`lifecycle_store/plan.rs` 1683/1683、`lifecycle_store/workspace.rs` 1730/1730、`streaming_provider/mod.rs` 2062/2062,以及 `provider_run.rs` 仅余 1 行)。
- 不向 reviewer prompt 注入边界块,不扫 reviewer 节点。
- 不接线 `ProviderRunResourceScope`(护栏语义对齐即可,见 §5.4)。

---

## 10. 与评审的差异

> 说明:风险评审纪要的**逐字原文**不在本次设计输入集内(本 brief 只携带其结论摘要)。下表按 owner 已裁定的决策编号(D3/D4/D6)逐条列出分歧与采纳理由;实施前应对照原始评审纪要复核表述,差异的**决策内容**已由 owner 裁定,不受复核影响。

| 争点 | 评审倾向 | 本方案/owner 裁定 | 采纳理由 |
|---|---|---|---|
| 约束层级 | 倾向物理护栏(收紧 codex sandbox / 隔离 `HOME`、`CODEX_HOME`) | **D3:不换 sandbox**,改为软约束 + 检测 + 生命周期 | `danger-full-access` 是 owner 已裁决基线;换 sandbox 触及信任与认证(`provider_trust_adapters.rs:591`);且换 sandbox 会连带改 envelope/指纹,放大回归面。代价是残余风险由检测层兜底,并在本方案 §7 明示 |
| 命中后的动作 | 倾向命中即阻断(更强治理) | **D4:只告警不阻断** | 阻断会误伤 provider 读自身历史(合法 resume)与读状态目录查证据,并加重「provider 不退出」类卡死;数据积累后再议升级 |
| 清理实现 | 告诫「必须用现成 registry 护栏 / 不要自写 rm -rf」 | **采纳护栏语义,暂不接线 registry**(见 §5.4) | registry 生产入口仍 `dead_code`、键语义不同、接线体量超出三步边界;本次自实现但护栏逐条对齐,并列入后续另议 |
| 孤儿回收节奏 | 对回收时机/周期有更激进建议 | **D6:TTL = 6 小时**,三段式(注册清空 + 收口删除 + 启动 sweep),不做定时常驻任务 | 不增常驻任务面;6 小时兼顾崩溃遗留回收与「活 run 不被误回收」;前提(须大于单 run 最长无写入时长)待实现时核实 |
| 交付粒度 | — | **D2:三步独立提交**,每步单独验收/回滚 | 每步纯增量、可独立回滚;第二步回滚后残留空目录无害 |

评审的**失败模式清单与风险排序**已折进本方案:失败模式逐条对应 §7 的残余风险(软约束被忽略、resume 无效、T13 与路径无关、启发式误报漏报、TTL 前提、多实例共存),风险排序按「先做软约束与检测、后做生命周期硬化」体现为 ①②③ 的分步顺序。

---

## 11. 实现期待核实清单

以下项在本设计中标注为推断或前提,实施时必须以真机/源码复核,不得写成已验证事实:

1. **claude 的 provider 历史目录** `~/.claude/projects` —— 推断,待真机核实(pi/kimi 同)。核实前用**可配置表**承载,表为空即不报该码。
2. **各 provider 的 skills 目录** —— 必须确认不在历史目录表内,避免误报。
3. **孤儿 TTL 与 watchdog 零活动阈值的数值关系** —— 6 小时是否足够,待实现时对照既有 watchdog 阈值核实。
4. **review 纪要逐字表述** —— 见 §10 说明。
5. **D5 的具体内容** —— 本 brief 未载明,待核对 owner 裁决原文。

---

## 12. 附:本次核对的源码事实清单

以下断言均已在本 worktree(`feat-b-0808-add-monorepo`)逐条读取核对:

| 断言 | 证据 |
|---|---|
| 聚合/LC 会话恒无基线 | `workspace_engine/prompts/baseline_teaching.rs:50`、`:53` |
| 作者漏斗三处已调基线教学 | `workspace_engine/prompts.rs:373`、`:505`、`workspace_engine/revision.rs:69` |
| SC markdown prompt 无落盘条款 / Draft 有 | `work_item_split_engine/prompts.rs:326-372` / `:1144`;prompt 构造 `:300` |
| codex danger 基线 / T13 只拦 FileChange | `codex_provider/session.rs:46-51` / `:199`、`:577`、`:707` |
| claude 只禁三个写工具 / LC allowlist 无 Bash | `claude_code_provider/mod.rs:49-56` / `projection.rs:38` |
| pi 只排除 edit,write | `pi_provider/mod.rs:50-53` |
| kimi 拒 fs 写、terminal 可用 | `kimi_code_provider/client_services/policy.rs:181-203` |
| 只读 envelope 必须零 `writable_roots` | `claude_code_provider/projection.rs:279-283`、`pi_provider/projection.rs:209-214`、`kimi_code_provider/projection.rs:242-247` |
| resume 指纹计算范围 | `logical_codebase/provider_gateway.rs:567-632` |
| codex 会话历史目录 | `cross_cutting/local_usage.rs:134` |
| 行数红线与各文件上限 | `tests/it_core/large_file_guard.rs`;`workspace_engine/prompts.rs` 1665、`workspace_engine/lifecycle.rs` 2167、`lifecycle_store/plan.rs` 1683、`lifecycle_store/workspace.rs` 1730、`streaming_provider/mod.rs` 2062、`provider_run.rs` 1677/1678、`manager/runs.rs` 701、`web/app.rs` 1868、`baseline_teaching.rs` 523、`workspace_engine/mod.rs` 226 |
| ActiveRun 仅两个登记点 / 唯一收口 | `manager/runs.rs:81`/`:122`、`:308`/`:344`、`:157`/`:166-175`;登记收敛 `provider_run.rs:153`、`:266-276`;收口调用 `provider_run.rs:1656`、`provider_run/cancel_settle.inc.rs:60`、`runs.rs:176` 注释 |
| 启动副作用在 bind 之后 | `web/app.rs:768-773` 及 B2/F3 注释 |
| 执行事件漏斗 / Command 类事件形状 / 按 id 去重 | `provider_drive/work_item_plan.rs:464`;调用点 `provider_drive.rs:632`/`:646`、`work_item_plan.rs:315`/`:328`/`:342`、`review/drive_parts/reviewer_provider_session.inc.rs:451`;`mappings.rs:131-143`、`:90`;`streaming_provider/mod.rs:1112-1120` |
| 状态根 = `.aria`,分区并列 | `app_paths.rs:13`、`:21`;`lifecycle_store/plan.rs:1374-1390`;`.gitignore:3` |
| schema 版本不需 bump | `product_data_schema.rs:6`、`:16-38` |
| 计划产物真实路径(不可披露) | `lifecycle_store/paths.rs:95` |
| 存活准入与 id 校验先例 | `lifecycle_store/tool_policy_run_audit.rs:238`;`json_store.rs:109` |
| 删除不跟随符号链接 / 既有删除语义 | `lifecycle_store/utils.rs:170` |
| 会话读 spy 陷阱 | `lifecycle_store/workspace.rs:582`、`:1230`;`lifecycle_store/tests/workspace_session_read_spy.rs` |
| registry 护栏与接线状态 | `cross_cutting/provider_resources.rs:189`、`:498`、`:511-512`、`:534`、`:1052`、`:1096`、`:1122`、`:1160`;生产调用 `coding_workspace_engine/provider_stream.rs:177`、`:287`、`code_review.rs:504`、`:549` |
| 无 `regex` 依赖 | `Cargo.toml`(`[dependencies]` 无 `regex`;`tempfile` 仅在 `[dev-dependencies]`) |
