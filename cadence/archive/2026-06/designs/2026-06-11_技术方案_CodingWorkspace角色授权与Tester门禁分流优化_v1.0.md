# CodingWorkspace 角色授权与 Tester 门禁分流优化技术方案

## 基本信息

- 文档类型：技术方案
- 方案日期：2026-06-11
- 目标分支：`bugfix_test_branch`
- 适用范围：Coding Workspace 真实 Provider 执行链路

## 背景

真实 E2E 测试中暴露出三个相关问题：

1. Coding Workspace 所有 Provider 角色都使用统一的 `Supervised` 权限模式，导致 Tester 等只读/验证角色频繁要求人工授权。
2. Tester 在 `plan_tests` 阶段可能输出 Markdown 最终报告，而后端期望 TestPlan JSON，最终触发 `missing_json_object` 并 blocked。
3. Tester blocked 后主流程仍自动进入 Analyst/Rework，导致测试基础设施或 Provider 输出契约问题被误判为代码返修问题。

本方案采用“角色级授权 + Tester 契约修复 + Testing 结果分流”的闭环优化，不做框架级策略引擎重构。

## 目标

- 支持按 Coding Provider 角色配置授权模式。
- Tester 默认自动授权，减少真实 E2E 的人工点击。
- `plan_tests` 与 `execute_test_plan` 均以结构化 JSON 为后端可信输入。
- Provider 输出不合约时先尝试修复，仍失败才 blocked。
- Testing blocked 不自动进入 Analyst；只有有真实测试证据的 failed 才进入 Analyst。
- 页面明确区分“测试失败”和“测试被阻塞”。

## 非目标

- 不取消所有人工门禁。
- 不把高风险测试步骤默认自动放行。
- 不重构所有 Workspace 的 Provider 权限模型。
- 不修改历史 attempt 产物内容。
- 不将 Analyst 作为 Tester 输出契约错误的默认兜底。

## 设计一：角色级授权策略

### 数据模型

在 Coding role provider config 中增加每个角色的授权模式。建议逻辑模型如下：

```json
{
  "coder": "codex",
  "tester": "claude_code",
  "analyst": "claude_code",
  "code_reviewer": "codex",
  "internal_reviewer": "claude_code",
  "review_rounds": 1,
  "permission_modes": {
    "coder": "supervised",
    "tester": "auto",
    "analyst": "auto",
    "code_reviewer": "supervised",
    "internal_reviewer": "supervised"
  }
}
```

旧配置缺少 `permission_modes` 时按默认值补齐，不要求迁移历史文件。

### 默认值

| 角色 | 默认授权模式 | 原因 |
|------|--------------|------|
| `coder` | `supervised` | 会修改源码，保留人工控制 |
| `tester` | `auto` | 以验证为主，减少真实 E2E 点击 |
| `analyst` | `auto` | 以分析为主，减少非写操作阻塞 |
| `code_reviewer` | `supervised` | 可能影响返修决策，保留人工控制 |
| `internal_reviewer` | `supervised` | 接近最终验收，保留人工控制 |

### 后端行为

当前统一的 `coding_provider_permission_mode() -> Supervised` 应替换为按角色读取：

- Coder 调用读取 `coder` 权限模式。
- Tester 调用读取 `tester` 权限模式。
- Analyst 调用读取 `analyst` 权限模式。
- CodeReviewer 调用读取 `code_reviewer` 权限模式。
- InternalReviewer 调用读取 `internal_reviewer` 权限模式。

Provider adapter 统一消费 `ProviderPermissionMode`：

- Claude Code：
  - `auto`：使用 Claude permission auto。
  - `supervised`：继续使用 `--permission-prompt-tool=stdio`。
- Codex：
  - `auto`：使用非交互 approval policy。
  - `supervised`：使用 `approvalPolicy=on-request`。
- Fake/TestControlled provider：
  - 支持相同字段，便于回归测试。

### 审计要求

自动批准不能静默。每次 auto approval 至少记录：

- role
- provider
- tool name
- risk level
- command/input 摘要
- `auto_approved=true`
- 所属 attempt/node

审计可以先通过 timeline execution event 和 chat entry 表达；后续如需长期合规再抽成独立 audit store。

## 设计二：Tester JSON 契约与修复

### `plan_tests` 契约

`plan_tests` 阶段只允许输出 TestPlan JSON，不执行测试，不输出最终报告。最小结构：

```json
{
  "summary": "验证计划摘要",
  "context_warnings": [],
  "assumptions": [],
  "steps": [
    {
      "id": "unit",
      "title": "Unit tests",
      "intent": "验证单元行为",
      "required": true,
      "tool": "run_command",
      "risk_level": "low",
      "command_or_tool_input": {
        "command": ["cargo", "test", "--locked", "--lib", "provider_dependencies"]
      },
      "evidence_expectation": "命令退出码为 0，并保存 stdout/stderr evidence"
    }
  ]
}
```

### 修复流程

第一次解析失败不立即 blocked，而是发起一次 repair turn：

```text
plan_tests -> parse
  ok -> save TestPlan -> execute_test_plan
  fail -> plan_tests_repair -> parse repair output
    ok -> save TestPlan -> execute_test_plan
    fail -> Testing blocked gate
```

repair prompt 必须包含：

- 原始 provider output。
- 解析错误类型。
- TestPlan JSON schema 摘要。
- 明确约束：只返回 JSON，不要 Markdown，不要解释。

raw output 落盘：

```text
provider-raw/testing/plan_tests_0001.txt
provider-raw/testing/plan_tests_repair_0001.txt
```

blocked reason 细分：

- `test_plan_missing_json`
- `test_plan_invalid_json`
- `test_plan_schema_invalid`
- `test_plan_repair_failed`

### `execute_test_plan` 契约

`execute_test_plan` 阶段必须输出：

```json
{
  "step_results": [
    {
      "step_id": "unit",
      "status": "passed",
      "evidence_refs": ["test-output/unit.stdout.log"],
      "provider_analysis": "单元测试通过"
    }
  ]
}
```

若缺少 `step_results` 或 required step 未覆盖，先进行一次 repair/rerun。仍失败时进入 Testing blocked gate。

### 高风险步骤

`risk_level=high` 的 required step 默认不随 Tester `auto` 权限直接执行。它应进入 blocked gate，并显示 `high_risk_test_step_requires_permission`。后续如需放开，可增加独立配置 `tester_high_risk_auto=true`，不与 Provider permission mode 混用。

## 设计三：Testing 结果分流

### 路由规则

Testing 完成后不再无条件进入 Rework/Analyst，而是按 report 分类：

| Testing 状态 | 条件 | 后续流程 |
|--------------|------|----------|
| `Passed` | 任意 | 继续后续阶段 |
| `PassedWithWarnings` | 任意 | 继续后续阶段，保留 warning |
| `Failed` | 有有效测试证据 | 进入 Analyst |
| `Failed` | 无有效测试证据 | 转为 Testing blocked |
| `Blocked` | 任意 | 停在 Testing blocked gate |
| `SkippedByUserDecision` | 任意 | 进入人工确认或风险接受路径 |

有效测试证据至少满足一项：

- `plan_id != null` 且 `steps` 非空。
- `commands` 非空。
- 存在测试命令输出 evidence refs。

### Analyst 使用边界

Analyst 默认只处理“测试已执行并发现代码问题”的结果。以下情况不自动进入 Analyst：

- TestPlan JSON 解析失败。
- Provider 启动失败。
- Provider 输出缺少 `step_results`。
- required step 缺失。
- 权限、环境或 Provider contract 问题。
- 高风险测试步骤未获批准。

blocked gate 中保留 `send_raw_output_to_analyst` 作为人工选项。只有用户主动选择时，才把 raw output 发给 Analyst 做诊断。

### 当前问题对应结果

本次 `test_plan_parse_failed / missing_json_object` 属于 Tester 输出契约错误，不是代码测试失败。优化后应停在 Testing blocked gate，不应自动进入 Analyst，也不应自动生成 rework instruction。

## 前端设计

### Role Provider 面板

在 Coding Workspace 的 Role Provider 配置区域增加授权模式选择：

```text
Provider: codex / claude_code / fake
Permission: Auto / Supervised
```

展示建议：

- `Auto`：自动批准该角色的 Provider 工具请求，并记录审计。
- `Supervised`：需要人工批准工具请求。

默认值从后端 role config 返回，修改后通过既有 provider config 更新通道或新增 role permission update 消息提交。

### Gate 文案

Testing blocked gate 需要按 reason code 显示更明确的标题：

- `test_plan_missing_json`：Tester 未返回测试计划 JSON。
- `test_plan_invalid_json`：Tester 返回的 JSON 无法解析。
- `test_plan_schema_invalid`：Tester 测试计划字段不完整。
- `test_plan_repair_failed`：Tester 测试计划修复失败。
- `missing_required_steps`：缺少 required 测试步骤证据。
- `high_risk_test_step_requires_permission`：高风险测试步骤需要人工确认。

页面必须区分：

- 测试失败：验证已执行，发现代码问题。
- 测试被阻塞：无法得出有效测试结论。
- Tester 输出契约错误：Provider 输出不符合后端协议。

## API 与 WebSocket

建议扩展 Coding Workspace WebSocket 消息：

```json
{
  "type": "permission_mode_select",
  "role": "tester",
  "permission_mode": "auto"
}
```

也可以复用 provider config 更新接口，但需要避免把 provider name 和 permission mode 绑定得过死。推荐独立消息，语义更清晰。

Session snapshot 应返回当前 role provider 与 permission mode，便于刷新页面后恢复显示。

## 测试方案

### 后端测试

- role permission config 默认值补齐。
- role permission config 持久化与读取。
- Claude Code adapter 正确映射 `auto/supervised`。
- Codex adapter 正确映射 `auto/supervised`。
- Tester `plan_tests` 输出 Markdown 时，repair 成功后继续执行。
- repair 失败时生成 Testing blocked gate。
- Testing blocked 不进入 Analyst。
- Testing failed 且有 evidence 时进入 Analyst。
- 用户主动 `send_raw_output_to_analyst` 时才进入 Analyst。
- auto approval 生成可观察审计事件。

### 前端测试

- Role Provider 面板展示每个角色的 permission mode。
- 修改 permission mode 后发送正确消息。
- Session snapshot 能恢复 permission mode。
- Testing blocked gate 根据 reason code 显示正确文案。
- `test_plan_parse_failed` 不显示成普通测试失败。

### E2E 验收

- Tester 默认 `auto`，执行真实 TestPlan 时不再频繁要求人工授权。
- Coder 默认 `supervised`，仍保留人工控制。
- `plan_tests` 输出 Markdown 的 fixture 可被 repair。
- repair 失败停在 Testing blocked gate。
- Testing blocked 后不自动进入 Analyst。
- Testing failed 且有 step evidence 时进入 Analyst 并可返修。

## 风险与缓解

| 风险 | 影响 | 缓解 |
|------|------|------|
| Auto approval 执行范围过大 | 误执行高风险命令 | 按角色默认启用，high risk step 仍 blocked |
| Provider repair 后仍输出 Markdown | 流程停滞 | 只重试一次，失败进入明确 blocked gate |
| 旧 attempt 配置缺字段 | 读取失败 | 读取时补默认值 |
| Analyst 入口变少影响自动返修 | 某些失败不再自动修 | 只有无有效证据的 blocked 被拦截；真实 failed 仍进 Analyst |
| 前端状态恢复不一致 | 刷新后显示错误 | snapshot 返回完整 role permission config |

## 验收标准

- Tester 默认不再反复弹工具授权。
- Coder/Reviewer 默认仍保留人工授权。
- `plan_tests` 输出 Markdown 时不会直接污染 Analyst 流程。
- `test_plan_parse_failed` 停在 Testing gate，不自动 rework。
- 只有真实测试失败才触发 Analyst 返修。
- 页面能解释清楚当前是“测试失败”还是“测试被阻塞”。
- 自动授权事件可在 timeline/chat 中追踪。
