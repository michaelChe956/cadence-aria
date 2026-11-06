# Tasks · 删除工作项组时自动化登记一并作废（REQ-WIGA-VOID-01）

> 契约：REQ-WIGA-VOID-01（ADDED，work-item-group-autopilot）+ REQ-DRG-18（MODIFIED，dangling-reference-governance）
> 设计：`design.md`（含「唯一的作废产生路径」与「唯一的漂移放宽点」两条硬边界）
> owner 裁决（2026-11-06）：登记是 issue 级 ⇒ **不删，只作废**；「执行中禁删」**保持现状**；换 provider 一次完成。

## 1. 数据模型与登记的作废态

- [x] 1.1 `src/product/models/automation.rs`：登记模型新增 `voided_at: Option<String>` 与 `void_reason: Option<String>`（可空；序列化兼容：缺字段按未作废读）；`terminated_at` 与 `voided_at` 是**两个独立维度**，SHALL NOT 互相推导
      - 验收：旧记录反序列化零报错；两个字段各自独立读写；无「有 terminated_at 就推断 voided_at」的代码路径
- [x] 1.2 `src/product/models/automation.rs`（或等价类型处）：登记的「状态投影」SHALL 能区分【在跑 / 已终止 / 已作废 / 已终止且已作废】四种，SHALL NOT 只靠 `terminated_at` 布尔判断
      - 验收：四种组合各有确定的投影值与测试

## 2. 唯一的作废产生路径（store 锁内写命令）

- [x] 2.1 `src/product/issue_automation_store.rs` + `command_writes_impl.inc.rs`：新增 `void_enrollment_for_deleted_plan(project_id, issue_id, plan_id, reason)`
      - 语义：不存在 ⇒ `NotFound`（**幂等**）；已作废 ⇒ `AlreadyVoided`（**不重复追加账本**）；绑定 plan 不匹配 ⇒ `NotBoundToPlan`（**不得作废别人的登记**）；成功 ⇒ 写 `voided_at`/`void_reason`、`revision`+1、绑定前代进 `binding_history.previous`、**命令账本追加一条**（带 reason 与被删 plan_id）
      - 硬边界：SHALL NOT 删除 `automation-enrollment.json`；SHALL NOT 清空或改写既有 history 与账本条目
      - 验收：四条分支各有单测；成功路径断言「字段已写 + revision+1 + previous 多了前代 + 账本多一条且旧条目逐字未变」
- [x] 2.2 作废的**唯一产生路径**自检：全仓检索 SHALL NOT 出现第二条能写 `voided_at` 的代码路径（除本命令与测试夹具）
      - 验收：检索结果只有 store 命令一处写入

## 3. 唯一的漂移放宽点（CAS）

- [x] 3.1 `src/product/issue_automation_store_parts/cas_internals.inc.rs`（漂移分治处）：**新增前置判定** —— 被取代的登记已作废 ⇒ `options`（含 provider）/ `source` / `target` 全部放行，按换代语义（身份轮换、绑定换代、旧代进历史、账本留痕）
      - 硬边界：★未作废登记的分治逻辑一个字都不许改★（含「悬空只放行 source」「target 两路都拒」）
      - 验收：作废记录 + 三项全漂移 ⇒ 接受并完成换代；★未作废记录的判定逐条与改动前一致（回归锁：同一组夹具在改动前后结论必须相同）
- [x] 3.2 作废态下重新开启 SHALL NOT 复活旧 plan（沿用既有取代语义：旧 plan 不复活、`prepare_intent_id` 轮换、旧 plan 全闭包物理删除）
      - 验收：断言旧 plan 仍被物理删除、新 `prepare_intent_id` 已轮换

## 4. 删除路径接入

- [x] 4.1 `src/web/handlers/lifecycle/deletion.rs`：删除工作项组**成功之后**调用 `void_enrollment_for_deleted_plan(project_id, issue_id, 被删 plan_id, "work_item_group_deleted")`
      - ★「执行中禁删」那一段（自动终止绑定该 WIG 的自动化）保持现状不改★（owner 裁决）
      - ★作废失败 MUST NOT 回滚已完成的删除★ ⇒ 记录可诊断事实，MUST NOT 静默吞掉
      - 验收：删除后登记被标作废；删除一个没有登记的 issue ⇒ 不报错；作废失败场景下删除仍已完成且有可诊断事实
- [x] 4.2 删除与作废的**顺序**：作废 SHALL 在删除**成功之后**发生（失败/未删成 ⇒ 不得作废）
      - 验收：删除失败 ⇒ 登记未作废

## 5. 前端

- [x] 5.1 `web/src/api/types/*`（登记类型）：新增 `voided_at` / `void_reason`
- [x] 5.2 `web/src/components/lifecycle/WorkItemPlanOptionsDialog.tsx`：登记已作废 ⇒ 显示「该 issue 的自动化登记已随旧工作项组删除作废；确认后将重新开启」；已终止 ⇒ 保持现有「已于 X 终止；确认『启用自动化』将重新开启」
      - 验收：两种状态的文案可区分（各有测试）；既有终止文案逐字未变
- [x] 5.3 `web/src/components/lifecycle/useIssueLifecycleGeneration.ts`：作废态下的 409 文案分流 SHALL NOT 出现 rebind 文案（正确文案是「重新开启即可自由选择 provider」）
      - 验收：作废态提交不出现 rebind 文案；非作废态的既有分流逐字未变
- [x] 5.4 其余读侧（`CockpitInboxWaitingCards.tsx` / `CodingWorkspacePage.tsx` / `coding-workspace-store.ts` 等）：只在【会把作废态读成终止态】的地方区分；纯读 `enabled` 的读侧可不改
      - 验收：逐处判定并在报告里列出「改了 / 不改 + 理由」

## 6. 契约与文档

- [x] 6.1 契约 delta 已写（REQ-WIGA-VOID-01 ADDED + REQ-DRG-18 MODIFIED），`openspec validate --strict` 通过
- [x] 6.2 交付/实施报告落 `cadence/reports/`，写清：owner 现象、根因链（磁盘实证 + 代码位置）、实现范围、未做项（不删登记 / 执行中禁删不改 / 悬空判定不改）
- [ ] 6.3 真机验收（★这次必须执行★，owner 2026-11-06 明确要求）：
      - [ ] 6.3.1 建 WIG → 删 WIG → 同 issue 新建 WIG → 以**不同** provider 启用自动化 ⇒ 一次完成，不报 rebind 文案
      - [ ] 6.3.2 弹窗显示「已随旧工作项组删除作废」
      - [ ] 6.3.3 登记未被物理删除（磁盘可查 `automation-enrollment.json` 仍在 + 有 `voided_at` + 账本多一条）
      - [ ] 6.3.4 换完后跑一轮完整生成 ⇒ 产物由**新** provider 产出；后台日志无 reconcile 持续冲突
      - [ ] 6.3.5 未作废的记录语义不变（回归：正常终止态换 provider 仍按既有规则处理）

## 7. 验收纪律

- [x] 7.1 TDD：先写失败测试（store 四分支 + CAS 放行 + 删除接入 + 前端两种文案），再实现
- [x] 7.2 既有断言**一条不许删/缩窄**；既有文案逐字不变
- [x] 7.3 `cargo fmt --check` + `cargo clippy --all-targets --all-features --locked -- -D warnings` + `cargo test --locked --all-targets` + `openspec validate --specs --strict` + `pnpm -C web test` + `pnpm -C web tsc -b` 全绿
- [x] 7.4 行数门禁：新增/改动文件行数在限内（`tests/it_core/large_file_guard.rs`）
