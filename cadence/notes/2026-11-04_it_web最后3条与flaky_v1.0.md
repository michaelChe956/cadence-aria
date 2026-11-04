# it_web 最后 2 条失败收口 + it_core「flaky」定位（v1.0）

- 日期：2026-11-04
- worktree：`.worktrees/bugfix-b-11-02`
- 范围：① `web_lc_bootstrap_repair::a03_*`（版本门误报）② `web_lc_bootstrap_repair::a04_*`（404 project_not_found）③ it_core「间歇红」定位（3 连跑）
- 第三条 `evidence_query::invalid_query_returns_422_and_uninitialized_index_returns_503` 为已登记环境态（codegraph 1.6.2 对未建索引目录恒 exit 0 ⇒ 503 不可达），不在本次范围，**未动**。

## 0. 结论速览

| # | 条目 | 结论 |
|---|------|------|
| ① | a03 版本门误报 | **夹具缺陷**（非生产缺陷）→ 改夹具，已转绿 |
| ② | a04 404 project_not_found | **夹具时序缺陷**（seed 先于 project 创建）→ 改夹具 + 1 处**旧形态断言迁移（已报 Main）**，已转绿 |
| ③ | it_core「flaky」 | **不是时序 flaky**：即 `large_file_guard::product_source_and_test_files_stay_under_line_limit` 的 **cap 漂移**（文件行数 vs 登记上限，随树内容确定性地红/绿）；本次我自己 +32 行把它顶红又压回绿，3/3 复现同一机制 |
| 终态 | cargo test --test it_web | **593 passed / 1 failed / 17 ignored**（唯一红 = 已登记环境态 evidence_query；基线 591/3 → +2 绿；瘦身后终态复跑 1066.97s 同数字） |
| 终态 | cargo test --test it_core | **197 passed / 1 failed**（唯一红 = 已裁定暂留红的 Pi 条） |
| 终态 | cargo clippy --all-targets -D warnings | exit 0（终态复跑确认） |

---

## ① a03：`codegraph_version_mismatch` 误报

### panic 原文（修复前）

```
tests/it_web/web_lc_bootstrap_repair.rs:115:5
poll member_index fails at the injected pre_check interruption did not converge:
notices: key=bootstrap:codegraph:codegraph_version_mismatch, reason_code=codegraph_version_mismatch,
step=member_index, summary=问题：codegraph 版本不在支持区间（>=1.6.0 <2.0.0）…
```

### 版本探测 × 区间比较核对（生产无缺陷）

- 实装版本：`codegraph --version` ⇒ stdout 恰为 `1.6.2\n`（`od -c` 逐字节核过），exit 0；即使 `env -i PATH=...` 净化环境同样输出 `1.6.2`（fnm shim 不依赖额外环境变量）。
- 区间比较（`src/product/logical_codebase/aggregate_index/codegraph_cli.rs:22-38,104-123`）：
  - `CODEGRAPH_MIN_VERSION=(1,6,0)`、`CODEGRAPH_MAX_VERSION_EXCLUSIVE=(2,0,0)`（49a3940 G2 放宽为 ^1.6 语义，已提交在树）；
  - `parse_codegraph_version` 取**最后一个空白分隔 token**，要求恰为三段数字，否则 `None`（fail-closed）；
  - 判定 `v >= MIN && v < MAX`：`(1,6,2)` 明确落在区间内。
  - ⇒ **真实 CLI 走生产探测必然通过**。生产探测与区间比较双双正确。

### 真正的根因：夹具 runner 的兜底分支

测试链路：`a03_app` → `with_available_provider_health(...)`（`tests/it_web/provider_capability_fixture.rs`）→ `state.with_provider_health(health, gate, runner)`（`src/web/state.rs:370` 把 `state.command_runner` 整体替换为 stub `AllAvailableRunner`）→ `.with_gateway_factory(...)` 重建生产依赖 → `AggregateInitializationDependencies::production`（`production_dependencies.inc.rs:963-967`）用**同一个 `state.command_runner` + executable `"codegraph"`** 构造 `CodeGraphEnvironmentProbe`。member_index 步的 `ensure_codegraph()` 于是打到 stub：

```rust
let version = match request.executable.as_str() {
    "codex" => "2.0",
    "pi"    => "0.83.0",
    "kimi"  => "0.34.0",
    _       => "1.0",     // ← codegraph 落进这里！
};
```

`"1.0"` 解析为 `(1,0,0) < (1,6,0)` ⇒ 生产版本门**正确地**拒绝为 `codegraph_version_mismatch`（`verify_version` 对不可解析/超区间一律 mismatch，fail-closed）。测试只把**索引操作**换成了 `FakeCodeGraphCli`（答 `1.6.0`），漏了**环境探针**也复用同一 runner。

### 改法（夹具层，未动生产）

`tests/it_web/provider_capability_fixture.rs` `AllAvailableRunner` 增加 `"codegraph"` 分支，应答取生产声明的区间下界（`CODEGRAPH_MIN_VERSION` 格式化，区间上调自动跟随，与 `FakeCodeGraphCli` 的 `1.6.0` 同源）。其余使用方核过不受影响：`web_lc_operations_api.rs` 注入自带 Noop coordinator（`environment_probe: None` 不探测）；`provider_gateway_envelope.rs` 不做 codegraph 探测。

### 顺带暴露的第二层夹具缺口（同一测试）

版本门过了以后，a03 在下一前置被挡：`skills_commands_missing`——`seed_local_skills_source` 原来只种 `demo` 一个技能，而 `ensure_skills_commands_linked`（`environment_probe.rs:124-146`）要求 shared/codex/claude 三层都能解析 `AGGREGATE_SKILL_COMMANDS`（pre-check / rule-config / mcp-configuration / project-rules-examples），链接同步只链接**源里存在**的技能 ⇒ 四条命令链接缺失。夹具补种这四条命令的 SKILL.md。

### 红→绿

```
前：FAILED 0.15s  panic :675 failure must name the interrupted turn:
      left "skills_commands_missing"  right "aggregate_pre_check_failed"
      （首轮修复前更早在 :115 codegraph_version_mismatch 不收敛）
后：test web_lc_bootstrap_repair::a03_... ok  （narrow 0.47s；全量 593/1 中亦绿）
```

---

## ② a04：404 `project_not_found`（期望 200）

### panic 原文（修复前）

```
tests/it_web/web_lc_bootstrap_repair.rs:951:5
{"code":"project_not_found","details":{},"message":"project not found"}  left: 404  right: 200
```

### project record 播种路径核对（根因链）

1. `seed_confirmed_claude(&paths, PROJECT_ID, None)` 在 **POST /api/projects 之前**执行（原 a04 第 847 行）；
2. `lc_id=None` ⇒ capability 写 `lc_scope_root` = `logical_codebase_root` = `.aria/projects/project_0001/logical-codebase/capabilities.json`（`app_paths.rs:76-78`：该子树**在 projects 目录下**）⇒ 目录 `projects/project_0001/` 被预先创建；
3. `POST /api/projects` → `ProjectStore::create` → `next_sequential_id_in_directory("project", projects_root)`（`id.rs:34-59`）**只按目录名计数、不要求 project.json**（其单测明文以裸目录计数）⇒ 新 project 分到 **`project_0002`**（响应体被测试忽略，断言只看 200）；
4. 测试后续路由全部硬编码 `project_0001`：GET identity-repair 不经 ProjectStore（读 journal/registry/repos.json）⇒ 200 假阳性通过；GET members → `resolve_lc_authority` → `ProjectStore::get("project_0001")` → `projects/project_0001/project.json` 不存在 ⇒ `NotFound{kind:"project"}` ⇒ 404 `project_not_found`。

a03 不踩坑的原因：其 seed 在 project/LC 创建**之后**（`register_lc_members` 末尾），a04 是唯一先 seed 后建 project 的离群时序。生产无此路径（project 创建是唯一入口，capability bootstrap 只在 LC 流程里发生于 project 存在后）⇒ 纯夹具时序缺陷。

### 改法（夹具层）

把 a04 的 `seed_confirmed_claude(..., None)` 移到 `POST /api/projects` 断言之后（`write repos.json` 之前），并留注释固化时序约束。

### 顺带暴露：1 条旧形态断言迁移（⚠️ 已报 Main）

404 修掉后测试推进到 ：1152 撞旧断言 `error["message"].contains("gateway start_streaming failed")`。当前契约（`src/web/handlers/dto/aggregate_initialization.rs:134-137` 明文）：

> 「主文案取自**同一**文案来源的 `title_zh`（不再是原始英文 `action`），三段成品放 `details.copy`，脱敏后的原始诊断放 `details.detail`；原始 action / stderr / cause / provider 仍按既有契约留在 details 里供折叠区展示。」

实测 durable JSON：`message` = `provider 的第一步（pre_check）没有完成`（copy 标题），机器事实在 `details.action`（`gateway start_streaming failed: ...`）与 `details.detail`/`details.stderr_summary`（`test-injected interruption during the pre_check provider turn`）。旧断言写在 message=原始 action 的年代，被前置 404 红条掩蔽至今。**同款先例**：`tests/it_web/web_repository_initialization/operation_http.rs:273` 已按 `error.details.action` 断言。

迁移（语义不变：注入中断可观察、可归因、非 admission 阻断）：负向双查（`member_language_rules_missing` / `provider admission preflight denied`）与正向双查（`gateway start_streaming failed` + 注入标记）改查 `details.action` / `details.detail`。**未放宽任何断言强度**（负向查面从 1 个字段扩为 2 个，正向从 1 个断言并为 1 个双条件断言）。

### 红→绿

```
前：FAILED 0.09s  panic :951 left 404 right 200 project_not_found
后：test web_lc_bootstrap_repair::a04_... ok  （narrow 0.40s；全量 593/1 中亦绿）
```

---

## ③ it_core「flaky」定位

### 现象复盘

- bg_1455 那次：196 passed / 2 failed；紧接着复跑：197 passed / 1 failed（唯一红 = 已裁定暂留红的 `workspace_ws_abort_with_pi_reaches_cancelled_state_and_stops_output`）；两次之间只改了 `large_file_guard.rs` 的 2 条登记 cap。

### 我的 3 连跑失败名单（flock 串行，同树同环境）

| run | 结果 | 失败名单 |
|---|---|---|
| 1 | 196 passed / 2 failed / 51.78s | `large_file_guard::product_source_and_test_files_stay_under_line_limit` + Pi 暂留红条 |
| 2 | 196 passed / 2 failed / 53.33s | 同上两条 |
| 3 | 196 passed / 2 failed / 53.10s | 同上两条 |

（当时 `web_lc_bootstrap_repair.rs` 被我改到 1228 行 > 1200 硬线，guard 明细：`产品源码与测试文件必须保持在 1200 行以内，当前超限：tests/it_web/web_lc_bootstrap_repair.rs: 1228 行`）

### 定位结论

- **「间歇红」= `large_file_guard::product_source_and_test_files_stay_under_line_limit`，间歇条件是「文件行数 vs 登记上限」的 cap 漂移，不是并发/时序/共享文件/锁顺序**。该用例对同一棵树是**完全确定性**的：文件超线（未登记豁免）即红，压回线内或登记即绿。
- bg_1455 的第二条红即此条：彼时 `review/drive.rs`（2559→2565）、`coding_ws_handler/runner.rs`（2817→2921）等文件涨过了登记值（Main 未提交的登记改动注释可证：2026-11-03 的类型收敛/归属必填清扫把多个文件顶过旧 cap），guard 确定性红；Main 补登记后复跑即绿——被误读为 flaky。
- 我的 3 连跑把该机制完整复演了一遍：+32 行 ⇒ 3/3 红；瘦回 1199 行 ⇒ guard 窄跑绿、全量 it_core 终态 197/1（见 §终态）。
- 排除时序 flaky 的旁证：3 × 53s 全量 it_core 中，除上述两条外 196 条无一意外红；结合 bg_1455→复跑的差值恰为 cap 登记这一单一变量，「存在另一条时序 flaky」无证据支撑。

### 处置

- guard 本体与登记表**未碰**（Main 统一管理）。
- 我侧处置：把 `web_lc_bootstrap_repair.rs` 从 1228 压回 **1199 行**（`seed_local_skills_source` 迁至 `provider_capability_fixture.rs` 共享 + 注释收敛 + 断言合并），guard 恢复绿。
- 建议（不代做）：凡改 `tests/it_web/**` 或 `src/**` 大文件后，把 `cargo test --test it_core large_file_guard` 纳入窄跑清单，cap 漂移在提交前即可见。

---

## ④ 各自的红→绿输出（命令与结果）

| 命令（均 `flock /tmp/aria-cargo.lock` 串行） | 前 | 后 |
|---|---|---|
| `cargo test --locked --test it_web a03_new_lc_reaches...` | FAILED（:115 版本门不收敛 → 修后 :675 skills_commands_missing） | `1 passed; 0 failed` ok |
| `cargo test --locked --test it_web a04_failed_identity...` | FAILED（:951 404 → 修后 :1152 旧断言形态） | `1 passed; 0 failed` ok |
| `cargo test --locked --test it_web`（全量） | 基线 591/3/17（1030.09s） | **593 passed / 1 failed / 17 ignored**，唯一红 = evidence_query 已登记环境态（修后首轮 1035.10s；瘦身后终态复跑 1066.97s，数字一致） |
| `cargo test --locked --test it_core`（终态） | 「flaky」期 196/2 | **197 passed / 1 failed（68.89s）**，唯一红 = Pi 暂留红条 |
| `cargo test --locked --test it_core large_file_guard` | FAILED（1228 行超限） | `2 passed; 0 failed` ok（1199 行） |
| `cargo clippy --locked --all-targets -- -D warnings` | — | exit 0 |
| `rustfmt --edition 2024 --check`（两被改测试文件） | — | 无 diff |
| `cargo check --locked --tests` | — | Finished，无错误 |

## ⑤ 被改文件与行数

| 文件 | 改动 | 说明 |
|---|---|---|
| `tests/it_web/provider_capability_fixture.rs` | +40 / −5（190→210 行） | `AllAvailableRunner` 增 `codegraph` 分支（答 `CODEGRAPH_MIN_VERSION` 下界）；新增共享 `seed_local_skills_source`（demo + 四条聚合命令） |
| `tests/it_web/web_lc_bootstrap_repair.rs` | 净 +3（1196→1199 行） | a04 seed 时序后移；技能 seed 改调共享助手；断言契约迁移（报 Main）；全程 ≤1200 硬线 |
| `cadence/notes/2026-11-04_it_web最后3条与flaky_v1.0.md` | 新增 | 本报告 |
| 生产代码 / spec / Plan / 技术方案 / tests-it_core | **零改动** | 未碰 `tests/it_core/large_file_guard.rs`；未删测试；未 git add/commit/stash/checkout |

## ⑥ concerns

1. **断言迁移待 Main 确认**：②中 `error.message` → `details.action/detail` 的迁移依据 DTO 明文契约与 operation_http.rs 先例，语义未放宽；如 Main 不认，回退方案是保留旧断言并让 a04 留红待契约裁决。
2. `next_sequential_id_in_directory` 以裸目录名计数（无 project.json 也计数）是 ② 的放大器。生产入口（POST /api/projects）天然安全，但「任何先写 `projects/project_NNNN/` 子树再建 project」的路径都会踩；若未来开放此类前置写入，建议 id 扫描要求 `project.json` 存在（**生产改动，需授权，本次未动**）。
3. `large_file_guard` 的 cap 漂移机制仍会在任何大文件增长时再现（确定性红、易被误读为 flaky）；登记权在 Main。
4. it_web 全量 ~1035s 的 crate 级 `CODEGRAPH_FIXTURE_LOCK` 串行化代价不变（环境态，非本次引入）。
5. 中间态一次全量 it_web（593/1）跑在「瘦身重构」之前；瘦身后 a03/a04/guard 窄跑均绿，终态双套件全量已在同链复跑（见 ④），数字一致。
