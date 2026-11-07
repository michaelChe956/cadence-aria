# 设计 · 删除工作项组时自动化登记一并作废（REQ-WIGA-VOID-01）

## 0. 为什么要「作废」而不是「删除」

```
登记是【按 (project, issue) 唯一】的一张（src/product/issue_automation_store.rs:55-66）
⇒ 新工作项组没有自己的登记 ⇒ 只能复用同一张 ⇒ 必须有「这代登记已作废」的表达
⇒ ★owner 已裁决：不物理删除★（保留账本留痕 + 满足 REQ-DRG-18「销账 MUST NOT 删除 enrollment 记录」）
⇒ ⇒ 作废 = 新增一个 durable 标记 + 账本追加一条 + 绑定前代进历史
```

## 1. 数据模型

`src/product/models/automation.rs`（登记模型）新增两个可空字段：

| 字段 | 含义 | 约束 |
|---|---|---|
`voided_at` | 作废时刻（RFC3339） | 有值 ⇒ 该登记已作废；SHALL NOT 与 `terminated_at` 同时为「无终止态」而并存地被读成「在跑」 |
`void_reason` | 作废原因（机器码，如 `work_item_group_deleted`） | 只追加不改写 |

**读侧口径**：作废态与终止态是两个独立维度 ⇒
- 「终止」= 这一代跑完了/被终止了（`terminated_at`）
- 「作废」= 这一代连同它的工作项组一起没了（`voided_at`）
⇒ ⇒ 界面两者都要能显示，但**作废优先**（更贴近 owner 的心智：连组一起没了）。

## 2. 新增一条锁内写命令（唯一的作废产生路径）

`src/product/issue_automation_store.rs` + `src/product/issue_automation_store_parts/command_writes_impl.inc.rs`

```
void_enrollment_for_deleted_plan(project_id, issue_id, plan_id, reason) -> VoidOutcome
```
语义（锁内、原子）：
1. 读当前登记；不存在 ⇒ `NotFound`（**幂等**：删 WIG 时没有登记是正常情况）
2. `voided_at` 已存在 ⇒ 幂等返回 `AlreadyVoided`（**不重复追加账本**）
3. 校验该登记确实绑定着被删的 plan（`plan_id` 一致）⇒ 不一致 ⇒ `NotBoundToPlan`（**不得作废别人的**）
4. 写 `voided_at` / `void_reason`；`revision` +1；**绑定前代进 `binding_history.previous`**；
   **命令账本追加一条**（kind = 作废，带 reason 与被删 plan_id）
5. **SHALL NOT** 删除 `automation-enrollment.json`；SHALL NOT 改写既有 history 与账本条目

## 3. 重新开启时的放行（唯一的漂移放宽点）

`src/product/issue_automation_store_parts/cas_internals.inc.rs`（`:724-745` 的漂移分治处）

```
现状：
  options 漂移 ⇒ 仅【非悬空源】放行（换代），悬空源拒
  target  漂移 ⇒ 两路都拒
  source  漂移 ⇒ 仅【悬空源】放行
改动：
  ★新增前置判定★：若被取代的登记【已作废】⇒ options / source / target 全部放行（按换代放行）
  ⇒ 作废记录走的是「换代」语义（身份轮换、绑定换代、旧代进历史、账本留痕）⇒ 与 REQ-WIGA-SWAP-01 同一条路径
  ⇒ ★未作废的登记分治逻辑一个字都不许改★（含「悬空只放行 source」）
```

**放行边界（必须严守）**：
```
放行只在「该登记已作废」这一状态下发生
⇒ 未作废 ⇒ 一律按现状判定
⇒ 作废只能由「删除工作项组」产生 ⇒ 不得由任何其它路径顺带产生
⇒ ⇒ 换 provider 一次完成（REQ-WIGA-SWAP-01）在作废路径上自然成立，无需改它
```

## 4. 删除路径的调用点

`src/web/handlers/lifecycle/deletion.rs`（删除工作项组主流程，`:152` 入口 / `:716-742` 落盘段）

```
删除工作项组成功之后 ⇒ 调用 void_enrollment_for_deleted_plan(project_id, issue_id, 被删 plan_id, "work_item_group_deleted")
⇒ 失败处理：★作废失败 MUST NOT 回滚已完成的删除★（owner 语义「删了就是全部都删了」）
   ⇒ 记录可诊断事实（错误面/日志），MUST NOT 静默吞掉
⇒ ★「执行中禁删」那段（自动终止绑定该 WIG 的自动化）保持现状不改★（owner 已裁决）
```

## 5. 前端

```
读侧：web/src/api/types/automation.ts（或等价类型文件）新增 voided_at / void_reason
展示面（web/src/components/lifecycle/WorkItemPlanOptionsDialog.tsx）：
  · 登记已作废 ⇒ 显示「该 issue 的自动化登记已随旧工作项组删除作废；确认后将重新开启」
  · 登记已终止 ⇒ 保持现有「已于 X 终止；确认『启用自动化』将重新开启」
展示面（其余读侧：useIssueLifecycleGeneration.ts / CockpitInboxWaitingCards.tsx /
          CodingWorkspacePage.tsx / coding-workspace-store.ts 等 8 处）：
  · 只在【会把作废态读成终止态】的地方需要区分；纯读 enabled/terminated 的读侧可不动
前端提交面：作废态下提交 Enable ⇒ 【SHALL NOT】再要求 provider 与旧记录一致（与后端放行口径同源）
文案面：409 的 rebind 文案（useIssueLifecycleGeneration.ts:73-75）在作废态下【SHALL NOT】出现
  ⇒ 作废态的正确文案是「重新开启即可自由选择 provider」
```

## 6. 测试纪律

```
Rust（先红后绿）：
  · store：void_enrollment_for_deleted_plan 的四条行为（不存在⇒NotFound / 已作废⇒幂等 /
            非绑定 plan⇒NotBoundToPlan / 成功⇒写字段+revision+1+前代进 previous+账本追加一条）
  · CAS：作废记录 + options/source/target 全漂移 ⇒ 接受并完成换代
  · CAS：★未作废记录 ⇒ 分治逻辑与改动前逐字一致（回归锁）
  · deletion：删除工作项组 ⇒ 登记被标作废；删除无登记的 issue ⇒ 不报错（幂等）
  · deletion：★作废失败时删除仍已完成★（不回滚）
前端：
  · 弹窗在作废态显示「已随旧工作项组删除作废」，在终止态显示「已于 X 终止」（两者可区分）
  · 作废态下选不同 provider 提交 ⇒ 不再出现 rebind 文案
  · 既有断言一条不许删/缩窄
```

## 7. 已知边界（不做）

```
✗ 不物理删除登记（owner 裁决）
✗ 不改「执行中禁删」的现状（owner 裁决）
✗ 不改悬空判定口径（只判存在性，REQ-DRG-18）
✗ 不改 REQ-WIGA-SWAP-01 / REQ-WIGA-02 的既有语义
✗ 不改「销账 MUST NOT 删除 enrollment 记录」
```
