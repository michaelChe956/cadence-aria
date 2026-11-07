# 终止回执的身份限定与换代失效（REQ-WIGA-14）

> **本 change 记录 2026-11-06 owner 真机复现的缺陷 + 审计判定。**
> 审计报告：`cadence/reports/2026-11-06_审计_删除workitem与自动化enrollment_设计vs实施_v1.0.md`（判定：**设计缺口**，非实施不合理）
> 诊断报告：`cadence/reports/2026-11-06_诊断_终止状态跨workitem泄漏_v1.0.md`

---

## 1. 现象（owner 真机）

```
① 启动一个 Work Item 自动化（正常跑）
② 中途「终止整个自动化」
③ 删除该 work item
④ 在同一 issue 上新建一个 Work Item 自动化
⇒ 新的那个界面显示「自动化已终止 <旧终止时刻> 本计划全部仓库已停止，不可继续；已产出文件保留。」
⇒ 而它的时间线实际推进到 Review Round 2 并全部 completed（14:54 最后写入）
⇒ ⇒ owner 读作「一直卡着不往下走」——★ 是【过期横幅误导】，不是真的卡住
```

## 2. 根因（诊断实证）

- 终止回执只存 `(project_id, issue_id, terminated_at, outcome)`（`coding-workspace-store.ts:591-600`），**REST 响应里本来就有的 `plan_id` 没进回执**（`client.ts:843`）
- 展示守卫只按 `(projectId, issueId)` 匹配（`CodingWorkspacePage.tsx:176-181`、`ChatCockpitPage.tsx:854-860`）⇒ **无 plan/session 身份比对**
- 唯一清理点 `reset()` 依赖 `[attemptId, issueId, projectId]` 不变 ⇒ 永不触发；cockpit 页甚至不挂该 hook
- **后端已证无泄漏**：磁盘上 `terminated_at` 字段不存在、旧终止时间戳零命中、`plan_id`/`session_id` 已换绑新值（`cas_internals.inc.rs:665,756` 的 `supersede_terminated_enrollment`）

## 3. 审计判定：设计缺口

| 侧 | 设计原文 | 判定 |
|---|---|---|
删除 | `dangling-reference-governance/spec.md:527`「销账 MUST NOT 删除 enrollment 记录」；`work-item-group-autopilot/spec.md:693-695` REQ-WIGA-09 终止后删除流程**零副作用** | ★ 有意语义，实施忠于设计 |
新建 | `work-item-group-autopilot/spec.md:700-702`「以**同 enrollment_id** 取代落盘：terminated_at 清空……必须轮换 prepare_intent_id」 | ★ 设计明写「同 id 换代」，**不是**开新 enrollment |
**回执/展示** | 全部 specs + 两份 11-03 归档 delta 检索「planTermination / 终止回执 / 横幅」⇒ **零命中** | ★★ **条款不存在本身即缺口** |

⇒ ⇒ **owner 记忆里的「删除 = 彻底删除」在设计语境里只指 WIG 闭包**（块1 设计 `:402`），后端从未打算删 enrollment。

## 4. 为什么一直没被发现

归档 tasks 的**真机验收项 9.2 / 9.3（终止 → 重新启用 → 走通）从未执行**、7.8 未勾 ⇒ ⇒ ⇒ **本 change 把 9.2 / 9.3 / 7.8 纳入验收，不再留未勾项。**

## 5. 范围

```
改：web/src/.../coding-workspace-store.ts（回执带 plan_id/binding_version）
改：web/src/pages/CodingWorkspacePage.tsx、web/src/pages/ChatCockpitPage.tsx（守卫加身份比对 + 缺身份 fail-closed）
测试：ChatCockpitPage.automation-termination.test.tsx / CockpitInboxWaitingCards.termination.test.tsx / coding-workspace-store.test.ts
不动：后端任何东西；REQ-WIGA-09 与 DRG 的 tombstone 条款；既有终止横幅文案与语义
```
