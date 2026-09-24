# Proposal

## Why

统一方案（f51-53-unified-design.md）C1，双 P0 来源 + F-56 新增：
- **F-51**：创建计划时的 `IssueWorkItemPlanOptions`（include_integration_tests 等）落库后从未进入生成期校验——`project_issue_work_item_plan` 从 IR 反推 options 使三族校验结构性不可触发，缺口延迟到 approve→Final Compile 才 fail-closed（用户 5 连 confirm 空转）；SC author prompt 零 options 教学。
- **F-52 L1**：finding 指纹=sha256(类别+reviewer 自由文本)，同题异拼写=异指纹、异题可撞同指纹→防环 9 轮零触发；reviewer 无前轮 findings 注入（判重纪律不可执行）；Verification 相位从未启用（每轮开放式 Initial）；cycle key 按 reviewer 节点（每轮新 key，预算永不耗尽）。
- **F-56（新增）**：plan 验收标准可引用基线树中不存在的文件路径（AC-023 引用 issue_0001 分支的 status.html），迫使 coder 越界恢复基线→完成门禁拦截→attempt 报废。生成期无「AC 引用路径 × 基线树」交叉核对。

## What Changes

1. **options 事实与预检前移**：存储 options 传入 single-candidate 校验上下文（三生产构造路径显式提供）；integration/e2e/split 三族语义在 generate/evaluate 期前置校验；机械 Error 适配 F5 既有 mechanical ReviewVerdict 回灌修订。
2. **SC author prompt options 教学**：三 flag 条件镜像教学（与校验口径逐字对齐）。
3. **AC 引用路径 × 基线树交叉核对**：plan 候选校验新增——AC/验证计划引用的仓库内文件路径必须存在于 plan 基线（worktree fork base / target ref），不存在即 Error（附路径清单与修复建议：删除 AC、改基线内路径、或声明基线恢复依赖）。
4. **finding 指纹结构化**：机械 finding 用确定性投影构造 canonical key（category+WI+CT+field+capability refs）；structured reviewer finding 受限抽取 WI/CT/AC ID 规范化；不可解析保持 unstable 身份 fail-safe 入人工。
5. **前轮 findings 注入 reviewer prompt**：注入结构化 canonical key/fingerprint/required_action 与当前候选 revision。
6. **Verification 相位启用 + cycle key 固定**：自动返修后 `ReviewInvocationScope::Verification`（只验证原集合）；`review_cycles` key 固定为 candidate/artifact identity（`sc:candidate:<source_revision_hash>`），同 source revision 的 reviewer 节点同 cycle。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `work-item-plan-single-candidate`: REQ-WSC-02（候选校验携带 options+AC 路径核对）、REQ-WSC-03（options 缺口不可延迟至 Approval）、REQ-WSC-06（教学/validator/identity 口径一致+前轮注入）
- `work-item-typed-outcome-policy`: REQ-TOP-02/03（重复 identity 不再自动 repair、Verification 语义）、REQ-TOP-04（cycle key 锚定候选、scope 违例）

## Non-Goals

- 不翻转 options 默认值（产品覆盖率语义，另立讨论）
- 不让 prose 取代 mechanical truth；不做 gate budget UI（C2 域）
- 不修改 CG typed turn/UI 语义（C2/C3 域）
- 不做跨 issue worktree fork base 治理（F-56 产品级 L2，后续观察）
