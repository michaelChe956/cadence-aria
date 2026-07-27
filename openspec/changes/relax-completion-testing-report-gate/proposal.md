## Why

Coding Workspace 的生产 pipeline 当前未编排 Testing 阶段，却在最终完成门禁中强制要求 required verification check 对应的 Passed testing report，导致 Coder、Code Reviewer 和 Internal Reviewer 均通过后 attempt 仍无法完成。产品已确认至少未来半年不引入 Testing 阶段，因此完成语义应以 internal PR review 通过及既有非 testing 门禁满足为准。

## What Changes

- 移除 Coding Workspace 所有完成路径对 testing report 的强制要求，包括 schema v2 group、legacy group 与 single-attempt 路径。
- 保留既有非 testing 完成门禁：运行时/文件范围校验、可见 handoff、unit 完成状态、共享 worktree 清洁性及其他已有一致性检查仍须满足。
- 当 internal PR review 已通过且其他非 testing 门禁满足时，attempt 可以进入完成状态，即使 canonical contract 声明 required verification check 且不存在 testing report。
- 保留 Testing stage、tester 配置、testing report 模型与存储等现有基础设施；本 change 不新增开关，也不清理 Testing 相关代码。
- 保留 `VerificationGateResultMissing` 公共错误变体，避免无必要的公共类型变更，并为未来可能恢复 Testing 门禁保留兼容性。
- 历史或当前已停滞 attempt 不做自动迁移或自动重放；部署后需通过现有恢复/重试入口重新触发最终完成流程，或新建 attempt 验证。

## Capabilities

### New Capabilities

- `coding-workspace-completion`: 定义 Coding Workspace 在 Testing 阶段未启用时，以 internal PR review 和非 testing 门禁作为完成标准的行为。

### Modified Capabilities

无。

## Impact

- 后端完成门禁：`src/product/coding_workspace_engine/gates.rs`、`src/product/coding_workspace_engine/gates/schema_v2.rs`。
- 测试：schema v2 completion gate 回归测试，以及断言旧 testing-report 门禁语义的 legacy 集成测试处理。
- UI、API、数据模型、Provider 调用与持久化格式不变。
- Testing 相关基础设施继续保留，但不参与 attempt 完成判定。
