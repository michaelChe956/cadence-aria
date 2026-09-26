# node-detail-hydration-resilience Specification

## Purpose

为 timeline 节点 detail 的前端一致性（WS 快照与 REST 水合并存不互相清空、水合集合完整、未水合有占位）与 provider usage 的落盘可靠性（pi 兜底读取加固、失败可观测）建立契约：token 等依赖 node detail 的信息一旦水合完成 SHALL 稳定可见，刷新与快照帧到达不导致静默丢失。

## Requirements

### Requirement: 快照不清空已水合 detail（REQ-NDR-01）

前端应用 WS session_state 快照时，SHALL NOT 将已通过 REST 水合的 node detail 重建为空壳：快照内联 detail 覆盖对应节点，未内联的已水合节点 detail SHALL 原样保留（merge 语义）；或等效实现（快照应用后重置水合去重标记，允许既有节点重新拉取）。已水合完成的节点在任意快照帧到达后，其 usage/token 等依赖 execution_events 的展示 SHALL 保持可见，不出现静默消失或不自愈的退化。

#### Scenario: 水合后快照到达不清空

- **WHEN** 某节点 REST detail 水合完成（含 usage 事件）后，一帧 session_state 快照到达且该节点 detail 未内联
- **THEN** 该节点已水合 detail 原样保留（或被重新拉取），token 展示不消失

#### Scenario: 快照内联 detail 仍优先生效

- **WHEN** 快照内联了某节点 detail
- **THEN** 该节点以快照内联数据呈现，与 REST 数据一致性按既有投影规则处理

### Requirement: 水合集合覆盖全部终态节点（REQ-NDR-02）

前端 detail 水合集合 SHALL 覆盖 completed 之外的非终态呈现节点（至少 failed/aborted/interrupted 的 author/reviewer/revision 节点），使失败节点的 usage/token 与执行事件同样可查，不因终态类别被排除在水合之外。

#### Scenario: failed 节点 token 可见

- **WHEN** 会话含 status=failed 的 author 节点且页面刷新
- **THEN** 该节点 detail 被水合，token 行（若 usage 事件在 durable）可见

### Requirement: 未水合 token 位占位（REQ-NDR-03）

节点 summary 已呈现而 detail 尚未水合完成时，该节点 token 位 SHALL 显示 pending 占位（而非不渲染），水合完成或确认无 usage 后转为终态展示。

#### Scenario: 水合期间 token 位不缺席

- **WHEN** 节点在列表中呈现但 detail 尚未到达
- **THEN** token 位显示 pending 占位；水合后转为实际数值或确认缺失态

### Requirement: pi usage 兜底读取加固（REQ-NDR-04）

pi provider 的本地会话文件 usage 兜底读取 SHALL 复用与 kimi 相同的反向块扫描策略与短重试语义（F-45 同构），SHALL NOT 因文件超过固定尾部字节窗口而静默丢失 usage；读取失败仍为 best-effort None，但 SHALL 可观测（见 REQ-NDR-05）。

#### Scenario: 大会话文件 usage 仍可读

- **WHEN** pi 会话 usage 记录位于文件尾部固定窗口之外（大行/长会话）
- **THEN** 兜底读取经反向扫描取得该轮 usage 并正常上报

### Requirement: usage 失败可观测（REQ-NDR-05）

provider usage 的 emit 失败与 durable 落盘失败 SHALL NOT 静默：以结构化诊断事件或可见结构化 warn 记录失败环节（RPC 获取/解析/文件读取/落盘），使「节点无 usage」可定责到环节；诊断写失败 SHALL NOT 影响 provider 会话与 gate 行为。

#### Scenario: 落盘失败留痕

- **WHEN** usage 事件落盘（node detail 写入）失败
- **THEN** 存在可检索的结构化失败痕迹（诊断事件或 warn 日志），事后调查可区分「无 usage 数据」与「落盘失败」
