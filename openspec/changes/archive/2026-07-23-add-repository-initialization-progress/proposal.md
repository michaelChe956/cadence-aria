## Why

当前“添加代码库”操作会在 Cadence-skills 准备和四个 Claude Code 初始化命令全部结束后才返回最终结果。用户在等待期间无法判断系统是否仍在工作、运行到哪个步骤或在哪一步失败，且初始化命令仍可能触发人工交互，影响无人值守完成。

## What Changes

- 将代码库注册改为可跟踪的异步初始化操作：提交后立即返回操作标识，客户端依据服务端真实状态逐项展示进度。
- 为以下五个固定步骤记录并暴露 `pending`、`running`、`completed`、`failed` 终态状态：Cadence-skills 下载与软链、`/rule-config --no-interrupt`、`/pre-check --no-interrupt`、`/mcp-configuration --no-interrupt`、`/project-rules-examples --no-interrupt`。
- 将四个 Claude Code 初始化提示词改为使用 README 定义的 `--no-interrupt` 参数，禁止初始化流程等待人工输入。
- 将“添加代码库”弹窗改为真实五步进度面板：执行中不可关闭、完成后展示既有初始化摘要、失败时停留在失败步骤并展示恢复信息；仅成功完成并持久化代码库后刷新工作台列表。
- 不展示步骤内部百分比、计时估算或模拟进度；不增加初始化日志流、后台继续执行或用户取消功能。

## Capabilities

### New Capabilities
- `repository-initialization-progress`: 为代码库初始化提供可轮询的操作状态、五步真实进度、失败诊断和最终结果。
- `non-interrupt-repository-bootstrap`: 以无中断模式执行 Cadence-skills 的四个 Claude Code 初始化命令。

### Modified Capabilities

- 无。

## Impact

- 后端代码库注册协调器、Claude 初始化器、产品运行时持久化及 HTTP 路由/DTO 将新增初始化操作状态契约与查询接口。
- 前端 API 类型与客户端、添加代码库弹窗和工作台提交流程将消费轮询状态并展示步骤面板。
- 需要扩展 Rust 单元/集成测试，以及 React 组件和工作台集成测试，覆盖成功、顺序推进、失败、轮询终态和无中断命令参数。
- 继续使用现有 Cadence-skills 准备和 Claude Code Provider；不新增第三方运行时依赖。
