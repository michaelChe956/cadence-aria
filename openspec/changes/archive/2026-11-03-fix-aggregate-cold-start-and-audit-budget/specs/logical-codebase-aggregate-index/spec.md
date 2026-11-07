## MODIFIED Requirements

### Requirement: 构建状态可见与失败可恢复

系统 SHALL 将聚合 index 的缺失、构建中、失败、降级和 active 状态与所属 LC authority、成员/checkout 快照及操作身份关联展示。首次构建失败时 SHALL 保留可诊断失败事实；存在 last-known-good 时继续只读投影并标记 degraded，无 last-known-good 时标记 missing/failed；任何自动继续或用户重试均不得伪造 active。重试成功后才可基于验证过的成员覆盖、代表性查询与排除边界将记录转换为 active。

**工具缺失（codegraph 不可用）的处置 SHALL 与上句同源**：无 last-known-good 的冷启动在工具缺失时 SHALL 视为 missing/failed（致命），且该失败 SHALL 在冷启动的**前置探测阶段**就被阻断；存在 last-known-good 的 rebuild / sync 在工具缺失时 SHALL 继续只读投影并标记 degraded（`codegraph_missing`），该降级语义 MUST NOT 被改成致命。

#### Scenario: 首建失败不伪造 active

- **WHEN** 首次 index 构建因成员材料缺失、范围错误或成员 HEAD 漂移失败
- **THEN** 系统 SHALL 返回失败原因和修复/重试操作，active index 保持不存在；不得以仅有数据库文件或 capability 记录宣称成功

#### Scenario: 保留可读的 last-known-good

- **WHEN** 已有 active index 的后续刷新失败
- **THEN** 系统 SHALL 保留 last-known-good 供只读规划，状态标记为 degraded 并向规划上下文注入可审计告警；不得把 degraded 当作新的 active 或静默丢弃失败事实

#### Scenario: 冷启动工具缺失时致命且在前置探测阶段阻断

- **WHEN** 无 last-known-good 的冷启动环境中 codegraph CLI 不可用
- **THEN** 索引 SHALL 处于 missing/failed（致命）而非 degraded；该失败 SHALL 由冷启动的前置探测阶段产出，MUST NOT 等到索引构建阶段才暴露

#### Scenario: 有 last-known-good 的刷新工具缺失仍为降级

- **WHEN** 已有 active index（存在 last-known-good）时发生 rebuild / sync，且 codegraph CLI 不可用
- **THEN** 系统 SHALL 保留 last-known-good 供只读投影、状态标记为 degraded 并注入可审计告警；MUST NOT 把该情形改成致命拒绝

## ADDED Requirements

### Requirement: 聚合索引工具缺失的致命与降级分界（REQ-IND-05）

聚合索引对「工具缺失」这一失败原因 SHALL 有明确的分界：**无 last-known-good 的冷启动**为**致命**（阻断并失败），**有 last-known-good 的 rebuild / sync** 为**降级**（保留只读投影）。致命路径的失败 SHALL 携带稳定判别码 `codegraph_missing`（版本不在支持区间时为 `codegraph_version_mismatch`），且 MUST NOT 以「降级」名义继续冷启动。

同一根因 SHALL 只在**一个**环节暴露：冷启动的前置探测失败时，索引构建 MUST NOT 被触发；成员索引步与聚合索引步 SHALL 投影为「未开始」（无可执行动作、无失败详情），使同一根因不产生多条红卡。冷启动的前置探测与索引构建 SHALL 使用同一份 CLI 配置，MUST NOT 出现「探测判定可用、构建判定不可用」的两套结论。

#### Scenario: 无 last-known-good 时工具缺失判致命

- **WHEN** 冷启动（无 last-known-good）时 codegraph CLI 缺失
- **THEN** 聚合索引 SHALL 判定为 missing/failed（致命）；失败记录 SHALL 携带 `codegraph_missing` 判别码；MUST NOT 降级为 degraded 后继续冷启动

#### Scenario: 有 last-known-good 时工具缺失判降级

- **WHEN** 存在 last-known-good 的聚合索引执行 rebuild / sync，而 codegraph CLI 缺失
- **THEN** 系统 SHALL 保留 last-known-good、标记 degraded 并注入可审计告警；MUST NOT 判为致命失败

#### Scenario: 同一根因不产生多条失败

- **WHEN** 冷启动的前置探测已因工具缺失失败
- **THEN** 索引构建 MUST NOT 被触发；成员索引步与聚合索引步 SHALL 投影为「未开始」且失败详情为空；MUST NOT 出现「一条探测失败 + 若干下游步骤失败」的多条红卡

#### Scenario: 探测与构建同源

- **WHEN** 冷启动的 codegraph 前置探测通过
- **THEN** 索引构建 SHALL 使用与探测相同的 CLI 配置执行；MUST NOT 出现探测与构建对同一 CLI 给出不同结论的情况

#### Scenario: 探测结果落 durable 证据

- **WHEN** 冷启动的前置探测完成
- **THEN** 探测结论（codegraph 是否可用及其版本）SHALL 落 durable 预检快照并可查询；MUST NOT 只存在于日志
