# 实施报告：CI 双 profile 常规门禁（release profile job）（v1.0）

| 项 | 内容 |
|---|---|
| 日期 | 2026-11-06 |
| 范围 | 在常规 CI 里补 **release profile** 的 A/B/C 三层门禁 job（双 profile 常规化），并让 release 档的逐对象体积数字**留在 CI 输出 / job summary / artifact** 里可追溯 |
| 改动文件 | `.github/workflows/ci.yml`（**新增一个 job**）、`openspec/changes/provider-run-volume-budget-and-gate-completion/tasks.md`（2.2 残留指向 + 8.7）、本报告 |
| **未改** | 任何 Rust 源码、`.cargo/config.toml`（`RUST_MIN_STACK = 32 MiB force` 一字未动）、前端代码、`rust` / `stack-2mib-gate` 两个既有 job 的任何步骤 |
| **未跑** | ★**本机 release 档逐对象实测未完成**（见 §4）⇒ 本报告**不含** release 实测数字★ |

---

## 0. 摘要（TL;DR）

1. **新 job 名**：`release-profile-gates`（显示名 `Release profile gates (A volume / B 2 MiB worker / C real WS)`），`runs-on: ubuntu-latest`，`timeout-minutes: 90`。
2. **为什么必须有**：release 与 debug 的 codegen 完全不同（`opt-level="s"` + `lto=true` + `codegen-units=1`）⇒ 同一份源码两个 profile 的 poll 帧预留**不是同一个数**。盘点实测（`cadence/reports/2026-11-06_盘点_全仓大future清单_v1.0.md`）：debug 口径 >16,384 B 的有 **79** 行，release 实测 >16,384 B 的只有 **5** 行；且 `spawn_engine_event_forward_task` 在 **debug 仅 4,096 B、release 73,728 B** ⇒ **release-only 越界** ⇒ **只跑 debug 的门禁看不到它** ⇒ 双 profile 是必需的，不是可选的。
3. **job 内跑了什么**：A 层体积（`poll_frame_budget`，带 `--nocapture` ⇒ 逐对象实测字节被打印出来）、B 层显式 2 MiB worker（矩阵 + 三守卫/新旧 run 隔离两条）、C 层真实 WS 送审轮 + 场景矩阵；外加 `cargo build --release` 先把 release 产物编出来。
4. **release 数字的留痕位置（三重）**：① CI job 日志（`--nocapture` 打印）② `$GITHUB_STEP_SUMMARY`（job summary，A 层步骤**自己**写，写在「cargo 跑完之后、退出码回传之前」⇒ **红的时候也一定写下来**）③ artifact `release-poll-frame-budget-evidence`。
5. **门禁不会「永远绿」**：所有门禁步骤**无任何条件豁免**（无 `continue-on-error` / 无 `|| true` / 无 `--ignored` / 无 skip / 无 allow）；A 层步骤的**退出码就是 cargo 的退出码**；唯一的 `if: always()` 只挂在**纯证据上传**步骤上（上传失败也会让 job 红），不参与门禁判定。
6. ★**本机 release 实测未完成**★：release 编译占 `/tmp/aria-cargo.lock`，与 owner 的**部署构建**（关键路径，B3 真机测试在等新二进制）排队 ⇒ 为让位部署，本机 release 编译**被 controller 终止** ⇒ **本报告不编造 release 数字**（§4）。release 数字改由本 job 在 CI 上首次运行时取证。

---

## 1. 落地内容

### 1.1 新增 job（`.github/workflows/ci.yml`）

```yaml
  # release profile 门禁（A 体积 / B 小栈 / C 真实 WS）。
  #
  # 为什么必须有：release 与 debug 的 codegen 完全不同（release 是
  # `opt-level="s"` + `lto=true` + `codegen-units=1`）⇒ 同一份源码在两个
  # profile 下的 poll 帧预留**不是同一个数**，debug 的数字不能冒充 release。
  # 盘点实测（`cadence/reports/2026-11-06_盘点_全仓大future清单_v1.0.md`）：
  # debug 口径 >16,384 B 的有 79 行，release 实测 >16,384 B 的只有 5 行；
  # 且 `spawn_engine_event_forward_task` 在 debug 仅 4,096 B、release 73,728 B
  # ⇒ **只跑 debug 的门禁看不到 release-only 的越界** ⇒ 双 profile 是必需的。
  #
  # 边界（owner 2026-11-06 裁决）：
  # - **不改** `.cargo/config.toml` 的 `RUST_MIN_STACK = { value = "33554432", force = true }`；
  # - **不改**现有两个 job（`rust` / `stack-2mib-gate`）的任何步骤；
  # - 无 `-j 1` / `--ignored` / skip / allow / `continue-on-error` / 无条件 true
  #   ⇒ 门禁红了就是红了（release-only 越界必须能把 CI 打红）。
  release-profile-gates:
    name: Release profile gates (A volume / B 2 MiB worker / C real WS)
    runs-on: ubuntu-latest
    # release 构建慢（LTO + codegen-units=1，每个测试二进制链接都是重活），
    # 且 C 层真实 WS 场景自带 240~300s 墙钟上限 ⇒ 整 job 留足余量（≥60 min）。
    timeout-minutes: 90
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@master
        with:
          toolchain: stable
      - name: Cache cargo artifacts
        uses: Swatinem/rust-cache@v2
      - name: Setup Node
        uses: actions/setup-node@v4
        with:
          node-version: "20"
      - name: Setup pnpm
        uses: pnpm/action-setup@v4
        with:
          version: 10
      # 前端必须先构建：aria 通过 rust-embed 在编译期嵌入 web/dist，
      # build.rs 在 web/dist 缺失时硬失败（web/dist 不在版本控制中）。
      - name: Build frontend
        run: pnpm -C web install --frozen-lockfile && pnpm -C web build
      # C 层真实 WS 夹具走生产 aggregate-index rebuild 端点，该端点 shell out
      # 到真实 `codegraph` CLI（executable="codegraph"）⇒ runner 上没有该工具
      # 时夹具直接失败。钉版本安装（与本地、另两个 job 一致）。
      - name: Install codegraph (C-layer real WS fixtures need the real CLI)
        run: npm install -g @colbymchenry/codegraph@1.6.2
      # release 产物先编出来（release profile 的体积数字必须来自 release 二进制）。
      - name: "Build release artifacts"
        run: flock /tmp/aria-cargo.lock cargo build --release --locked
      # A 层（体积）：poll 帧预留逐对象实测（22 个受门禁对象）。
      # ★`--nocapture` 是硬要求：门禁逐对象用 println! 打印实测字节，不加
      #   `--nocapture` 会被 libtest 吞掉 ⇒ release 的体积数字无处可查。
      # ★本步同时把逐对象实测字节写进 job summary：**门禁红的时候恰恰最需要
      #   留下数字做归因** ⇒ 总结写在「cargo 跑完之后、退出码回传之前」，
      #   无论 cargo 红绿都会写；**本步退出码 = cargo 的退出码**，
      #   没有任何条件豁免 ⇒ 门禁红了就是红了。
      - name: "A layer: release poll frame budget (per-object measured bytes)"
        shell: bash
        run: |
          # 显式关掉 errexit（GitHub 的 `shell: bash` 默认带 -e）：本步要自己
          # 回传 cargo 的退出码，不能在 cargo 失败时提前退出而丢掉总结。
          set +e
          set -uo pipefail
          dir=/tmp/aria-release-evidence
          mkdir -p "$dir"
          log="$dir/poll-frame-budget.log"
          tc="$dir/toolchain.txt"
          : > "$log"
          rustc -vV > "$tc"
          flock /tmp/aria-cargo.lock cargo test --release --locked --lib poll_frame_budget -- --nocapture 2>&1 | tee "$log"
          cargo_status=${PIPESTATUS[0]}
          {
            echo '## Release profile —— poll 帧预留逐对象实测字节'
            echo
            echo '- profile: `release`'
            echo "- target: \`$(grep '^host:' "$tc" 2>/dev/null | cut -d' ' -f2-)\`"
            echo "- rustc: \`$(grep '^release:' "$tc" 2>/dev/null | cut -d' ' -f2-)\`"
            echo '- 命令: `flock /tmp/aria-cargo.lock cargo test --release --locked --lib poll_frame_budget -- --nocapture`'
            echo '- 用途: 解除交付说明 U1/U2/U-b（release 档帧值此前未实测）'
            echo "- cargo 退出码: \`$cargo_status\`"
            echo
            echo '```text'
            if [ -f "$log" ] && grep -F '[poll-frame-budget]' "$log"; then
              :
            else
              echo '(日志内无 [poll-frame-budget] 行：门禁在量测阶段即失败，详见同 job 日志)'
            fi
            echo '```'
          } >> "$GITHUB_STEP_SUMMARY"
          echo '--- release poll frame budget (逐对象实测字节) ---'
          if [ -f "$log" ] && grep -F '[poll-frame-budget]' "$log"; then
            :
          else
            echo '(日志内无 [poll-frame-budget] 行)'
          fi
          exit "$cargo_status"
      # B 层（小栈）：显式 2 MiB worker runtime（`.worker_threads(1)`
      # `.thread_stack_size(2*1024*1024)`）——显式栈值覆盖 RUST_MIN_STACK，
      # 与仓级 32 MiB force 无关。两条：worker 矩阵 + 三守卫/新旧 run 隔离。
      - name: "B layer: release explicit 2 MiB worker matrix"
        run: flock /tmp/aria-cargo.lock cargo test --release --locked --lib provider_run_2mib_worker_matrix -- --nocapture
      - name: "B layer: release explicit 2 MiB three-guards / new-vs-old run isolation"
        run: flock /tmp/aria-cargo.lock cargo test --release --locked --lib real_run_guards_keep_depth_lease_and_new_token -- --nocapture
      # C 层（真实 WS 送审轮）：显式 2 MiB worker 上跑真实 WS 深链到 ReviewOnly
      # 送审轮 ⇒ 栈回归必红。判据永远是 2 MiB 那条（matrix 是同口径的场景矩阵）。
      - name: "C layer: release real WS review round on explicit 2 MiB worker"
        run: flock /tmp/aria-cargo.lock cargo test --release --locked --test it_core provider_run_2mib_real_ws_review_round_gate -- --nocapture
      - name: "C layer: release real WS scenario matrix on explicit 2 MiB worker"
        run: flock /tmp/aria-cargo.lock cargo test --release --locked --test it_core provider_run_2mib_real_ws_matrix -- --nocapture
      # release 的体积数字必须留在 CI 里可追溯（job summary 由上面的 A 层步骤
      # 自己写；此处再留一份 artifact），用途是解除交付说明 U1/U2/U-b
      #（「release 帧值未实测」）。
      # ★本步只做证据上传，**不参与门禁判定**；`if: always()` 的唯一作用是
      #   「门禁红的时候也把数字留下来做归因」——门禁步骤本身没有任何条件豁免，
      #   也没有 continue-on-error / skip / allow / 无条件 true。
      - name: Upload release volume evidence
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: release-poll-frame-budget-evidence
          path: /tmp/aria-release-evidence
          if-no-files-found: warn
```

### 1.2 用例名核对（★不照抄任务书，全部从代码读出★）

| 层 | 用例名（**从代码核对**） | 位置 | 为何是这个名字 |
|---|---|---|---|
| A | `poll_frame_reservations_stay_within_budget`（+ `fail_closed_guard_tests::{missing_tool_fails_closed_with_actionable_message, zero_probe_measurement_fails_closed}`） | `src/web/workspace_ws_handler/run/provider_run/poll_frame_budget.rs:312` | 门禁本体；**过滤名用模块名 `poll_frame_budget`** ⇒ 一条命令同时跑门禁 + 两条 fail-closed 护栏自测 |
| B | `provider_run_2mib_worker_matrix` | `src/web/workspace_ws_handler/tests/provider_run_events/lifecycle_guards.rs:620` | B 层**显式 2 MiB worker** 矩阵（`explicit_2mib_runtime()` → `.worker_threads(1).thread_stack_size(2*1024*1024)`，行 587-593） |
| B | `real_run_guards_keep_depth_lease_and_new_token` | 同上，行 840 | 三守卫 / 新旧 run 隔离（同样走 `explicit_2mib_runtime()`，行 842） |
| C | `provider_run_2mib_real_ws_review_round_gate` | `tests/it_core/workspace_ws_integration/provider_run_small_stack.rs:714` | C 层**送审轮**门禁（隔离子进程跑 `provider_run_2mib_real_ws_child_review_round`，墙钟 300 s） |
| C | `provider_run_2mib_real_ws_matrix` | 同上，行 688 | C 层**场景矩阵**（`story_author` / `plan_relay` / `sc_manual_gate`，各自 240/300/240 s） |

> 核对依据：`grep` 全仓，B 层两条用例名**唯一**（不与 `provider_run_2mib_real_ws_*` 混淆——后者在 `it_core` 测试目标里，不在 `--lib` 里）；C 层两条在 `it_core` 测试目标里唯一。**全仓无同名**。

### 1.3 设计要点与依据

| 点 | 依据 |
|---|---|
| **先构建前端** | aria 通过 rust-embed 在编译期嵌入 `web/dist`；`build.rs` 在 `web/dist` 缺失/为空时**硬失败**。`web/dist` 不在版本控制中 ⇒ 干净 checkout 后必须先构建。做法与 `rust` job **完全一致**（`pnpm -C web install --frozen-lockfile && pnpm -C web build`）。 |
| **装 codegraph CLI** | C 层真实 WS 夹具走**生产** aggregate-index rebuild 端点（shell out 真实 `codegraph` CLI）⇒ runner 上没有该工具时夹具直接失败。钉版本 `@colbymchenry/codegraph@1.6.2`（与另两个 job、本地一致）。 |
| **A 层必须 `--nocapture`** | 门禁逐对象用 `println!("[poll-frame-budget] {needle}: poll 帧预留 {best} B（预算 {budget} B）\| {best_name}")` 打印实测字节。libtest 对**通过**的用例会吞掉 stdout ⇒ **不加 `--nocapture` 就一个字都看不到**（debug 档就是这样：实测 `cargo test --locked --all-targets` 的日志里门禁 `... ok`，但日志里 `grep '[poll-frame-budget]'` **零命中**）⇒ `--nocapture` 是 release 数字可追溯的**硬前提**。 |
| **A 层步骤自己写 job summary** | 门禁红的时候恰恰最需要数字做归因 ⇒ 总结写在「cargo 跑完之后、退出码回传之前」，`set +e` 显式关掉 GitHub `shell: bash` 默认的 `-e`，再用 `PIPESTATUS[0]` 取 cargo 的真实退出码并 `exit "$cargo_status"` ⇒ **红的时候总结照样写，退出码照样原样回传**（不是无条件 true）。 |
| **每条 cargo 带 `flock`** | 与仓内既有约定一致（`tasks.md` 2.2：「每条 cargo 带 `flock`；**不 matrix 并发同 target**」）。本 job 的 cargo 命令**顺序执行**、不 matrix、不并发同 target。 |
| **不加 `-j 1`** | 仓规禁止（`.cargo/config.toml` 注释：`-j 1` 会退化为单核串行）；仓级 `jobs = 8` 已定好。 |
| **不加 `--ignored` / skip / allow** | 五条门禁都是**常规** `#[test]`（非 `#[ignore]`）⇒ 不需要 `--ignored`；门禁红就是红。 |
| **不改 `.cargo/config.toml`** | owner 2026-11-06 裁决：保留 `RUST_MIN_STACK = { value = "33554432", force = true }`。本 job 因此**不覆盖** `RUST_MIN_STACK`——A/B/C 三层门禁都自带显式栈值或与栈无关，显式值覆盖环境变量。 |
| **不改两个既有 job** | `rust` / `stack-2mib-gate` 的既有步骤**一字未改**（本次 diff 是**纯追加**：单个 hunk `@@ -101,3 +101,125 @@`，只新增 `release-profile-gates`）。 |
| **timeout 90 min** | release 是 LTO + `codegen-units=1` ⇒ 链接是重活；C 层真实 WS 场景自带 240~300 s 墙钟上限（矩阵 3 个场景）⇒ 90 min 留足余量（硬要求 ≥60）。 |
| **`if: always()` 只在证据上传步** | 唯一一个条件豁免挂在 `Upload release volume evidence`（`actions/upload-artifact@v4`）上：它**不判定门禁**，只把 `/tmp/aria-release-evidence` 留档；上传本身失败也会让 job 红。**五条门禁步骤全部无条件**。 |

---

## 2. 验证（本机实跑，非纸面）

### 2.1 ★release 档逐对象实测：未完成（不编造）★

```
$ flock /tmp/aria-cargo.lock cargo test --release --locked --lib poll_frame_budget -- --nocapture
   Compiling cadence-aria v0.1.1 (.../.worktrees/bugfix-b-11-02)
   ← 被 controller 终止（为让位 owner 的部署构建）
```

**经过（时间线）**：

| 时点 | 事件 |
|---|---|
| 21:18 | 本机起 `cargo test --release --locked --lib poll_frame_budget -- --nocapture`（`flock /tmp/aria-cargo.lock`），排队等锁 |
| 21:29 | 拿到锁，开始 release 编译（`Compiling cadence-aria`） |
| 21:33 | 本机编译被 **controller 终止**（owner 的**部署构建**占着同一把 `/tmp/aria-cargo.lock`，是 B3 真机测试的关键路径）⇒ 释放锁给部署 |
| 21:34 | 本机进程全部清理（`cargo test --release` 残留进程数 = 0，`flock … cargo test --release` 残留 = 0），锁交还部署 |

⇒ ★**本机没有 release 档的逐对象实测值**★ ⇒ **本报告不列 release 数字**（列了就是编造）。
⇒ release 数字的取证路径已经**铺好**：本 job 的 A 层步骤会在 CI 上跑出全部 22 行的 `[poll-frame-budget] …` 并写进 job summary + artifact（§1.1）。
⇒ ★**release 下是否出现 debug 下没出现的越界对象，本轮无法下结论**★：已知事实是盘点报告（`cadence/reports/2026-11-06_盘点_全仓大future清单_v1.0.md`）在**全仓**口径上已确认存在 **release-only 越界**（`spawn_engine_event_forward_task`：debug 4,096 B → release 73,728 B）；但**这 22 个受门禁对象**在 release 下的逐对象数值要等 CI 首跑才有。

### 2.2 debug 档：门禁在常规 suite 内实测绿（旁证，非 release）

peer 的常规全量跑（`cargo test --locked --all-targets`，日志 `/tmp/aria-full12.log`）里，A 层门禁三条全绿：

```
test web::workspace_ws_handler::run::provider_run::poll_frame_budget::fail_closed_guard_tests::missing_tool_fails_closed_with_actionable_message ... ok
test web::workspace_ws_handler::run::provider_run::poll_frame_budget::fail_closed_guard_tests::zero_probe_measurement_fails_closed ... ok
test web::workspace_ws_handler::run::provider_run::poll_frame_budget::poll_frame_reservations_stay_within_budget ... ok

test result: ok. 6054 passed; 0 failed; 7 ignored; 0 measured; 0 filtered out; finished in 127.76s
     Running tests/it_core.rs (target/debug/deps/it_core-9282711acce8fce6)
test result: ok. 221 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 59.05s
```

⇒ debug 档**没有**门禁红；但日志里 `grep -F '[poll-frame-budget]'` **零命中** ⇒ 正是 `--nocapture` 缺失导致 debug 的逐对象数字也没留下 ⇒ **这正是新 job 必须带 `--nocapture` 的直接证据**。

### 2.3 debug 档逐对象实测（同一套 `nm`/`objdump` 方法，当前源码的 debug 测试二进制）

**被测二进制**：`target/debug/deps/cadence_aria-ca390a49ceebf822`（mtime `2026-11-06 21:11`，即 peer 常规 suite 用的那一份，**当前源码**）
**方法**：复刻 `poll_frame_budget.rs` 的量测逻辑（`nm -C` 取符号地址 → `objdump -d --start-address/--stop-address` 取 0x200 B 序言窗口内的最大栈预留立即数），needle 必须匹配 `…::{closure#0}`。

| # | 受门禁对象 | 档位 | 预算 | **debug 实测 B** | 判定 |
|---|---|---|---|---|---|
| 1 | `provider_run_drive_body` | P0 核心 | 65,536 | **4,096** | OK |
| 2 | `provider_run_task_body` | P0 核心 | 65,536 | **4,096** | OK |
| 3 | `provider_run_task` | P0 核心 | 65,536 | **45,056** | OK |
| 4 | `drive_author_arm` | P0 核心 | 65,536 | **40,960** | OK |
| 5 | `drive_author_choice_followup_arm` | P0 核心 | 65,536 | **40,960** | OK |
| 6 | `drive_revision_arm` | P0 核心 | 65,536 | **4,096** | OK |
| 7 | `drive_review_only_arm` | P0 核心 | 65,536 | **1,016** | OK |
| 8 | `drive_single_candidate_author_arm` | P0 核心 | 65,536 | **4,096** | OK |
| 9 | `drive_legacy_outline_arm` | P0 核心 | 65,536 | **61,440** | OK |
| 10 | `drive_plan_draft_arm` | P0 核心 | 65,536 | **4,096** | OK |
| 11 | `drive_plan_batch_arm` | P0 核心 | 65,536 | **4,096** | OK |
| 12 | `drive_human_gate_sc_revision_arm` | P0 核心 | 65,536 | **61,440** | OK |
| 13 | `spawn_provider_run_from_event` | P0 spawn/准入 | 32,768 | **4,096** | OK |
| 14 | `spawn_provider_run_from_handler` | P0 spawn/准入 | 32,768 | **4,096** | OK |
| 15 | `spawn_provider_run_claiming_idle` | P0 spawn/准入 | 32,768 | **4,096** | OK |
| 16 | `drive_plan_revision_arm` | 宏体（过渡） | 131,072 | **118,784** | OK |
| 17 | `drive_provider_run_followups` | 宏体（过渡） | 131,072 | **86,016** | OK |
| 18 | `drive_review_session_via_gateway_with_window` | P1 engine | 262,144 | **110,592** | OK |
| 19 | `drive_review_session_with_window` | P1 engine | 262,144 | **102,400** | OK |
| 20 | `drive_review_session` | P1 engine | 262,144 | **110,592** | OK |
| 21 | `drive_review_session_via_gateway` | P1 engine | 262,144 | **110,592** | OK |
| 22 | `complete_review` | P1 engine | 262,144 | **155,648** | OK |

```
violations: []
missing: []
```

⇒ ★**22/22 全部定位成功、debug 档零越界**★（最大三个：`complete_review` 155,648 B、`drive_plan_revision_arm` 118,784 B、`drive_review_session*` 110,592 B）。
⇒ 这同时验证了**门禁覆盖面与当前源码一致**（`provider_run_task_body` / `spawn_provider_run_claiming_idle` 两个较新的符号在**当前源码**的测试二进制里都能定位到 ⇒ 新 job 在 release 下也会定位到；否则门禁会 fail-closed 报 `[missing-symbol]` 并把 CI 打红——这是设计意图，不是缺陷）。
⇒ ★**这列是 debug 数字，不是 release 数字**★（`tasks.md` 7.3：体积数字一律标 profile；**debug 数字不得冒充 release**）。

### 2.4 A 层步骤脚本逻辑实跑验证（不跑真 cargo，用 stub）

```
$ bash -e <A 层步骤脚本>          # PATH 里放 stub `cargo`，不触发任何真编译
[poll-frame-budget] provider_run_drive_body: poll 帧预留 36864 B（预算 65536 B）| sym::{closure#0}
[poll-frame-budget] provider_run_task_body: poll 帧预留 4096 B（预算 65536 B）| sym::{closure#0}
--- release poll frame budget (逐对象实测字节) ---
STEP_EXIT=0
```

生成的 job summary（`$GITHUB_STEP_SUMMARY`）：

```
## Release profile —— poll 帧预留逐对象实测字节

- profile: `release`
- target: `x86_64-unknown-linux-gnu`
- rustc: `1.99.1`
- 命令: `flock /tmp/aria-cargo.lock cargo test --release --locked --lib poll_frame_budget -- --nocapture`
- 用途: 解除交付说明 U1/U2/U-b（release 档帧值此前未实测）
- cargo 退出码: `0`

```text
[poll-frame-budget] provider_run_drive_body: poll 帧预留 36864 B（预算 65536 B）| sym::{closure#0}
[poll-frame-budget] provider_run_task_body: poll 帧预留 4096 B（预算 65536 B）| sym::{closure#0}
```
```

**红路径**（stub 返回 101）：`STEP_EXIT=101` ⇒ 步骤**红**；且 job summary **照样写出**（含 `cargo 退出码: 101` 与逐对象字节）⇒ ★「数字留痕」与「门禁会红」两件事**互不干扰**★。

> ★实测中抓到并修掉的一个真 bug★：summary 行里 Markdown 反引号写在**双引号**字符串里 ⇒ bash 把 `` ` `` 当成命令替换 ⇒ `target` / `rustc` 两行变成空且报 `x86_64-unknown-linux-gnu: command not found`。已改为 `\`` 转义（单引号行本来就安全）⇒ 现已实测输出正确值（见上）。

### 2.5 YAML 可解析性 + 既有 job 未被触碰

```
$ python3 -c "import yaml;d=yaml.safe_load(open('.github/workflows/ci.yml'));print(list(d['jobs'].keys()))"
['rust', 'stack-2mib-gate', 'release-profile-gates']

$ python3 -c "import yaml;d=yaml.safe_load(open('.github/workflows/ci.yml'));j=d['jobs']['release-profile-gates'];print(j['timeout-minutes']);[print(' -',s.get('name') or s.get('uses'),'| if:',s.get('if','(none)')) for s in j['steps']]"
90
 - actions/checkout@v4 | if: (none)
 - dtolnay/rust-toolchain@master | if: (none)
 - Cache cargo artifacts | if: (none)
 - Setup Node | if: (none)
 - Setup pnpm | if: (none)
 - Build frontend | if: (none)
 - Install codegraph (C-layer real WS fixtures need the real CLI) | if: (none)
 - Build release artifacts | if: (none)
 - A layer: release poll frame budget (per-object measured bytes) | if: (none)
 - B layer: release explicit 2 MiB worker matrix | if: (none)
 - B layer: release explicit 2 MiB three-guards / new-vs-old run isolation | if: (none)
 - C layer: release real WS review round on explicit 2 MiB worker | if: (none)
 - C layer: release real WS scenario matrix on explicit 2 MiB worker | if: (none)
 - Upload release volume evidence | if: always()
```

⇒ ★**五条门禁步骤全部无条件**★；唯一的 `if: always()` 在纯证据上传步。

```
$ git diff --stat
 .github/workflows/ci.yml                                  | 122 +++++++++++++++++++++
 openspec/changes/provider-run-volume-budget-and-gate-completion/tasks.md |  17 ++-

$ git diff .github/workflows/ci.yml | grep -E '^@@|^\+  [a-z-]+:'
@@ -101,3 +101,125 @@ jobs:
+  release-profile-gates:
```

⇒ 改动是**纯追加**（单个 hunk，只新增一个 job）⇒ `rust` / `stack-2mib-gate` 两个既有 job **一字未改**。

### 2.6 改动面复核

```
$ git status --short
 M .github/workflows/ci.yml
 M openspec/changes/provider-run-volume-budget-and-gate-completion/tasks.md
（+ 本报告为新增文件）
```

⇒ **未改任何 Rust 源码、`.cargo/config.toml`、前端代码**。

---

## 3. 文档同步

| 文档 | 改动 |
|---|---|
| `openspec/changes/provider-run-volume-budget-and-gate-completion/tasks.md` | ① **2.2** 条目的「残留」行改写：release 体积数字由独立 job **`release-profile-gates`** 承接，并明确标注「release 实测数字待 CI 首次运行取证」（不编造）；② **8.7** 勾选状态：**CI job 已落地✓，但验收证据（release 数字入报告 + `--release` C 层复跑留痕）待 CI 首次运行**⇒ 条目**保持未勾选**并写清原因、job 名、job 内全部命令原文、前置、边界、留痕位置与本报告路径。 |

---

## 4. 边界与未做项（如实登记）

| 项 | 状态 |
|---|---|
| `.cargo/config.toml` 的 `RUST_MIN_STACK = { value = "33554432", force = true }` | **未改**（owner 裁决保留） |
| `rust` / `stack-2mib-gate` 两个既有 job 的步骤 | **未改**（diff 为纯追加） |
| 任何 Rust 源码 / `.cargo/config.toml` / 前端代码 | **未改** |
| `continue-on-error` / `\|\| true` / `--ignored` / skip / allow / 无条件 true | **一律未用**于门禁步骤 ⇒ release-only 越界能把 CI 打红 |
| `-j 1` | **未用**（仓级 `jobs = 8`） |
| ★**release 档逐对象实测值**★ | ★**本机未完成**（§2.1）★——release 编译与 owner 部署构建争 `/tmp/aria-cargo.lock`，为让位关键路径被 controller 终止 ⇒ **不编造 release 数字**；取证改由本 job 在 CI 上完成 |
| ★**release 下是否出现 debug 下没出现的越界对象**★ | ★**本轮无法下结论**★（全仓口径的 release-only 越界已由盘点报告确认：`spawn_engine_event_forward_task` debug 4,096 B → release 73,728 B；但这 22 个受门禁对象在 release 下的逐对象数值待 CI 首跑） |
| ★**`--release` 下 C 层 2 MiB 门禁复跑留痕**★ | ★**本机未完成**（同 §2.1 原因）★；CI job 的 C 层两条命令即为此取证而设 |
| `cargo build --release --locked`（release 产物本身） | job 内已加（先编出 release 制品）；本机未跑完 |
| CI 上的**首次**真实运行结果 | 本次只在本机验证脚本逻辑 + YAML 可解析 + debug 档实测；**GitHub runner 上的实际执行结果尚未观测**（需 push 后看 Actions run） |
| 交付说明 `cadence/notes/2026-11-06_交付说明_阶段1_爆栈根治与可用版_v1.0.md` 的 §4/§5（U1/U7） | **本轮未改**（不在本次改动面内）⇒ U1「release profile 的实际帧值」在 CI 首跑取证前**仍应记为未验证** |

---

## 5. 下一步（谁做什么）

1. **CI 首跑取证**（owner / 集成 owner）：push 后看 `release-profile-gates` job 的 Actions run ⇒ 从 **job summary** 与 artifact `release-poll-frame-budget-evidence` 取 release 下 22 行 `[poll-frame-budget] …`（含 profile/target/toolchain）⇒ 回填本报告 §2.1 并勾选 `tasks.md` 8.7。
2. **release-only 越界判定**：若 CI 上出现某对象 release > 预算 ⇒ 该门禁**正确地**把 CI 打红 ⇒ 按盘点报告 §7 的 Top5 排序处置（**不得**放宽档位预算以求绿）。
3. **交付说明回填**：§5 的 U1（release 实际帧值）、U7（双 profile 固定 2 MiB 真实 WS）在取证后由 owner 裁定更新。
