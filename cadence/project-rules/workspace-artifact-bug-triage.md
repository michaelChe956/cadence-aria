# Workspace 产物链路 Bug 三模块联动排查规则

## 适用范围

当 Bug、展示异常、状态恢复异常、交互定位异常、审核/返修流程异常涉及以下任一产物 Workspace 时，本规则必须适用：

- Story Spec
- Design Spec
- Work Item

## 强制要求

- 不得只按当前反馈的单一产物类型做局部判断。即使用户只提到 Story Spec，也必须检查 Design Spec 与 Work Item 是否复用同一套后端流程、前端页面、状态 store、timeline/chat 重建逻辑或测试夹具。
- 若三者复用同一链路，应优先在共享层修复，并补充覆盖 Story Spec、Design Spec、Work Item 三种 workspace type 的回归测试。
- 若某个产物类型不受影响，必须在结果汇报中明确说明不受影响的原因，例如代码路径不同、节点类型不同、功能未启用或测试数据不覆盖该流程。
- 修改涉及以下节点或阶段时，必须特别检查三种产物类型的一致性：
  - `author_run`
  - `author_confirm`
  - `reviewer_run`
  - `review_decision`
  - `revision`
  - `human_confirm`
  - timeline 节点恢复
  - chat entry 重建
  - artifact version 绑定
  - 左侧 timeline 定位到右侧对话/产物内容

## 验证要求

- 前端共享逻辑变更时，优先使用表驱动测试覆盖 `story`、`design`、`work_item`。
- 后端 Workspace Engine 或 WebSocket contract 变更时，若行为应跨产物一致，单元测试或集成测试必须覆盖全部适用 workspace type，或在测试说明中写明排除理由。
- 汇报验证结果时，应明确说明三种产物类型是否已覆盖。
