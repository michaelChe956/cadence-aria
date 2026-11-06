# 实施报告 · it_web 聚合索引夹具改走假 codegraph CLI v1.0

- 日期：2026-11-06
- 工作树：`/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`
- 分支 / HEAD：`bugfix-b-11-02` / `274337e6`
- 范围：`tests/it_web/**` 的 4 处夹具前置（**`src/**` 零改动**）
- 相关历史：`cadence/notes/2026-11-04_it_web聚合索引重建超时_夹具修复_v1.0.md`

---

## 1. 一句话摘要

`cargo test --locked --test it_web` 的 **20 条红**不是产品缺陷，而是 **4 处 it_web 夹具前置打生产 rebuild HTTP 端点、真跑 `codegraph sync .`、被 30s 增量预算判超时 ⇒ 422 `codegraph_timeout`** 所致；本轮把这 4 处夹具改走既有假 CodeGraph CLI（`aggregate_index_fixture::seed_active_aggregate_index`），**`src/**` 零改动**，改后 it_web 全组 **602 passed / 0 failed / 17 ignored**（54.76s），与历史绿基线逐项一致。

---

## 2. 问题定性

### 2.1 是什么

`cargo test --locked --test it_web` 有 **20 条红**，失败签名完全固定：

```json
{"code":"aggregate_index_unavailable",
 "details":{"copy":{"machine_line":"codegraph_timeout","triage":"retry_only",
                    "title_zh":"codegraph 命令超时","action_zh":"等系统负载下降后点「重试」。"},
            "detail":"codegraph sync . timed out after 30s",
            "reason_code":"codegraph_timeout"}}
```

HTTP 状态码 **422**。关键字段：`machine_line = codegraph_timeout`、`triage = retry_only`、`detail = "codegraph sync . timed out after 30s"`。

⇒ **性质：夹具前置失败**，红在与被测行为无关的准备工作上；产品行为本身未被触达。

### 2.2 不是什么（逐条排除）

| 候选定性 | 结论 | 依据 |
|---|---|---|
| 产品代码 bug | **不是** | `src/**` 零问题；失败发生在真 CLI 子进程超时层，`src/web/handlers/aggregate_index.rs` 只是把下层错误**如实映射**成 422 |
| provider 侧问题 | **不是** | provider 在流程中更靠后；超时发生在聚合索引 rebuild 阶段，provider **根本没被调用到** |
| 本轮改动引入 | **不是** | 登记作废的三笔提交 `6f816b4d` / `d8b3fa06` / `e362077a` 均**未触碰** `tests/it_web/**`；聚合索引生产链最近改动 `8489e53a` / `0896dc10` **更早** |

---

## 3. 为什么此前被判成「环境性」而现在改判

### 3.1 此前判定的依据

2026-11-04 的历史轮次曾把同款签名归因为「机器有其它 cargo/codegraph 负载」⇒ 环境偶发。历史 full10（作废改动之前）全绿，full12（作废改动之后）出现 20 红，于是形成「环境性 / 负载偶发」的判断。

### 3.2 本轮改判的实测依据

本轮修复前，在**机器空闲、只跑 38 个用例**的条件下定向复跑：

```
cargo test --locked --test it_web web_lifecycle_api
→ 18 passed / 20 failed / 581 filtered out；finished in 605.87s
```

**605.87s ≈ 600s = 20 × 30s 纯干等** —— 没有任何机器负载参与，时间全部耗在 20 次 30s 超时上。

⇒ ★ **「负载偶发」不成立**：定向跑、机器空闲仍 20/38 红 ⇒ 该夹具路径在现环境下**必然** 30s 超时。这不是环境波动，是**路径固有属性**。

### 3.3 四轮实测对照表

| 轮次 | 命令 | 结果 | 耗时 |
|---|---|---|---|
| 历史 full10（作废改动之前，机器空闲） | `cargo test --locked --test it_web` | **602 passed / 0 failed** / 17 ignored | — |
| 历史 full12（作废改动之后，机器在跑 release 编译） | 同上 | 582 passed / **20 failed** / 17 ignored | — |
| 本轮修复前·定向（机器空闲，只跑 38 个用例） | `cargo test --locked --test it_web web_lifecycle_api` | 18 passed / **20 failed** / 581 filtered out | **605.87s**（600s = 20×30s 纯干等） |
| 本轮修复前·全组 | `cargo test --locked --test it_web` | 582 passed / **20 failed** / 17 ignored | 678.01s |

### 3.4 判断的修正过程（如实记录）

1. 初判（继承历史）：「环境负载偶发」——依据是 full10 绿 / full12 红，且 full12 恰逢 release 编译占用。
2. 反证（本轮定向复跑）：机器空闲、只跑 38 个用例，仍 20/38 红，且耗时 605.87s ≈ 20×30s ⇒ 负载不是必要条件。
3. 改判：**「夹具路径固有属性」**——4 处夹具打生产 rebuild 端点 ⇒ 真 `codegraph sync .` ⇒ 必吃 30s 预算 ⇒ 必 422。
4. 修正后的处置：不再等待环境改善，改为**让夹具不再真跑 codegraph**。

---

## 4. 根因链（逐层带 文件:行号）

```
it_web 夹具前置（4 处）
  └─ POST /api/projects/{p}/logical-codebases/{lc}/aggregate-indexes/rebuild   ← 生产 HTTP 端点
       └─ AggregateIndexOperation::rebuild
            └─ CodeGraphCli::sync  → 真实子进程 `codegraph sync .`
                 src/product/logical_codebase/aggregate_index/codegraph_cli.rs:154-158
                 └─ 预算：AggregateIndexBudget::INCREMENTAL.fail_secs = 30
                      src/product/logical_codebase/aggregate_index/types.rs:115-119
                      （target_secs: 3 / warn_secs: 10 / fail_secs: 30）
                 └─ 超时 ⇒ AggregateIndexError::Failed{ code: "codegraph_timeout" }
                      codegraph_cli.rs:252-256（run(argv, root, budget.fail_secs)）
            └─ 错误映射 ⇒ HTTP 422
                 src/web/handlers/aggregate_index.rs:273-296  aggregate_index_api_error
                 （handler 调用点：src/web/handlers/aggregate_index.rs:136 / :156）
```

补充事实：

- 30s 是**生产侧既有预算**，本轮**未改**（`src/**` 零改动）。
- 该链路上 provider 完全未被触达 ⇒ 排除 provider 定性。
- `src/product/logical_codebase/aggregate_index/snapshot.rs:21` 的 git 超时同样取 `AggregateIndexBudget::INCREMENTAL.fail_secs`，但本轮失败签名明确为 `codegraph sync . timed out after 30s`，落点在 CLI 同步而非 git 快照。

---

## 5. 改法

### 5.1 四处夹具 × commit × old→new

| commit（时间） | 文件 | 夹具函数（当前行号） | old | new |
|---|---|---|---|---|
| `56f9cfe7`（2026-11-06 22:46:43 +0800） | `tests/it_web/web_lifecycle_api/part_02.rs` | `migrate_repository_to_lc_and_rebuild`（`:535`） | `request_json(app.clone(), Method::POST, ".../aggregate-indexes/rebuild")` + `assert_eq!(status, StatusCode::OK)` | `crate::aggregate_index_fixture::seed_active_aggregate_index(&app_paths, "project_0001", &lc_id)`（`:547`）；形参 `app` ⇒ `_app`；doc 注释重写 |
| `274337e6`（2026-11-06 22:47:19 +0800） | `tests/it_web/web_lifecycle_api/part_06.rs` | `create_batch_confirm_fixture` 内 rebuild 块（`:156`，调用点 `:255`） | 同款 HTTP rebuild + 断言 | 同款 `seed_active_aggregate_index(&app_paths, "project_0001", &lc_id)`（`:255`） |
| `50a05d35`（2026-11-06 22:43:10 +0800） | `tests/it_web/web_work_item_generation/part_01.rs` | `bootstrap_project_repo_issue_and_specs`（`:340`，调用点 `:418`） | 同款 HTTP rebuild + `assert!(status.is_success(), ...)` | 同款 `seed_active_aggregate_index(&app_paths, "project_0001", &lc_id)`（`:418`） |
| `52af734b`（2026-11-06 22:43:13 +0800） | `tests/it_web/web_workspace_recovery_consistency/part_01.rs` | `app_with_gateway_launchable_design_history`（`:165`，调用点 `:283`） | 同款 HTTP rebuild + `assert!(status.is_success(), ...)` | 同款 `seed_active_aggregate_index(&app_paths, "project_0001", &lc_id)`（`:283`） |

**改动面统计**：`src/**` 零改动；`tests/it_web/aggregate_index_fixture.rs` 零改动（**既有 API 已足够，未新增任何 API**）；**新增测试用例 = 0，删除 = 0**；未改任何 `use` / `import`。

> 这 4 处正是 2026-11-04 笔记 §6 concerns #1 逐条点名的「仍走真 codegraph 的夹具」（当时记录的行号：`web_lifecycle_api/part_02.rs:541`、`part_06.rs:255`、`web_work_item_generation/part_01.rs:418`、`web_workspace_recovery_consistency/part_01.rs:282`）。本轮**补齐剩余 3 处**（第 1 处 2026-11-04 已改）。

### 5.2 假 CLI 只替换「CLI 应答层」

`tests/it_web/aggregate_index_fixture.rs` 的 `FakeCodeGraphCli`（`:116-158`）只替换子命令应答：

| CLI 命令 | 假 CLI 应答 | 位置 |
|---|---|---|
| `--version` | 生产兼容区间下界 `CODEGRAPH_MIN_VERSION`（`fake_version()`，`:108-112`） | `:132` |
| `init` | 在聚合根真实物化 `.codegraph` 库目录（保持 `codegraph_library_present` 判据与真实链一致） | `:133-139` |
| `sync` | 空成功应答 | `:141` |
| `files --json` | 按 LC 权威**合成**成员清单：每成员恒 1 个 `<成员根>/src/main.rs` | `:142-151` |
| `query --json` | `[]` | `:152` |
| 其他 | `Err(BoundedCommandError::Io { .. })`（未知命令立即暴露） | `:153-157` |

可执行名常量 `FAKE_CODEGRAPH_EXECUTABLE = "fake-codegraph"`（`:30`）⇒ 夹具误配到真 CLI 时会**立刻暴露**，不会静默跑真 codegraph。

### 5.3 仍然走生产链路的部分（未被替换）

经 `fake_index_operation`（`:161-180`）用**生产同构**依赖装配（`LogicalCodebaseStore` / `AggregateIndexStore` / `CodeGraphExcludeGenerator` / `AggregateIndexSnapshotCollector::for_paths`），以下全部为**真实行为**：

1. 真实 git 成员快照（`AggregateIndexSnapshotCollector`）；
2. 真实成员覆盖校验（`verify_member_coverage`）；
3. 真实排除清单写盘（`CodeGraphExcludeGenerator`）；
4. 真实 durable 世代落盘（`AggregateIndexStore`）；
5. 真实 active 投影；
6. 真实 `manifest.active_aggregate_index_id` 回写。

`seed_active_aggregate_index`（`:182-198`）仍调 `AggregateIndexOperation::for_lc(..).rebuild(project_id)` ⇒ 走的是**同一生产操作**，只是把 CLI 执行器换成假实现。

---

## 6. 门禁证据

### 6.1 修复后（controller 实测，2026-11-06）

```
cargo fmt --check                                                          FMT_EXIT=0
cargo clippy --all-targets --all-features --locked -- -D warnings        CLIPPY_EXIT=0   (Finished in 1m10s)
cargo test --locked --test it_web                                          602 passed; 0 failed; 17 ignored; 0 filtered out; finished in 54.76s
```

### 6.2 前后对比

| 指标 | 修复前 | 修复后 | 对照 |
|---|---|---|---|
| it_web passed | 582 | **602** | +20（正是 20 条红的用例转绿） |
| it_web failed | **20** | **0** | 归零 |
| it_web ignored | 17 | 17 | 不变 |
| it_web filtered out | 0（定向轮为 581） | 0 | — |
| 全组耗时 | 678.01s | **54.76s** | ↓ 约 92% |
| 定向轮耗时（`web_lifecycle_api`） | 605.87s | 不再适用 | — |
| `cargo fmt --check` | — | FMT_EXIT=0 | — |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | — | CLIPPY_EXIT=0 | — |

### 6.3 与历史绿基线的逐项一致性

修复后的 **602 passed / 0 failed / 17 ignored** 与历史 full10（作废改动之前、机器空闲）**逐项一致** ⇒ **无用例丢失、无用例被弱化、无用例被 skip 掉**（17 ignored 数量不变）。

---

## 7. 覆盖影响评估

### 7.1 端点成功路径覆盖**不丢**

`tests/it_web/web_aggregate_index_api.rs:31`
`rebuild_endpoint_publishes_active_generation_and_active_projection_reads_it`
用**同一假 CLI** 覆盖：真实路由 + 生产 handler + HTTP **200** + `state == "active"` + durable 世代可读（`active_required`）+ `codegraph_version` 取**假 CLI 的实测应答**（`:81`，证明走的是注入的 CLI 而非真 CLI）+ active 投影读取一致。断言行 `:68-97`。

> 与 2026-11-04 笔记 §3 的结论一致：端点覆盖与「夹具前置」解耦，覆盖不丢、不弱化。

### 7.2 端点 404 / 409 覆盖**不丢**

`tests/it_web/web_lc_operations_api.rs`：
- `:490` `lc_aggregate_index_new_path_active_rebuild_and_conflict` ⇒ 404 `logical_codebase_not_found`；
- `:530` `lc_aggregate_index_rebuild_new_path_conflicts_while_lc_registered` ⇒ 409 `aggregate_index_rebuild_in_progress`。

### 7.3 4 处夹具的下游**不依赖真实 codegraph 索引内容**

逐处核对断言取值来源：均取自 **manifest / checkout 的成员 inventory**，或**会话与规划面状态**（issue / spec / session / plan 状态机、timeline 节点、stage 投影）。⇒ 假 CLI 的 `files --json` 合成内容不参与被断言语义。

### 7.4 新出现的覆盖缺口（如实记录）

rebuild 端点的 **HTTP 失败映射路径**（**422** + `aggregate_index_unavailable` + `details.reason_code` / `details.copy`）在改道后于 **it_web 集成层无任何断言**；`src/web/handlers/aggregate_index.rs:273` 的 `aggregate_index_api_error` **无单元测试**。

判定：此前它只是「夹具前置失败」的副产品，**从未被断言过** ⇒ 这是**覆盖缺口**，**不是行为回归**（行为本身未改，`src/**` 零改动）。

---

## 8. 遗留与风险面（6 条，逐条带证据）

1. **失败防线弱化**
   假 CLI 的 `files --json` 按 LC 权威**合成**（每成员恒 1 个 `<成员根>/src/main.rs`）⇒「成员仓无源码 ⇒ `aggregate_index_member_coverage_failed`」与「负向排除命中 ⇒ `aggregate_index_exclusion_failed`」这两条真实防线在**夹具层**不再被触发（设施层无对应变体入口，本轮未新增 API）。

2. **读侧仍握真 CLI**
   应用状态里的 freshness service 仍用生产 `CodeGraphCli("codegraph")`：`src/product/logical_codebase/planning_context_resolver.rs:190-202`（`CodeGraphCli::new(...)` 于 `:193-196`）。若成员仓在播种后发生 **git 漂移**，读侧 `assess` 判 stale ⇒ 触发 `sync_and_verify` ⇒ **真 CLI** ⇒ 30s 超时 422。
   已逐处核对：**4 处夹具播种后无 git 写操作** ⇒ 当前安全，但这是**残留风险面**。

3. **`/tmp` 副作用外溢（仅 `web_lifecycle_api` 两处）**
   该模块的成员仓 = `tempdir()` 本身 ⇒ legacy 迁移算出的 `provider_context_root` = **父目录 = `/tmp`** ⇒ 假 CLI 的 `init` 会在 `/tmp/.codegraph` 建空目录、生产排除清单会写 `/tmp/codegraph.json`（真实链路今天也写同一文件 ⇒ **非新增副作用**，但会 `read_dir(/tmp)` 生成上万个排除项）。
   已核查：仓内**唯一**的真 CLI rebuild 在 `tests/it_core/workspace_ws_integration/part_05.rs:721`，其聚合根是**专属 tempdir** ⇒ 不受影响。
   可选加固：把该模块成员仓移入专属底座（照抄 `tests/it_web/web_coding_attempt_api/part_05.rs:283` 的 `member_repo_base()` 做法）。

4. **`CODEGRAPH_FIXTURE_LOCK` 半冗余**
   改道后 it_web 内不再有夹具经 rebuild 端点跑真 CLI；该守卫现只用于「读侧真 sync 时与其它模块互斥」。本轮**保留**（保守、无成本）。

5. **同类隐患未覆盖**
   `tests/it_core/workspace_ws_integration/part_05.rs:721` 仍真跑 codegraph（当前 it_core 全绿 ⇒ 本轮未动）。建议后续**同款改道**。

6. **阶段 2 清单更新建议**
   `cadence/notes/2026-11-06_交付说明_阶段1_爆栈根治与可用版_v1.0.md` §7 / §14 所列阶段 2 条目中，2026-11-04 笔记 §6 concerns #1「统一改走假 CLI」**本轮已完成 it_web 部分**（it_core 部分未做）；本轮应作为阶段 2 的**一个已完成项**记录。详见第 9 节。

---

## 9. 阶段 2 清单更新建议

依据：`cadence/notes/2026-11-06_交付说明_阶段1_爆栈根治与可用版_v1.0.md` §7（阶段 2 已立 change，本轮只记录不实施）与 §14（全仓大 future 盘点结论），叠加 2026-11-04 笔记 §6 concerns #1。

| 阶段 2 条目 | 本轮状态 | 依据 |
|---|---|---|
| 「其它夹具统一改走 `aggregate_index_fixture` 的假 CLI」（2026-11-04 §6 #1 建议）——**it_web 部分** | ★ **已完成** | 4 处夹具全部改道（`56f9cfe7` / `274337e6` / `50a05d35` / `52af734b`）；it_web 全组 602 passed / 0 failed / 54.76s |
| 同上 ——**it_core 部分** | **未做** | `tests/it_core/workspace_ws_integration/part_05.rs:721` 仍真跑 codegraph；it_core 当前全绿，本轮未动 |
| 「成员仓移出 `/tmp`、聚合根专属化」（2026-11-04 §2.3 配套） | **未做（可选加固）** | 仅 `web_lifecycle_api` 两处仍以 `/tmp` 为 `provider_context_root`；可选做法见 `web_coding_attempt_api/part_05.rs:283` 的 `member_repo_base()` |
| 「rebuild 端点失败映射（422）的覆盖补齐」 | **未做（新登记）** | `src/web/handlers/aggregate_index.rs:273` 无单元测试；it_web 集成层无 422 断言（见 §7.4） |
| 阶段 2 主体（provider-run 体积预算与门禁护栏、终止回执身份限定、全仓大 future 治理） | **本轮未触及** | 2026-11-06 交付说明 §7 / §14；本轮不属该 change 范围 |

⇒ 建议在阶段 2 清单中**新增一行**「it_web 聚合索引夹具统一改走假 CodeGraph CLI —— 已完成（2026-11-06，4 处夹具 / 4 commit）」，并把「统一改走假 CLI」条目**拆成 it_web（已完成）/ it_core（未做）两行**。

---

## 10. 未决问题（需 owner 或设计角色裁决）

| 编号 | 未决项 | 背景 | 本轮立场 |
|---|---|---|---|
| Q1 | 是否补 **rebuild 端点失败映射路径**的用例（422 + `aggregate_index_unavailable` + `details.reason_code` / `details.copy`），以及 `aggregate_index_api_error` 的单元测试？ | §7.4：该路径在 it_web 集成层与单元层**均无断言**；此前只是夹具失败的副产品 | 本轮**未做**（补了会新增用例，与「新增 = 0」的最小改动口径不符）；建议裁决为独立 change |
| Q2 | 是否做 **`/tmp` 加固**（`web_lifecycle_api` 两处成员仓移入专属底座）？ | §8-3：`provider_context_root` = `/tmp`，会写 `/tmp/codegraph.json` 并生成上万排除项 | 本轮**未做**（真实链路今天也写同一文件 ⇒ 非新增副作用）；建议裁决是否值得单独一轮 |
| Q3 | 是否对 **it_core 同款改道**（`tests/it_core/workspace_ws_integration/part_05.rs:721`）？ | §8-5：仓内最后一处真 CLI rebuild；it_core 当前全绿 | 本轮**未做**（超出 it_web 范围）；建议纳入阶段 2 |
| Q4 | 是否需要**补强假 CLI 的负向变体**（无源码成员 / 排除命中），以恢复 §8-1 的失败防线覆盖？ | 设施层无对应变体入口，本轮未新增 API | 本轮**未做**（需新增设施 API ⇒ 超出「零新增」范围）；建议裁决优先级 |
| Q5 | `CODEGRAPH_FIXTURE_LOCK` 半冗余后是否收敛/清理？ | §8-4：改道后仅守护读侧真 sync | 本轮**保留**（保守、无成本）；建议留待 Q3 落地后一并评估 |

---

## 11. 附录：引用到的文件 / 行号索引

| 文件 | 行号 | 事实 |
|---|---|---|
| `src/web/handlers/aggregate_index.rs` | `273-296` | `aggregate_index_api_error`：把 `AggregateIndexError` 映射为 422 `aggregate_index_unavailable`（含 `details.copy` / `details.detail` / `details.reason_code`） |
| `src/web/handlers/aggregate_index.rs` | `136`, `156` | handler 内两处 `aggregate_index_api_error` 调用点 |
| `src/product/logical_codebase/aggregate_index/types.rs` | `115-119` | `AggregateIndexBudget::INCREMENTAL`：`target_secs: 3` / `warn_secs: 10` / **`fail_secs: 30`** |
| `src/product/logical_codebase/aggregate_index/codegraph_cli.rs` | `154-158` | `sync` 子命令执行（失败码 `codegraph_sync_failed`），本轮超时的落点 |
| `src/product/logical_codebase/aggregate_index/codegraph_cli.rs` | `124-127` / `191-194` / `223-226` | `--version` / `files` / `query` 子命令执行 |
| `src/product/logical_codebase/aggregate_index/codegraph_cli.rs` | `252-256` | `run(argv, root, budget.fail_secs)` ⇒ 超时 ⇒ `AggregateIndexError::Failed` |
| `src/product/logical_codebase/aggregate_index/snapshot.rs` | `21` | git 超时同样取 `INCREMENTAL.fail_secs`（本轮失败签名不落此处） |
| `src/product/logical_codebase/planning_context_resolver.rs` | `190-202`（`CodeGraphCli::new` 于 `193-196`） | **读侧** freshness service 仍用生产 `CodeGraphCli("codegraph")` ⇒ 残留风险面 |
| `tests/it_web/aggregate_index_fixture.rs` | `1-12` | 模块头注释：假 CLI 只替换「CLI 应答」层；其余走生产链路 |
| `tests/it_web/aggregate_index_fixture.rs` | `30` | `FAKE_CODEGRAPH_EXECUTABLE = "fake-codegraph"`（误配真 CLI 立即暴露） |
| `tests/it_web/aggregate_index_fixture.rs` | `108-112` | `fake_version()`：取 `CODEGRAPH_MIN_VERSION` 下界 |
| `tests/it_web/aggregate_index_fixture.rs` | `116-158` | `FakeCodeGraphCli::run`：5 类子命令的假应答 |
| `tests/it_web/aggregate_index_fixture.rs` | `161-180` | `fake_index_operation`：生产同构依赖装配 |
| `tests/it_web/aggregate_index_fixture.rs` | `182-198` | `seed_active_aggregate_index`：调生产 `rebuild`，断言 `status == Active` |
| `tests/it_web/web_lifecycle_api/part_02.rs` | `535` / `547` | 夹具 `migrate_repository_to_lc_and_rebuild` / 改道后的 `seed_active_aggregate_index` 调用 |
| `tests/it_web/web_lifecycle_api/part_06.rs` | `156` / `255` | 夹具 `create_batch_confirm_fixture` / 改道后的调用点 |
| `tests/it_web/web_work_item_generation/part_01.rs` | `340` / `418` | 夹具 `bootstrap_project_repo_issue_and_specs` / 改道后的调用点 |
| `tests/it_web/web_workspace_recovery_consistency/part_01.rs` | `165` / `283` | 夹具 `app_with_gateway_launchable_design_history` / 改道后的调用点 |
| `tests/it_web/web_aggregate_index_api.rs` | `31` / `68-97` / `81` | 端点成功路径用例 / 断言块 / `codegraph_version` 取假 CLI 实测应答 |
| `tests/it_web/web_lc_operations_api.rs` | `490` / `530` | 端点 404 用例 / 端点 409 用例 |
| `tests/it_web/web_coding_attempt_api/part_05.rs` | `283` | `member_repo_base()`：`/tmp` 加固的可参考做法 |
| `tests/it_core/workspace_ws_integration/part_05.rs` | `721` | 仓内**唯一**残留的真 CLI rebuild（it_core，未改） |
| `cadence/notes/2026-11-04_it_web聚合索引重建超时_夹具修复_v1.0.md` | §2.2 / §2.3 / §3 / §6 #1 | 假 CLI 边界定义；底座隔离配套；端点覆盖不丢的论证；点名 4 处仍走真 codegraph 的夹具 |
| `cadence/notes/2026-11-06_交付说明_阶段1_爆栈根治与可用版_v1.0.md` | §7 / §14 | 阶段 2 已立 change 清单；全仓大 future 盘点结论 |
| commit `6f816b4d` / `d8b3fa06` / `e362077a` | — | 登记作废的三笔提交，**均未触碰** `tests/it_web/**` |
| commit `8489e53a` / `0896dc10` | — | 聚合索引生产链最近改动（早于本轮） |
| commit `56f9cfe7` / `274337e6` / `50a05d35` / `52af734b` | — | 本轮 4 处夹具改道（2026-11-06 22:43:10 ~ 22:47:19 +0800） |
| commit `274337e6` | — | 本轮结束时 HEAD |

---

本文为实施记录；未做最终裁决，未决项见第 10 节。
