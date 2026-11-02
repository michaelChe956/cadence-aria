# 变更提案：多仓评审范围收敛与 coding 跨仓最终评审

## Why

多仓库逻辑代码库（`IssueRecord.logical_codebase_id == Some(_)`）今天仍在复用单仓库的评审骨架：规划侧按「先解析出主成员仓（primary）」再开评审窗，材料按 soft/hard 预算裁剪并附 `omitted_member_ids`，基线靠 `main`/`master` 猜测；coding 侧在每仓收口时各评一次 internal review，跨仓一致性没有任何一次整体评审。结果是评审范围、材料与身份都不匹配「一个 issue 覆盖 1~N 个真实仓库」的事实：漏员、漏跨仓影响、评审器身份随窗口而非随会话、跨仓契约不一致要等人工发现。

owner 已批准两份技术方案，本 change 只把它们落成可验收契约，不含实现：

- `cadence/designs/2026-10-22_技术方案_story-design-workitem评审修正_v1.2.md`（commit `24fedc80`）
- `cadence/designs/2026-10-22_技术方案_coding跨仓最终评审_v1.1.md`（commit `ded59f08`）

硬约束（owner 原话）：「这两个方案都不能影响单仓的使用哈，都是在多仓上做改造。」因此**单仓库代码库（`logical_codebase_id == None`）的行为必须与今天逐项一致**，所有新行为只挂在多仓分支。

## What Changes

- **评审范围 = issue 选定的真实仓库集合**：多仓三环节（story / design / work item）评审不再有「主仓 / 主成员（primary）」概念，不再回落到 manifest 首成员或 `repo_id` 投影；selection 解析失败即显式阻断。
- **代码库形态门禁**：`logical_codebase_id` 是唯一判定键；单仓的评审窗口、目标/cwd 解析、基线语义、门控、容错、文案与 canonical 摘要全部保持不变。
- **多仓文档环节评审不开窗**：不开评审窗、不生成窗口记录、不做基线快照/对账/恢复、不产出 `plan_review_window_*` 错误码；材料缺失改为 `plan_review_material_unresolved: <原因>`。
- **材料全量 + 稳定注入**：仓库清单全量注入（绝对上限 500 KiB，超限以 `planning_material_over_budget` 阻断，不静默缺员）；清单与逐仓实况进不随历史压缩丢失的稳定注入面；逐仓实况五行（职责/关键目录/入口/构建测试命令/基线），缺项写「未识别」。
- **逐仓基线取 HEAD 分支**：`git rev-parse --abbrev-ref HEAD`；detached HEAD / 空仓只把该仓标「基线未识别」，其余仓继续；单仓仍 fail-closed。
- **评审会话身份**：多仓 reviewer 与 author 的运行身份改为每会话新生成的唯一 id（UUID），与会话一一绑定、删除即解绑；单仓沿用窗口编号。
- **判据与容错（只多仓）**：must-fix 追加四类必查（需求漏项 / 仓库选择不合理 / 验收标准不可判定 / 跨仓影响缺失）；finding 缺 `evidence` 或 `required_action` 不收，判 `needs_human` + `malformed_findings`；旧轮历史保留 finding 的 message + evidence；Work Item splitter 目录摘要分层、读取失败显式报错。
- **明确不改**：verdict 结构错误的静默降级（列为已知缺陷待办）、severity 分级规则、revision 轮只复核旧问题的规则，三者在两种形态下都维持现状。
- **coding 跨仓最终评审（多仓）**：多仓形态停用「每仓各评一次」internal review 与 GroupFinalReview（保留 code reviewer 及其返工/分诊门、保留组形态但无组终审）；全部 work item 完成后触发**恰好一次** issue 级跨仓最终评审，材料为逐仓 diff 分段，provider 取 `internal_reviewer`、缺失回退 `code_reviewer`、两者皆缺则 Skipped。
- **该评审 advisory**：结论不得进入任何交付门/采信门/自动返工/人工分诊路径，不阻止客户确认，不做零锁恢复。
- **留痕**：用户忽略提醒仍确认时持久化一条「已知风险，仍然确认」（时间 + 评审记录 id + 结论 + findings 摘要），不记确认人、不记通道。
- **来源枚举**：新增 `findings.source_stage = IssueCrossRepoReview`（序列化 `issue_cross_repo_review`），既有取值不变，新成员永不出现在 attempt 的 `stage`。

## Capabilities

### New Capabilities

- 无。本次所有行为都落在既有 capability 的既有语义邻域内，不为「多仓评审」另立一套并行契约。

### Modified Capabilities

- `logical-codebase-aggregate-planning`：新增多仓评审范围与形态门禁、评审材料全量注入与预算阻断、稳定注入面、逐仓实况与逐仓基线、must-fix 必查类别、finding 完整性与旧轮证据、splitter 目录摘要七条要求（ADDED）。不改在途 change 正在修订的 REQ-PLN-01/03/05/06/07。
- `session-policy-envelope`：新增多仓评审与 author 的会话唯一身份、多仓文档环节评审不开评审窗两条要求（ADDED）。不触碰在途 change 正在修订的 REQ-ENV-01..09、REQ-ENV-10/11/12/HOST-01/REVIEW-B/CHECK-01。
- `group-final-review-triage`：新增多仓停用每仓内部评审与组终审、存量 `internal_pr_review` attempt 的形态分派、issue 级跨仓最终评审恰好一次、advisory 红线、已知风险确认留痕、跨仓 findings 来源标记与兼容六条要求（ADDED）。

## Impact

- 本 change 只形成契约工件：不修改任何 `src/`、不改 `cadence/designs/`、不创建新 store 语义、不引入新抽象层。
- 受影响的实现面（由后续 Plan 展开）：LC planning resolver 材料渲染、评审 prompt 构建与历史压缩、结构化输出解析分支、Work Item splitter 目录摘要、workspace session 记录与 reviewer-verify 摘要域、coding 收口钩子与 issue 级评审记录/留痕、`CodingExecutionStage` 序列化表。
- 单仓库代码库零行为变化：窗口记录、window id、主成员早返回路径、基线 fail-closed、prompt 字节、internal review / GroupFinalReview / 阶段机全部保留。
- 已知缺陷保持待办：verdict 结构错误静默降级（`ReviewCompletionError::Schema(_)` 不进 repair，直接 `needs_human` 且诊断码埋在 `structured_output_diagnostic`）本批不修；severity 与 revision 轮规则不改。

### 与在途 change 的关系

- `provider-desandbox-full-open` 的 `session-policy-envelope` delta 已 ADDED `REQ-ENV-REVIEW-B`（B 档审查窗口，原文覆盖「所有 Reviewer 运行」）。本 change 的 `REQ-ENV-14` 对多仓**文档环节材料评审**作出范围限定；两者归档顺序与须补的适用范围修订见 `design.md`「与在途 change 的重叠面与归档顺序」，并在 `tasks.md` 设为归档硬门。
- `logical-codebase-aggregate-planning` 的在途修订（`lc-root-initialization`、`provider-desandbox-full-open` 改 REQ-PLN-01/06；`fix-planning-session-anchor-recovery` 改 REQ-PLN-03/05/07；`fix-error-experience-structural-governance` ADDED REQ-PLN-09/10）与本 change 的 ADDED 条款**无 header 重叠**，因此不覆盖任何在途条款。
