## Purpose

把 workspace 会话的 run 生命周期与 WebSocket 连接解耦：session-owned run manager 唯一持有 engine/run/事件源，连接成为可替换订阅者；driver/observer 显式角色与会话级 lease 经 Hello 可选字段实现（双字段容忍）；连接关闭不再污染业务终态；重连经 event cursor 重订阅并获 session 级广播；连接级中性归因日志使断连起点一次复现可归因；存量数据零迁移、降级回滚安全。RCA §6 四条断连验收矩阵为本 capability P2 部分的 done 定义，不可裁剪。

## ADDED Requirements

### Requirement: session-owned run 管理器（REQ-WCR-01）

run SHALL 由 session 级 manager 唯一持有：manager 持有单一 `WorkspaceEngine` 实例、`ActiveRun`（run token、cancellation、command 通道、状态、lease epoch）与完成/取消原因；连接只持有 attachment（connection id 与订阅），MUST NOT 拥有 run——run 所有权 MUST NOT 再由 socket-local `current_run` 代表。run 的开始/取消/命令/完成/lease 释放 SHALL 进入 manager 内单一原子临界区，以 token + lease epoch 比较后生效。engine 实例 SHALL 在会话运行期创建一次、MUST NOT 随连接销毁或重建；manager SHALL 是唯一能构建 session 快照与驱动 provider 的位置。内存回收点 SHALL 为「run 终态且无订阅者」。进程重启后系统 SHALL 从 durable 重建 manager 状态，MUST NOT 虚称 in-flight provider run 跨进程连续运行（不可恢复运行与可恢复状态分开处置，按恢复链处理）。

#### Scenario: 连接关闭 run 持续

- **WHEN** driver 连接在 run 运行中关闭
- **THEN** run 在 manager 中持续运行至真实终态，不被取消、不被改写、不因连接关闭丢失事件源

#### Scenario: 重连不产生第二 engine 实例

- **WHEN** run 运行中 driver 断开后重连
- **THEN** 重连 attachment 订阅同一 manager/engine/journal，不创建第二个 engine 视图

#### Scenario: 命令进入单一临界区

- **WHEN** 并发发生「开始新 run」「旧 run 完成」「连接关闭清理」等生命周期事件交错
- **THEN** 全部经 manager 原子临界区按 token + lease epoch 判等裁决，不出现双重启动、双重取消或所有权悬挂

#### Scenario: 进程重启诚实恢复

- **WHEN** 服务进程在 provider run 运行中重启后连接恢复
- **THEN** 系统从 durable 重建会话状态，in-flight run 按恢复链处置（不虚称连续运行），界面呈现可诊断的恢复路径而非伪进行中

#### Scenario: 内存回收点

- **WHEN** 某 session 的 run 进入终态且无任何订阅者
- **THEN** manager 的该 session 运行态可被安全回收，再次访问时从 durable 重建

### Requirement: driver/observer 角色与会话级 lease（REQ-WCR-02）

Hello SHALL 增可选 `role` 字段（`driver` / `observer`），并按**双字段容忍**落地：旧客户端（无 role，含全部既有 driver 脚本）→ 新服务端时缺席字段被容忍并归一；新客户端 → 旧服务端时未知字段被忽略。缺席 role SHALL 在服务端入口处归一为内部显式常量（语义 = legacy driver 等价），命令仲裁 SHALL 只认内部角色——legacy 容忍分支 MUST NOT 外溢到仲裁逻辑。observer MUST NOT 能发送改变 run 的消息：服务端 SHALL 拒绝并返回可诊断错误（MUST NOT 只依赖前端隐藏入口）。lease SHALL 为会话级、至多一个活性 driver 授权：由连接持有，连接关闭只撤销 lease / 记录状态，MUST NOT 取消或改写 run；新连接 SHALL 可显式接管 lease（epoch 递增，旧 epoch 的迟到写命令 MUST 被拒）。旧枚举/既有客户端行为在阶段 4 退役门满足前 SHALL NOT 被删除或破坏。

#### Scenario: 缺席 role 归一为 legacy driver

- **WHEN** 既有 driver 脚本（hello 无 role 字段）连接新服务端并发送业务命令
- **THEN** 服务端按内部归一的 legacy driver 角色受理命令，脚本零修改可用

#### Scenario: observer 写被服务端拒绝

- **WHEN** 声明 role=observer 的连接发送改变 run 的命令（如 confirm/abort/推进）
- **THEN** 服务端拒绝该消息并返回可诊断错误，run 状态不变

#### Scenario: driver 关闭只撤 lease

- **WHEN** 持有 lease 的 driver 连接关闭而 run 运行中
- **THEN** 仅 lease 被撤销/记录，run 持续运行，业务终态不被改写

#### Scenario: lease 显式接管与旧 epoch 拒绝

- **WHEN** 新连接显式接管 driver lease（epoch 递增）后，原连接以旧 epoch 发送写命令
- **THEN** 接管成功，旧 epoch 命令被拒且可诊断，不产生双写

#### Scenario: wire 双向兼容零 flag-day

- **WHEN** 新客户端（带 role/after_event_seq 字段）连接旧服务端，或旧客户端连接新服务端
- **THEN** 双方均正常工作（未知字段被忽略 / 缺席字段被归一），不要求任何脚本族同步升级

### Requirement: 连接关闭不写断连终态（REQ-WCR-03）

连接关闭（含服务端 idle close、前端自关、页面卸载、TCP 断开任一形态）MUST NOT 写 `aborted_by_disconnect`、MUST NOT 把活动节点标为 Failed、MUST NOT 触发把 durable session 拉回 prepare_context 的断连整流。只有以下真实原因 SHALL 写对应终态：provider token 实际被取消、provider 失败、明确的人类 abort。显式 abort 与新 run supersede 既有取消路径 SHALL 保持既有语义。run 完成与连接关闭并发时 SHALL 只产生一个真实 terminal reason。

#### Scenario: 断连后业务终态不被覆盖（0437 形态根治）

- **WHEN** reviewer 完成后 human_confirm 门刚开启，此时连接关闭（复现 0437 亚秒窗口）
- **THEN** 活动节点不被标「连接断开，运行已中止」，durable 无 `aborted_by_disconnect`，门等待正常呈现

#### Scenario: 已终态 run 不被断连改写（0429 形态根治）

- **WHEN** run 已以业务终态（如 validation_reject）结束，随后连接读循环结束
- **THEN** 业务终态保持原样，不追加断连审计、不回退 session 状态

#### Scenario: 真实取消仍写终态

- **WHEN** 用户显式 abort 或新 run supersede 取消旧 run
- **THEN** 对应真实取消终态照常写入（本 requirement 只消除连接关闭路径的伪终态，不取消真实取消语义）

#### Scenario: 完成与关闭并发唯一 terminal

- **WHEN** provider run 完成落终态与连接关闭清理并发
- **THEN** durable 只有一个真实 terminal reason，无双重终态写入

### Requirement: 重连 cursor 重订阅与 session 级广播（REQ-WCR-04）

事件流 SHALL 以 session-scoped 单调 `event_seq` 为序（MUST NOT 复用 node id 作为 cursor）。Hello SHALL 支持可选 `after_event_seq` 字段（与 `role` 同批、同双字段容忍纪律）；重连 SHALL 经 cursor 重新订阅：manager 保存有界事件 journal，cursor 过旧或存在缺口时 SHALL 先发送带 cursor 的完整 snapshot 建立基线，再续发后续事件；客户端 SHALL 按 seq 去重。流式内容的回放口径：journal 保留至 run 完成，或 snapshot 携带当前累计文本——二者取一实现，但断线重连后流式呈现 MUST NOT 丢字或重复，MUST NOT「承诺可回放却只回放 timeline」。慢订阅者 SHALL 使用有界通道：溢出后标记需要 snapshot 降级重取，MUST NOT 反压 provider 或阻塞其他订阅者。每连接 SHALL 只管理自己的出站泵：关闭任一连接 MUST NOT 影响 run、engine 或其他 attachment。广播范围 SHALL 为 workspace WS；coding WS 广播为条件扩展项（P2 主体滑窗则 defer 并显式登记 backlog，MUST NOT 以「顺带」名义混入本 requirement 验收）。

#### Scenario: 断线重连回放不丢不重

- **WHEN** 连接中断期间事件继续产生，随后带 cursor 重连
- **THEN** 客户端经 journal 回放或 snapshot 基线恢复完整事件流，无丢失、无重复（按 event_seq 去重）

#### Scenario: cursor 过旧走 snapshot 基线

- **WHEN** 重连提交的 cursor 早于 journal 保留窗口
- **THEN** 服务端先发送带 cursor 的完整 snapshot，再续发后续事件，客户端以 snapshot 建基线

#### Scenario: 慢订阅者不反压

- **WHEN** 某 observer 订阅消费过慢导致通道溢出
- **THEN** 该订阅被标记需 snapshot 降级重取，provider 运行与其他订阅者不受阻塞

#### Scenario: 被动连接收到实时事件

- **WHEN** driver 启动的 run 持续产生事件，另一 cockpit observer 连接订阅同一 session
- **THEN** observer 实时收到事件流（消除「driver 侧有事件、页面零行」的盲区，workspace 面内）

#### Scenario: 关闭一连接不影响其他

- **WHEN** 多连接订阅同一 session，其中任一连接关闭
- **THEN** 其余连接订阅与 run 运行不受影响

### Requirement: 连接级中性归因日志（REQ-WCR-05）

系统 SHALL 提供连接级归因日志：服务端为每个 WS 连接生成 `connection_id`，并记录 receiver 退出类型（close frame 的 code/reason、EOF、error）、是否由 idle 超时触发、session、当前 run/token、drive depth、最后双向 activity 时间；前端 SHALL 记录 `onclose` 的 code/reason/wasClean、`document.visibilityState`、最后 pong/server-message 与最后 ping 时间。归因记录 SHALL 为**中性连接诊断事实**，与业务 timeline terminal reason 分离——MUST NOT 把断连事实重新编码为「运行已中止」。过渡期（本能力落地 → REQ-WCR-03 落地前）断连清理仍写 `aborted_by_disconnect` 时，同一 `connection_id` SHALL 写入其 detail 供关联归因；REQ-WCR-03 落地后该写入随架构消失，中性记录成为唯一连接级事实源。归因能力 SHALL 满足闭环判据：**一次复现即可唯一判定 close 起点**（server-idle / 前端 4000 自关 / 页面卸载 1000 / 代理或 TCP 断开 / 浏览器 discard）。

#### Scenario: 四种人为 close 唯一归因

- **WHEN** 分别人为制造服务端 idle close、浏览器 `ws.close(4000)`、页面卸载 close 1000、TCP drop 各一次
- **THEN** 归因日志对每次 close 给出唯一确定的起点判定（connection_id 关联服务端退出类型与前端 onclose 记录），互相可区分

#### Scenario: 中性载体与业务终态分离

- **WHEN** 检查归因记录的载体与字段
- **THEN** 记录为中性连接诊断（connection_id/close code/reason/退出类型/活性时间），未被编码为业务运行终态

#### Scenario: 过渡期关联归因

- **WHEN** 过渡期内断连清理仍写入 `aborted_by_disconnect`
- **THEN** 该 marker 的 detail 携带同一 connection_id，可与中性记录关联完成归因（不新增语义负担）

#### Scenario: 归因闭环解锁口径纪律

- **WHEN** 归因日志落地并经一次真实复现完成归因定案
- **THEN** 此后方可按证据表述断连根因；此前任何文案 MUST NOT 宣称「断连主嫌已根治」（热修表述限「消除已证实的前端断连源」）

### Requirement: 存量数据与降级回滚兼容（REQ-WCR-06）

本 capability 的 durable 变更 SHALL 只改写入行为、MUST NOT 改 durable schema 结构；存量 `aborted_by_disconnect` 标记 SHALL 保留为历史事实（事件前缀不可变），新代码 MUST NOT 因其存在误判。卡死 session（0166/0167/0168/0199/0277 及 0429）的处置 SHALL 为**复活验证**：修复落地后以新二进制对每个存量 session 验证快照门 confirm 可达（复活续链）或获得明确可诊断拒绝（真实缺口则以可诊断终态化关闭），结果逐个登记；MUST NOT 回写或修复历史 durable，MUST NOT 承诺全部复活。降级回滚 SHALL 安全：回退旧二进制读新 durable 数据 MUST 无 schema 断裂（回退后恢复旧写入行为，即重新引入误标，属行为面代价而非数据损坏）；wire 双字段容忍 SHALL 保证新旧二进制与客户端脚本任意组合不炸。P2 关闸证据 SHALL 含一次回退读验证。

#### Scenario: 存量标记保留为历史

- **WHEN** 新代码读取含历史 `aborted_by_disconnect` 标记的会话
- **THEN** 标记原样保留为历史事实，不触发改写，不因存在该标记误判当前状态

#### Scenario: 存量卡死 session 复活验证

- **WHEN** 修复落地后对 0166/0167/0168/0199/0277/0429 逐个执行复活验证
- **THEN** 每个 session 获得「复活续链」或「可诊断拒绝终态化」之一的结果并登记，无静默悬挂、无历史数据回写

#### Scenario: 回退旧二进制读新数据

- **WHEN** P2 部署后回退到旧二进制并以新 durable 数据读取会话状态
- **THEN** 读取无 schema 断裂、服务正常（回退代价仅为恢复旧写入行为面），关闸证据含本次验证记录

### Requirement: 断连验收矩阵（RCA §6 全量，P2 done 定义）（REQ-WCR-07）

RCA §6 四条断连验收矩阵 SHALL 全量作为 P2 部分的 done 定义，MUST NOT 因预估压力裁剪（排期滑窗时 SHALL 整体 defer 该部分，MUST NOT 以裁剪矩阵换取部分通过）：

1. **后台节流存活**：Chrome 官方 intensive-throttling 测试 flag 下 tab 隐藏运行超过阈值——断言每个 25 秒应用 ping 的实际间隔、服务端是否发 idle close、run 持续且无 `aborted_by_disconnect`。
2. **门等待静默存活**：human-gate revision 期间 provider 静默超过 60 秒——断言前端不自行发送 close 4000、服务端不 idle close、完成后状态为业务结果而非断连覆盖。
3. **四种 close 唯一归因**：人为 server idle、浏览器 `ws.close(4000)`、页面卸载 1000、TCP drop 各一次——断言归因日志唯一归因，且 durable terminal reason 各自正确。
4. **多连接零影响**：driver + cockpit observer 多连接——关闭 observer 对 driver run 零影响；driver close 只影响订阅/lease，run 持续。

#### Scenario: 矩阵① 后台节流下 run 存活

- **WHEN** 以 intensive-throttling flag 隐藏 tab 运行超过节流阈值
- **THEN** ping 实际间隔被记录、run 持续且 durable 无 `aborted_by_disconnect`；服务端不误关——active-run idle guard 在场时 idle close 设计上不可发生，若因守卫失效仍发生关闭，该次关闭须可归因（REQ-WCR-05）且不污染终态（REQ-WCR-03），并作为守卫缺陷跟进、不作为本条通过依据

#### Scenario: 矩阵② 门等待静默不断连

- **WHEN** human-gate revision 中 provider 静默超过 60 秒
- **THEN** 前端不自关 4000、服务端不 idle close、终态为业务结果而非断连覆盖

#### Scenario: 矩阵③ 四种 close 归因与终态正确

- **WHEN** 人为制造 server idle / 前端 4000 / 卸载 1000 / TCP drop 各一次
- **THEN** 归因日志对四次 close 唯一归因，durable terminal reason 各自正确（无一条被写成断连中止）

#### Scenario: 矩阵④ 多连接零影响

- **WHEN** driver 与 cockpit observer 同时连接，关闭 observer 后 driver run 继续运行
- **THEN** observer 关闭对 run 零影响；随后 driver 连接关闭时只影响订阅/lease，run 持续至真实终态

#### Scenario: 矩阵不可裁剪

- **WHEN** P2 排期出现压力
- **THEN** 处置为整体 defer 该部分，任何一条矩阵 MUST NOT 被裁剪或降级为部分通过
