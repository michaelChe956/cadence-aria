# Tasks: cockpit-fullcourse-connection-resilience

**工作包性质**：本文件只登记**高层工作包**与验收口径（映射 specs 的 requirement 与 design.md 的 D 系列决策）。精确文件、命令、测试与提交步骤由 `superpowers:writing-plans` 展开到 `cadence/plans/`；Plan 只能展开工作包，不能重定义契约。

**主契约**：`cadence/designs/2026-09-16_设计文档_3.8驾驶舱全程化与连接韧性_v1.0.md`（权威）+ 三方决议 `p38-tripartite-decision.md`（决议承接对照表 = 设计文档附录 B）。

**全局边界（每工作包均继承）**：

- 3.7 已验收交互面语义零改动（红线）：快捷键/批量同会话单门/审计三层/四码归因/ConfirmTwiceButton 与 GateFeedbackEditor 归一/返回父级双链/autopilot 锚点/四层提醒/双轨开关——只做加法（设计文档 §6.1）。
- **R-2 口径纪律**：归因日志落地并经一次复现闭环前，任何文案不得宣称「断连主嫌已根治」（热修 af4f7ccf 表述限「消除已证实的前端断连源」）（REQ-WCR-05）。
- **R-3 时点纪律**：P3a/P2/P3b 均在本 change 批准后实施（REQ-WCR-05 场景、设计文档 §6.3）。
- **R-4 矩阵纪律**：RCA §6 四条矩阵为 P2 done 定义，不可裁剪；排期滑窗 = 整体 defer，不裁矩阵（REQ-WCR-07）。
- P1 全部工作包保持引擎零改动（C1 安全面）；阶段 4 边界不抢先（legacy 不删、role 不收紧必填、DEF-4/5/6 不做）。

**排序总则**（决议 §五/§三.4）：首批 = WP1 ∥ WP2（明早用户可验证）→ WP3 → WP4（族内串行：P3b 最前 → manager → 角色+close → cursor/广播；P2 引擎主体与 WP3 真并行，P2 前端尾巴排在 P1 后）→ WP5。

## 1. WP1 首批·P3a 安全/归因包 + driver readback 前置（OpenSpec 批准后）

- [x] 1.1 连接级中性归因日志：服务端 connection_id + receiver 退出类型（close code/reason/EOF/error/idle 标记）+ session/run token/drive depth/最后双向活性时间；前端 onclose code/reason/wasClean/visibilityState/最后 pong 时间进诊断面；过渡期同 connection_id 写入既有 `aborted_by_disconnect` detail 供关联（REQ-WCR-05）——验证：人为制造四种 close（server idle/前端 4000/卸载 1000/TCP drop）各一次，中性记录唯一归因（RCA §6-③ 归因半链）；断言载体中性（未编码为业务终态）；过渡期关联字段在场
- [ ] 1.2 快照门投影终态守卫：引擎 stage 终态或门相位失配时门控件锁定+原因说明，无 confirm 可达发送路径（0017 形态）（REQ-CFC-05）——验证：0017 形态复现（终态会话门不可操作、无 `INVALID_MESSAGE_FOR_STAGE` 可撞）；引擎真实门开（stage=human_confirm 且相位一致）不受误伤的反例；守卫不改引擎拒绝语义（引擎侧既有拒绝测试不变）
- [x] 1.3 driver readback 修复：`stage3_group_snapshot_readback` `elapsedMs is not a function`（driver 脚本 JS bug）修复+复跑（M5-4，Q4 必做且前置）——验证：readback 复跑零错（RCA §6 矩阵 driver 种子效率恢复）
- [ ] 1.4 WP1 关闸——验证：1.1-1.3 证据齐备；R-2 口径检查（全部文案无「根治」宣称）；改动面=服务端连接日志（增量诊断）+前端守卫+driver 脚本，无 P2 半成品混入

## 2. WP2 首批·P1 薄纵切（明早用户可验证）

- [x] 2.1 形态路由默认表演化：未显式设置的已知类型（story/design/plan 含未开始态）默认 cockpit；显式设置优先/跨会话归属守卫/未知类型安全 legacy 全保留；「空 plan 必须 legacy」前提删除（守卫不删）；首帧定形态（消除中途 unmount+close(1000)+重连）；类型感知三轮防回归既有测试同步迁移（不删用例）（REQ-CFC-04）——验证：终态判定表四行用例（未设置 story→cockpit/空 plan→cockpit 开始按钮可达/显式 legacy 尊重/未知安全 legacy）；跨会话首帧不串形态；无中途形态切换重连的专项回归
- [x] 2.2 cockpit 生成入口+Provider 配置接线：复用 `ChatInputBar`+`handleStartGeneration` 语义（stage/连接状态/recoverable run/optimistic entry）与 `ProviderConfigPanel`+既有 store（单一状态源）；不另开生成 socket、不建第二份 provider 状态（REQ-CFC-01、REQ-CFC-02）——验证：cockpit 内发起生成用例（守卫齐备）；单一状态源断言（无第二份拷贝）
- [ ] 2.3 WP2 关闸（薄纵切真实链）——验证：真实链人工证据——cockpit 内选 Provider → 开始 story/design 生成 → 看到流式/阶段推进 → 到达门，全程不切 legacy、不开命令行；前提确认=现有 ChatEntryList+timeline 已满足流式可见（不满足则按设计 §2.3 升级移入 P2 契约并如实呈报）

## 3. WP3·P1 全量

- [x] 3.1 生成入口全量：plan 未开始态入口与发起流完整语义（含 recoverable run 提示路径、乐观条目）（REQ-CFC-01）——验证：plan 未开始态真实链入口用例；recoverable run 呈现用例
- [x] 3.2 生成过程运行态完整呈现：运行中状态标识（provider/阶段/已耗时）+空/失败/完成三边界；只消费既有 WS 事件面（REQ-CFC-03）——验证：四边界用例（运行中/空态引导/失败可辨识/完成可达）；未新增后端事件/字段断言
- [x] 3.3 Provider 配置完整面：默认 Provider 记忆（前端本地）+可编辑阶段暴露+运行中只读（REQ-CFC-02）——验证：默认 Provider 生效用例；运行中只读用例
- [x] 3.4 形态路由回归全量：守卫清单五条专项回归+既有测试迁移收尾（REQ-CFC-04）——验证：全部迁移用例通过且无删除；显式 legacy 全流程仍可用
- [ ] 3.5 WP3 关闸（P1 全量真实链）——验证：story/design+plan 真实链 UI 全程——cockpit 内发起（含 Provider 选择）→流式进度可视→门点确认→advance→coding→落地，全程不开命令行、不切 legacy；显式 legacy 回滚演练（翻开关回旧形态可用）；3.7 交互面语义零改动自查（红线清单）；引擎零改动核对（改动全落 `web/src/`）

## 4. WP4·P2 族（P3b 族内最前；族内串行）

- [x] 4.0 P3b phase 回门节点 + REQ-CG-04 补句（**族内最前**）：approval compile 失败/修订中止后 phase/active_node 回滚到门节点（新状态写，不改事件前缀）；REQ-CG-04 人工权威升级语义显式化补句（契约化 4f34ba58，零行为变更）（REQ-CG-04、REQ-CG-05）——验证：0429 形态复现（断连中止快照门 confirm 后门真实关闭或明确可诊断拒绝，非 STAGE_INVALID 死拒）；compile 失败后 confirm 可达用例；事件前缀不可变断言；既有 REQ-CG-04/05 场景全数保持；承接 3.7 挂账「C 正向门关闭」（Q5 移交）
- [ ] 4.1 存量卡死 session 复活验证（随 4.0 落地）：0166/0167/0168/0199/0277/0429 逐个复活验证（复活续链或可诊断终态化），登记 defer-ledger，不回写历史（REQ-WCR-06）——验证：每个 session 有登记结果；无 durable 历史改写
- [x] 4.2 session-owned run manager 骨架：`ActiveRun`（token/cancellation/command 通道/state/lease epoch）+单一 engine 实例+单一原子临界区（token+epoch 判等）+内存回收点+进程重启诚实恢复（REQ-WCR-01）——验证：连接关闭 run 持续；重连无第二 engine 实例；生命周期交错并发用例；重启恢复口径用例
- [x] 4.3 driver/observer 角色与 lease：Hello 可选 `role` 字段双字段容忍+服务端缺席归一内部常量+仲裁只认内部角色；observer 写服务端拒绝+可诊断错误；会话级单活性 lease（关闭撤销不取消 run/显式接管 epoch 递增/旧 epoch 迟到写被拒）；driver 脚本族零强制升级（REQ-WCR-02）——验证：缺席归一用例；observer 拒写用例；driver close 只撤 lease 用例；接管与旧 epoch 拒绝用例；新旧 wire 双向兼容用例；6 处脚本 hello 发送点零改动核对
- [x] 4.4 close 终态语义：连接关闭不写 `aborted_by_disconnect`/不标 Failed/不整流回 prepare_context；真实取消（显式 abort/supersede）照旧；完成与关闭并发唯一 terminal（REQ-WCR-03）——验证：0437 形态复现（门开 104ms 窗口断连不覆盖）；0429 业务终态不被改写复现；真实取消路径既有测试全绿；并发唯一 terminal 用例；过渡期归因关联随架构消失的切换核对
- [x] 4.5 cursor 重订阅+session 级广播：session-scoped 单调 `event_seq`+Hello 可选 `after_event_seq`+有界 journal+cursor 过旧发 snapshot 基线+客户端按 seq 去重+慢订阅者有界通道降级不反压+每连接独立出站泵；广播范围限 workspace WS（coding 广播=条件扩展，触发时单独估时，不蚕食本包）（REQ-WCR-04）——验证：断线重连回放不丢不重（RCA §6-①）；cursor 过旧 snapshot 基线用例；慢订阅者不反压用例；被动连接实时收事件用例（盲区消除）；关一连接不影响其他用例
- [x] 4.6 P2 前端尾巴（排在 WP3 后）：hello 新字段/重连逻辑/websocket 类型；observer 连接声明 `role=observer`（REQ-WCR-02/04 前端侧）——验证：与 4.3/4.5 的 wire 契约用例在前端侧贯通；`useWorkspaceWs` 族既有测试迁移
- [ ] 4.7 WP4 关闸：RCA §6 四条断连矩阵全量真跑（不可裁剪）——①intensive-throttling flag tab 隐藏超阈值（ping 实际间隔/服务端不误关/run 持续无 `aborted_by_disconnect`）；②human-gate 静默>60s（前端不自关 4000/服务端不 idle close/终态=业务结果）；③四种 close 唯一归因+durable terminal reason 各自正确；④driver+observer 多连接（关 observer 零影响/driver close 只影响订阅与 lease/run 持续）（REQ-WCR-07）——验证：四条矩阵证据齐备+回退读验证（旧二进制读新数据无断裂）+存量处置记录（4.1）；排期滑窗时本包整体 defer（不裁矩阵）

## 5. WP5·P4 清尾

- [x] 5.1 跨会话批量确认（**P2 条件项**）：P2（WP4.2-4.5）落地则交付——收件箱跨会话多选批量确认，依赖服务端角色仲裁（无差别可命令竞态消除）与 session 广播（结果跨连接可见）；每会话恰一次；危险动作不参与；P2 滑窗则整体 defer 至阶段 4 前夜并登记 defer-ledger 呈报（REQ-CFC-06）——验证（若交付）：两个真实会话各有开态门时批量确认各自恰一次；其他连接经广播观察到关闭；危险动作反例
- [x] 5.2 杂项清算清点表：M5-6 逐项「已修/已 defer+理由」——timer 节流=P2 重连重订阅架构无害化口径登记（非「不修」）；vite dev 重载 defer（生产稳定）；L1 toast 活抓 defer（证据不可得如实留档）；mock 工厂 newCommandId 修（随 WP2/3 测试迁移同批）；0017 污染条目=文档处置；截图冻结吞点击 defer（测试工件）；M1-5 arch-code 入口=显式 defer（文档指引 127.0.0.1:4317 直连）——验证：清点表逐项有处置与理由，无沉默项
- [ ] 5.3 WP5 关闸——验证：清点表齐备；（若 5.1 交付）跨会话批量真实链证据；全程 R-2 口径终检
