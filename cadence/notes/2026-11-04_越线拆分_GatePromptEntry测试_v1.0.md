# 越线拆分：GatePromptEntry.test.tsx（1292 → 1177）v1.0

- 工作树：`.worktrees/bugfix-b-11-02`
- 时间：2026-11-04 15:0x（动手前目标文件 mtime 09:54:11，距动手 >5h，无 20 分钟内碰撞，避让通过）
- 背景：本波块1 7.8（REQ-HGC-01 卡片刷新）为 `GatePromptEntry.test.tsx` 追加 3 例组件级回归（+115 行），使文件 1292 行越过 1200 硬线（报告：`cadence/notes/2026-11-04_块1_7.8与块2_11.10_前端缺口补齐_v1.0.md`）。按本波 6 例先例手法纯搬移拆回线下；未新增 `GRANDFATHERED` 登记项，未碰 `tests/it_core/large_file_guard.rs`（登记上限归 Main 统一处理）。

## ① 装配方式 + 切点理由

**既有切法（同目录实锤先例）**：`entries/` 目录对 `GatePromptEntry` 测试族早已采用「兄弟文件」多文件切法——`GatePromptEntry.{compile-failure, guidance, revoke, stale-completed, wsc}.test.tsx` 共 5 个；其中 `GatePromptEntry.wsc.test.tsx` 头注明示该切法的由来：

> `// 独立文件：GatePromptEntry.test.tsx 已 1151 行，行数门禁（1200）余量不足。`

即：**头注说明独立性 + 自带 imports + 自带 `gateEntry`/`actions` helper 副本 + 完整顶层 describe 块**。vitest 默认 include（`**/*.test.tsx`，见 `web/vite.config.ts` 的 `test` 段无自定义 include）自动收集新文件，零配置改动。

**本次照抄**：新分片 `GatePromptEntry.card-refresh.test.tsx` = 13 行 glue（5 行头注 + 7 行 imports + 1 空行）+ 38 行 helper 副本（原文件 14-51 行逐字节复制）+ 115 行搬移区间。

**切点**：原文件行 **1178-1292**（1 空行 + 7 行 `// REQ-HGC-01「卡片刷新」…` 注释 + 107 行 `describe("REQ-HGC-01 卡片刷新：当前卡读同门当前事实，旧卡只读历史", …)` 至 EOF）。

**与 7.8 新增 3 例的关系（零交集口径）**：搬移区间与 7.8 之前的**全部既有用例零交集**——区间整体正是 7.8 追加的尾块（7.8 报告自述该文件 `+115 / −0`，与搬移区间 115 行完全吻合），搬出后原文件回到 7.8 波前状态（见 ③ 的 HEAD 比对）。对既有用例的触碰为零；被移动的只有 7.8 自己的 3 例（测试内容逐字节不动）。

## ② 行数

| 文件 | 拆前 | 拆后 | 硬线 |
|---|---|---|---|
| `web/src/components/chat-workspace/entries/GatePromptEntry.test.tsx` | 1292 | **1177** | < 1200 ✅ |
| `web/src/components/chat-workspace/entries/GatePromptEntry.card-refresh.test.tsx`（新建） | — | **166**（13 glue + 38 helper + 115 搬移） | < 1200 ✅ |

## ③ 逐字节一致性核验（双向）

| 区间 | 核验 | 结果 |
|---|---|---|
| 原文件拆后 vs 原区间 1-1177 | `cmp` 双向 | IDENTICAL（md5 `6918a1a4a6662c4e80edd1d2ac6ed8e6` 两端一致） |
| 新分片尾 115 行 vs 原区间 1178-1292 | `diff` 双向 | IDENTICAL（md5 `f67e69adaca03497b063a319c62e197d` 两端一致） |
| 新分片 helper 区（14-51 行）vs 原文件 14-51 行 | `diff` | IDENTICAL（md5 `530f01442ead7a9f93d9683ed8f1cee1` 两端一致） |
| **原文件拆后 vs `git show HEAD:`（7.8 波前版本，1177 行）** | `cmp` | **IDENTICAL**（md5 `6918a1a4…` 相同） |

末行即最强证明：拆分把 7.8 增量整体搬出后，原文件与波前版本逐字节一致，零残留。

## ④ 用例数与测试全路径

- **拆前基线**（动手前实跑）：`Test Files 260 passed (260)`，`Tests 2670 passed (2670)`。
- **拆后**：`Test Files 261 passed (261)`（+1 = 新分片），`Tests 2670 passed (2670)`——**用例数前后相等，不增不减**。
- 族内分布：`GatePromptEntry.test.tsx` 53 → 50 例；`GatePromptEntry.card-refresh.test.tsx` 3 例；合计 53，无丢失无重复。未删测试、未改任何断言/测试名/describe 名。
- **测试全路径**：describe / it 名逐字未动；**文件路径分量有变化**——7.8 的 3 例现位于 `entries/GatePromptEntry.card-refresh.test.tsx`（原 `entries/GatePromptEntry.test.tsx`）。影响面评估：CI（`.github/workflows/ci.yml`）对前端只跑 `pnpm -C web build`，**不跑 vitest，无 CI 文件过滤依赖**；但历史 plan 文档存在按文件路径过滤跑测的命令样例（如 `cadence/plans/2026-11-03_计划文档_实施_块1_自动化授权换代与人工门文案_v1.0.md:1220`），若按旧路径单文件过滤，7.8 的 3 例不在其中（用 `pnpm test GatePromptEntry` 前缀过滤则两文件全命中）。

## ⑤ pnpm test 输出（拆后，2026-11-04 15:1x）

```
 ✓ src/components/chat-workspace/entries/GatePromptEntry.test.tsx (50 tests) 801ms
 ✓ src/components/chat-workspace/entries/GatePromptEntry.card-refresh.test.tsx (3 tests) 60ms
 Test Files  261 passed (261)
      Tests  2670 passed (2670)
   Duration  27.43s
```

## ⑥ 被改文件行数

| 文件 | 变化 |
|---|---|
| `web/src/components/chat-workspace/entries/GatePromptEntry.test.tsx` | 1292 → **1177**（仅删除搬移区间，其余与 HEAD 逐字节一致） |
| `web/src/components/chat-workspace/entries/GatePromptEntry.card-refresh.test.tsx` | 新建 **166** 行 |

## ⑦ concerns

1. **测试全路径（文件分量）变化**（唯一实质 concern，已报 Main）：7.8 的 3 例移至 `GatePromptEntry.card-refresh.test.tsx`。CI 无影响；仅当有人复用历史文档里「按旧文件路径过滤」的命令时需改用 `GatePromptEntry` 前缀过滤。
2. helper `gateEntry`/`actions` 为副本（与 wsc 等兄弟文件同款既有约定，各文件自治）；日后改 helper 需同步各副本——本目录 5 个兄弟文件已是此格局，未引入新范式。
3. 未跑 `tsc -b`/eslint（`pnpm test` 为指定验证且全绿；新文件 imports 全部被使用，类型面与原区间一致）。
4. 遵守清单：未碰 `src/` 生产、`tests/`（Rust 侧）、`tests/it_core/large_file_guard.rs`、spec/delta/tasks.md 勾选框/Plan/技术方案；无 git add/commit/stash/checkout；未用 Playwright（本任务纯测试拆分，无需浏览器验证）。
