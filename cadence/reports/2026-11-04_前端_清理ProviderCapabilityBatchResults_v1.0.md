# 清理 ProviderCapabilityBatchResults（前端）

- 日期：2026-11-04
- 范围：`web/src/components/lifecycle/` 三个文件（组件本体 + 其回归断言的迁移）
- 触发：owner 裁定「清理掉」——本波把「Provider 能力核验」面板改成**行级列表**后，
  `ProviderCapabilityBatchResults` 在应用内已无调用点，但组件本体与其 6 条回归断言还在。

---

## 1. 全部引用点清单（清理前，全仓 grep）

命令：`grep -rn "ProviderCapabilityBatchResults" .`（排除 `target/`、`node_modules/`）

| # | 文件:行 | 性质 | 处理 |
|---|---|---|---|
| 1 | `web/src/components/lifecycle/ProviderCapabilityBatch.tsx:321` | **组件本体**（`export function ProviderCapabilityBatchResults({state, installTargets, onSettled})`） | **已删除** |
| 2 | `web/src/components/lifecycle/ProviderCapabilityBatch.test.tsx:12` | import 语句 | **已删除** |
| 3 | `web/src/components/lifecycle/ProviderCapabilityBatch.test.tsx:162` | 测试宿主 `BatchHost` 里的渲染调用 | **已删除**（6 条断言的去向见 §3） |
| 4 | `cadence/notes/2026-11-04_前端_一键核验按钮位置与添加代码库布局说明_v1.0.md:32,45` | 设计说明文档里的组件提及 | **保留**（`cadence/notes/**` 不在本次授权改动范围；记录的是历史布局决策，不是代码引用） |
| 5 | `cadence/reports/2026-11-04_前端_去掉provider方格卡片只留行级列表_v1.0.md:12,126` | 上一波报告里的组件提及与「待裁定」条目 | **保留**（历史报告；第 126 行那条「请裁定：保留还是另开一次迁移」由本报告闭合） |

### 1.1 随组件一并消失的 testid（组件独占）

- `provider-capability-batch-results`（结果区容器）
- `capability-batch-row-<provider_type>`（逐项行）
- `capability-batch-outdated-<provider_type>`（版本落后标记）

### 1.2 行级列表继续复用的 testid（断言对象搬家后的落点）

- `capability-batch-status-<provider_type>`（批量逐项状态）
- `capability-batch-retry-<provider_type>`（失败项【重试】）

### 1.3 随组件一并删除的死代码（同文件内、仅被该组件引用）

`ProviderCapabilityBatch.tsx` 内：`PROVIDER_LABELS`（批量模块私有副本，能力卡另有一份）、
`InstallEntryKind` 类型、`installEntryKind()` 函数、`InstallTargetView` 类型导入、
`ProviderInstallActionButton` 组件导入。

> 说明：能力卡的行级列表用**自己的**判定（`install-targets.action`）决定
> 【安装】/【更新】入口，不复用 `installEntryKind`；删除后无任何调用点残留。

---

## 2. 改动摘要

| 文件 | 动作 |
|---|---|
| `web/src/components/lifecycle/ProviderCapabilityBatch.tsx` | 删除 `ProviderCapabilityBatchResults` 组件本体 + 仅被它引用的死代码；文件头注释改写为「逐项结果由能力卡行级列表承载」；本模块只保留 hook、按钮、说明区与 `capabilityBatchStatusLabel` |
| `web/src/components/lifecycle/ProviderCapabilityBatch.test.tsx` | 删除组件 import 与渲染调用；与逐项结果相关的 4 组断言迁出（见 §3）；测试宿主只保留 hook + 按钮 + 说明区 |
| `web/src/components/lifecycle/ProviderCapabilityCard.test.tsx` | **新增 4 个用例**承接迁出的断言（断言对象换成行级列表的行，断言内容不弱化）+ 新增 `runningBatchJob()` 夹具 |

**未改动**：`web/src/components/lifecycle/ProviderCapabilityCard.tsx` 一字未动
（`git diff --numstat` 中不出现该文件 ⇒ 行级列表渲染逻辑保持定稿版本）。

---

## 3. 6 条断言逐条去向

原「6 条回归断言」= `ProviderCapabilityBatch.test.tsx` 里的 6 个 `it` 用例。
逐条核对「它在验什么」后决定：**组件专属行为 ⇒ 迁到行级列表上的等价断言；
作业面/按钮面行为 ⇒ 原地保留**（组件删除与之无关）。

### ① 「点击后显示「核验中（N/4）」、按钮禁用，逐项状态随轮询更新」

| 原断言 | 去向 |
|---|---|
| a. start 按钮 `disabled` | **原地保留**（作业面，与逐项结果组件无关） |
| b. 按钮文案「核验中（0/4）」 | **原地保留** |
| c. `capability-batch-status-claude_code` = 「已确认」 | **迁** → `ProviderCapabilityCard.test.tsx`「keeps the per-item batch status in the row list」 |
| d. `capability-batch-status-codex` = 「核验中」 | **迁** → 同上 |
| e. 按钮文案「核验中（1/4）」（逐项状态数 N 驱动） | **原地保留**（N 由 hook 的 `done` 计数给出，不经逐项结果组件） |
| f. POST `…/capabilities/revalidate-all` 恰好 1 次 | **原地保留** |

迁后写法（断言对象换地方，内容不弱化）：

```tsx
const claudeRow = await screen.findByTestId("provider-capability-provider-claude_code");
expect(within(claudeRow).getByTestId("capability-batch-status-claude_code")).toHaveTextContent("已确认");
const codexRow = screen.getByTestId("provider-capability-provider-codex");
expect(within(codexRow).getByTestId("capability-batch-status-codex")).toHaveTextContent("核验中");
```

> 行级列表里同一 testid 的文案带来源前缀（「批量：已确认」「批量：核验中」），
> 断言沿用 `toHaveTextContent` 子串匹配，**匹配面不变弱**；作用域从「全屏该 testid」
> 收紧到「该 provider 所在的行内」，是**加强**不是弱化。

### ② 「失败行保留【重试】，只打单 provider 端点且不重跑整批」

**整组迁** → `ProviderCapabilityCard.test.tsx`「keeps the retry entry for a failed batch item and does not re-run the whole batch」。

- a. codex 状态「核验失败」⇒ 迁到行 `provider-capability-provider-codex` 内
- b. `capability-batch-retry-claude_code` 不存在（只有失败项有【重试】）⇒ 迁到行 `provider-capability-provider-claude_code` 内
- c. 点【重试】⇒ POST `…/capability-revalidate` 恰好 1 次、body `{ provider_type: "codex" }` ⇒ 保留
- d. 不重跑整批（无 POST `…/capabilities/revalidate-all`）⇒ 保留

### ③ 「跳过行显示【安装】入口，不显示为核验失败」

**整组迁** → `ProviderCapabilityCard.test.tsx`「keeps the install entry for a skipped batch item (not a probe failure)」。

- a. pi 状态「跳过」⇒ 迁到行 `provider-capability-provider-pi` 内
- b. 行内无「核验失败」⇒ 保留（同一行内查询）
- c. 行内有【安装】按钮 ⇒ 保留
- d. 行内有可抄手动命令（`npm install -g`）⇒ 保留（`provider-install-manual-command-pi`）
- e. 行内无【重试】⇒ 保留

### ④ 「终态后停止轮询，不无限轮询」

**原地保留**（`ProviderCapabilityBatch.test.tsx`）：验的是 hook 的轮询停机与按钮回态，
与逐项结果组件无关——组件删除不影响该行为。

### ⑤ 「版本落后行渲染「版本落后」+【更新】与当前/最新版本（不是【安装】）」

**整组迁** → `ProviderCapabilityCard.test.tsx`「keeps the version-gap entry (版本落后 / 更新 / 当前→最新, not 安装) in the row list」。

- a. 行内「版本落后」⇒ 断言对象从 `capability-batch-outdated-codex`（批量结果行专属）
  换成行级列表的 `provider-capability-outdated-codex`，**文案与位置断言原样**
- b. 行内有【更新】按钮 ⇒ 保留
- c. 版本区含 `1.0.0` 与 `2.0.0` ⇒ 保留（`provider-install-versions-codex`，数字原样）
- d. 行内无【安装】按钮 ⇒ 保留

### ⑥ 「按钮区说明核验作用域与成本」

**原地保留**（`ProviderCapabilityBatch.test.tsx`）：验的是 `ProviderCapabilityBatchScope`
组件本身，与逐项结果组件无关——组件删除不影响该行为。

### 小结

| 原用例 | 去向 |
|---|---|
| ① 逐项状态（c/d） | 迁 → 行级列表 |
| ② 失败项【重试】 | 迁 → 行级列表 |
| ③ 跳过项【安装】 | 迁 → 行级列表 |
| ④ 终态停轮询 | 原地保留（hook 面） |
| ⑤ 版本落后【更新】 | 迁 → 行级列表 |
| ⑥ 按钮区说明 | 原地保留（Scope 组件面） |

**没有任何一条断言被直接丢弃或改松**：①③④⑤⑥ 中每一条原始 `expect` 都以等价
（部分更强）形式存在于新位置。

---

## 4. 残留引用复查

`web/src` 下 `grep -rn "ProviderCapabilityBatchResults"` 的剩余命中**全部是注释/说明文字**，
无任何 import、无任何调用点：

- `ProviderCapabilityBatch.tsx:11` — 文件头注释里说明「原 `ProviderCapabilityBatchResults` 逐项结果组件已删除」
- `ProviderCapabilityBatch.test.tsx:8` — 文件头注释里说明断言迁往何处
- `ProviderCapabilityCard.test.tsx:545` — 迁入用例前的注释里说明来源
- `ProviderCapabilityCard.test.tsx:540` — 负向守卫
  `expect(screen.queryByTestId("provider-capability-batch-results")).toBeNull()`
  （保留：它是「面板只有一组行级列表、逐项结果组件不得复活」的守门断言；
  同用例另有 `expect(screen.getAllByRole("listitem")).toHaveLength(4)` 佐证只有一组列表）

`web/src` 下已无 `capability-batch-row-*` / `capability-batch-outdated-*` 引用
（这两个 testid 属被删组件，已随组件消失）。

---

## 5. 验证输出

工作目录：`/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02/web`

### 5.1 typecheck

```
$ pnpm -s typecheck
TYPECHECK_EXIT=0
```

### 5.2 lint

```
$ pnpm -s lint
…（仓库既有 warning，均不在本次改动文件上）
LINT_EXIT=0

$ pnpm -s lint 2>&1 | grep -cE "ProviderCapabilityBatch|ProviderCapabilityCard"
0        # 改动文件 0 条新增告警
```

### 5.3 定向测试（改动的两个测试文件）

```
$ pnpm vitest --run src/components/lifecycle/ProviderCapabilityBatch.test.tsx \
                  src/components/lifecycle/ProviderCapabilityCard.test.tsx
 ✓ src/components/lifecycle/ProviderCapabilityBatch.test.tsx (3 tests) 74ms
 ✓ src/components/lifecycle/ProviderCapabilityCard.test.tsx (18 tests) 257ms

 Test Files  2 passed (2)
      Tests  21 passed (21)
```

### 5.4 全量测试（完整计数，未截断）

```
$ pnpm -s test

 Test Files  262 passed (262)
      Tests  2683 passed (2683)
   Start at  23:03:03
   Duration  34.77s (transform 11.47s, setup 53.32s, collect 80.04s, tests 154.27s, environment 144.34s, prepare 22.59s)
```

---

## 6. 被改文件行数

| 文件 | HEAD 行数 | 现行数 | +新增 | -删除 |
|---|---:|---:|---:|---:|
| `web/src/components/lifecycle/ProviderCapabilityBatch.tsx` | 415 | 282 | 11 | 144 |
| `web/src/components/lifecycle/ProviderCapabilityBatch.test.tsx` | 407 | 239 | 16 | 184 |
| `web/src/components/lifecycle/ProviderCapabilityCard.test.tsx` | 883 | 1127 | 244 | 0 |
| 合计 | 1705 | 1648 | 271 | 328 |

（`git diff --numstat -- web/` 输出：`16 184 ProviderCapabilityBatch.test.tsx`、
`11 144 ProviderCapabilityBatch.tsx`、`244 0 ProviderCapabilityCard.test.tsx`；
`ProviderCapabilityCard.tsx` 不在列表中 ⇒ 未改动。）

---

## 7. 未做的事 / 边界

- 未改 `ProviderCapabilityCard.tsx` 的行级列表渲染逻辑（只补断言、换断言对象位置）。
- 未改 `src/**`、`openspec/**`、`spec/delta/tasks.md`、`cadence/**`（本报告除外）。
- 未做任何 git 操作（无 add / commit / stash / checkout / reset）。
- `cadence/notes/2026-11-04_前端_一键核验按钮位置与添加代码库布局说明_v1.0.md` 里对
  该组件的历史提及**保持原样**（notes 非本次授权范围，且记录的是历史布局决策）。
