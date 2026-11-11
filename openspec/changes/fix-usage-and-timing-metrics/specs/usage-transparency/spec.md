# Spec Delta

## ADDED Requirements

### Requirement: 计量口径与耗时埋点（SHALL）

平台 SHALL 固定 usage 与耗时的计量口径。per-run 累计取 `run.json` 的 `elapsed_ms` 与 usage，usage SHALL 取 CLI `result` 的 running total，MUST NOT 把 per-step 事件逐条相加当作总量。per-node 合计取 `timeline_node_details/*.json` 的 usage 事件，各事件并存不覆盖，由消费方求和。节点级 `duration_ms` SHALL 于 `completed_at` 落盘时算出；run 级 `elapsed_ms` SHALL 于写 `run.json` 时落盘。claude 侧 CLI `result.usage` 的精确范围（invocation 累计或末轮值）尚未由证据判定，MUST NOT 对外声称该口径下的具体数值。

#### Scenario: per-run 累计取 running total

- **WHEN** 平台为一次 provider run 写 `run.json`
- **THEN** `run.json` 落盘 `elapsed_ms` 与该 run 的 usage；该 usage 取 CLI `result` 的 running total，MUST NOT 把该 run 的 per-step usage 事件逐条相加当作总量

#### Scenario: per-node usage 事件并存

- **WHEN** 同一 timeline 节点落盘多条 usage 事件
- **THEN** 各条并存保留、MUST NOT 互相覆盖；per-node 合计由消费方对并存事件求和得出

#### Scenario: 节点级耗时与 completed_at 同批落盘

- **WHEN** 一个 timeline 节点进入终态并落盘 `completed_at`
- **THEN** 同一次落盘 MUST 算出该节点的 `duration_ms`，MUST NOT 恒为 null

#### Scenario: run 级耗时落盘

- **WHEN** 一次 provider run 结束并写 `run.json`
- **THEN** `run.json` MUST 含 `elapsed_ms`

#### Scenario: output 占位值不作总量

- **WHEN** 消费方汇总 assistant message 的 per-step `output_tokens`
- **THEN** MUST NOT 把这些 per-step output 逐条相加当作该 run 的总 output

#### Scenario: input 与 cache 按 message.id 去重求和

- **WHEN** 校验落盘 usage 的 input 与 cache
- **THEN** 落盘值 SHALL 等于转录按 `message.id` 去重求和的 input 与 cache 值

#### Scenario: 未判定的口径不得声称数值

- **WHEN** claude 侧 CLI `result.usage` 的范围（invocation 累计或末轮值）尚未由证据判定
- **THEN** 平台 MUST NOT 对外声称该口径下的具体累计数值

## MODIFIED Requirements

### Requirement: 本地会话文件 usage 采集（SHALL）

系统 SHALL 在 pi/codex/kimi 的 provider turn 终止时，按各 CLI 的本地会话文件布局读取最近的 per-turn usage 记录并经 UsageReport 上报；文件缺失、格式变化或字段缺省时 SHALL 静默不上报（不影响主流程）。kimi SHALL 读取 agents/*/wire.jsonl 的 usage.record（usageScope=="turn"）并覆盖 subagent 用量。

同一 role 的两个不同 provider run 的 usage MUST 分别落盘、并存而不互相覆盖；同 run 的同一次 usage 重放 MUST 幂等，MUST NOT 重复落盘，MUST NOT 把累计事件逐条相加造成重复计费。codex/reviewer 侧采用累计口径前，MUST 先判定 `total_token_usage` 属会话累计还是本 turn 累计；判定未确立前 MUST NOT 用累计值替换 per-turn 上报值。

#### Scenario: 正常采集

- **WHEN** kimi turn 完成且 wire.jsonl 存在 usage.record
- **THEN** UsageReport 发出且 input≈inputOther+inputCacheCreation、cache_read=inputCacheRead

#### Scenario: 容错降级

- **WHEN** 会话文件不存在或无 usage 记录
- **THEN** 不上报、不报错、主流程不受影响

#### Scenario: 同 role 跨 provider run 的 usage 并存

- **WHEN** 同一 role 的两个不同 provider run 各自上报一次 usage
- **THEN** 落盘 MUST 有两条 usage 事件、其标识不同、互不覆盖；任一 run 的 usage MUST NOT 因另一 run 上报而丢失

#### Scenario: 同 run 重放幂等

- **WHEN** 同一次 provider run 的同一条 usage 事件被重复上报
- **THEN** 落盘 MUST 仍只有一条该事件，MUST NOT 重复计费

#### Scenario: 累计口径的跨 turn 前置判定

- **WHEN** codex 的 `turn/completed` 事件同时含扁平 usage 与嵌套 `total_token_usage`
- **THEN** 平台 MUST 先以跨 turn fixture 判定 `total_token_usage` 的语义域并记录结论；判定未确立前 MUST NOT 把该累计值当作该 turn 的 per-turn 上报值

#### Scenario: 判定为会话累计时的分工

- **WHEN** 跨 turn fixture 判定 `total_token_usage` 为会话累计
- **THEN** per-turn 上报 MUST 仍取 per-turn 值；该累计值 MUST 由 run 级口径单独承载，MUST NOT 覆盖 per-turn 值

#### Scenario: 判定为本 turn 累计时的取值

- **WHEN** 跨 turn fixture 判定 `total_token_usage` 为本 turn 累计
- **THEN** 平台 MAY 直接以该值作为该 turn 的 per-turn 值上报

#### Scenario: 差分异常不猜零

- **WHEN** 会话累计值与前次基线相减为负、缺基线或会话已重置
- **THEN** 该 turn 的累计口径 MUST 标记为未知或新起点，MUST NOT 猜零

### Requirement: 消息气泡 token 展示（SHALL）

前端 SHALL 在 assistant/reviewer 消息气泡渲染对应 turn 的 token 用量（input/output/cache_read 紧凑展示）；usage 缺失时 SHALL 不显示该行。

气泡只渲染 per-turn 口径值：会话累计口径的值 MUST NOT 被渲染为该 turn 的用量，累计值另由 run 级口径承载。差分出现负值、缺基线或会话重置时，SHALL 标记为未知或新起点，MUST NOT 渲染为 0。

#### Scenario: 气泡显示

- **WHEN** WS 收到 kind=usage 的 execution_event 且对应消息气泡存在
- **THEN** 气泡显示 token 行（如 "输入 89,035 · 输出 8,896 · 缓存 230,976"）

#### Scenario: 无数据不显示

- **WHEN** 无 usage 事件
- **THEN** 气泡无 token 行，无占位符

#### Scenario: 气泡只显示该 turn 的 per-turn 值

- **WHEN** 同一会话存在多个 turn 的 usage，其中一条属会话累计口径
- **THEN** 每个气泡 MUST 只显示对应 turn 的 per-turn 值，MUST NOT 显示跨 turn 的累计值

#### Scenario: 会话累计值不得渲染为 turn 用量

- **WHEN** 上报的 usage 值属会话累计口径
- **THEN** 气泡 MUST NOT 渲染该值为该 turn 的 token 用量；该值由 run 级口径承载

#### Scenario: 未知或新起点不显示为 0

- **WHEN** 该 turn 的累计口径被标记为未知或新起点
- **THEN** 气泡 MUST NOT 显示 0 或其它猜测值作为该 turn 用量
