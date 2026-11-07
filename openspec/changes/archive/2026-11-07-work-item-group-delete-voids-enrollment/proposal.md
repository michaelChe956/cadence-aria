# 删除工作项组时，自动化登记一并作废（REQ-WIGA-VOID-01）

> **本 change 记录 2026-11-06 owner 真机撞到的缺陷 + owner 亲自给出的业务语义裁决。**

## Why

**真机现象（owner 2026-11-06，开发期 5173）**：

```
同一 issue 上：建 WIG#1 → 删掉 WIG#1 → 建 WIG#2
⇒ 在「启用自动化」弹窗里把 Reviewer Provider 从 pi 改成 codex
⇒ 后端 409 automation_enrollment_conflict
⇒ 前端文案：「重新开启的自动化授权与终止记录不一致（原授权源仍有效但与当前不同）：
              请先通过自动化控制面执行『换代 / 重新绑定（rebind）』更新授权源，或恢复原授权源后再重新开启」
```

**根因（磁盘实证 + 代码定位）**：

```
· 自动化登记按 (project, issue) 唯一（src/product/issue_automation_store.rs:55-66），
  模型里没有 WIG 主键，只有 plan_id / session_id 指向某个 WIG
· 删除 WIG 的处理器只调 terminate_plan：写 enabled=false / terminated_at / revision+1，
  ★source / target / binding / plan_id 原样保留★
  ⇒ 终止记录仍指向已删的 story/design ⇒ 被判「悬空源」
· 悬空源路径上 options（含 provider）漂移被拒
  （src/product/issue_automation_store_parts/cas_internals.inc.rs:730-739：
   「options 漂移仅在悬空源重授权路径构成拒绝条件」）
⇒ ⇒ 换 provider 被堵死。
```

**owner 的业务语义裁决（2026-11-02 亲口 + 2026-11-06 补充）**：

```
「删了就是全部都删了。想换 provider 也一样，删了重来一遍，不要做接续。」
「当前这个已终止 ⇒ 必须先删掉它，才能开新的 WIG 并切 provider。」
「★删掉 work item group 就需要删掉对应的登记吗？不应该影响新的 work item group 的执行呀★」
⇒ ⇒ 自动化登记是挂在那个 WIG 上的；WIG 删了 ⇒ 登记不该被下一个 WIG 继承。
⇒ ⇒ ⇒ 「换 provider 被堵死」的根因不是规则冲突，而是【该跟着 WIG 一起作废的登记没作废】。
```

**为什么一直没人发现**：归档 change `2026-11-03-fix-automation-enrollment-provider-swap`
的 9 条真机验收（9.1~9.8 与 7.8）★一条都没执行过★ ⇒ 悬空源这条路径从未被验证。

## What Changes

```
① 自动化登记新增【作废】状态（voided_at + void_reason），不物理删除
   —— 保留账本留痕（只追加不清空）、保留 REQ-DRG-18 的「销账 MUST NOT 删除 enrollment 记录」
② 删除工作项组时，把该 issue 的自动化登记标成已作废（锁内、revision+1、账本追加一条、前代进 previous）
③ 新工作项组重新启用时：若旧登记已作废 ⇒ provider / source / target 全部不再逐字段比对 ⇒ 换 provider 一次完成
④ 界面如实显示「该自动化登记已随旧工作项组删除作废」（不再一律显示「已于 X 终止」）
⑤ 契约改动：
   · REQ-DRG-18 增加「作废」语义，并改写其最后一条 Scenario（悬空源只放行 source）
   · 新增 REQ-WIGA-VOID-01（删除 ⇒ 作废；作废 ⇒ 放行全部漂移；界面提示）
```

## 不做（owner 已明确裁定）

```
✗ 「WIG 在自动化执行中 ⇒ 不能删除」按「拒绝」实现 —— ★保持现状★：
  删除时自动终止绑定该 WIG 的自动化（失败才 409 automation_termination_failed）
✗ 不物理删除自动化登记（issue 级别的一张账，保留只读）
✗ 不推翻「悬空源只放行 source」这条判定本身（判定口径本身合规：REQ-DRG-18「悬空只判存在性」）
```

## Impact

- **数据**：登记模型新增 `voided_at` / `void_reason` 两个字段（可空）；已有记录不变。
- **后端**：`src/product/issue_automation_store.rs`（模型 + 新增一条锁内写命令）、
  `src/product/issue_automation_store_parts/cas_internals.inc.rs`（作废记录的漂移放行）、
  `src/web/handlers/lifecycle/deletion.rs`（删除后调用作废）。
- **前端**：登记状态多一个「已作废」 ⇒ 弹窗与门卡文案区分「已终止」/「已作废」。
- **契约**：`dangling-reference-governance`（REQ-DRG-18 MODIFIED）+ `work-item-group-autopilot`（REQ-WIGA-VOID-01 ADDED）。
- **风险**：作废后旧登记不可再被复用为约束 ⇒ 换 provider 放行范围变大 ⇒ 必须**只**在「已作废」这一状态下放行，且作废只能由「删除工作项组」这一条路径产生。
