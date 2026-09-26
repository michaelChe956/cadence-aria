# Tasks: work-item-group-autopilot

**工作包边界**：仅登记 P0–P3 高层交付与可观察验收；精确实施文件、命令、测试及提交由获批契约后的 `writing-plans` 展开至 `cadence/plans/`。以下 `REQ-WIGA-*` 指 `specs/work-item-group-autopilot/spec.md`；三份 MODIFIED delta 各自保留原 requirement id。各 Phase 必须分别交付替身/自动化与真实 provider/人工链两份证据清单；替身不得冒充真实链。所有工作包均继承默认 off、恰一 logical repository、Interactive 人工 plan 门、Final Confirm 人工门与非 enrolled 零回归约束。

## 1. P0 — 契约、归属与无 driver 控制面（不开放自动触发）

- [ ] 1.1 固化三处既有契约例外与五条红线、单 target 限制，并同步登记 `src/product/workspace_engine/advance_split.rs:37` 注释随实施修改，不将 auto coding 启动描述为随 advance 发生（REQ-ADV-05、REQ-CG-04、REQ-MTG-03）——验收：advance 仍只到 Ready；旧多 target 逐仓人工语义和门关闭≠Confirmed 的边界一致；注释不再暗示只有人工 WS 可发 StartCoding。
- [ ] 1.2 建立 durable enrollment/策略修订/精确源绑定与默认 off 基础，先提供服务端归属投影并使前端 `useCockpitAutopilot` 对 owner=server 退位；本阶段不发任何自动 prepare/advance/coding 动作（REQ-WIGA-01、REQ-WIGA-02、REQ-WIGA-08）——验收：旧记录缺 enrollment 按 client/manual、首帧归属未知不抢发；归属变化清理旧意图；启停并发以修订线性化，未消费许可可撤销。
- [ ] 1.3 把 workspace 和 coding choice 的 REST 答复、WS/REST 共用认领/真实交付回执及驾驶舱就地卡片接通；保持原人工计划门、compile recovery 无 driver 入口和 observer 只读权限（REQ-WIGA-05）——验收：完整多问题答案、HTTP 200/202/404/409/410、同键重试、并发 first-wins、回执超时/旧 run 失效可见；不等 provider 长时 engine 锁，pending 卡不提前消失。
- [ ] 1.4 P0 双清单关闸（REQ-WIGA-05、REQ-WIGA-08）——替身清单：HTTP/WS 竞争、重复/异 payload、过期/取消/重启回执及只读 observer 拒写，旧会话及原手动行为对照；真实链清单：手动启动→关闭 driver→只在驾驶舱回答 choice、处理人工计划门及 compile recovery→运行续接/刷新一致；确认**尚无自动触发**。

## 2. P1 — design 选择后后台 plan 生成、人工批准与确认信息

- [ ] 2.1 在人工 design 确认后接「自动化模式」选择，持久化绑定的 story/design 身份版本、provider/options 与唯一 logical repository；不自动化不创建 enrollment，不扫描既有 plan（REQ-WIGA-01、REQ-WIGA-02、REQ-MTG-03）——验收：默认不自动化、同键重选幂等、异源冲突/零或多 target fail-closed；story/design 生成及确认仍只由人操作。
- [ ] 2.2 引入薄编排器的 `EnsurePreparedPlan`/`StartPlanGeneration` 两动作，使用 enrollment-bound 创建身份、共用 prepare/manager run 面与启动扫描/有界补偿；choice/人工计划门及 compile recovery 都停等人（REQ-WIGA-03、REQ-CG-04）——验收：plan 已建/绑定未写中窗恢复同 plan/session、活 run 重复唤醒不被 supersede、丢事件/关页/重启都能收敛，Interactive 不会绕过门。
- [ ] 2.3 从真正成功的 publication/compile + durable Confirmed 派生 plan confirmed 信息，与驾驶舱门/恢复卡联动，不因点击 approve 或 compile 失败误报（REQ-WIGA-07）——验收：同一事实去重、失败及 recovery 不报成功、通知不增加待处理数。
- [ ] 2.4 P1 双清单关闸（REQ-WIGA-01、REQ-WIGA-02、REQ-WIGA-03、REQ-WIGA-07、REQ-CG-04）——替身清单：重复 enroll/多 plan、不确定中窗、关闭/重开、漏唤醒、活 run 竞争、人工门与 compile 失败恢复；真实链清单：人工确认 design→选自动化→关工作区→驾驶舱处理 choice/门/recovery→唯一 plan Confirmed 及一次提示，并验证无页面重启可续接、不自动批准。

## 3. P2 — 后台 advance、共用 coding 首启与无 socket 运行

- [ ] 3.1 接 `AdvancePlan` 与共用 typed `StartCoding` 服务，WS 人工与 enrolled 共用 Ready/绑定准入、授权复核、per-attempt 单发与 runner reservation/start barrier，不裸调 runner、不把首启塞进 advance（REQ-WIGA-02、REQ-WIGA-03、REQ-WIGA-04、REQ-ADV-05、REQ-MTG-03）——验收：仅单 target Confirmed→Ready→独立首启；未 Ready 报 `SC_CODING_REQUIRES_ADVANCE`、关闭后未消费许可不启动、手工/后台并发只一 runner、Failed/Aborted 不隐式 restart、多 target 无 sibling fan-out。
- [ ] 3.2 实现无人打开 Coding Workspace 时的首启与已认领 run 恢复，并解除 amendment 业务应用/恢复对 live coding socket/事件写回执的承重依赖；保留真实未送达事实和补读（REQ-WIGA-03、REQ-WIGA-04、REQ-WIGA-06）——验收：journal/barrier 各中窗无重复 plan/attempt/provider 首启，零 socket 下 choice/amendment/resume 可推进，观察端慢/断连不改变业务终态、不伪造 Delivered。
- [ ] 3.3 以 durable `WaitingForHuman ∧ FinalConfirm` 加完整 readiness 快照为编码执行结束事实，派生「待最终确认」信息与 Coding Workspace 下钻；真正 `Completed` 仅在人工 Final Confirm 后出现（REQ-WIGA-07、REQ-WIGA-08）——验收：执行结束立即一次提醒、人工不代点、未最终确认不宣称全交付、不增加待处理数。
- [ ] 3.4 P2 双清单关闸（REQ-WIGA-03、REQ-WIGA-04、REQ-WIGA-06、REQ-WIGA-07、REQ-ADV-05、REQ-MTG-03）——替身清单：Ready/精确授权/并发守卫、journal+启动 barrier 中窗、关闭/重开、Failed/Aborted 与旧 attempt、零 socket choice/amendment/resume、慢 observer 与多 target 禁令；真实链清单：plan 批准后不打开 coding 页自动 advance 并启动 coding，关闭所有订阅覆盖一次 choice、一次 amendment/恢复及进程重启，重开能读 durable 产物；通知在 FinalConfirm 等待时发，人工最终确认后才实际 Completed。

## 4. P3 — 有界补读、通知联动与全链关闸

- [ ] 4.1 提供有界近期 durable 完成读取，驾驶舱拆开 `displayItems`、`actionableCount`、`notificationCandidates`，以稳定 key/occurred_at 实现 TTL、首次 hydration 只展示、后续新事实提醒去重（REQ-WIGA-07）——验收：K=8 观察窗口外近期完成可补读，重连/乱序/漏事件及刷新不续期、不通知风暴；信息不进入标题/角标待处理计数、批量或危险动作。
- [ ] 4.2 核实 WIG 入口编码仪表盘、Coding Workspace 实时/重放按钮、Final Confirm 等待/实际终态的文案与上下文；全范围保留非 enrolled 手工链语义（REQ-WIGA-07、REQ-WIGA-08、REQ-MTG-03）——验收：后台无需页面持续流式渲染；默认 off、手动 choice/plan/advance/coding 的门禁、错误与恢复对照一致；多 target 自动启动仍严格关闭。
- [ ] 4.3 逐项关闸 Issue 1 十五条并产出两模式双清单证据（REQ-WIGA-01～08、REQ-CG-04、REQ-ADV-05、REQ-MTG-03）——替身清单：重复/乱序/丢唤醒、K 外完成、首次补读、授权隔离、choice 各结果码、单发与五红线逐项通过；真实链清单：不自动化与单 target 自动化各完整跑一遍 story→design→plan 人工批准→advance→coding→人工 Final Confirm，逐条映射需求 #1–#15，实际关闭 driver/observer 与重启覆盖 choice、recovery、amendment，核对通知联动及人工不可跳过。resilience WP4.7 可共享断连证据但独立挂尾，不阻塞本 change。

**Issue 1 逐条验收映射（4.3 清单核对依据）**：#1 人工 story→design（REQ-WIGA-01、08）；#2 design 后模式选择（01）；#3 不自动化旧流程（01、08）；#4 自动触发 plan、人工确认仍在驾驶舱（03、05、REQ-CG-04）；#5 后台生成→批准→compile→advance→coding→结束（03、04、06、07）；#6 WIG 查看进度（08）；#7 确认和 advance 后自动首启（03、04、REQ-ADV-05）；#8 Coding Workspace 实时下钻（08）；#9 后台无需流式渲染（03、08）；#10 choice、人工门、compile recovery 驾驶舱动作（05）；#11 plan confirmed 通知（07）；#12 coding 执行结束/待最终确认通知（07）；#13 提醒与待处理联动（05、07）；#14 业务流程及人工门不变（02、03、08、REQ-CG-04）；#15 非 enrolled 零回归（01、08、REQ-MTG-03）。
