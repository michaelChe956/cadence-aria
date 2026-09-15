# 3.7 Phase 2 UI 真实链代理预检报告（2026-09-14）

> 执行者：controller 代理（用户策略：能修的修、能验的验、人工项统一打包）。
> 环境：v16d（md5 81e9e3f6，引擎 v16 等价+1a/1b/P1/P2 修复+Phase 2 仪表盘前端嵌入）同源直连 4317；真实 pi provider。
> 纪律：不代填 REQ-UI37-12 人工结论；截图 shot-p2-dashboard.png 在本目录。

## 1. 验证结果

| 项 | 结果 | 证据 |
|---|---|---|
| 仪表盘渲染（完成态 attempt） | ✅ issue_0253：拓扑 1 节点（minimal 单元）+预算门双条冻结显示（剩 58m31s/1h28m）+免责口径+日志空态 | DOM 检查 |
| 仪表盘渲染（运行态 attempt） | ✅ issue_0258/0259：拓扑「WI-001 · 执行中」实时快照→重载后「已完成」+最终提交可见 | 截图+重载对照 |
| 预算门冻结语义 | ✅ 完成态不再播报/pulse（budgetFrozenPulse=false） | DOM 检查 |
| 拓扑 a11y | ✅ div[role=figure]（T7 顺检生效）；节点 tabindex 键盘可达 | DOM 检查 |
| 依赖链视图 | ✅ 按设计自隐藏（minimal 语料无依赖链；组件空态行为正确） | DOM 检查 |
| **实时日志流** | ⚠️ **架构受限未验成**（见发现 1） | 160s 保活轮询 0 行 |

## 2. 发现

1. **[P2-架构事实] 被动连接看不到 driver 启动的编码运行实时事件**：coding WS 为每连接私有事件通道（与 1b observer 发现同族，socket.rs 引擎按连接新建）——driver 连接驱动编码运行（coding11 其 WS 收到 1122 条 coding_stream_chunk，durable 全链 completed），但浏览器页面的连接**零事件到达**：日志控制台 0 行、状态停留连接时快照（「执行中」直到重载才变「已完成」）。T5/T6 的接线与虚拟化/贴底已被 jsdom 31+5 用例覆盖，服务端事件流存在性经 driver 侧证实；缺口在**跨连接广播**（破引擎零改动 C1）或**UI 侧启动编码**（驾驶舱连接自身成为驱动连接，属 Phase 4「操作收口」范围——coding 页尚无「开始编码」入口）。处置建议：Phase 4 补 UI 启动编码入口后复验实时日志+贴底+定高链三项；后端广播登记 3.7 后续 backlog。
2. **[验证边界] 贴底/定高链/multi-node 拓扑依赖连线**：均需实时日志流或多单元语料（levels）才能在真实浏览器观察——与发现 1 同因受限，源码级（max-h-[40vh] 类断言/latestLineId 依赖/分层算法单测）已覆盖，留人工清单。

## 3. 人工清单（并入统一终验包）

1. 打开 `/workbench/projects/project_0001/issues/issue_0259/coding/coding_attempt_84c404dd65964d41b673b80b3d031415` 看仪表盘四件（拓扑已完成态/预算冻结/日志空态）
2. 视觉过 SVG 拓扑 focus ring（tab 键）与图例
3. Phase 4 后补：UI 启动编码→实时日志增长/贴底跟随/多节点拓扑
