# change 实施 C3 human-gate-termination-reliability 计划 v1.0

> 按 superpowers:executing-plans / subagent-driven-development 执行。

**Goal:** abort 诚实化（无 run 回执+矩阵收编）+门态呈现分层+degraded 关键帧投递，根治 F-53「点了没反应」。

**Architecture:** 后端（inbound/protocol 矩阵+resync_required+degraded 打点）+前端（门开态隐藏中止钮+指路+死代码清理）。

**Spec:** `openspec/changes/human-gate-termination-reliability/`（strict valid）；统一方案 §5-C3；V0 结论 f53-v0-frame-order.md；实证 f53-diagnosis.md。

## Global Constraints

- abort 不升级为 abandon；不恢复全局豁免；不动 event_seq 去重；CG-04 abandon 语义不变。
- in-flight 修订期 Abort 窄 escape 不实现（watchdog 兜底，design Open Question 留档）。
- 等待式投递有界（如 2 次退避）后必发 resync_required。
- 测试命令照仓规。

## Review Focus

1. 矩阵收编破坏既有 Abort 消费者 → T1 以 F53Diag 按钮全景表回归
2. resync_required 客户端拉取死循环（重复 resync）→ T3 幂等断言
3. 门开态隐藏中止钮误伤 run 态 → T2 stage 判定负例
4. degraded 打点风暴（高频轮询连接）→ T3 打点滚动上限

---

### Task 1: abort 诚实化（后端）

- [ ] 1.1 失败测试：①门开态 Abort→ProtocolError（abort_no_active_run+阶段+指路文案）②run 态 Abort 正常取消回执 Aborted ③会话状态零变化断言 ④F53Diag 全景表既有消费者回归（campaign 脚本 Abort 仅 run 态发）。
- [ ] 1.2 跑红→实现（无 run 回执+移出豁免收编矩阵）→绿。

### Task 2: 门态呈现分层（前端）

- [ ] 2.1 失败测试：①门开态输入条不渲染中止钮 ②run 态仍渲染 ③错误面指路（lease→接管/门态→门卡）。
- [ ] 2.2 跑红→实现（ChatInputBar 渲染条件+指路文案）→绿；死代码删除（useStageUI actions/rollback sender 全仓 grep 动态消费者核查先行，报告留核查证据）。

### Task 3: degraded 关键帧投递（后端+前端）

- [ ] 3.1 失败测试：①degraded 连接门开关键帧不被静默吞（等待式投递到达 或 resync_required 发出）②客户端收 resync_required→主动拉取一次全量（幂等，不循环）③degraded 转移打点落盘（进入/退出/原因）④打点滚动上限 ⑤打点失败零影响。
- [ ] 3.2 跑红→实现（关键帧白名单+有界等待+resync_required 帧+客户端拉取+append-only 打点）→绿；V0 打点复现方案对照记录。

### Task 4: 门禁收口

- [ ] 4.1 lib/it_core（Abort 矩阵/attachment 面）/it_web/vitest 全量+strict+fmt/clippy；F-53 现场回放（门开态点中止→明确报错指路）。

---

## Self-Review

REQ-HTR-01→T1、02→T2、03→T3；Review Focus 四条全挂测试。后端 T1/T3 与前端 T2 文件面分离（T3 客户端拉取小改与 T2 同域由同 worker 串行）。
