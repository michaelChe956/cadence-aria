# CodingWorkspace 五角色 Analyst 路由状态机技术方案

## 基本信息

- 文档类型：技术方案
- 方案日期：2026-06-12
- 版本：v1.0
- 适用范围：Coding Workspace 中 Coder、Tester、Analyst、CodeReviewer、InternalReviewer 五角色协作流程
- 背景分支：`bugfix_branch`
- 关联上下文：
  - `cadence/designs/2026-06-10_技术方案_CodingWorkspaceProvider驱动测试审查与恢复机制_v1.0.md`
  - `cadence/designs/2026-06-11_技术方案_CodingWorkspace角色授权与Tester门禁分流优化_v1.0.md`

## 背景

真实 E2E 测试中，Coding Workspace 的 Testing blocked 场景暴露出流程语义不清的问题：

1. Tester 可以产出 `blocked`、`missing_required_steps` 或 `skipped_required_steps`，但当前流程倾向将 blocked 停在人工 gate，或在人工继续后进入 CodeReviewer。
2. 对于“缺测试、缺实现、测试覆盖不足”等 code-actionable 问题，直接进入 CodeReviewer 并不合理，因为 CodeReviewer 的职责是审查已有代码，而不是判断 Testing blocked 应如何返修。
3. Tester、CodeReviewer、InternalReviewer 都是证据生产节点。如果它们各自决定流程路由，状态机会变得分散且难以解释。

因此，本方案将 Analyst 明确定义为 Coding Workspace 的统一路由决策节点。Coder 负责修改，Tester 和 Reviewer 负责产出证据，Analyst 负责基于证据决定下一步。

## 目标

- 明确五角色职责边界。
- 将 Analyst 作为默认流程路由节点。
- Testing 结束后统一进入 Analyst，由 Analyst 判断回 Coder、重跑 Tester、进入 CodeReviewer 或进入人工门禁。
- CodeReviewer 和 InternalReviewer 结束后同样进入 Analyst，由 Analyst 判断是否继续返修或推进下一阶段。
- 将 `manual_continue` 明确定义为质量豁免动作，而不是普通继续动作。
- 保留 `max_auto_rework`，防止 Analyst 和 Coder 之间无限循环。
- 让前端能清楚展示“证据节点”和“路由决策节点”的区别。

## 非目标

- 不取消人工门禁。
- 不取消 role-level provider 配置和权限模式。
- 不重写 Tester 的 TestPlan / TestingReport 契约。
- 不在本方案中引入独立规则引擎。
- 不让 Aria 核心硬编码某种语言或框架的测试策略。

## 核心原则

### 证据节点不做路由

Tester、CodeReviewer、InternalReviewer 只负责产出结构化报告和证据，不直接决定下一阶段。

- Tester 产出 `TestingReport`。
- CodeReviewer 产出 `CodeReviewReport`。
- InternalReviewer 产出 `InternalPrReview`。

### Analyst 统一做路由

Analyst 是默认的流程决策者。它读取上一个证据节点的报告、raw provider output、上下文、diff、OpenSpec 约束、项目规则和质量豁免记录，然后输出结构化决策。

### Coder 只负责执行修改

Coder 不判断是否完成验收，也不决定是否进入审查。Coder 接收 Analyst 的返修指令并修改代码。

### 人工继续是质量豁免

`manual_continue` 不应表示“继续正常流程”。它表示用户接受当前质量风险，允许流程带着豁免记录继续。

## 五角色职责

| 角色 | 核心职责 | 不应承担的职责 |
|------|----------|----------------|
| Coder | 实现和修改代码，执行 Analyst 返修指令 | 判断测试是否充分、决定是否进入审查 |
| Tester | 制定 TestPlan、执行测试、产出 TestingReport 和证据 | 判断 blocked 应返修还是豁免 |
| Analyst | 读取证据并决定下一阶段 | 直接修改代码、替代 Tester 执行测试 |
| CodeReviewer | 审查代码 diff、架构、安全、正确性、测试覆盖合理性 | 决定 Testing blocked 是否可接受 |
| InternalReviewer | 最终合入前审查，关注 PR 范围、影响面、交付风险 | 直接修复代码或绕过 Analyst 决策 |

## 推荐状态机

```text
Coder
  -> Tester
  -> Analyst
      -> Coder
      -> Tester
      -> CodeReviewer
      -> Human Gate

CodeReviewer
  -> Analyst
      -> Coder
      -> InternalReviewer / ReviewRequest
      -> Human Gate

InternalReviewer
  -> Analyst
      -> Coder
      -> FinalConfirm
      -> Human Gate
```

## Testing 后的 Analyst 路由

Testing 完成后，无论 `TestingReport` 是 passed、failed 还是 blocked，都默认进入 Analyst。

```text
Tester passed
  -> Analyst
      -> 测试证据充分: CodeReviewer
      -> 测试覆盖不足: Coder 或 Tester
      -> 需求/设计冲突: Human Gate

Tester failed
  -> Analyst
      -> 代码问题: Coder
      -> 测试计划问题: Tester
      -> 环境/权限问题: Human Gate

Tester blocked
  -> Analyst
      -> 缺实现/缺测试: Coder
      -> TestPlan 不合理: Tester
      -> 外部环境/凭据/人工浏览器问题: Human Gate
      -> 用户明确质量豁免: CodeReviewer
```

### Testing blocked 分类

| blocked 类型 | 推荐路由 | 原因 |
|---------------|----------|------|
| `missing_required_steps` | Analyst -> Coder 或 Tester | required 步骤没有完成，需要判断是实现缺口还是测试计划缺口 |
| `skipped_required_steps` | Analyst -> Coder、Tester 或 Human Gate | skipped 可能是测试覆盖缺口，也可能是环境限制 |
| `test_plan_repair_failed` | Analyst -> Tester 或 Human Gate | 先判断是 Provider 输出契约问题还是上下文不足 |
| `high_risk_test_step_requires_permission` | Human Gate | 需要人工批准风险 |
| provider unavailable | Human Gate | 环境或依赖问题，不应直接返修代码 |

## CodeReviewer 后的 Analyst 路由

```text
CodeReviewer approve
  -> Analyst
      -> 证据完整: ReviewRequest / InternalReviewer
      -> 测试或实现风险仍存在: Coder 或 Tester

CodeReviewer request_changes
  -> Analyst
      -> 代码问题: Coder
      -> 审查误判或上下文不足: Human Gate

CodeReviewer blocked
  -> Analyst
      -> 审查输出不合约: 重跑 CodeReviewer
      -> Provider/环境问题: Human Gate
      -> 已有足够人工判断: manual_continue 后进入下一阶段
```

## InternalReviewer 后的 Analyst 路由

```text
InternalReviewer approve
  -> Analyst
      -> FinalConfirm

InternalReviewer request_changes
  -> Analyst
      -> Coder
      -> Tester
      -> Human Gate

InternalReviewer blocked
  -> Analyst
      -> 重跑 InternalReviewer
      -> Human Gate
```

## Analyst 决策契约

Analyst 输出必须结构化。建议最小 schema：

```json
{
  "verdict": "needs_fix | rerun_testing | proceed | human_required | blocked",
  "next_stage": "coding | testing | code_review | internal_pr_review | final_confirm | human_gate",
  "reason": "简明说明",
  "evidence_refs": ["testing_report_0001.json"],
  "raw_provider_output_refs": [],
  "rework_instructions": {
    "summary": "需要 Coder 修改的内容",
    "required_changes": [],
    "verification_expectations": []
  },
  "human_gate": {
    "reason_code": null,
    "available_actions": []
  }
}
```

### Analyst 输入

Analyst 至少应读取：

- 当前 attempt 状态和 stage。
- Issue、Story Spec、Design Spec、Work Item。
- 当前 diff 和 changed files。
- 最近一个证据报告：
  - `TestingReport`
  - `CodeReviewReport`
  - `InternalPrReview`
- raw provider output 引用。
- context notes。
- quality bypass audits。
- `max_auto_rework` 和当前 rework count。

## 人工门禁策略

### Human Gate 触发条件

- Analyst 判断当前问题不是自动返修能解决。
- 自动返修达到 `max_auto_rework`。
- Provider 输出不合约且重试后仍失败。
- 高风险测试步骤需要人工批准。
- 环境、凭据、外部服务、人工浏览器验收等问题阻塞。

### Human Gate 动作

| 动作 | 含义 |
|------|------|
| `provide_context` | 用户补充上下文，流程等待或重跑 Analyst |
| `retry_stage` | 重跑当前证据节点 |
| `request_rework` | 明确要求进入 Coder 返修 |
| `manual_continue` | 质量豁免继续，必须填写原因 |
| `abort` | 终止 attempt |

`manual_continue` 必须写入 `quality-bypass-audits`，并在后续 Analyst、CodeReviewer、InternalReviewer 的 EvaluationContextPack 中可见。

## 与当前实现的差异

当前实现的关键差异：

- Testing blocked 不自动进入 Analyst。
- Testing blocked 的 `manual_continue` 会推进到 CodeReviewer。
- Testing passed 可以直接进入 CodeReviewer。

本方案建议调整为：

- Testing 完成后统一进入 Analyst。
- Testing blocked 不应默认停在人工 gate，除非 Analyst 判断必须人工处理。
- `manual_continue` 只作为 Human Gate 的质量豁免动作。
- CodeReviewer 和 InternalReviewer 结束后也统一回 Analyst 路由。

## 前端展示建议

页面需要区分两类节点：

1. 证据节点：
   - Tester
   - CodeReviewer
   - InternalReviewer
2. 路由节点：
   - Analyst

建议 Timeline 展示：

```text
Coder: 代码编写完成
Tester: 测试被阻塞，skipped_required_steps = [...]
Analyst: 判断为测试覆盖缺口，要求 Coder 补充测试
Coder: 返修中
```

对于 Human Gate，页面文案应明确说明：

- 当前阻塞原因。
- Analyst 推荐动作。
- 用户选择 `manual_continue` 的风险。
- 该豁免会被后续审查节点看到。

## 测试验收建议

### 后端状态机测试

- Tester passed 后进入 Analyst，再由 Analyst 决定进入 CodeReviewer。
- Tester failed with evidence 后进入 Analyst，再回 Coder。
- Tester blocked with `skipped_required_steps` 后进入 Analyst，Analyst 可回 Coder。
- Analyst 判断 provider/environment blocked 时创建 Human Gate。
- CodeReviewer approve 后进入 Analyst，再进入 InternalReviewer 或 ReviewRequest。
- CodeReviewer request_changes 后进入 Analyst，再回 Coder。
- InternalReviewer approve 后进入 Analyst，再进入 FinalConfirm。
- `manual_continue` 写入 quality bypass audit，并继续到 Analyst 指定的下一阶段。
- 达到 `max_auto_rework` 后进入 Human Gate。

### 前端测试

- Timeline 能显示 Analyst decision entry。
- TestingReport 旁能显示“等待 Analyst 决策”或“Analyst 已决策”。
- Human Gate 显示 Analyst 推荐动作。
- `manual_continue` 必填原因。
- quality bypass audit 在后续审查上下文中可见。

### 真实 E2E 验收

使用当前真实场景中的 `skipped_required_steps`：

```text
B6, B7, B8, B9, B10, F3, F5
```

期望流程：

```text
Tester blocked
  -> Analyst 判断为 required 测试覆盖缺口
  -> Coder 补充缺失测试或实现
  -> Tester 重跑
```

只有用户明确选择质量豁免时，才允许继续 CodeReviewer。

## 风险与取舍

### 优点

- 流程语义清晰，职责边界稳定。
- blocked、failed、review findings 都由 Analyst 统一解释。
- 更容易向用户解释为什么回 Coder、为什么重跑 Tester、为什么进入人工门禁。
- 后续扩展更多 Reviewer 或 Provider 时，路由点仍集中。

### 风险

- Analyst 调用次数增加，整体流程更长。
- Analyst prompt 和输出 schema 必须更稳定。
- 如果 Analyst 判断质量差，会导致错误路由。

### 缓解

- 保留结构化 schema 和严格解析。
- 保留 `max_auto_rework`。
- Analyst 决策必须引用 evidence refs。
- UI 展示 Analyst 决策原因，允许用户人工覆盖。

## 推荐实施拆分

1. 后端引入 Analyst decision schema 和持久化。
2. 调整 Testing 后固定进入 Analyst。
3. 调整 CodeReviewer / InternalReviewer 后固定进入 Analyst。
4. 重构 Human Gate action，使 `manual_continue` 明确成为质量豁免。
5. 前端展示 Analyst decision 和 Human Gate 推荐动作。
6. 补充后端状态机测试、前端展示测试和真实 E2E 验收。
