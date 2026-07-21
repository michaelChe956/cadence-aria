# Coding Workspace 审核 JSON Schema 兼容与诊断技术方案

## 1. 背景与问题

Coding Workspace Code Reviewer 当前直接将 Provider 输出反序列化为 `RawCodeReviewProviderPayload`。当 JSON 语法合法、但任一字段不符合 Rust 业务枚举时，整个 payload 会回退为 `blocked_review_payload()`，统一显示“review 输出不是有效 JSON”。

真实 `code_review_0007` 与 `code_review_0009` 均是合法 JSON，但在 `verdict="blocked"` 时生成了 `finding.severity="blocked"`。`ReviewVerdict::Blocked` 合法，而 `FindingSeverity` 仅接受 error/warning/info 及 high/medium/low 等别名，因此整体反序列化失败、合法 summary 和 findings 被丢弃。

当前 Prompt 允许 `verdict=blocked`，却只要求 finding 包含 severity，没有列出 severity 允许值。这会诱导 Reviewer 将 verdict 值复用为 severity，形成稳定复现的 Prompt/Schema 不一致。

## 2. 目标

- 合法的 blocked Review 不再因 `severity="blocked"` 被误判为无效 JSON。
- Prompt 与后端 Schema 使用同一组明确枚举。
- JSON 语法错误与业务 Schema 错误展示不同诊断。
- 保留当前 Coding Review blocked Gate、retry/send-to-coder/abort 路由语义。
- 不修改、重放或自动推进现有 Attempt；修复部署后由用户手动重试 Reviewer。

## 3. 非目标

- 本次不为 Coding Workspace 新增第二次 Provider repair 调用。
- 本次不改变 Work Item 的 Exclusive/Forbidden Write Scope。
- 本次不自动接受 Reviewer 报告，也不自动解除真实 dependency blocker。
- 本次不修改 Workspace Review 的共享 structured-output repair 链路。

## 4. 方案比较

### 方案 A：只修改 Prompt

在 Prompt 中声明 severity 只能为 error/warning/info。

优点是改动最小；缺点是 Provider 仍可能输出别名或旧值，线上兼容性不足，无法修复已观察到的重复模式。

### 方案 B：Prompt 约束 + 兼容解析 + 精确诊断（采用）

在 Prompt 中声明固定值；后端将 `blocked` 作为阻塞级 finding 的兼容别名映射为 `FindingSeverity::Error`；解析失败时根据 `serde_json::Error::classify()` 区分语法错误与数据/Schema 错误，并在 summary 中保留具体解析错误。

该方案不增加 Provider 调用，能够直接修复当前真实输出，同时改善后续诊断，风险和改动面最小。

### 方案 C：引入 Coding Review Provider repair

首次 Schema 失败后启动第二次 Reviewer turn 修复 JSON。

该方案覆盖面更广，但会增加 Provider 成本、权限请求、超时和断流恢复状态，且当前问题可通过确定性的本地兼容解决。本次不采用，可在未来统一 Coding/Workspace structured-output 协议时再评估。

## 5. 详细设计

### 5.1 FindingSeverity 兼容

`FindingSeverity::deserialize()` 新增：

- `blocked` → `FindingSeverity::Error`

现有映射保持不变：

- error/blocker/blocking/critical/high/must_fix → Error
- warning/medium/strong_recommend_fix/suggestion → Warning
- info/low/minor/optional → Info

`blocked` 仅作为输入兼容别名；序列化输出仍为标准 `error`，避免扩展公共输出枚举。

### 5.2 Prompt 契约

Code Review 与 Group Final Review Prompt 明确：

- verdict：`approve | request_changes | blocked`
- finding severity：`error | warning | info`
- 当 verdict 为 blocked 时，阻塞 finding 使用 `severity="error"`，不得使用 `severity="blocked"`。

Prompt 示例继续只输出 JSON，不引入新的外层 sentinel 协议。

### 5.3 解析诊断

`parse_review_payload()` 保留原来的 JSON object 提取行为，但不再丢弃 `serde_json::Error`：

- `Category::Syntax` 或 `Category::Eof`：summary 使用“review 输出不是有效 JSON”，并附带 serde 错误位置。
- `Category::Data`：summary 使用“review JSON Schema 校验失败”，并附带 unknown variant/invalid type 等具体错误。
- `Category::Io`：按解析内部错误处理，使用“review JSON 解析失败”。

fallback 仍生成 `ReviewVerdict::Blocked`、空 findings，并保留原始 Provider 输出引用；Gate 路由不变。

### 5.4 数据与状态安全

- 解析兼容只影响后续新生成的 Code Review Report。
- 当前 `coding_attempt_0001`、`coding_role_run_0020`、`code_review_0009` 和 blocked Gate 不做原地改写。
- 修复部署后，用户手动点击“重试代码审查”才会创建新的 Reviewer Run。
- 如果 Reviewer 再次给出真实 dependency blocker，报告将被正常解析为 blocked，并保留 findings，供用户决定修改 Work Item scope、补上游 handoff 或终止。

## 6. 测试设计

遵循 TDD，先验证 RED：

1. `review_parser_accepts_blocked_finding_severity_as_error`
   - 输入与真实 `code_review_0009` 同构的合法 JSON。
   - 期望 verdict=Blocked、findings 不为空、blocked severity 映射为 Error。
   - 修复前应回退为空 findings。
2. `review_parser_distinguishes_schema_error_from_json_syntax_error`
   - 未知 severity 触发 Schema 诊断。
   - 截断 JSON 触发语法诊断。
3. `code_review_prompt_lists_exact_finding_severity_values`
   - Code Review 和 Group Final Review Prompt 均包含 severity 固定枚举和 blocked 使用 error 的约束。

GREEN 后运行受影响回归：

```bash
cargo test --locked --lib review_parser
cargo test --locked --lib parser_prompt
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
```

所有 Cargo 命令禁止使用 `-j 1`。

## 7. 验收标准

- 真实 `code_review_0009` 同构 payload 可解析为 Blocked，4 个 findings 全部保留。
- `severity="blocked"` 在持久化模型中规范化为 `error`。
- 未知 severity 显示 Schema 校验失败，不再声称 JSON 语法无效。
- 真正截断/畸形 JSON 仍显示无效 JSON。
- blocked Gate 和人工操作路由保持不变。
- 完整 Rust 门禁通过，且现有 Attempt 数据哈希在实现和验证期间不发生变化。
