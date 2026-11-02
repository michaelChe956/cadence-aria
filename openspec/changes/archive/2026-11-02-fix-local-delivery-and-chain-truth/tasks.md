# Tasks

本文件只列高层工作包，映射到本 change 的 requirement；精确文件、命令、测试与提交边界由获批后写入 `cadence/plans/` 的实施 Plan 展开，Plan 只能展开不能重定义契约。工作包规模为派工拆分用的 `[INFERENCE]`，单项不超过 4h。

实施已在契约之前按设计终稿并行启动，工作区存在未提交改动（见 `design.md` 落点表的「在途」项）。下列「复核」类工作包的验收标准是：逐条对照 spec 场景，偏差回改实现；未经独立复核与 Main 验证不得勾选。

## 1. 本地交付后端（REQ-COD-06、REQ-MTG-04）

- [ ] 1.1 推送状态新增 `skipped_not_configured` 与「已交付」单一谓词；`issue` 级与 plan 级聚合改用该谓词，`maybe_complete_issue_delivery` 因此将本地交付计入完成。验证：serde 往返为 `skipped_not_configured`；谓词真值表（`pushed`、`skipped_not_configured` 为真，`not_pushed`、`failed` 为假）；`Completed`＋本地交付使 issue 聚合为 `all_pushed`；`Completed`＋`failed` 仍为部分（REQ-COD-06 场景「本地交付计入 Issue 完成」、REQ-MTG-04 场景「未配置远端不掩盖真实推送失败」）。
- [ ] 1.2 评审请求阶段的远端未配置判定与本地交付收口：判定前置于任何远端探测与推送，持久化 ReviewRequest（含本地交付提示文案）、timeline 节点标完成，`reopen_failed_review_git_operation` 保持只接受 `Failed`。验证：有与无 origin 的临时仓库各一例；无 origin 返回本地交付、journal 完成、attempt 有 ReviewRequest；构造 journal 停在 `PushStarted` 且无 origin 的现场形态，重入后收敛；远端已配置但不可达仍为 `Indeterminate`/失败，不降级；本地交付记录不可被重开（REQ-COD-06 全部新增场景）。
- [ ] 1.3 回归锚：既有单仓与多仓交付测试（重推、多目标交付、fallback commit、git 操作对账、cross target、issue 交付、plan 聚合投影、组完成、组启动上下文、SC 依赖门）零改动全绿，其 fixture 均配置 origin。验证：上述套件由 Main 统一运行，无断言被放宽。

## 2. 投影真实性与数据契约（REQ-MTG-07、REQ-MTG-08、REQ-MTG-09）

- [ ] 2.1 plan 聚合进度的 `execution_state` 增加 attempt 状态守卫与四个新取值，`waiting_reason` 在 AMR 时透传人工恢复原因；同步修正受影响的既有断言（启动回执时刻 attempt 为 Created，应为 `initialized`）。验证：AMR＋unit Running＋claim 已达 provider ⇒ `awaiting_manual_recovery`；同条件 attempt Running ⇒ `running`；Blocked/WaitingForHuman/Failed/Aborted 各自取值；已 Completed 的 unit 在 AMR 下仍为 `completed`；unit、claim、active 指针在投影前后未被改写（REQ-MTG-08、REQ-MTG-09 全部场景）。
- [ ] 2.2 `target_attempts[]` 增加冻结的九字段（`repository_name`、`branch_name`、`base_branch`、`stage`、`head_commit`、`push_status`、`push_error`、`review_request_id`、`manual_recovery_reason`），字段名与类型以 `design.md` D7 为准。验证：字段齐全且含 ReviewRequest 数据；无 ReviewRequest 与成员 attempt 已删时对应字段为 null；既有五字段形状不变；`group_projection` 与 `target_attempts[]` 对同一 target 的推送状态一致（REQ-MTG-07 场景「字段来源与取值域」「数据契约 additive」「无 ReviewRequest 或成员已删」「人工恢复原因透传」）。

## 3. 运行事实收尾与恢复（REQ-CRO-07、REQ-CRO-08）

- [ ] 3.1 runner 死亡转 AMR 时收尾遗留 Running role run；显式恢复时清理残留 Running role run 为已取代；收尾失败告警不吞没、不阻止转 AMR；不改 unit、claim、active 指针。验证：仿既有 runner 恢复测试先例——死亡后 Running role run 变 Failed 且带原因码；RecoverCoding 后残留 Running 变 Superseded；注入收尾失败时仍转入 AMR 且有告警；非 AMR 重发被拒无副作用（REQ-CRO-07 场景「runner 死亡后 role run 不再显示运行中」「收尾失败不阻止转入人工恢复」；REQ-CRO-08 场景「恢复清理残留运行中 role run」「重复或错对象的恢复被拒」）。

## 4. prompt 声明与汇报契约（REQ-CWC-04、REQ-CWC-05）

- [ ] 4.1 平台指令段承载提交责任优先声明、禁止手动推送与汇报契约，并在 coding 首次与增量、返修与增量、provider 重试三类路径的最终组装后追加，渲染路径与回落路径都覆盖；对全部 attempt 注入。验证：断言各路径最终 prompt 文本均含提交优先声明与汇报契约；既有执行上下文哈希相关测试不变；新增 prompt 路径遗漏该段会令测试失败（REQ-CWC-04 场景「各 prompt 路径一致」「声明不改变哈希」、REQ-CWC-05 全部场景）。

## 5. 前端类型、标签与交付判定

- [ ] 5.1 前端类型联合新增 `skipped_not_configured`、四个 `execution_state` 取值与九字段，`waiting_reason` 放宽为字符串；attempt 状态、stage、推送状态三组标签收敛为单一来源并导出，含「本地交付(无远端)」；`DeliveryStatusPanel`、`PlanGroupProjectionPanel`、`CodingWorkspaceReports` 的已交付判定与徽标改用同一口径。验证：标签覆盖全部取值且编译期保证完整；未知值回落原文；`skipped_not_configured` 在三处均显示为本地交付且不显示为已推送；既有交付面板与聚合面板测试保持绿（REQ-MTG-04 场景「本地交付计入全部交付」、REQ-COD-06 场景「本地交付计入 Issue 完成」）。

## 6. 前端逐仓面板、仪表盘覆盖与 AMR 恢复（REQ-MTG-06、REQ-MTG-10、REQ-CRO-08）

依赖 2.2 的契约与 5.1 的类型标签；两者落定后才可开工。

- [ ] 6.1 coding 页逐仓交付面板：判据为 attempt scope 为 work_item_group 且聚合进度绑定 target-attempt ≥2，满足时每 target 一行并显示本地交付；不满足时顶部 DOM 与文案零变化；面板只读无操作入口。验证：2 个 target 渲染 2 行且含本地交付行；1 个 target 的页面快照与改前一致；非 group 不渲染；面板内无按钮（REQ-MTG-06 场景「多仓逐仓渲染」「单仓页面零变化」「面板无操作入口」）。
- [ ] 6.2 仪表盘覆盖：聚合 ≥2 target 时，AMR 与 Failed 的 attempt 下未完成 unit 画为失败，已完成 unit 与其他 attempt 不变；单 target 与 v1 路径零变化。验证：六态映射用例覆盖 AMR、Failed、Blocked、已完成；聚合为 1 个 target 时与改前一致（REQ-MTG-10 场景「仪表盘把 AMR 画成失败」「单 target 路径零变化」）。
- [ ] 6.3 AMR 恢复动作：对 awaiting_manual_recovery 的 attempt 显示人工恢复原因与恢复按钮，经既有 WS 通路发送恢复消息；其他状态不显示。验证：AMR 时按钮可见且点击发送恢复消息；Running 等状态不可见；重复点击由后端拒绝后界面保持一致（REQ-CRO-08 场景「驾驶舱提供恢复动作」）。

## 7. 平台确定性交付摘要（REQ-CWC-06，可选后置）

依赖 4.1；展示载体见 `design.md` 的待确认项，确认前不开工。不阻塞现场解堵。

- [ ] 7.1 Work Item 完成后按 durable 事实生成「交付事实（平台生成）」时间线条目（完成提交、起止区间变更文件、已记录验证证据、handoff 标识），缺失显示「未记录」，同一 UnitRun 幂等，不写仓库，失败告警不阻断完成；前端渲染该条目。验证：完成后时间线出现一条平台条目；缺失项为「未记录」；重入不重复；目标仓库暂存区与提交不变；Coder 自述与平台条目并存且互不引用（REQ-CWC-06 全部场景）。

## 8. 契约收口与现场验收

- [ ] 8.1 Main 一次性统一验证与独立审查：受影响 cargo 模块、web 单测、1.3 回归锚、严格校验 `openspec validate fix-local-delivery-and-chain-truth --strict`；审查对照 spec 场景核对在途改动与契约无偏差，特别是 D1 前置位置、D4 优先级、D7 字段名。验证：留存命令与输出，红→绿证据齐备；worker 不各自重复运行全量构建与测试。
- [ ] 8.2 现场恢复演练（需 owner 在场）：按 `design.md` 迁移计划先做只读预检，再恢复两个 AMR attempt，观察到 FinalConfirm 并人工确认；确认服务重启不自动放行 AMR。验证：真实页面操作留痕，逐仓面板显示本地交付，role run 无永久运行中条目。退路 abort＋restart 属破坏性操作，需 owner 明确同意。

## Workflow follow-up

- 契约获批后才调用 writing-plans，将精确文件、命令、测试与提交边界写入 `cadence/plans/`。
- 实施、验证与独立审查完成后，再按 OpenSpec sync/archive 流程处理；`fix-multi-repo-group-coding-start` 若先归档，复核 REQ-MTG-08、REQ-MTG-09 与 REQ-CS-06 并存关系。
- 跨 attempt handoff 解析缺陷修复不在本 change 内，作为恢复演练前置另行跟踪。
