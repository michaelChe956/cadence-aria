# Proposal: bound-review-findings-to-work-item-scope

## Why

真机会话 `workspace_session_auto_e5f6b290-f231-479a-9c5c-c7d63c93515e` 的 reviewer 连续三轮报同一组 finding，其中一条是「仓库里那份前端规范文档的『中文 locale 尚未配置』缺口必须回写，但没人负责」。这条事实根本不在本 Work Item 的写入范围里：**改 Work Item 永远不可能让它消失**。owner 两次用自然语言说「这不归本 WI / 交给流程统一回写」，第一次被当成普通反馈收下，第二次被判 `validation_reject`，自动化停在原地，界面上只有「✓ 可直接确认（需先处理预判）」，没人知道「预判」是什么、下一步做什么。

三个现象同一病根：**reviewer 的 finding 范围没有约束**（模型可把任何事实报成本 WI 缺口）、**finding 没有归属与处置字段**（平台无法机器判别一条 finding 能否通过修订本 WI 收敛，owner 也没有「排除此项」的结构化出口）、**owner 的「不归我管」没有语义**（只被当成修改指令 → author 改 plan → 文档状态没变 → 下一轮再报 → 死循环）。

## What Changes

- **finding 契约扩展**：7 处 reviewer 输出 schema 增加 `disposition`（`actionable`|`advisory`）与 `target_work_item_id`；共享输出契约新增两条硬约束——范围约束（本 WI 写入范围/验收标准/能力契约之外的事实 MUST 标 advisory 且 required_action 不得要求作者执行）与收敛约束（已被 owner 处置的事实 MUST NOT 再报为 actionable；重复报告 MUST 说明上一轮处置为何没解决）。
- **平台硬规则**：阻断判定、pass→返修归一、必关闭 finding 清单只计 `disposition=actionable`。判定落在平台侧——模型标错也不会让 advisory 卡住流程。缺失 `disposition`（含全部历史记录）＝`actionable`（fail-safe）；非法值走既有 `invalid_finding_field` 不合规路径。
- **`finding_key` 内容身份**：平台按 `sha256(severity␁message)` 前 16 位小写 hex 现算，序列化时恒输出——live 投影与 rebuild（durable verdict 反序列化）算出同一个 key，前端「排除此项」与呈现共用，模型 MUST NOT 生成。
- **owner 排除项落 durable**：`human_gate_feedback` 入站新增 `excluded_finding_ids`（缺省＝旧行为）。排除项（`finding_key` + message 摘要 + 处置方式）落到该会话的 durable 已处置事实清单，下一轮 reviewer prompt 携带 `[human_dispositions]` 段并声明已处置事实不得重复报为 actionable。
- **门 turn 拒收可操作化**：门 turn 以 `validation_reject` 失败时，durable 落盘并可回执 `rejected_feedback` / 可读 `rejection_reason` / `rejection_next_actions` / 逐字 `rejection_diagnostics`（此前只有 `failure_class`，全盘 grep 不到一条诊断）。

## Capabilities

### Modified Capabilities

- `work-item-plan-single-candidate`：ADDED REQ-WSC-12（finding 范围与 advisory 降级）、REQ-WSC-13（owner 排除项 durable + 回喂下一轮）、REQ-WSC-14（重复报告必须说明未收敛原因）、REQ-WSC-15（人工门在拒绝与 advisory 场景下必须可操作）。不改 REQ-WSC-01..11。REQ-WSC-15 的前端呈现面由并行的门卡 UI 改动实现，契约条款在本 change 内成文（同一 change 目录只有一位 owner）。

## 规格来源与时序

owner 2026-10-30 真机反馈与本会话裁定：**取舍 1=B**（范围外事实「报但降级为 advisory」，不阻断、不进返修流、owner 仍可见）、**取舍 2=A**（门卡逐条「排除此项」，结构化落 durable，下一轮 prompt 携带）。现场证据见
`cadence/notes/2026-10-30_问题记录_审查finding越出WorkItem范围致人工回复无效与评审不收敛_v1.0.md`。

## Non-Goals

- 不改 reviewer 的 severity 三分级语义、不改 `class_hint`/`category` 既有分类与策略评估链。
- 不新建第二套 store：已处置清单与拒收记录落在既有 `workspace-sessions/{session}/` 同族目录，走同一 live-write 准入。
- 不改 Work Item Plan compiler、worktree 写面（REQ-WSC-10 已交付的写面）。
- 不为拿不到上轮上下文的审查路径硬塞已处置清单（Outline/Draft/Batch/Projection 新候选轮次不注入）。
- 不自动重试、不自动回写产品侧文档；owner 的「不归本 WI」只改变 finding 的处置归属与可见性，不改变文档本身。

## Impact

产品：reviewer 输出契约（7 处 prompt）、findings 解析与平台归一路径、durable 已处置清单与拒收记录、门 turn 拒收回执。协议：`human_gate_feedback.excluded_finding_ids`（optional，旧客户端兼容）、新出站帧 `human_gate_turn_rejected`。前端：门卡 findings 携带 `disposition`/`finding_key`，门卡 metadata 增加 `rejectedFeedback`/`rejectionReason`/`rejectionNextActions`/`rejectionDiagnostics`。验收：TDD 钉住 advisory 不阻断（不依赖模型标对）、live/rebuild 同 key、清单幂等落盘、拒收诊断逐字落盘。
