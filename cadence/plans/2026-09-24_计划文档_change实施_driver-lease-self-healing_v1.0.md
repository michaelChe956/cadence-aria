# change 实施 driver-lease-self-healing 实施计划 v1.0

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** driver 租约「attach 零效应 + 悬空首写自愈 + 前端单次自动重试 + 转移打点」，修复 F-50-1（家族第 4 次复发）。

**Architecture:** 仲裁层三文件族（lease.rs/attachment.rs/arbitration.rs）承担 A+B（同批，A 是无 hello 首写承重墙）；lease-diagnostics.jsonl append-only 打点；前端仅 STALE 自动重试腿。

**Tech Stack:** Rust（lib/it_core lease 族）+ TS（vitest）。

**Spec:** `openspec/changes/driver-lease-self-healing/`（strict valid）；诊断 `f50-lease-diagnosis.md`；oracle `f50-lease-oracle.md`（同 sdd 目录）。

## Global Constraints

- 不放宽 observer 写拒绝；holder=Some(活跃) 仍拒 STALE；不引入连接争抢。
- attach 对租约零效应（REQ-DLS-04）：provisional_lease 快照与回滚机制整体删除。
- 自愈放款须同步刷新 attachment.lease_epoch（R2 承重墙钉子）。
- bind_role same_channel 反查不动；删回滚分支后复核 pending→live 迁移（R3）。
- 三个语义反转用例逐名改写不删除：part_06b.rs:83 / part_03.rs:579 / part_03.rs:807；补「无 role hello 保归一」用例。
- 打点失败零影响仲裁；打点不进仲裁状态机。
- 测试命令照仓规；前端只动 STALE 自动重试相关。

## Review Focus

1. attach 零效应后多 tab 驾驶切换（hello(driver) 无条件接管）仍工作 → Task 3 用例
2. 自愈与「旧持有者活着但分区」并存时双写风险 → Task 1 用例（活跃持有者仍拒）
3. lease_epoch 刷新遗漏导致自愈后旧 epoch 写入 → Task 1 联合断言
4. pending→live 迁移在回滚删除后的路径 → Task 3 回归
5. 前端自动重试对非幂等命令 → Task 4（仅 facade 写命令+恰一次）

---

### Task 0: 现状基线取证

- [ ] 0.1 读 lease.rs/attachment.rs/arbitration.rs 全文，列出 provisional_lease 快照/回滚的全部触点清单（含测试引用）落报告——删除面的完整清单是 Task 3 的验收基准。

### Task 1: 写时自愈（REQ-DLS-01，后端）

**Files:** arbitration.rs（arbitrate 写路径）、lease.rs（授予原子性）。

- [ ] 1.1 失败测试：①悬空自愈（holder=None+Driver 首写放行+重授+lease_epoch 同步刷新断言）②活跃偷窃者仍拒 ③observer 写拒不变 ④幂等（同连接二次写不重复授予）。
- [ ] 1.2 跑红→实现（放行+原子授予+epoch 刷新）→跑绿；既有 lease 族零回归。

### Task 2: 租约打点与诊断端点（REQ-DLS-03，后端）

**Files:** 新 `lease_diagnostics.rs`（或 lifecycle_store 旁挂）+ workspace 诊断 HTTP 面挂点。

- [ ] 2.1 失败测试：①五事件枚举落 jsonl（hold/self_heal/release/write_rejected_stale/write_rejected_observer）②端点返回当前持有者+最近序列 ③打点失败零影响 ④滚动上限。
- [ ] 2.2 跑红→实现（append-only jsonl，usage-diagnostics 同构）→跑绿。

### Task 3: attach 零效应（REQ-DLS-04，后端）

**Files:** attachment.rs（删 provisional acquire/回滚）、arbitration.rs、it_core 三用例改写。

- [ ] 3.1 失败测试先行（新语义）：①observer 纯 attach 不偷租约 ②无 hello 首写经自愈放行 ③无 hello 且他人持有时拒 ④多 tab 驾驶切换（hello(driver) 无条件接管）不变 ⑤无 role hello 保归一。
- [ ] 3.2 按 Task 0 清单删除快照/回滚机制；三用例逐名改写（part_06b.rs:83→「observer attach 不偷」、part_03.rs:579/807 按新语义重述 F-24 现场锚）；pending→live 迁移复核。
- [ ] 3.3 REQ-DLS-01×04 联合回归（无 hello 首写全路径）。

### Task 4: 前端单次自动重试（REQ-DLS-02，前端）

**Files:** web/src 出站写命令 facade（workspace-ws-message-handler/cockpit-action-routing 出站侧）。

- [ ] 4.1 失败测试：①STALE 后自动 hello+单次重放原命令成功无感 ②重放仍 STALE→回退手动面不循环 ③仅写命令触发（ observer 拒绝不触发）。
- [ ] 4.2 跑红→实现（恰一次守卫）→跑绿；vitest 全量+tsc。

### Task 5: 门禁收口

- [ ] 5.1 `cargo test --locked --lib` 全量 + `--test it_core`（lease 族全绿，known-flaky 复跑判定）+ `--test it_web`；前端 vitest 全量；strict 复跑；fmt/clippy；F-50-1 现场复现路径模拟回归（多 tab attach→租约不动→写无感）。
- [ ] 5.2 报告：逐 Task 红绿证据、Task 0 清单对照删除面、commit 列表。

---

## Self-Review

覆盖：REQ-DLS-01→T1、02→T4、03→T2、04→T3；oracle 登记要求全落（三用例改写 T3.2/epoch T1/R3 T3.2/交叉引用在 spec）。Review Focus 五条全挂测试。后端 T1-T3 同文件族须串行；T2 可与 T1 并行（新文件）；T4 前端独立。
