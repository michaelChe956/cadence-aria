# 修复报告：poll 帧门禁的环境护栏改为 fail-closed

- 日期：2026-11-06
- 工作树：`/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`（分支 `bugfix-b-11-02`）
- 改动文件（仅一个）：`src/web/workspace_ws_handler/run/provider_run/poll_frame_budget.rs`
- 触发：阶段 1 终审 I-2（Important）+ M-3（Minor）

---

## 1. 改前 / 改后行为对照

| 情形 | 改前 | 改后 |
|---|---|---|
| `nm` / `objdump` 不可用 | `eprintln!` 告警 + `return` ⇒ **用例仍显示 ok**（门禁在无 binutils 环境静默失效且全绿） | **用例 FAIL**：错误信息点名缺失工具、给出可执行处理办法（安装 binutils / llvm-tools）、写明显式 opt-in 开关名；失败分支**立即返回 Err/panic**，无任何 await / channel / join（不阻塞） |
| 量测结果为 0（预算 > 0） | 被当作「0 字节很棒」放过（假绿方向） | **用例 FAIL**：错误信息说明真实含义（探针窗口 0x200 B 可能失效 / 符号定位错误），明确「不得当作 0 字节达标放过」 |
| 受管符号未在测试二进制中定位到 | `assert!` 失败（已是失败） | 保持失败，并归类为 `[missing-symbol]` |
| 分级阈值 | P0 核心 ≤64 KiB / P0 spawn 侧 ≤32 KiB / P1 engine ≤256 KiB / 宏体过渡 ≤128 KiB | **完全不变**（未放宽、未收紧、未加 `#[ignore]`/skip/allow） |
| 文件头注释 | 自称「缺失时不静默通过」，与实际行为矛盾 | 与行为一致：显式写明 fail-closed 合同（工具缺失 ⇒ 失败；量测 0 ⇒ 失败；默认失败、仅显式 opt-in 可跳过工具缺失） |

**显式 opt-in 开关**：`ARIA_POLL_FRAME_BUDGET_ALLOW_MISSING_TOOLS=1`
—— 仅当被显式设为 `"1"` 时，**「工具缺失」**这一种情形才允许跳过（默认＝失败）；
「量测为 0」「符号未定位」「阈值越界」**永不可跳过**。该开关名写在失败信息里。

---

## 2. 失败信息原文

工具缺失（`nm` 缺失时的真实输出，其余同构）：

```
[missing-tool] poll 帧门禁无法执行：必需工具 `nm`（路径 `aria-no-such-nm`）不可用。处理办法：安装 binutils（Debian/Ubuntu: `apt-get install -y binutils`；Alpine: `apk add --no-cache binutils`；Fedora/RHEL: `dnf install -y binutils`；macOS: `xcode-select --install`；Rust 工具链自带 llvm-tools 时可用 `llvm-nm`/`llvm-objdump` 替代）。若确需在无 binutils 的本地环境跳过本门禁，请**显式**设置环境变量 `ARIA_POLL_FRAME_BUDGET_ALLOW_MISSING_TOOLS=1`（默认＝失败，不允许静默跳过）。
```

量测为 0：

```
[probe-window] poll 帧预留量测失败：符号 `symbol_x`（needle `needle_x`）在序言扫描窗口 0x200 B 内未读到任何栈预留立即数（实测 0 B）——多半是探针窗口失效（编译器布局变化）或符号定位错误；**不得**当作「0 字节达标」放过，请同步更新探针窗口或核对符号名。
```

符号未定位：

```
[missing-symbol] 以下受门禁对象的 poll 符号未在测试二进制中定位到：{…}——门禁覆盖面被削弱，需同步更新 GATED_POLL_SYMBOLS
```

---

## 3. 新增 / 修改的用例

| 用例名 | 作用 |
|---|---|
| `web::workspace_ws_handler::run::provider_run::poll_frame_budget::fail_closed_guard_tests::missing_tool_fails_closed_with_actionable_message` | **新增**：注入不存在的工具路径（`aria-no-such-nm` / `aria-no-such-objdump`）⇒ 必须 `Err`；断言失败信息**点名缺失工具**、含 `binutils` 处理办法、含 opt-in 开关名、带 `[missing-tool]` 分类标记 |
| `web::workspace_ws_handler::run::provider_run::poll_frame_budget::fail_closed_guard_tests::zero_probe_measurement_fails_closed` | **新增**：量测值为 0 ⇒ 必须 `Err`；断言带 `[probe-window]` 标记、含探针窗口 `0x200`、点名失败符号 |
| `web::workspace_ws_handler::run::provider_run::poll_frame_budget::poll_frame_reservations_stay_within_budget` | **修改**：改为 fail-closed（工具缺失 ⇒ panic；仅「工具缺失 + 显式 opt-in」才跳过；量测 0 / 符号缺失 ⇒ panic）；阈值表不变 |

---

## 4. 验证命令与真实输出

```
$ timeout 300 flock /tmp/aria-cargo.lock cargo test --locked --lib poll_frame_budget -- --nocapture --test-threads=1
running 3 tests
test …::poll_frame_budget::fail_closed_guard_tests::missing_tool_fails_closed_with_actionable_message ... ok
test …::poll_frame_budget::fail_closed_guard_tests::zero_probe_measurement_fails_closed ... ok
test …::poll_frame_budget::poll_frame_reservations_stay_within_budget ... [poll-frame-budget] provider_run_drive_body: poll 帧预留 4096 B（预算 65536 B）
…（逐对象实测 22 项全部在各自预算内：P0 核心最大 61440 B / P0 spawn 侧 4096 B / 宏体 118784+86016 B / P1 engine 最大 155648 B）
ok
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 6041 filtered out; finished in 29.51s
EXIT=0

$ flock /tmp/aria-cargo.lock cargo test --locked --lib provider_run
test result: ok. 87 passed; 0 failed; 0 ignored; 0 measured; 5957 filtered out; finished in 48.09s
PR_EXIT=0

$ flock /tmp/aria-cargo.lock cargo fmt --check
FMT_EXIT=0

$ flock /tmp/aria-cargo.lock cargo clippy --locked --all-targets --all-features -- -D warnings
CLIPPY_EXIT=0
```

> 说明：一次全量 `provider_run` 跑（PR_EXIT=101）发生在同机多 agent 并行构建期，
> 测试二进制被外部 `kill -9`（见终审记录）⇒ 非本改动引起；缩小范围重跑与
> 隔离子进程用例重跑均全绿（3 passed / 87 passed）。fail-closed 分支全部是
> **立即返回 Err/panic**，实现中没有任何 await / channel / join，不会阻塞。

---

## 5. commit

- **见本报告末行「commit hash」**（提交信息：`fix(provider-run): poll 帧门禁改 fail-closed —— 工具缺失/量测为 0 均显式失败`）

---

## 6. concerns

1. **量测为 0 的护栏是「双刃」**：若将来某受管符号被重构为真正的零帧函数
   （无任何 `sub $imm,%rsp`/帧探针），门禁会失败并要求更新探针窗口或符号名
   ——这是刻意的 fail-closed 取向（宁可停下也不假绿）。
2. **opt-in 开关是环境变量**：默认失败；只有显式 `=1` 才允许在「工具缺失」
   时跳过。该跳过**不覆盖**量测为 0 / 符号缺失 / 阈值越界三类失败。
3. 门禁仍依赖 binutils（`nm`/`objdump`）：CI 的 ubuntu-latest 预装 binutils，
   但**macOS / 精简镜像**需按失败信息安装（或用 llvm-tools 替代）——本次未在
   代码里自动探测 llvm 工具链（避免引入新的可选路径与静默回退）。
4. 本次只改 `poll_frame_budget.rs`，**未触碰**任何产品逻辑、阈值或其它文件。
