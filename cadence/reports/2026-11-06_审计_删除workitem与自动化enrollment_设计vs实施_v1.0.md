# 审计：删除 work item 与自动化 enrollment —— 设计 vs 实施

- 日期：2026-11-06 · 版本 v1.0 · 性质：**只读审计**（未改任何产品代码/文档、未提交、未跑 cargo；不碰 `/tmp/aria-verify` 与 `/tmp/aria-deploy`）
- 审计问题：owner 记忆的目标是「**删除 work item = 彻底删除它及所有相关自动化内容；重新新建 = 开新的**」；现问题 = 「终止 → 删除 → 新建 ⇒ 新 work item 继承旧『已终止』状态而卡住」。判定这是【设计不合理/不完整】还是【实施漏做/做偏】。
- 文档与代码基线：`.worktrees/bugfix-b-11-02` @ `b94a58fb`（下文所有路径均相对该 worktree；cwd 根下 `cadence/designs/`、`cadence/plans/` 为空，11-02/11-03 文档实际位于该 worktree）
- 运行时证据引用：同日诊断报告 `cadence/reports/2026-11-06_诊断_终止状态跨workitem泄漏_v1.0.md`（只读取证 `/tmp/aria-verify`）

## 0. 结论速览

**判定：设计缺口（设计不完整），不是实施漏做/做偏。**

- 「删除侧」设计条款**明文**要求：删除实体时 enrollment 只**终止/失效、永不删除**（tombstone 留存是有意语义）；「重新打开 = 同 `enrollment_id` 取代换代、必须轮换 `prepare_intent_id`、建立全新 plan/session」——实施（后端三处点名代码 + 运行时证据）**逐条忠于这些条款**。
- 「新建侧隔离」设计只写到**持久层身份**（intent 轮换、binding 换代、plan 精确匹配门）；**没有任何 requirement 覆盖「前端终止回执（`planTermination.terminated`）的生命周期」**——而运行时证据证明泄漏恰恰发生在这一层（终止回执只按 `(project_id, issue_id)` 粗键留存与渲染，换代后不失效）。漏掉的正是这一条（**当前无编号 —— 缺陷在于条款不存在**）。
- owner 记忆目标中「删除 = enrollment 记录也彻底消失」与既有设计的「tombstone + 同 id 换代」语义**不一致**，但后端以「全闭包物理删除 + 留痕换代」实现了语义等价的「重建 = 新的」；owner 感知到的「继承已终止」来自前端回执残留，不需要推翻既有设计即可修复。

## 1. 审计对象

| 类别 | 文档 |
|---|---|
| 设计 | `cadence/designs/2026-11-02_技术方案_块1_自动化授权换代一步完成_v1.0.md`（425 行）；`cadence/designs/2026-11-02_技术方案_块3_去单仓入口与统一多仓流程_v1.0.md`（430 行） |
| 计划 | `cadence/plans/2026-11-03_计划文档_实施_块1_自动化授权换代与人工门文案_v1.0.md`（1608 行）；`cadence/plans/2026-11-03_计划文档_实施_块3_去单仓入口与统一多仓流程_v1.0.md`（1149 行） |
| 问题源 | `cadence/notes/2026-11-02_排期_全量问题清单与优先级排期_v1.0.md`（S1/S8 等 18 组） |
| 归档 change | `openspec/changes/archive/2026-11-03-fix-automation-enrollment-provider-swap/`（proposal 128 / design 149 / tasks 154 / 两份 delta spec）；`openspec/changes/archive/2026-11-03-remove-single-repository-path/`（8 份 delta + tasks） |
| 关联权威 spec（同步后） | `openspec/specs/work-item-group-autopilot/spec.md`（918 行，REQ-WIGA-09 @ :656、REQ-WIGA-SWAP-01 @ :863）；`openspec/specs/work-item-plan-single-instance/spec.md`（REQ-WPS-01/02）；`openspec/specs/work-item-group-deletion/spec.md`（REQ-GRP-01）；`openspec/specs/dangling-reference-governance/spec.md`（销账条款）——后两者是真正管辖「删除实体时 enrollment 怎么办」的条款来源（源自 2026-10-28 归档 change `add-dangling-reference-governance` 与 `harden-work-item-group-deletion` 系） |

## 2. Q1｜设计的规范条款原文：「删除实体时对绑定的 automation enrollment 做什么」

### 2.1 块1 设计/归档 delta 的条款（终止 + 取代重开，**不是删除记录**）

1. `openspec/specs/work-item-group-autopilot/spec.md:658`（REQ-WIGA-09 正文，源自 2026-11-03 归档 delta `specs/work-item-group-autopilot/spec.md:103`）：
   > 「plan 与 attempt 终止不可恢复。**terminated SHALL 阻断全部自动路径，且 SHALL 只可由人工显式 Enable 以同 `enrollment_id` 新 `policy_revision` 取代重开（清 `terminated_at`、解绑旧 plan/session、账本留痕）**；被终止 plan 与 attempt 不复活。」

2. `…/work-item-group-autopilot/spec.md:691-695`（Scenario「terminated 记录阻断一切自动路径」，归档 delta `:136-140`）：
   > 「**WHEN** enrollment 已 terminated 后 reconcile/自动生成/自动首启/续驱/重启补偿扫描运行，**或绑定实体删除流程再次运行** / **THEN** 一律零副作用：不写 enrollment、不建 plan/session……**MUST NOT 代发 Enable（重新开启唯一入口为人工 `PUT`）**」
   ⇒ 删除流程对 terminated enrollment 的规定动作是「什么都不做（保留）」。

3. `…/work-item-group-autopilot/spec.md:697-702`（Scenario「terminated 记录经人工 Enable 重新开启（取代并留痕）」，归档 delta `:142-147`）：
   > 「**THEN** 系统以**同 `enrollment_id`**、`policy_revision=n+1` 取代落盘：`terminated_at` 清空、`enabled=true`、`plan_id`/`session_id` 为空、`binding_history` 新代（旧代进 `previous`、`binding_version` 递增……）/ **AND** 随后既有 prepare 路径建立**全新** plan/session 并绑定 / **AND** 同 `enrollment_id` 取代**必须轮换 `prepare_intent_id`**（`plan_id`/`session_id` 由其派生；沿用会让 prepare 幂等复用被终止的旧 plan 实体）」

4. `…/work-item-group-autopilot/spec.md:714-718`（归档 delta `:161-163`）：
   > 「**AND** 绑定实体**已被删除**（墓碑）且**无未终结 attempt** 时视为无可收口之物，**允许**人工重新开启（不因“清理任务失败”本身而永久阻断）」
   ⇒ 设计**预见了**「终止 → 删除实体 → 重新开启」这条链路，处置是「墓碑放行人工重开」，不是「删除 enrollment」。

5. `…/work-item-group-autopilot/spec.md:863-867`（REQ-WIGA-SWAP-01 正文，归档 delta `:189-191`）：
   > 「对**已终止且无法继续**的自动化授权，owner SHALL 能通过**一次操作**完成「**删除旧工作项组 → 以新 provider 从零重建**」……该操作 SHALL 保持既有换代留痕（授权身份轮换、绑定换代、旧代进历史、命令账本留痕、**旧工作项组全闭包物理删除**），且 SHALL NOT 残留旧 provider 的绑定身份、生成意图或半成品产物。」
   ⇒ 「删除」的范围 = **工作项组（WIG/plan 及其闭包）**，**不含 enrollment 记录**。

6. 块1 设计文档对「删除」的定位——`cadence/designs/2026-11-02_技术方案_块1…v1.0.md:58-70`：
   > 「### 1.2 『删除旧 WIG』不是副作用，是换 provider 的前提 …… 当前 WIG 已终止且无法继续 → **必须先删掉它**，才能开新的 WIG 并切 provider …… 删除 issue 下全部存活 plan……**物理删除** attempts/worktree/shared-worktree/reviewer 资源与 plan 根，只留一条无内容删除事实。」
   以及 `:402`（owner 裁定）：
   > 「换 provider 时旧 WIG 保留还是作废 | **全部删除**（owner：**「删了就是全部都删了」**）。代码已实现物理删除，无需改动」
   ⇒ owner 这句「都删了」在文档语境里指 **WIG 闭包**（plan/attempts/worktrees/reviewer），**没有条文把 enrollment 记录纳入删除范围**。

### 2.2 真正管辖「删除实体 × enrollment」的条款：DRG（悬空引用治理）与 WIG 删除闭包

7. `cadence/designs/2026-10-27_技术方案_悬空引用治理与读侧容错_v1.0.md:120`（对应归档 change `2026-10-28-add-dangling-reference-governance/design.md:90`）：
   > 「删除 story/design 时**已终止**绑定该实体的 automation enrollment（逻辑失效）……但**失效 ≠ 删除**：enrollment 文件仍在」
   `:143`：
   > 「自动化 enrollment 被「终止 / 失效」**而非删除**……→ **文件仍在** → 其 `source` 字段继续指向已删实体」
   ⇒ **删除时是 terminate，不是删除记录 —— 这是被明文写死的设计语义**（且 `design.md:90` 注明该终止行为当时「已存在」：`deletion.rs:532/:569`）。

8. `openspec/specs/dangling-reference-governance/spec.md:527`：
   > 「自动化 enrollment 记录在 story/design 被删后 source 指向已删实体，属于在途变更覆盖的已知状态。巡检 MUST NOT 将其判为未归类；**销账 MUST NOT 删除 enrollment 记录。系统 MUST NOT 重新裁定 enrollment 的重新授权语义。**」
   同设计 `:1062`：「销账**永不**删除 `automation-enrollment.json`」。

9. `openspec/specs/work-item-group-deletion/spec.md:29`（REQ-GRP-01）与 `:67-69`：
   > 「系统 SHALL 在 legacy 与 schema-v2 Plan/group 永久删除时物理清除该 Plan 拥有的主记录、全部会话及附属内容……**只留无内容事实**」（闭包清单逐项枚举，**不含 enrollment**）；「删除 MUST 只影响该 group 的产物，MUST NOT 删除 issue 本身……」

10. `openspec/specs/work-item-plan-single-instance/spec.md:31-38`（REQ-WPS-02，三个建立触发点删旧 plan）：
    > 「(a) 自动化 enrollment 在 terminated 后经人工 Enable 重新开启并**轮换 `prepare_intent_id`**（旧 plan/session 由旧 intent 派生，沿用会让 prepare 幂等复用被终止的旧实体）；(b) 重新 prepare；(c) 重新编译确认……」

### 2.3 Q1 小结（逐条回答审计问题）

- **删除时是 terminate 还是删除记录？** —— **terminate/逻辑失效，明文永不删除记录**（条款 2/7/8/9）。
- **有没有写「删除后 enrollment 必须消失」？** —— **没有，且反向禁令存在**（条款 8：「销账 MUST NOT 删除 enrollment 记录」）。
- **有没有写「重新新建必须开新 enrollment」？** —— **没有**。写的是「同 `enrollment_id` 取代换代 + `prepare_intent_id` 轮换 + 全新 plan/session」（条款 1/3/10）——「新建隔离」只覆盖**持久层身份**。
- **有没有写「新建后旧终止态不得出现在新 work item 上」（展示层）？** —— **没有任何 requirement**。对全部 specs 与两份 11-03 归档 delta 检索 `planTermination|终止回执|终止横幅`：**零命中**（`models/automation.rs:145-150` 只规定 wire 字段「plan 与终止事实——REST/WS 一致」，未规定前端回执的留存与失效）。

## 3. Q2｜计划承诺了什么：任务项、验收判据与勾选状态

### 3.1 块1 计划（2026-11-03）与归档 tasks.md 的对应

- 块1 计划 `cadence/plans/2026-11-03_计划文档_实施_块1…v1.0.md` 中与「删除」相关的任务**只有失败面文案与换代删除旧 WIG**，没有「删除实体时让 enrollment 失效/消失」或「新建 plan 时换/新建 enrollment」的任务项：
  - 第 4 组「『删了重来』失败的失败面」（归档 `tasks.md:79-84`）：`4.1 失败面文案如实告知「上一次的工作已经被清空，但新的没建起来」+ 重试指引`、`4.2 用例：模拟换代后失败 → 旧 WIG 已删、新 WIG 未建、失败面文案正确` —— 均 **[x]**。对象是**换代路径的报错文案**，不是删除/新建的 enrollment 语义。
  - `:1547`（D-8）：「片 4 的失败面『已删未建』**判定口径**（重读 enrollment 看 `terminated_at`）—— 不新增后端错误码 | REQ-WIGA-SWAP-01『换 provider 失败时旧工作项组已删除且失败面如实告知』」——同样只判文案。
  - `:808`：弹窗文案「换 provider 会先删除现有工作项组」——删除范围再次锚定为 WIG。
- 归档 `openspec/changes/archive/2026-11-03-fix-automation-enrollment-provider-swap/tasks.md` 勾选状态：第 1~3、5~8、10 组全部 **[x]**（含 `1.6 换代后 binding_version 递增且旧代进 previous`、`2.4 一次提交即完成换 provider`）；未勾项：
  - `7.8 [ ] 卡片刷新：当前卡重建 / 重连 SHALL 刷新同门当前事实；旧卡只读历史`
  - `9.1~9.8 [ ] 全组真机验收` —— 其中 **`9.2 按指引到驾驶舱点「终止整个自动化」→ 回弹窗重新启用 → 走通`** 与 **`9.3 终止态换 provider → 一次操作换成功`** 正是「终止→删除→新建」链路的真机验收，**未执行**。本问题的暴露面恰好落在这两个未勾项的验收真空里。
- 块1 设计 `:121-123` 明确「REQ-WIGA-02 / REQ-WPS-01 / **REQ-WIGA-09（终止后禁止启用）保持不变**」——即块1 的计划范围**刻意不触碰**删除/终止的 enrollment 语义，只放宽 terminated 上 Enable 的 options 检查。
- 块3（去单仓入口）计划与归档 tasks（51 项 [x]，仅 `11.4 真机验收` 未勾）：其「删除」全部指**删除单仓代码路径/入口**（如 `:305`「`:384` 单仓臂删除」），与「删除实体 × enrollment」**无关**。问题源排期表（`cadence/notes/2026-11-02_排期…v1.0.md:41-58`）中 S1（换代被抢 revision，块1 源头）、S8（项目删除后级联残留 148 文件）同样**不含**「删除→新建继承终止态」——该问题在排期清单里**从未立项**。

### 3.2 Q2 小结

- **没有任何 task 承诺**「删除时让 enrollment 失效/消失」——因为设计（Q1 条款 7/8）反向禁止。
- 「新建 plan 换/新建 enrollment」也无 task；最近的等价物是 `1.1/1.6`（supersede 换代、旧代进 previous）与 REQ-WPS-02(a) 的 intent 轮换，**均已 [x]**。
- 唯一与本问题路径直接相关的未完成项：**tasks 9.2 / 9.3（真机验收未做）与 7.8（旧卡只读历史——展示层陈旧态刷新，恰是泄漏同族）**。

## 4. Q3｜实施是否忠于设计：三处点名代码逐条比对

| # | 代码点 | 对应设计条款 | 比对结论 |
|---|---|---|---|
| 1 | `src/web/handlers/lifecycle/deletion.rs:146-152`（F3 自动化占用预解除：「绑定该实体的 enabled enrollment……终止是语义必然。复用与人工终止/autopilot 自愈同源的 terminate CAS；失败 → `automation_termination_failed`，删除不执行」） | DRG 设计 `2026-10-27…:120/:143`（失效≠删除）与归档 `2026-10-28…/design.md:90`（删实体时终止绑定 enrollment，同源 terminate CAS）；REQ-WIGA-09 `:693`（删除流程对 terminated 零副作用——已 terminated 则零改写） | **忠于设计**。删除前 terminate、不删记录；该行为可追溯到 2026-10-28 change（commit `49a39400`，2026-11-01），非本次 11-03 两块引入 |
| 2 | `src/product/coding_attempt_store/gate.rs:429-451`（`automation_plan_terminated_for_attempt`：**仅当** enrollment 显式绑定该 attempt 的 `work_item_group_id` 且 `terminated_at` 非空才返回 Some；manual/无绑定/异 plan 一律 None） | REQ-WIGA-09（批 2 T1，函数 doc 注释自证引用）；「terminated 阻断」`:693-695` | **忠于设计且按精确身份匹配**。运行时证据（诊断报告 §0/§2 Q2）：新 attempt 的 `work_item_group_id` ≠ 旧 plan_id ⇒ 本函数返回 None，**无辜** |
| 3 | `src/product/issue_automation_store.rs` 无 delete 接口（pub API 仅 `get/compare_and_set/compare_and_set_reenable/bind_plan/with_current_enrollment_locked/ensure_plan_binding/claim_plan_generation/mark_plan_generation_phase/ownership_*`，`:55-630`） | DRG `:527`「销账 MUST NOT 删除 enrollment 记录」、设计 `:1062`「销账永不删除 automation-enrollment.json」 | **忠于设计**（无 delete 正是条款要求的结果） |
| 4 | `bind_plan`（`:320-325`）与 `ensure_plan_binding`（`:379-384`）在 `!saved.enabled` 时 fail-closed Conflict | REQ-WIGA-09 `:658`「terminated SHALL 阻断全部自动路径；MUST NOT 代发 Enable」 | **忠于设计**：terminated（enabled=false）后不存在绕过取代路径的绑定/建 plan 通道 |
| 5 | 取代路径 `issue_automation_store_parts/cas_internals.inc.rs:665/:756-782`（`terminated_at` 清空、旧代进 `previous`、`prepare_intent_id` 轮换、plan/session 空串占位） | REQ-WIGA-09 `:700-702`、REQ-WIGA-SWAP-01 `:865-867` | **忠于设计**。运行时证据（诊断报告 §1.1）：同一 `enrollment_id`、binding_version 1→2、旧 plan（`…9df5de30`）进 previous、新 plan（`…8f6819c7`）绑定、**当前 JSON 无 `terminated_at`** —— 与条款逐字段吻合 |
| 6 | **泄漏点（前端，实施超出任何条款）**：`web/src/state/…/coding-workspace-store.ts:591-600` 终止回执 `planTermination.terminated` 只按 `(project_id, issue_id)` 写入、仅 `reset()` 清；`web/src/pages/CodingWorkspacePage.tsx:176-181`、`web/src/pages/ChatCockpitPage.tsx:854-860` 按同粗键匹配渲染终止横幅 | **无对应条款**（Q1 2.3 第三条：全 specs/delta 检索零命中） | 实施做了设计**从未规定**的事（回执跨 plan 存活）；因设计缺条款，无法判「违条款」，只能判「落在设计未覆盖面」 |

补充时间线（诊断报告 §1.3，`/tmp/aria-verify` 只读）：终止 06:30:07 → 旧 plan 删除墓碑 06:30:13 → 重建 Enable 06:31:26 → 新 session `running`、timeline 推进至 Review Round 2 —— **后端整条「终止→删除→换代重建」链路按设计走通**；「卡住」是旧终止横幅叠加在新运行界面上的观感（横幅时间戳 06:30:07 在服务端 durable 层已 grep 零命中，只存在于前端内存回执）。

## 5. Q4｜判定

**判定 = 设计缺口（设计不合理/不完整：语义只写到持久层身份，没写到新建侧的展示隔离）；实施忠于设计，未发现漏做或做偏。**

按审计给定三条规则逐条对号：

1. 「设计允许/要求保留 tombstone 而没有要求新建隔离」——**成立**：
   - 删除侧明文「terminate 保留、销账永不删」（`work-item-group-autopilot/spec.md:693-695`、`dangling-reference-governance/spec.md:527`、DRG 设计 `:120/:143/:1062`）；
   - 新建侧的隔离条款**全部止步于身份层**（REQ-WIGA-09 `:700-702` intent 轮换/全新 plan/session、REQ-WPS-02、REQ-WIGA-SWAP-01 `:867` 不残留旧链）；
   - **没有任何条款要求「新建后旧终止态不得再出现在该 issue 的新 work item 界面上」**——检索 `planTermination|终止回执|横幅` 于全部 specs 与两份 11-03 delta：零命中。⇒ 命中规则一 ⇒ **设计缺口**。
2. 「设计明确写了新建必须开新 enrollment / 不得继承旧终止态而实现没做」——**不成立**：设计写的是「同 enrollment_id 取代换代」，且实现已做到（比对表 #5 + 运行时证据）；「不得继承」在身份层也已做到（比对表 #2/#4）。
3. 「双失」——**不成立**：后端可审计的每条条款都被忠实实现；唯一偏差面（前端回执）无条款可违。

**漏掉的那条要求**：「终止回执/终止展示必须按 plan/binding 身份限定，并在 enrollment 换代（supersede 清空 `terminated_at`）后失效」——**当前无 REQ/Task 编号**（缺失本身即答案；最近的近似物是未勾的 `tasks 7.8`「旧卡只读历史」与 `tasks 9.2/9.3` 真机验收，二者都只差一步之遥但均未覆盖该语义）。

另需留痕的语义分歧：owner 记忆的「删除 = enrollment 记录也彻底消失」与设计明文（`MUST NOT 删除 enrollment 记录` + 同 id 取代留痕）**字面冲突**。若按字面推翻，将断裂命令账本/binding_history 审计链（诊断报告 §2 Q6 候选①已否决该路线）；本审计建议不推翻，理由见 Q5。

## 6. Q5｜最小补法落点

**需要先改设计（补一条新增 requirement），而不是改旧条款**——REQ-WIGA-09 的 tombstone + 取代语义是被 DRG 明文「MUST NOT 重新裁定」保护的有意设计（`dangling-reference-governance/spec.md:527`），「重建隔离」应作为**新增 requirement** 落在新 OpenSpec change：

1. **主修（补缺口）**：新 change 在 `work-item-group-autopilot`（或归入 `human-gate-termination-reliability` capability）**ADDED** 一条 requirement（建议名：**REQ-WIGA-14 终止回执的身份限定与换代失效**），正文方向：
   - 终止回执（`planTermination.terminated` 及其一切展示派生物）MUST 绑定终止时的 `plan_id`/`binding_version` 身份；仅当当前展示对象身份与回执一致时才渲染；
   - enrollment 完成取代换代（`terminated_at` 清空、`binding_version` 递增）后，旧代终止回执 MUST 在所有页面失效；旧代回执仅可作为历史记录查看，MUST NOT 叠加在运行中的新代界面上；
   - 场景判据直接复用本问题：「终止 → 删除 → 同 issue 重建后，新 work item 的驾驶舱/工作台 MUST NOT 出现旧代终止横幅；新代运行状态照常展示」。
   - 代码落点（实施 task 对应）：`web/src/state/…/coding-workspace-store.ts:591-600`（回执写入带身份 + 换代失效）、`web/src/pages/CodingWorkspacePage.tsx:176-181`、`web/src/pages/ChatCockpitPage.tsx:854-860`（按身份匹配渲染）。
2. **护栏（可选）**：同 change 内以 MODIFIED scenario 给 REQ-WIGA-09 补「换代后管理投影与页面不残留终止展示」——注意按 OpenSpec 纪律在**新 change 的 delta** 里提，不改已归档文档。
3. **验收补做**：把归档 tasks 未勾的 `9.2/9.3`（终止→重新启用→走通 / 终止态一次换成功）纳入新 change 的真机验收清单，`7.8`（旧卡只读历史）一并纳入——三者共同锁住「旧代状态不得污染新代」。
4. **不建议**的路线：把「删除实体 ⇒ 物理删除 enrollment 记录 / 新建 ⇒ 全新 enrollment」立为 requirement——与 `dangling-reference-governance/spec.md:527`、REQ-WIGA-09 `:658` 的留痕换代语义正面冲突，需 owner 明确裁决推翻既有审计链后才可立项；且运行时证据表明后端并不泄漏，修不到点上。

## 7. 未验证项

- 未跑 cargo / vitest / 前端 build（纪律禁止；target 锁被并行实施 agent 占用）。全部代码结论基于静态阅读 + 引用同日诊断报告的运行时取证。
- 前端三个泄漏点行号（`coding-workspace-store.ts:591-600`、两处 Page 守卫）引自同日诊断报告（基线 `b94a58fb`），本审计未逐行复核 web/src 源码（只逐行核了后端三处：`deletion.rs:119-178`、`gate.rs:429-451`、`issue_automation_store.rs:299-479`）。
- 「卡住」是否另有 provider 子进程/daemon 旧映像因素（诊断报告 §3 提及二进制 14:42 重建、daemon 13:45 启动）未在本文档范围核实。
- `main`（`827a9901`）分支同构性未逐文件比对（诊断报告称同构；本审计全部行号引用 `bugfix-b-11-02` @ `b94a58fb`）。
- `cas_internals.inc.rs` 空串占位（`:781-782` `plan_id: String::new()`）与 `ensure_plan_binding` 匹配臂（`:426-439`）之间的边界（Some("") 是否会在换代后被误判 PlanPhaseConflict）只做了静态走查、未见运行时触发证据，如需钉死建议随新 change 加回归测试。

---
*审计人：AuditDesignVsImpl（只读）；引用行号基线 `.worktrees/bugfix-b-11-02` @ `b94a58fb`。*
