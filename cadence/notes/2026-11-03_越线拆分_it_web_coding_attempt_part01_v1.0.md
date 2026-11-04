# 越线拆分：tests/it_web/web_coding_attempt_api/part_01.rs（v1.0）

日期：2026-11-04（执行）/ 2026-11-03（任务批次命名）
执行者：SplitItWebAttemptPart01（worker）
基线：part_01.rs 1203 行 > large_file_guard 1200 行硬线（仅超 3 行）；Main 裁决不新增 GRANDFATHERED 登记项，拆分解决，需挪出 ≥4 行。

## 1. 装配方式（谁 include 谁）

```
tests/it_web/web_coding_attempt_api.rs                  ← 族装配文件（24 行，本轮未动）
  ├── include!("web_coding_attempt_api/part_01.rs")     ← part_01.rs 本身即被 include! 的文本包含
  │     └── include!("part_01b.rs")                     ← 本轮新增：嵌套挂载，相对 part_01.rs
  │                                                       所在目录（web_coding_attempt_api/）解析
  ├── include!("web_coding_attempt_api/part_02.rs")
  └── … part_24.rs
```

- 外层父清单按 `tests/it_web/` 解析（`web_coding_attempt_api/part_0X.rs`）；本轮新增的内层
  include! 按 `tests/it_web/web_coding_attempt_api/` 解析（`part_01b.rs`）。两级不同，未混。
- 与本波前 5 例（`tests/it_core/workspace_ws_integration/part_05.rs:1025+part_05b:269`、
  `src/product/workspace_engine/tests/part_09.rs:1116`、
  `src/product/coding_workspace_engine/tests/provider_failure_recovery/retry_and_gate.rs:1059`、
  `src/product/coding_workspace_engine/tests/code_review_triage.rs:1070`、
  `src/web/coding_ws_handler/tests/runner_recovery.rs:1042`）完全同手法：在拆分文件自身内部
  嵌套 include!，父清单不动。include! 是纯文本展开 ⇒ 分片与 part_01.rs 同 crate（it_web test
  target）同模块作用域，item 顺序无关 ⇒ 命名空间 / 测试全路径 / helper 可见性零变化。
- 装配方式系实测确认（`grep -rn 'part_01' tests/it_web/…` + `grep -rn 'include!\|mod part_0'`），
  全族 24 个分片全部为 include! 文本包含，无 mod 独立模块。

## 2. 切点选择理由

part_01.rs 刚被异构簇 worker 改过（`git diff --stat`：+38/−9，11 个 hunk），改动行集合（新行号，
`git diff -U0` 的 `+` 侧）：**{10, 93, 235, 262-264, 266, 632-637, 639, 647-668, 672, 682, 685}**
（集中在 use 块与 5 个测试函数：`creates_…_surfaces_latest_attempt`、
`rejects_…_dependency_work_item_is_not_completed`、
`rejects_second_active_work_item_on_same_repo_shared_worktree`、
`rejects_group_coding_attempt_for_unconfirmed_plan`、
`rejects_group_coding_attempt_when_a_same_target_single_item_attempt_is_active`）。

**选定切口：原 741-761 行（21 行）** —— 完整独立测试函数
`creates_coding_attempt_when_completed_dependency_has_no_handoff_summary`
（`#[tokio::test]` 741 起至 `}` 761 止，740/762 为空行，干净 item 边界）。

理由：
1. **与异构簇本轮改动零交集**：741-761 与改动行集合 {10, 93, 235, 262-266, 632-685} 无任何
   重叠（最近改动点 685 距切口上界 741 尚隔 56 行），切错风险最小。
2. **完整顶层 item**：单个 `#[tokio::test]` + async fn 块，不切断任何 impl/use/struct；其引用的
   helper（`git_repo` / `request_json` / `assert_global_attempt_id` /
   `bootstrap_completed_dependency_without_handoff`）均定义于同族其他分片，include! 拼接后
   同一模块作用域，可见性不变。
3. **最小扰动**：只超 3 行，仅需挪出 ≥4 行；21 行的独立测试函数是最小的干净切分单元之一，
   拆后 1184 行即回到线下，未为大动干戈而多切。
4. mtime 避让核对：part_01.rs 拆前 mtime 02:55:37，接令时（03:38:22）距改约 43 分钟，
   在 20 分钟避让窗口外；装配文件 `web_coding_attempt_api.rs` mtime 前日 20:08，亦在窗口外
   （且本轮未动它）。

## 3. 拆分结果与文件行数（large_file_guard 口径）

| 文件 | 拆前 | 拆后 | 判定 |
|---|---|---|---|
| `tests/it_web/web_coding_attempt_api/part_01.rs` | 1203 | **1184** | < 1200 ✓ |
| `tests/it_web/web_coding_attempt_api/part_01b.rs`（新增） | — | **21** | < 1200 ✓ |

part_01.rs 拆后构成 = 原 1-740 行（740 行）+ 挂载注释 1 行 + `include!("part_01b.rs");` 1 行 +
原 762-1203 行（442 行）。挂载点上下文（拆后 738-744 行）：

```rust
    assert_eq!(retry_body["active_unit_id"], "coding_unit_0001");
}

// part_01 拆分（large_file_guard 1200 行上限）：独立测试函数经 include! 挂载，与本文件同一模块作用域，测试全路径与行为零变化（纯移动）。
include!("part_01b.rs");

#[tokio::test]
async fn delete_coding_attempt_releases_active_lock_when_clean() {
```

被改/新增文件清单（供 Main 统一处理 large_file_guard 登记核对）：仅上述两个文件；
`tests/it_core/large_file_guard.rs`、`tests/it_web/web_coding_attempt_api.rs`（族装配文件）、
`tests/it_core/workspace_ws_integration/`、`tests/it_web/web_coding_ws_handler/`、`src/`、
spec / Plan / 技术方案 / tasks.md 全部未动。part_01.rs 未登记在 GRANDFATHERED，无删项需求，
亦未新增任何登记项。

## 4. 逐字节一致性核验（双向）

拆前快照：`/tmp/part_01.rs.bak`（md5 `10c991caac83f5064c39d38d29dc5acf`，1203 行）。

- **核验 A（分片 vs 原区间，双向）**：`sed -n '741,761p' /tmp/part_01.rs.bak | diff - part_01b.rs`
  与反向 `diff part_01b.rs -` → **双双空输出，IDENTICAL**（part_01b.rs md5
  `7d3e82fe3a0184708ee421f98894c2cd`，21 行）。
- **核验 B（拆后 vs 拆前 diff 审阅）**：`diff /tmp/part_01.rs.bak part_01.rs` → 唯一差异为单一
  hunk：`741,761c741,742`（原 21 行替换为注释 + include! 两行），无任何其他修改/删除行。
- **核验 C（include! 展开模拟，最强双向证明）**：拆后 part_01.rs 把挂载两行（注释 + include!）
  整体替换回 part_01b.rs 内容，与拆前全文比对 → **逐字节相等（restored==orig: True）**：
  拆后文件精确还原拆前全文，即纯搬移。
- 落盘方式：python 锚定组装（锚定 740=空行 / 741=`#[tokio::test]` / 762=空行，任一不符即中止），
  规避 edit 工具「回显成功不落盘」坑；写盘前后 md5 双核对（拆后 part_01.rs 最终 md5
  `2c8cc4e8240b94105a1cf926c39717b0`）。

## 5. 测试函数数（拆前 vs 拆后）

- part_01.rs：拆前 `#[tokio::test]` 计数 **25**；拆后 part_01.rs **24** + part_01b.rs **1**
  = **25**，无增减。
- （对应 `async fn` 计数：拆前 26 → 拆后 25 + 1 = 26，含 1 个非测试 helper
  `seed_group_attempt_with_published_handoffs`，同样无增减。）
- 测试名 / 测试全路径 / 断言 / 模块作用域：零变化（纯文本展开，核验 C 背书；
  `it_web::web_coding_attempt_api::creates_coding_attempt_when_completed_dependency_has_no_handoff_summary`
  路径不变）。

## 6. 窄跑验证

命令：`flock /tmp/aria-cargo.lock -c 'cargo test --locked --test it_web web_coding_attempt_api
-- --nocapture 2>&1 | tail -25'`

（结果：**ok. 100 passed; 0 failed; 0 ignored; 511 filtered out; finished in 318.73s**，该域全绿，
拆分未引入任何红条；尾部摘录：）

```text
test web_coding_attempt_api::rejects_group_coding_attempt_when_a_same_target_single_item_attempt_is_active ... ok
test web_coding_attempt_api::rejects_group_coding_when_plan_projection_dependencies_do_not_match_the_graph ... ok
test web_coding_attempt_api::rejects_second_active_work_item_on_same_repo_shared_worktree ... ok
test web_coding_attempt_api::returns_group_coding_attempt_snapshot_with_units ... ok
test web_coding_attempt_api::single_repo_lifecycle_create_repair_delete ... ok

test result: ok. 100 passed; 0 failed; 0 ignored; 0 measured; 511 filtered out; finished in 318.73s
```

注：过滤词 `web_coding_attempt_api` 在 it_web target 内匹配 100 条（任务书基线描述为「47 条」系
异构簇收尾时点的计数；其后本波其他改动使族内测试数增长），判定口径以「该域 0 failed 全绿」为准，
未因拆分变红。

被搬移测试的精确复核（`-- --exact` 全路径单跑）：

```text
test web_coding_attempt_api::creates_coding_attempt_when_completed_dependency_has_no_handoff_summary ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 610 filtered out; finished in 3.26s
```

⇒ 搬移后测试全路径逐字不变（`web_coding_attempt_api::` 前缀下同名同路径）且绿。

## 7. Concerns

1. **guard 转绿依赖**：`tests/it_core/large_file_guard.rs` 登记表未动（Main 统一处理）；拆后
   part_01.rs 1184 / part_01b.rs 21 均低于 1200 硬线，guard 无新增要求；part_01.rs 未登记在
   GRANDFATHERED，无需删项、更未新增项。
2. **include 路径层级**：内层 include! 相对 `web_coding_attempt_api/` 目录解析，与外层父清单
   （相对 `tests/it_web/`）层级不同；后续若有人往 part_01.rs 再加分片，照此层级写路径。
3. **族分片编号**：新分片命名 `part_01b.rs` 跟随本波先例（it_core part_05b），未占用
   `part_25.rs` 编号，父清单 24 行保持不动。
4. 纯搬移之外的改动仅 part_01.rs 内新增 2 行（1 行注释 + 1 行 include!）；无其它任何改动
   （无 src/、无断言/测试名/测试全路径变化、无 Legacy 变体恢复、无自造 reason_code、无删测试、
   无 git add/commit/stash/checkout、未碰 large_file_guard.rs /
   workspace_ws_integration/ / web_coding_ws_handler/ / spec / Plan / 技术方案 / tasks.md）。
