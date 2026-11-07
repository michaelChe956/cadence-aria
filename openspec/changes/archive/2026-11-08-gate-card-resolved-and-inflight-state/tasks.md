# Tasks: gate-card-resolved-and-inflight-state

> 高层工作包；精确文件、命令、测试与步骤见 `cadence/plans/2026-11-07_计划文档_实施_门卡收口态与在途态修复_v1.0.md`。
> 每个工作包先写失败测试再实现（TDD）。

## 1. D1：已收口门卡去行动化

映射 requirement：ADDED REQ-WSC-26

- [x] 1.1 文案常量（收口标题三种 + 徽章说明四种 + 被中断修订句）集中到 `gate-prompt-copy.ts`
- [x] 1.2 `GatePromptEntry` 已收口时：陈述式标题、不渲染 `ReviewOwnerGuidanceBlock`、决定收口时不渲染 `gate-meta`、徽章旁渲染 `gate-resolution-note`
- [x] 1.3 「被中断修订证据」选择器（时间线 `aborted` + 含明说句的节点）
- [x] 1.4 留档卡（`superseded`）：标题陈述式 + 不渲染选项块，保留 meta 行与 archive-note
- [x] 1.5 既有断言自查（`p1-entries` 精确匹配「已终止」、`GatePromptEntry*.test`、`ReviewVerdictEntry.test`、`ChatCockpitPage.inbox.test`、F-50 投影断言）

## 2. D2：在途态表达

映射 requirement：ADDED REQ-WSC-27

- [x] 2.1 `selectGateInflightPhase(state)`：`revising | reviewing | null`（`activeRunId` + 活动节点类型 + 收口 / live-turn / 自动重写让位）
- [x] 2.2 `GatePromptEntry` 在途锁：状态行、隐藏反馈编辑器与确认按钮、保留终止、顶部指引新文案
- [x] 2.3 已收口卡不被激活的负向用例；live-turn 路径不变的回归用例

## 3. D3：取消 ⇒ aborted + 明说选择未生效

映射 requirement：ADDED REQ-CG-10

- [x] 3.1 `conversational_gate_cancel.rs`：中止哨兵常量 + `revision_drive_error_outcome` 分类函数 + 节点收口文案常量
- [x] 3.2 `close_revision_author_node_after_turn_failure` 按失败类分流（`Cancelled` ⇒ `Aborted`，已 `Aborted` 不回写 `Failed`）
- [x] 3.3 `sc_manual_revision.rs` 驱动错误分支用分类函数；`work_item_plan.rs` 两处哨兵返回改用常量
- [x] 3.4 `provider_run.rs` 取消兜底帧文案补「你的选择未生效」
- [x] 3.5 订正 `human_gate_revision_cancel.rs` 中与 REQ-WIGA-16 相抵触的 `Failed` 断言
- [x] 3.6 `buildChatEntries` 为「aborted 且含明说句」节点追加系统条目

## 4. 门禁与发布态

- [x] 4.1 `pnpm test` / `pnpm tsc -b` / `pnpm lint`
- [x] 4.2 `flock /tmp/aria-cargo.lock cargo test --locked --all-targets`
- [x] 4.3 `openspec validate gate-card-resolved-and-inflight-state --strict` 与 `openspec validate --specs --strict`
- [x] 4.4 重建 4317 发布态并做字节级指纹自检
