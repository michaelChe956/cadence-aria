# Proposal: add-provider-run-boundary

## Why

2026-10-27 现场（`workspace_session_auto_510c9278…`，work_item_plan / codex / single_candidate / auto_if_valid，16:22:17 起）出现跨 run 污染：作者侧 provider 在推导计划时，读取了前序 run 遗留的通用名草稿、provider 自身的原生会话历史、旧状态备份与服务日志，并把前序 assistant 输出当作事实来源。owner 已批准「提示词边界 + per-session scratch + 宿主检测告警」三件全做、分三步独立提交；本 change 把该方案固化为可验收契约。

**归因订正（必须先读，避免错误归因）**：写 `/tmp/work-item-plan*.md` 的是**前序 run**（codex rollout 15:30:59 / 15:58:17 / 16:06:21），本次 run（`workspace_session_auto_510c9278…`，16:22:17 起）**只读没写**（本次只写了 `/tmp/prior_assistant.txt`、`/tmp/current_assistant.txt`）。本次 run 的四类越界读取是：① `/tmp` 通用名草稿（`find /tmp -iname '*work*item*plan*'`、`/tmp/plan/*`）② provider 自己的原生会话历史 `~/.codex/sessions/**/rollout-*.jsonl`（用 jq 抽前序 assistant 输出）③ 旧状态全量备份 `/tmp/aria-restore/**`（10-26 14:04，含 `entity-deletions/plan/*.json`）④ 服务日志 `/tmp/aria-deploy/session-*.log` 与前序 run 的 `/tmp/prior-plan*`。

当前缺口：

1. **prompt 无边界文案。** 事故 prompt 里没有任何「事实来源 / 禁读 / 不要落盘」条款：`resolve_issue_baseline_tree` 对聚合/LC 会话与无 `repository_path` 的会话恒返回 `None`（`src/product/workspace_engine/prompts/baseline_teaching.rs:50`、`:53`），既有「不得访问仓库工作区之外的路径」教学块因此从未注入；SC markdown prompt（`src/product/work_item_split_engine/prompts.rs:300` 发出的 `build_work_item_plan_markdown_prompt`，其 `[source_boundary]` 块 `prompts.rs:326-372`）也没有「不要落盘」条款（对照 Draft prompt 有：`prompts.rs:1144`「不得创建 cadence/plans/ 或任何 workspace 文件」）。模型没有被告知「最终响应流是唯一权威通道、磁盘文件不是事实来源」。
2. **物理边界不存在。** codex 以 `danger-full-access` + `on-request` 运行（`src/cross_cutting/codex_provider/session.rs:46-51`）；claude 只禁 `Edit,Write,NotebookEdit`（`src/cross_cutting/claude_code_provider/mod.rs:49-56`，LC 模式 allowlist `src/cross_cutting/claude_code_provider/projection.rs:38` 不含 Bash）；pi 只排除 `edit,write`（`src/cross_cutting/pi_provider/mod.rs:50-53`）；kimi 拒绝 fs 写但 terminal 可用（`src/cross_cutting/kimi_code_provider/client_services/policy.rs:181-203`）。没有任何 provider 能物理阻止作者侧通过 shell 读写任意路径。codex 的 T13 只在原生 `FileChange` item 上失败 turn（`codex_provider/session.rs:199`、`:577`、`:707`），与路径无关；前序 rollout 15:58 用 `apply_patch` 走 CommandExecution（item 统计：CommandExecution 35 / FileChange 0），故未被检测。
3. **宿主无读侧记录。** provider 的 tool-call 事件会落入 node detail，但没有任何分类：读 provider 历史、读 Aria 状态目录、触碰 `/tmp` 通用草稿名、在 scratch 之外写入，一律静默通过。宿主既不拦也不记。
4. **无宿主托管的暂存落点与回收。** 模型确需暂存时只能自选路径（`/tmp` 根、家目录等），这些文件无主、无生命周期，成为下一个 run 的「发现面」。
5. **既有行数红线限制了落点。** `workspace_engine/prompts.rs`（1665/1665）、`workspace_engine/lifecycle.rs`（2167/2167）、`lifecycle_store/plan.rs`（1683/1683）、`lifecycle_store/workspace.rs`（1730/1730）、`streaming_provider/mod.rs`（2062/2062）均已到祖父条款上限（`tests/it_core/large_file_guard.rs`），`web/workspace_ws_handler/run/provider_run.rs` 仅余 1 行（1677/1678）；方案必须落在新文件或未登记文件。

## What Changes

- **作者侧 prompt 无条件注入「本轮运行边界」块**：声明本轮身份与「产物唯一权威通道是本轮最终响应流」，给出事实来源白名单与禁读清单，规定默认不落盘。聚合/LC、无基线、无 `repository_path` 的会话同样注入；无持久 store 的内存态 engine 除外（与基线教学块的跳过面同构）。**不告知计划产物的真实状态路径**（`issue-work-item-plans/` 由 daemon 写入，告知等于邀请 provider 读写状态树）。
- **新增 per-session scratch 目录**：宿主在状态根 `<workspace_root>/.aria` 下为每个 workspace session 创建 0700 的专属暂存目录，位于仓库工作区与 cwd 之外，路径在边界块中显式告知；它是「确需暂存时的唯一合法落点」，不是新增权限。
- **新增 scratch 生命周期**：run 注册时清空并重建，同 run 内 provider 重启不清空；最后一个活动 run 收口时删除；进程崩溃遗留由下次 run 注册时的清空与服务启动 sweep 回收（孤儿 TTL 6 小时）。所有删除均带护栏：只删自建分区内的合法子目录，路径漂移不删，不跟随符号链接。
- **新增宿主侧越界检测告警**：对作者侧 provider 的 tool-call 事件做纯函数分类，命中四类越界（读 provider 历史目录 / 读 Aria 状态目录 / 触碰 `/tmp` 通用草稿名 / 在 scratch 之外写入）时，向 node detail 追加稳定码告警事件，原事件不受影响。
- **只告警不阻断**：检测失败或命中均不改变 run 的成败、审批与自动确认路径；阻断会误伤 provider 读自身历史（合法 resume）与读状态目录查证据，并加重「provider 不退出」类卡死。
- **provider 侧零改动**：不换 codex sandbox，不改 gateway envelope、`ProviderToolPolicy` token/digest、`SessionResumeFingerprint`、`StreamingProviderInput` 结构与 daemon canonical writeback。
- **分三步独立提交**：①边界块注入 ②per-session scratch ③检测告警；每步可单独验收、单独回滚。

### 非目标

不换 codex sandbox（D3）；不阻断（D4）；不做 `TMPDIR`/`CODEX_HOME`/`HOME` 覆盖或 bwrap 类隔离；不扫描 reviewer 节点、不向 reviewer prompt 注入 scratch；不做产物引用的编译期校验；不接线 `ProviderRunResourceScope`；不清理现场既有 `/tmp` 残留，不删除历史版本产物；不引入 `regex` 依赖（`Cargo.toml` 无 `regex`，`tempfile` 仅 dev-dependencies），生产目录操作用 `std::fs`；不 bump `PRODUCT_DATA_SCHEMA_VERSION`；不改动已到行数红线的文件。

## Capabilities

### New Capabilities

- `provider-run-boundary`：作者侧 provider 运行的软边界注入、per-session scratch 生命周期、越界读写检测告警，以及对 provider 侧零改动的约束。

### Modified Capabilities

无。当前主 specs 中没有同名能力；`project-rule-aware-prompts`、`provider-stream-log-placement`、`session-policy-envelope` 等现有 requirement 不被放宽或改写。

## Impact

- **受影响 spec**：仅新增 `provider-run-boundary`；不修改既有 spec，不删改既有 change。
- **代码面**：状态路径（新增与 `tool-policy-run-audit/`、`work-item-split-runs/` 并列的分区）、lifecycle store（新增 scratch 准备/清理/孤儿回收）、workspace session manager 的 run 登记与收口点、web 启动序列（bind 之后的一次 sweep）、workspace engine 的作者侧 prompt 漏斗与产物自动续写重试、provider 执行事件漏斗（追加告警事件）。精确文件与 file:line 见技术方案文档，由 writing-plans 展开。
- **下游零改动**：provider 适配层（codex/claude/pi/kimi）、gateway 与 envelope、`ProviderToolPolicy`、`SessionResumeFingerprint`、`StreamingProviderInput`、daemon canonical writeback、reviewer 路径、既有 prompt 预算与内存态 engine 测试。新分区不 bump `PRODUCT_DATA_SCHEMA_VERSION`（`src/product/product_data_schema.rs:6` 当前为 v2；`ensure_product_data_schema` 只在缺 `schema.json` 时扫遗留数据，`:16-38`）。
- **验收**：每步定向测试 + 控制器统一回归；边界块体积受固定预算断言；真机验收（codex author 在 `/tmp` 诱饵文件存在时重复生成，核对 prompt 含边界块、node detail 出现稳定码告警、run 结束后 scratch 被清理）需 owner 另行授权 provider 调用，不以 fake provider 冒充。
- **残余风险明示保留**：提示词是软约束，模型可忽略，故检测层是必须的兜底；resume 场景禁令无效（已读入原生历史的内容无法撤回）；codex 原生 FileChange 即使写 scratch 仍被 T13 判失败（与路径无关）。技术方案文档：`cadence/designs/2026-10-27_技术方案_Provider运行边界与跨run隔离_v1.0.md`。
