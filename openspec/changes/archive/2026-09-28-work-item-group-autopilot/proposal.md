# Proposal: work-item-group-autopilot

## Why

现行 Work Item Group 从确认 design 到 coding 虽已有 durable 会话和驾驶舱观察面，plan 准备/生成、advance、coding 首次启动仍依赖页面命令，choice 仍依赖 driver WS；关闭页面后流程会卡住。需要为明确授权的单 target issue 增加后端续跑与驾驶舱就地处理能力，同时保持人工审批、最终确认及非自动化路径原样。

## What Changes

- design 由人确认后显示「自动化模式」选择；默认不自动化。选择自动化才创建持久、绑定确切 story/design 版本、plan/session 和唯一 logical repository 的 issue enrollment。关闭授权阻止尚未认领的动作，绝不顺带取消已启动 provider；旧 issue/旧 plan 不自动接管。
- 同进程薄编排器依据 durable 事实依次请求准备 plan、启动生成、等待人批准及既有 compile 成功、调用独立 advance、对单一 Ready attempt 发起一次共用 StartCoding。事件仅负责唤醒，重启扫描及有界补偿使页面/WS 不成为进度条件；plan provider 仍由 session manager 唯一驱动，coding 经现有 runner，advance 本身只到 Ready。
- 驾驶舱加入 choice 的 REST 直接答复通路，REST/WS 共享仲裁及真实交付回执；plan 人工门、compile recovery 和 coding 阶段的可答 choice 均有可见、可操作入口，observer 始终只读。计划确认与编码执行结束（durable FinalConfirm 等待态，文案「待最终确认」）的通知从持久事实派生、和待处理列表同处展示，但信息条目不增加待处理计数；实际 Completed 仍须人工 Final Confirm。
- WIG 入口沿用现有编码进度仪表盘及 Coding Workspace 下钻入口；后台执行不需要流式页面渲染。服务端归属的会话使前端 autopilot 退位，手动会话仍保持既有操作方式。
- **契约变更**：修改 `work-item-plan-conversational-gate` 的服务端已授权推进例外、`work-item-plan-advance` 的明确 opt-in StartCoding 通道及 `multi-target-group-coding` 的单 target 边界。零默认授权、每 attempt 单发、绝不批量、唯一人工门不变。
- **不包含**：多 target 自动拉起及跨 target 依赖调度、自动批准 plan 或代点人工 Final Confirm、另建通知数据库/第二个 provider 驱动点、Issue 2 引导向导；resilience WP4.7 独立挂尾、不作为本 change 前置条件。

## Capabilities

### New Capabilities

- `work-item-group-autopilot`: 单 target enrollment 的后台 plan→advance→coding 编排、驾驶舱无 driver 交互及 durable 完成信息投影、可恢复幂等与手动路径隔离。

### Modified Capabilities

- `work-item-plan-conversational-gate`: REQ-CG-04 的 advance 发起者允许仍授权的 enrolled 编排器，但门关闭不等于 Confirmed，也不触发 advance。
- `work-item-plan-advance`: 修改 Purpose 和 REQ-ADV-05，原条件 defer 在本 change 中显式定义为独立 opt-in StartCoding 应用命令；advance 仍只到 Ready。
- `multi-target-group-coding`: REQ-MTG-03 明确本次 enrollment 限恰一 logical repository/单 attempt；多 target 原人工逐 target 规则不变。

## Impact

- 后端涉及 issue/lifecycle 持久层与 plan preparation、workspace session manager/choice bridge、advance 和 coding runner/attempt store 的共用命令边界；amendment 的业务应用必须解除 live coding socket 依赖，同时保留真实投递状态。前端涉及 design 确认后的选择面、会话归属、驾驶舱收件箱/提醒及现有编码进度下钻。
- 增加 REST choice 答复与必要的策略/归属/完成补读接口或投影，不改变 observer 只读权限；既有 WS 手工命令通过共用服务保持可用。持久化新记录缺席按 off/Manual 解释，不批量迁移旧 plan。
- 按 P0 控制面→P1 plan→P2 coding→P3 补读与全链关闸交付；每段均须分别证明替身回归与真实 provider 链，人工和不自动化对照不可用替身替代。
