# Delta: work-item-plan-single-candidate

## MODIFIED Requirements

### Requirement: 旧协议退役与单路径收敛（REQ-WSC-08）

REQ-WSC-07 退役门已按 `legacy-protocol-retirement` REQ-RET-01 全口径重测解锁后(2026-09-19 用户终裁 B:pi 全子项达标=协议质量实证;codex Confirmed 子项登记已知例外——系统性 provider 内容缺陷与协议无关,门文本据此显式修订,属 1c 裁决预留的「后续专项裁决」路径显式行使;**后续义务:codex/claude_code/kimi_code 全部 provider 最终 SHALL 全测通过(DEF-PVR-ALL),在义前不得视为 provider 面收官**),旧协议(generation-mode 决策、逐段确认消息、review_decision 双选项语法、`HumanConfirmDecision` 旧枚举及其消息族、SelectRevisionPath 族及其专属 DTO)SHALL 删除。删除后:新会话 SHALL 一律走单候选流(prepare → generate → evaluate → approval → completed,另加吸收态 failed);多仓 Issue 的确定性 preflight SHALL 按 selection **resolved 上界**放行——上界=focus_repository_ids 非空时取该集,为空(存量 AllMembers)时取 resolve_effective_members 有效成员集;design involved 为上界的子集且(非空,或空且上界恰一仓的回退口径成立)时通过,其余情形确定性失败;失败以新路径 durable fatal/recoverable 终态记录并含原因,系统不存在任何 legacy fallback 或 `flow_kind` 切换路径。plan items 的 `target_repository_id` SHALL ⊆ design involved(下游按 target 分流为 MTG 已验收现状)。

#### Scenario: 已删除消息协议错误拒绝

- **WHEN** 退役完成后客户端发送任一已删除的 legacy 决策消息(generation-mode 决策、逐段确认、review_decision 双选项、human_confirm)
- **THEN** 系统返回 stage-specific protocol error 且零副作用,会话状态与事件流不变

#### Scenario: 多仓 preflight 按勾选上界放行

- **WHEN** 多仓 Issue 的 selection.focus_repository_ids 非空(≥2 或 =1),design involved 为 focus 集的非空子集(含全集)
- **THEN** 确定性 preflight 通过,plan 事务按单候选流继续

#### Scenario: 多仓 preflight 失败收敛新路径终态

- **WHEN** 多仓 Issue 的确定性 preflight 失败(无论新路径是否已产生副作用)
- **THEN** 失败以新路径 durable fatal/recoverable 终态记录并含原因,系统不存在任何 legacy fallback 或 `flow_kind` 切换路径

#### Scenario: 多仓 preflight 界外/空集确定性失败收敛终态

- **WHEN** design involved 为空集,或含 focus 界外成员(preflight 无论新路径是否已产生副作用)
- **THEN** 失败以新路径 durable fatal/recoverable 终态记录并含原因,系统不存在任何 legacy fallback 或 `flow_kind` 切换路径

#### Scenario: 新会话一律单候选流

- **WHEN** 退役完成后创建任意 workitem workspace 会话
- **THEN** 会话走单候选流(prepare → generate → evaluate → approval → completed),不存在 legacy 逐段路径选项

#### Scenario: 历史 legacy 记录只读保留

- **WHEN** 退役完成后读取历史 legacy session 的 durable 记录
- **THEN** 记录与事件前缀原样保留可读,未被迁移或清洗

#### Scenario: design involved 界内收敛(钉定上界)

- **WHEN** issue 勾选 N 个成员,AI 生成的 design involved 实际只含其中 M 个(1 ≤ M ≤ N)
- **THEN** sentinel 输出的 involved/change_order 均 ⊆ 勾选集且 involved 非空;界外仓出现在 sentinel 时系统 SHALL 拒绝该回写并以可见诊断提示走修订反馈
