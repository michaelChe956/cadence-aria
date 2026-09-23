# legacy-protocol-retirement Specification

## Purpose
旧 workitem 决策协议退役轮契约：退役门按 REQ-WSC-07 判据原文全口径在现行 build 重测并全证据留档后方可解锁删除；删除面按 wire 层/引擎层/前端层分层推进且残留归零；SC 门关门决策 typed 重承载；历史 durable 记录只读保留；coding_run_campaign 未测区顺带核。判据零改（用户 1c 裁决钉死）与「重测仍超标挂起等用户终裁」是本 capability 的两条红线。

## Requirements

### Requirement: 退役门全口径重测与判据零改（REQ-RET-01）

系统 SHALL 在删除旧协议前，按 REQ-WSC-07 判据原文的全部子项在重测执行时的现行 build 上重出完整证据矩阵：codex 与 pi 各 1 案例到达 Confirmed（2/2）、单案例时长 ≤12 分钟、初评 ≤1 次且复评 ≤1 次（总 ≤2）、自动返修 ≤1 次（均从服务端持久计数读取）、14 条 classifier golden（rep2/3/4 的 9 条、rep1 round-1 的 2 条 Advisory、3 条人工标注 class_hint 变体） 全部归入预期 finding 分类、仅明确属 grammar/lowering 的 reviewer finding 通过 compiler diagnostic golden（其余明确仅为 prompt few-shot 素材）、断线重连/恢复测试通过、legacy 路径回归全绿、多仓 preflight 不静默回落。判据 SHALL 零改：MUST NOT 放宽阈值、MUST NOT 改计数口径、MUST NOT 删减子项。重测 SHALL 前置设定单案例超时上限与总预算并留档。证据矩阵逐子项留档证据锚后删除面方可解锁；任一关键子项重测仍超标时本 change SHALL 挂起等用户终裁（问法三选：A=维持门不删 legacy、B=授权登记例外并修订 REQ-WSC-07 门文本放行、C=限定 N 次重测取最佳），三方 MUST NOT 自行放宽或静默行使任一选项。

#### Scenario: 全绿解锁删除

- **WHEN** 证据矩阵显示 REQ-WSC-07 全部子项在现行 build 上全绿并留档
- **THEN** 删除面工作包解锁，退役按本 capability 其余 requirement 推进

#### Scenario: 判据零改

- **WHEN** 重测执行与证据矩阵出具
- **THEN** 各子项阈值、计数口径、golden 清单与 REQ-WSC-07 判据原文逐字一致，无任何放宽、改口径或删减

#### Scenario: 重测仍超标挂起等用户终裁

- **WHEN** pi 任一关键子项（如单案例时长）重测仍超标
- **THEN** 本 change 挂起，删除面工作包冻结（DEF-4 契约显式化工作包可独立先行），问法三选如实呈报用户终裁；终裁前不执行任何删除或门文本修订

#### Scenario: 超时预算前置

- **WHEN** 重测 campaign 启动前
- **THEN** 单案例超时上限与总预算已设定并留档，provider 挂死不拖垮矩阵出具

### Requirement: 删除面分层与残留归零（REQ-RET-02）

旧协议删除 SHALL 按「SC 门关门决策 typed 重承载 → 前端 legacy 消费面迁移 → 后端删除跨端原子收口」的分层推进，每层独立可验收。必删集 SHALL 至少覆盖：WS 入站协议的 legacy 逐段确认族（generation-mode 决策、逐段确认消息、review_decision 双选项语法、SelectRevisionPath 族及其专属 DTO）与 `HumanConfirmDecision` 旧枚举及其消息族；引擎侧 legacy 决策路由分支与逐段确认引擎面；`flow_kind` 单路径收敛（新会话一律单候选流）。SC 门关门决策 SHALL 以门专属 typed 决策承载：approve 保持 `Confirm` 语义，abandon 以显式 typed 入站命令承载，MUST NOT 复用 legacy `human_confirm` 通道或其枚举。删除完成后全仓残留 SHALL 归零：必删集符号在全仓产码与测试不可检索（历史归档文档与 durable 数据除外），收到已删除消息类型系统 SHALL 返回 protocol error 且零副作用，全量门禁（后端+前端）双绿。

Cockpit 为已保留且有明确 durable 状态的 Final Compile recovery action 提供 typed 消费入口，属于现有协议能力的可达性修复，不构成旧逐段确认协议的恢复；旧 generation-mode、逐段确认、review_decision 双选项和旧枚举消息族仍不得重新暴露。收到不匹配阶段的 recovery action 或已删除 legacy 消息时，系统 SHALL 返回 stage-specific protocol error 且零副作用。

#### Scenario: SC 门关门决策 typed 重承载先行

- **WHEN** 后端以显式 typed 入站命令承载 SC 门 abandon、关门决策不再依赖 `HumanConfirmDecision`
- **THEN** SC 门 approve/abandon/反馈行为与重承载前等价（含幂等、预算、恢复语义），legacy 双轨此时仍保留且全量门禁绿

#### Scenario: 前端 legacy 消费面全量迁移

- **WHEN** 前端 legacy 决策发送与动作面（WS 消息 union、stage 动作配置、cockpit/plan-repair 路由、批量确认发送、legacy 页面决策面）切换为 typed
- **THEN** 前端不再发送任何 legacy 决策消息，前端测试面同步收敛，前端测试全绿

#### Scenario: 后端删除跨端原子收口

- **WHEN** 后端删除 legacy 变体族、DTO、决策路由分支与逐段引擎面
- **THEN** 前端残余 legacy 类型同批归零，中间双轨态不对外存在，收到已删除消息类型返回 protocol error 且零副作用

#### Scenario: 残留归零断言

- **WHEN** 删除完成判定执行
- **THEN** 必删集符号在全仓产码与测试的检索结果为零（历史归档文档与 durable 数据除外），全量门禁双绿，legacy 回归测试族随删除退役且其重测证据留档在案

#### Scenario: Cockpit recovery 入口不恢复 legacy

- **WHEN** Cockpit 用户在合法可恢复 Final Compile 状态选择既有 recovery action
- **THEN** 系统经现有 typed recovery 消息完成操作，前端不发送 generation-mode、逐段确认、review_decision 或 `HumanConfirmDecision`

#### Scenario: 退役消息与非法 recovery 仍 fail-closed

- **WHEN** 客户端发送已删除 legacy 决策消息，或在不匹配阶段发送 recovery action
- **THEN** 系统返回 stage-specific protocol error 且不改变会话、门、compile、预算或 provider 状态

### Requirement: 存量与历史记录处置（REQ-RET-03）

历史 legacy session 的 durable 记录与事件前缀 SHALL 只读保留：MUST NOT 被迁移、清洗或篡改。`flow_kind` durable 字段对存量记录 SHALL 保持只读兼容（读侧容忍 legacy 取值、不再以其路由到 legacy 引擎）。部署时点在途的 legacy 会话不承诺决策通道连续性：其后续人机决策消息 SHALL 收到 protocol error，会话以现状终态留存，该限制 SHALL 如实登记。系统 MUST NOT 提供 legacy 复活开关或回退开关。

#### Scenario: 历史记录只读保留

- **WHEN** 退役完成后读取任一历史 legacy session 的 durable 记录与事件流
- **THEN** 内容与退役前一致（事件前缀不可变），系统不提供任何修改或清洗入口

#### Scenario: 在途 legacy 会话决策拒绝

- **WHEN** 部署后在途 legacy 会话的客户端发送 legacy 人机决策消息
- **THEN** 系统返回 protocol error 且零副作用，会话以现状终态留存，该迁移限制已如实登记

### Requirement: coding_run_campaign 未测区顺带核（REQ-RET-04）

退役门重测工作包 SHALL 顺带核对 `cadence/reports/workitem-coding-campaign` 驱动器族的未测区：逐区核对后判定冗余或仍被引用；判定冗余的区 SHALL 标记退役并登记（不静默删除、不虚编用途），仍被引用的区 SHALL 如实登记引用面。

#### Scenario: 冗余区标记退役

- **WHEN** 未测区核对判定某驱动器区已无引用且用途被取代
- **THEN** 该区标记退役并登记核对证据，不静默删除

#### Scenario: 仍被引用如实登记

- **WHEN** 未测区核对判定某区仍被重测或验证轮引用
- **THEN** 该区保留并登记引用面，不标记退役
