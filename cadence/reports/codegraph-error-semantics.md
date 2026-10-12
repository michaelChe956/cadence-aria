# codegraph 错误语义化：聚合索引空仓/未初始化报业务语义

**日期**：2026-10-12 · **worktree**：`.worktrees/feat-b-0808-add-monorepo` · **范围**：纯后端 + 前端既有失败卡文案（不新建卡型）

## 背景与根因（诊断冻结）

`aggregate_index_failed:codegraph_files_invalid_json: expected value at line 1 column 1` 的「JSON 文件」是 `codegraph files --json` 的 **stdout**。codegraph 1.6.x 对 0 文件库以 **exit 0** 输出纯文本 `ℹ No files indexed. Run "codegraph index" first.`（`--json` 契约被上游违反），产品侧 `run_checked` 只看退出码即放行，随后 serde 解析失败。现场触发条件：/tmp/notify-center 四个成员仓仅 `.git`+`README.md`（零可索引代码文件）。另一独立链路：durable Active 记录与磁盘库漂移（`.codegraph` 被清理）时 stale→sync 对未初始化库 exit 1 恒败。详见 `local://diag-codegraph-index.md`（只读诊断）。

## 修法（三项）

### ① 空索引语义化（codegraph_cli.rs + operation.rs）

- `CodeGraphCli::files()` 解析 JSON 前识别空索引形态（stdout 含 `No files indexed`，合法 files JSON 不可能含该文本）→ 新失败码 **`codegraph_files_empty_index`**；`codegraph_files_invalid_json` 仅保留给真正的意外输出。
- `AggregateIndexAcceptance::verify` 把 `codegraph_files_empty_index` 归并为 **`aggregate_index_member_coverage_failed`** 并点名**全部**成员（空索引＝每个成员都无可索引文件）。
- `verify_member_coverage` 从「首个未覆盖成员即 fail-fast」改为**收集全部未覆盖成员**再失败；消息统一为业务语义：
  `no indexable code file under included members [api, web]; each included member needs at least 1 indexable code file`。

### ② Sync 模式防漂移（codegraph_cli.rs::sync_or_reinit + operation.rs）

- `apply_index` 的 Sync 分支改用 `sync_or_reinit`：
  - **探测路径**：`<root>/.codegraph` 目录缺失 → 直接 init＋sync（不先跑必败的 sync）；
  - **捕获路径**：目录存在但 sync 失败且消息含 `not initialized` → 回退 init＋sync。
- 等价 Initialize/Rebuild 的建库序列，stale→sync 不再是死路。

### ③ 版本透传（codegraph_cli.rs + types.rs + operation.rs）

- `verify_version()` 返回实测版本字符串（stdout trim）；`apply_index` 在激活世代写入 `record.codegraph_version`。
- `AggregateIndexRecord::building` 的硬编码默认值 `"1.6.0"` 改为空串（＝尚未实测），building/failed 世代不再伪造版本号误导排障。

## UI 呈现（沿既有 bootstrap 失败卡通道）

- `bootstrap_projector.inc.rs`：Failed 记录的 warning 含 `aggregate_index_member_coverage_failed` 时投影专用 reason_code，其它 CLI 失败保持通用 `aggregate_index_failed`；失败步仍允许 Retry（G3 派发 init+sync 重建）。
- 前端 `workspace-cockpit-projection.ts`：
  - 标题映射新增 `aggregate_index_member_coverage_failed: "聚合索引失败:成员代码覆盖不足"`；
  - summary 前置业务指引：从 detail 契约 `members [a, b]` 提取成员清单 → `无可索引代码文件的成员：api, web · 每成员至少 1 个可索引代码文件`；提取不到时保留指引文案；通用失败不受影响。

## TDD 证据

**红（新语义断言对旧实现全部失败）**

- Rust 7 红：`files_on_empty_index_reports_dedicated_code_instead_of_json_gibberish`（现报 invalid_json）、`build_on_empty_index_reports_member_coverage_naming_all_members`、`build_names_all_uncovered_members_in_coverage_failure`、`build_requires_member_coverage_and_negative_exclusion`（版本 1.6.0≠实测 1.6.2）、`sync_reinitializes_when_index_library_drifted_away`（[sync]≠[init,sync]）、`sync_recovers_from_not_initialized_failure`（codegraph_sync_failed 恒败）、`aggregate_coverage_failure_projects_dedicated_reason_code`。
- 前端 2 红：新标题回落通用文案、指引缺失。

**绿**：`cargo test --lib aggregate_index` 62 passed（含真意外输出对照——`"not files json"` 仍报 `codegraph_files_invalid_json`）；投影器 1 passed；前端 71 passed。

**回归**：`cargo test --lib logical_codebase` 470 passed（aggregate_index 全族 + bootstrap 流 + freshness/store/web handlers）；前端全量 vitest 2112 passed；全量 lib 4412 passed、3 failed 均位于并行工作者在制文件 `src/web/workspace_ws_handler/tests/single_candidate_lc_admission.rs`（开工前已为 M 状态，与本改动无关）。rustfmt/clippy 对改动文件零新告警。

## 改动清单

| 文件 | 改动 |
|---|---|
| `src/product/logical_codebase/aggregate_index/codegraph_cli.rs` | ①空索引识别+专用码 ②`sync_or_reinit` ③`verify_version` 返回实测值 |
| `src/product/logical_codebase/aggregate_index/operation.rs` | ①归并+全名单覆盖 ②Sync 分支防漂移 ③激活透传版本 |
| `src/product/logical_codebase/aggregate_index/types.rs` | ③building 默认版本改空串 |
| `src/product/logical_codebase/bootstrap_projector.inc.rs` | 专用 reason_code 派生 |
| `web/src/state/workspace-cockpit-projection.ts` | 标题映射 + 成员清单提取 + 指引文案 |
| `Cargo.toml` / `Cargo.lock` | dev-dependency `parking_lot = "0.12"`（测试 fake 锁迁移，项目规则 rs-parking-lot） |
| 测试 | `codegraph_cli.rs`/`operation_tests.inc.rs`/`bootstrap_tests.inc.rs`/`workspace-cockpit-projection.test.ts` 新增/扩展用例 |

## 现场恢复指引（不变，见诊断 d 节）

裸 README 成员仓永远无法激活（coverage 要求每成员 ≥1 索引文件）：给四个成员仓各补至少一个真实代码文件并提交（推进 git revision），再触发 bootstrap **Retry**（＝init+sync 重建）；或 `rm -rf <root>/.codegraph` 后 Retry 全量重建。
