# 3.7 Phase 1b UI 真实链代理预检报告（2026-09-14）

> 执行者：controller 代理（用户授权「先帮我测完，测得没问题告诉我，我再验证」）。
> 环境：生产构建（rust-embed 嵌入式前端，服务器 4317 v16b=引擎 v16 同源直连，零代理）+ 真实 pi provider + 真实 durable 数据。
> 纪律声明：本报告为**代理预检**，不代填 REQ-UI37-12 的人工结论栏；截图五张在本目录。

## 1. 人工清单四项结果（代理预检口径）

| 清单项 | 结果 | 证据 |
|---|---|---|
| 建会话 → 看计划 | ✅（会话创建经 driver 种子——UI 无 workitem 会话创建面，见发现 4；**看计划**经 UI：真实会话 0408 驾驶舱三区渲染，7 节点执行流含计划生成/评审轮原文） | shot-1b-01/02 |
| request-change / confirm / advance | ✅（request-change 轮由 driver 消费后杀 driver 留持久门；**confirm 由浏览器 UI 点击落地**：durable 门关闭→compile→Confirmed→**autopilot 自动 advance**（advance_cmd-advance-2-f10ba15b5f ready→attempt ffb59409 sc_advance 创建，command_id 为前端格式）→终门（确认产物）再开） | shot-1b-02b + durable advance-records |
| 看编码进度 → 看 push | ✅（issue_0253 全链：编码工作区 UI 显示 completed+1/1 单元全阶段+最终提交 d6e5e998ba1f=远端 ls-remote 实测一致） | shot-1b-03 |
| L1-L4 提醒 | ✅ 大部（L2 sticky 跨页横幅「待处理 7 项+去处理」在 /image-create 实拍；L3 标题「🔴待处理×7 · aria」实拍；L2「去处理」点击跨页导航→卡壳会话 0017 实证；L4 升级文案「剩余修复轮次 0，请优先处理」入 banner；L1 toast 基础设施在 shell 挂载、jsdom 全覆盖，真实活抓未取得——断连间隙冻结窗口错过） | shot-1b-04 |

附加验证：设置面板六组控件+默认值+**K 8→12 实时生效（文案即时+localStorage 落盘）**+K=32 时跨页聚合亮起；0199 形态（compile 拒→门重开）在真实链复现并被 UI 正确呈现；重载后从 durable 恢复正确。

## 2. 发现（按严重度）

1. **[P1-体验缺口] driver/连接断开后 UI 反馈路径不可用**：typed 门无活 turn 时（重连/刷新后）UI 显示「等待门禁命令同步后再提交反馈」并禁发反馈；但协议允许客户端自生成 command_id 发 HumanGateFeedback（3.6 driver/amendment 链即此用法）。真实用户刷新页面后即无法 request-change（确认/终止不受影响）。建议 1b 补丁：无活 turn 时用 newCommandId() 发反馈（幂等键语义已具备）。
2. **[P2-dev 工件] vite dev 模式整页重载循环**：observer 15s 周期重连经 vite 代理触发 ws proxy error 洪流→HMR 客户端掉线→全页 reload 循环（生产 rust-embed 同源零代理完全稳定，60s+ 零重载）。登记 backlog：dev 代理优化（不阻产品）。
3. **[P2-设计张力] 后台 tab 断线不重连与后台告警目的矛盾**：shouldReconnect 含 !document.hidden——后台 tab WS 断后永不重连，而 L2/L3/L4 恰为后台提醒设计。建议：后台时降频重连（如 60s）而非禁连。
4. **[观察] UI 无 workitem 会话创建面**：建会话仍需 driver/脚本（3.6 既定边界），1b 范围外；3.7 后续/Phase 4 可考虑工作台直建。
5. **[观察] 「最近 K 个候选」排序无时间戳**：终审已登记的方案 a 口径残余——250 issues 时 K=8 默认窗未覆盖最新卡壳（提 K=32 后亮起）。1b 验收通过不受阻（降级语义已如实呈现），Phase 2 可消化。

## 3. 会话与产物清单

- 真实卡壳渲染：workspace_session_0277（82h 门，快照形态正确）
- 交互链：issue_0255/workspace_session_0408（driver 种子+修订中杀留门→UI confirm→autopilot advance→attempt ffb59409）
- 编码链：issue_0253（driver 全链+coding 段 298s completed，push 远端一致）
- 服务器：4317 重启为 v16b（引擎 v16 等价+新前端嵌入，md5 4c83449f）持续运行

## 4. 人工结论栏

> **人工验收：____**（由用户亲自浏览器走查后填写；建议 5 分钟路径：开 4317 → localStorage 置 aria.chat.cockpit=cockpit → 开 /workbench/workspace/workspace_session_0408 看终门 → image-create 看横幅/标题 → 设置面板改 K。）
