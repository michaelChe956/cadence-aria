## Why

Work Item Draft 的 Provider 输出目前可以通过 JSON 解析，却因 Canonical Contract 的跨字段引用、必需验证命令和 blocker 目标不一致而在本地语义校验中失败。失败时“接受”按钮仅被隐藏，用户无法在确认区获知原因，也无法确保重写会携带具体纠错信息。

需要将 Draft 生成从单次 Prompt 尝试提升为可观测、可测试、可调优的质量闭环，同时保留无效 Draft 不得接受的硬门禁。

## What Changes

- 新增 Work Item Draft 生成可靠性能力：以 ID 注册表、可信验证命令目录和跨字段自检规则约束 Provider 输出；对可修复的语义校验失败执行一次有界自动修复。
- 新增 Prompt 试运行验证：只使用一至两个脱敏案例对操作者指定的 Provider 进行临时验证；不新增运行时评估模块、CLI、持久化报告或正式接口。
- 新增 Draft 校验失败反馈能力：在当前确认区直接展示错误摘要和完整错误入口；重写操作自动携带校验反馈。
- 保持现有安全语义：校验失败的 Draft 仍不可接受，自动修复失败后仍由用户决定重写或暂停。

## Capabilities

### New Capabilities

- `work-item-draft-generation-reliability`: 生成、校验、一次自动修复及质量评估的可验收行为。
- `work-item-draft-validation-feedback`: Draft 校验失败时的确认区反馈、可访问性和重写反馈行为。

### Modified Capabilities

无。

## Impact

- 后端：Work Item Draft Prompt、输出 Schema、Canonical Contract 校验失败后的重试编排、评估记录与测试夹具。
- 前端：Work Item Draft 确认区、Artifact 错误入口和相关状态展示。
- WebSocket：复用现有 `validator_findings` 与重写 feedback 字段，不新增对外消息类型。
- 运行成本：Claude Code Prompt 试运行仅在操作者明确授权后执行，不加入 CI，也不写入版本库。
