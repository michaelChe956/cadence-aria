## Why

Work Item Group 的 Outline、Draft 与 Final Compile 可以成功生成并持久化正式 revision，但确认后创建子 Workspace 时仍依赖缺失的运行期 Work Item 记录，导致子 Workspace 上下文初始化失败。该断链让已确认的 Group 无法进入后续执行流程。

## What Changes

- 在 Final Compile 成功后、创建 Work Item 子 Workspace 前，从已编译的正式 revision 生成运行期 Work Item 投影。
- 将运行期投影作为既有 Workspace 与 Coding 链路的兼容输入；正式 revision 继续是 Work Item 内容与版本的唯一权威。
- 将 Plan 确认的完成条件扩展为：运行期投影、子 Workspace 与其启动上下文均已准备成功；任一步失败不得报告确认成功。
- 为初始 Final Compile 的完整链路增加回归测试，并验证 Story、Design Workspace 未受影响。
- 不回填或修改本变更实施前已确认的 Work Item Group。

## Capabilities

### New Capabilities

- `work-item-runtime-projection`: 将已编译 Work Item revision 安全投影为后续运行链路可消费的运行期记录，并保证确认后的子 Workspace 可初始化。

### Modified Capabilities

无。

## Impact

- 受影响后端：Work Item Plan Final Compile、Lifecycle Store、Work Item Workspace 上下文构建和 WebSocket 确认流程。
- 不改变 Provider 输入输出、前端 API 或既有正式 revision 数据格式。
- 需要新增 Rust 单元测试与端到端/集成链路测试。
