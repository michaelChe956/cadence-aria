# Spec Delta

> 基线：`openspec/specs/web-workbench-autopilot/spec.md` 现有 REQ-UI37-01 至 REQ-UI37-18。本 delta 只新增 REQ-UI37-19，不修改既有 requirement；REQ-UI37-06 四层递进卡壳提醒与 REQ-UI37-07 跨会话聚合语义保持不变。

## ADDED Requirements

### Requirement: 驾驶舱全局消费 coding 待办（REQ-UI37-19）

驾驶舱 SHALL 将 issue lifecycle 返回的 `coding_attention_items` 全量纳入全局待办展示与计数：以 `[project_id, issue_id, key]` 为身份，不受 watched 会话列表、近期完成 TTL 与条数上限限制；同一 key 存续期间 MUST NOT 重复提醒，新 key 出现时 SHALL 提示（含系统通知）。待办条目 SHALL 提供「去处理」入口，经统一 target 回调（`onOpenAttentionTarget`）导航到 coding_attempt 详情（focus 定位卡片）或 coding_plan 总览，MUST NOT 把 coding item id 当作 sessionId；浏览器通知权限被拒时平台内消息与提醒仍可用。通知只导航，MUST NOT 在通知上执行确认/终止或参与批量门动作。

#### Scenario: 在别的 plan 也能收到并处理

- **WHEN** 用户停留于 plan B 的页面，plan A 的某 attempt 落需人门（choice/分诊/挂起/最终确认）且 A 不在 watched 列表
- **THEN** 驾驶舱出现 A 的待办条目并计数/提醒；点击后导航到 A 的对应卡片或 plan 总览，处理入口可用

#### Scenario: 同 key 不重复提醒且处理即消失

- **WHEN** 同一待办 key 在多次扫描中持续存在，随后用户完成处理（作答/resolve/确认）
- **THEN** 存续期间不重复弹提醒；事实关闭后的扫描中该条目从展示与计数中消失

#### Scenario: 通知权限拒绝不阻断平台消息

- **WHEN** 浏览器系统通知权限被拒绝
- **THEN** 平台内待办条目、徽标与提醒照常工作，点击导航不受影响
