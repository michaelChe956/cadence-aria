# 决策记录：Provider 运行边界与跨 run 隔离

日期：2026-10-27
状态：设计已批准，契约编写中（OpenSpec），实施未开始

## 一、触发事件

owner 重跑全流程时发现：story 的 author 会去取这些文件——

```
/tmp/work-item-plan-refined.md
/tmp/work-item-plan.md
/tmp/work-item-plan-draft.md
/tmp/plan/work-item-plan.md
/tmp/plan/work-item-plan.prev.md
/tmp/aria-restore/projects/project_0001/issues/issue_0001
/tmp/aria-deploy/session-1130.log
```

owner 初始猜测「删除 work item group 没删干净」。**该猜测已被证据推翻**：`.aria/.../issue-work-item-plans/` 下只剩本次一个文件，旧 group 产物已删干净。

## 二、事故取证（已证）

### 2.1 归因订正

写 `/tmp/work-item-plan*.md` 的是**前序 run**（codex rollout 15:30:59 / 15:58:17 / 16:06:21），**本次 run（`workspace_session_auto_510c9278…`，16:22:17 起）只读没写**（自身只写了 `/tmp/prior_assistant.txt`、`/tmp/current_assistant.txt`）。

### 2.2 本次 run 的四类越界读取

| 类 | 内容 |
|---|---|
| a | `/tmp` 通用名草稿：`find /tmp -iname '*work*item*plan*'`、`/tmp/plan/*` |
| b | **provider 自己的原生会话历史** `~/.codex/sessions/**/rollout-*.jsonl`（用 jq 抽前序 assistant 输出） |
| c | **旧状态全量备份** `/tmp/aria-restore/**`（含 `entity-deletions/plan/*.json`） |
| d | 服务日志 `/tmp/aria-deploy/session-*.log` 与前序 run 的 `/tmp/prior-plan*` |

### 2.3 三条根因

1. **事故 prompt 里没有任何边界文案**：`resolve_issue_baseline_tree` 对聚合/LC 会话恒返回 `None`（`workspace_engine/prompts/baseline_teaching.rs:50/53`）→ 既有「不得访问仓库工作区之外的路径」条款从未注入；SC markdown prompt（`work_item_split_engine/prompts.rs:326-372`）亦无「不要落盘」条款（draft prompt 有：`prompts.rs:1144`）。
2. **物理边界不存在**：codex `danger-full-access`+on-request（`codex_provider/session.rs:46-51`）；claude 只禁 `Edit,Write,NotebookEdit`（`claude_code_provider/mod.rs:49-56`），LC 模式 allowlist 不含 Bash（`projection.rs:38`）；pi 只排除 `edit,write`（`pi_provider/mod.rs:50-52`）；kimi 拒 fs 写但 terminal 可用（`kimi_code_provider/client_services/policy.rs:181-203`）。codex 的 T13 只在 `FileChange` item 上失败 turn（`session.rs:199/577/707`），与路径无关 → 前序 rollout 15:58 用 `apply_patch` 走 CommandExecution（CommandExecution 35 / FileChange 0）未被检出。
3. **宿主侧无读侧记录**：上述四类读取全程零告警，事后无法举证「这份 plan 是否被污染」。

### 2.4 风险排序（评审结论）

1. 被污染的计划**能静默通过现有校验**（`artifact_constraints.rs` 只查 heading/token/ID/结构）→ 下游 coding 照错计划执行。
2. scratch 若落进 worktree → 污染用户仓库；`validate_write_path` 只在 final_confirm 消费 → 「先污染、后 fail-closed」。
3. 多角色共享 scratch → reviewer 独立性丧失（比误删更难发现）。

## 三、owner 已确认决策（2026-10-27）

| 编号 | 决策 | 理由 |
|---|---|---|
| 总体 | 三件全做，**分三步独立提交**（每步可单独验收/回滚） | ① 提示词无条件注入「本轮运行边界」块 ② per-session scratch 目录 ③ 宿主侧检测告警 |
| D3 | **不换 codex sandbox** | 保持 `danger-full-access`+on-request；换 sandbox/CODEX_HOME/HOME 触及信任与认证（`provider_trust_adapters.rs:591`），且原生 FileChange 与路径无关会误杀 |
| D4 | **只告警、不阻断** | 阻断会误伤合法动作（provider 读自身历史=合法 resume；读 `.aria` 查证据），并加重「provider 不退出」类卡死 |
| D6 | **孤儿 scratch TTL = 6 小时** | 每次 run 注册时亦清空同名目录，TTL 只兜崩溃残留 |

### 残余风险（明示保留）

- 提示词是软约束，模型可忽略 → 第 ③ 层检测是唯一能把静默污染变成可观测事件的一层
- resume 场景：前序内容已在原生会话历史里，禁令无效
- codex 用原生 FileChange 写 scratch 仍被 T13 判失败（与路径无关）
- 三件都不做 ⇒ 跨 run 污染照旧

## 四、明确不做

不改 provider sandbox / tool policy token 与 digest；不改 gateway envelope 与 `writable_roots`；不改 `StreamingProviderInput` 结构；不删 `/tmp` 历史残留（用户机器文件，不属宿主所有权）；不动历史版本产物保留策略；不做定时清理常驻任务；不 bump `PRODUCT_DATA_SCHEMA_VERSION`。

## 五、本次附带清理（owner 授权）

| 路径 | 内容 | 处置 |
|---|---|---|
`/tmp/aria-restore/` | 10-26 14:04 的旧状态全量备份（213 文件 / 2.8M）；含 live 已无的会话 0019/0020、`story_spec_0015`、cross-repo-reviews | **已删**（唯一损失：这两个旧会话不可回溯；事故证据不受影响——记录在 live 的 `timeline_node_details/timeline_node_001.json`，16 处引用） |
`/tmp/plan/` | 10-20 的 `work-item-plan.md`、`work-item-plan.prev.md`、`validate.py` 临时脚本 | **已删** |
`/tmp/work-item-plan{,-draft,-refined}.md` | 当前 run 正在使用（15:36–16:13） | **保留**（未中断在跑的 run） |

## 六、后续产物与顺序

1. OpenSpec 契约：`openspec/changes/<name>/`（proposal/design/specs/tasks）— 达到强制阈值（行为变化 + 数据变化 + 验收面变化）
2. 技术方案：`cadence/designs/2026-10-27_技术方案_Provider运行边界与跨run隔离_v1.0.md`
3. 实施计划：`cadence/plans/2026-10-27_计划文档_Provider运行边界与跨run隔离_v1.0.md`
4. 实施：三次独立提交（边界块 / scratch / 检测告警），每步 TDD + 单独验收与回滚
5. 完成后由 owner 做真机测试

## 七、实现期必须遵守的硬约束

- `tests/it_core/large_file_guard.rs` 祖父条款：`workspace_engine/prompts.rs` 1665/1665、`workspace_engine/lifecycle.rs` 2167/2167、`lifecycle_store/plan.rs` 1683/1683、`lifecycle_store/workspace.rs` 1730/1730、`streaming_provider/mod.rs` 2062/2062 已到顶；`web/workspace_ws_handler/run/provider_run.rs` 1677/1678 仅剩 1 行 → 方案必须绕开这些文件
- `Cargo.toml` 无 `regex` 依赖；`tempfile` 仅 dev-dependencies → 扫描器手写切词，生产目录操作用 `std::fs`
- 清理必须带护栏：只删自建、路径漂移不删、不跟随符号链接、共享资源不删（参照 `provider_resources.rs` 的 `cleanup_owned_scope` 语义）
