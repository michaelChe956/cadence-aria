# Tasks: human-gate-termination-reliability

**全局边界**：abort 不升级为 abandon；不恢复全局豁免；不动 event_seq 去重；CG-04 abandon 语义不变。

## 1. abort 诚实化（后端）

- [x] 1.1 无 active run 的 Abort→ProtocolError（abort_no_active_run+阶段+指路）；Abort 收编 stage 矩阵（门态拒）；红绿：门开态拒并指路/run 态正常取消/会话零变化断言（REQ-HTR-01 场景 1-2）；F53Diag 按钮全景表消费者回归。

## 2. 门态呈现分层（前端）

- [x] 2.1 门开态隐藏输入条中止钮；错误面指路（lease→接管/门态→门卡）；死代码删除（全仓消费者核查先行）；红绿（REQ-HTR-02）。

## 3. degraded 关键帧投递（后端）

- [x] 3.1 关键帧白名单+degraded 连接等待式投递（有界）或 resync_required；客户端收 resync_required 主动拉取；红绿：门开不被静默吞（REQ-HTR-03 场景 1）。
- [x] 3.2 degraded 转移 append-only 打点；红绿：可观测用例（REQ-HTR-03 场景 2）；V0 打点复现方案对照。

## 4. 门禁收口

- [x] 4.1 lib/it_core（Abort 矩阵/attachment 面）/it_web/vitest 全量+strict；F-53 现场回放（门开态点中止→明确报错指路而非无反应）。
