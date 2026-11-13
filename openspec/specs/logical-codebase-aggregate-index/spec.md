# logical-codebase-aggregate-index Specification

## Purpose

在非 git 公共父目录建立一份统一 CodeGraph 索引覆盖全部成员仓，管理索引范围、快照、freshness 与生命周期。

## Requirements

### Requirement: 聚合索引建立（REQ-IND-01）
系统 SHALL 在非 git 聚合根建立一份统一 CodeGraph 索引（exact version 钉定 v1.5.0）；CodeGraph 按目录递归扫描而非按 git 边界识别成员（spike 2 实测：非 git 目录 `not-a-repo` 同被纳入），主路径不依赖 `includeIgnored`（D3 非 git 父目录）。索引构建成功以「成员覆盖 + 代表性跨子仓查询命中 + 非成员/`.worktrees`/`.aria`/构建产物负查询不命中」为验收，而非仅 DB 文件存在。

**成员覆盖判定 SHALL 走「逐成员目录级查询」**：平台 SHALL 经 codegraph **库 API**（`CodeGraph.open(root)` + 逐成员 `hasFilesUnder(member)`）判定「该成员目录下是否有可索引的文件」，MUST NOT 把「拉取全量文件清单」（`codegraph files --json`）作为**常规**验收路径 —— 该载荷与仓库规模线性相关（`codegraph.db` : JSON ≈ 13 : 1；用户现场 3.1 GB 索引 ⇒ 清单约 230 MB），任何抓取上限都装不下 ⇒ 验收必失败。

目录边界 MUST 是**目录语义**：成员 `member-al` 的覆盖判定 MUST NOT 命中 `member-alpha`；成员覆盖 MUST NOT 以裸前缀匹配判定。

排除性查询（确认非成员目录 / `.worktrees` / `.aria` / 构建产物未被索引）SHALL 保持既有 CLI 查询机制与判定口径不变。

当库 API 不可用（SDK 未安装 / 与受支持版本不兼容）时，平台 MAY 回退到既有 CLI 文件清单路径并留下**可审计痕迹**；回退 MUST NOT 成为常规路径。

索引健康（索引状态不完整、仍有待处理变更、需要重建、文件计数为零）SHALL 以**告警**形式呈现，MUST NOT 单独成为阻断激活的理由（保持既有「不因健康问题阻断」的语义）。

`codegraph files --json` SHALL 使用有文档上限的**专用抓取配额（64 MiB）**，与其余 codegraph 命令的既有抓取上限相区分；其余命令的 stdout 与全部命令的 stderr SHALL 保持既有上限不变。

stdout 被截断时 SHALL 以专用机器码 `codegraph_files_output_truncated` 报告，MUST NOT 伪装成 `codegraph_files_invalid_json`；截断判定 MUST 先于 JSON 解析（被截断的残缺载荷 MUST NOT 被当合法清单解析），诊断 SHALL 说明截断事实与上限字节数。

文件清单调用 SHALL 只下发 `files --json` 这一个调用形态，MUST NOT 附加任何未经实测证明能降低载荷的额外旗标（实测无收益的旗标只会引入「上游 CLI 移除该旗标 ⇒ 文件清单全线失败」的风险）。

#### Scenario: 非 git 聚合根建索引
- **WHEN** 完成成员登记并在非 git 聚合根初始化索引
- **THEN** 聚合根产生一份 `.codegraph/codegraph.db`，覆盖全部 manifest 成员；索引构建成功以「成员覆盖 + 代表性查询命中 + 非成员/excluded 负查询不命中」为验收，而非仅 DB 文件存在

#### Scenario: 文件清单走专用抓取配额

- **WHEN** 平台执行 `codegraph files --json`
- **THEN** 该调用的 stdout 抓取配额 SHALL 为 64 MiB 的专用上限；MUST NOT 与其余 codegraph 命令共用同一抓取上限

#### Scenario: 截断以专用机器码报告

- **WHEN** `codegraph files --json` 的 stdout 被抓取上限截断（即使保留片段恰好可被解析）
- **THEN** 平台 SHALL 以专用机器码 `codegraph_files_output_truncated` 报告该次失败，诊断 SHALL 说明输出被截断并给出上限字节数；MUST NOT 报成 `codegraph_files_invalid_json`、MUST NOT 把被截断的残缺载荷当合法成员清单解析

#### Scenario: 未截断的完整载荷照常解析

- **WHEN** `codegraph files --json` 的 stdout 未被截断
- **THEN** 平台 SHALL 正常解析出成员清单并按既有验收建立索引；MUST NOT 因使用了更大的抓取配额而改变解析口径或引入新的失败判据

#### Scenario: 抓取配额不扩散到其它命令

- **WHEN** 平台执行 files 之外的 codegraph 命令（版本探测、query、status、init、sync 等）
- **THEN** 那些调用的 stdout 与所有命令的 stderr SHALL 保持既有上限不变；既有超时、取消与退出码检查语义 MUST NOT 改变

#### Scenario: 文件清单调用不带额外旗标

- **WHEN** 平台执行 `codegraph files --json`
- **THEN** 下发的调用 SHALL 只携带 `--json` 一个旗标；MUST NOT 附加未实测证明能降低载荷的额外旗标
- **AND WHEN** 附加旗标会让上游 CLI 报未知/不支持参数
- **THEN** 该风险 SHALL 被视为「不应引入该旗标」的理由；平台 MUST NOT 因此依赖运行时去旗标重试来掩盖真实失败

#### Scenario: 真非法 JSON 与空索引不改码

- **WHEN** stdout 未被截断但内容确实不是合法 JSON，或上游输出的是已识别的空索引提示
- **THEN** 平台 SHALL 分别沿用既有的非法 JSON 码与空索引专用码；MUST NOT 把两者都归为截断

#### Scenario: 成员覆盖走库 API 的逐成员目录级查询

- **WHEN** 平台对聚合根执行成员覆盖验收
- **THEN** 平台 SHALL 经 codegraph 库 API（`CodeGraph.open(root)` + 逐成员 `hasFilesUnder(member)`）判定每个成员目录下是否有可索引文件；MUST NOT 以 `codegraph files --json` 的全量文件清单作为常规验收路径

#### Scenario: 目录边界按目录语义判定

- **WHEN** 成员名为 `member-al`、而索引中另有成员目录 `member-alpha`
- **THEN** `member-al` 的覆盖判定 SHALL 为假；MUST NOT 因裸前缀匹配而把 `member-alpha` 下的文件算作 `member-al` 已覆盖

#### Scenario: 排除性查询保持既有 CLI 机制

- **WHEN** 平台执行排除性验收（确认非成员目录 / `.worktrees` / `.aria` / 构建产物未被索引）
- **THEN** 该验收 SHALL 保持既有 CLI 查询机制与判定口径不变；MUST NOT 因成员覆盖改走库 API 而一并改写排除性验收

#### Scenario: 库 API 不可用时回退并留痕

- **WHEN** 平台定位不到 codegraph 库 API（SDK 未安装），或库 API 与受支持的 codegraph 版本不兼容
- **THEN** 平台 MAY 回退到既有 CLI 文件清单路径完成成员覆盖验收；SHALL 在索引激活记录上留下可审计的回退痕迹；MUST NOT 静默切换路径

#### Scenario: 回退不是常规路径

- **WHEN** 库 API 可用且受支持
- **THEN** 成员覆盖验收 SHALL 走库 API 路径；MUST NOT 把回退到 CLI 全量清单当作常规验收口径

#### Scenario: 索引健康异常只告警不阻断

- **WHEN** 库 API 返回的索引健康字段显示索引状态异常、仍有待处理变更、需要重建或文件计数为零
- **THEN** 平台 SHALL 以告警形式记录该事实并随索引记录留存；MUST NOT 仅因该健康问题阻断索引激活

#### Scenario: 回退路径仍受专用配额与专用截断码约束

- **WHEN** 平台走 CLI 文件清单回退路径、且 `codegraph files --json` 的 stdout 被抓取上限截断
- **THEN** 平台 SHALL 以专用机器码 `codegraph_files_output_truncated` 报告；MUST NOT 误报成 `codegraph_files_invalid_json`；MUST NOT 把截断当成「协议不符」

### Requirement: 索引范围与排除（REQ-IND-02）
系统 SHALL 由 Cadence 在聚合根生成并维护 `codegraph.json` 的 `exclude`（denylist，根相对 gitignore 模式；spike 2 实测：CLI 无 allowlist 参数，`exclude` 是唯一范围控制机制），至少覆盖：非成员目录、`**/.worktrees/`、`**/.aria/`、构建产物目录与凭据目录；CodeGraph 内建排除 `.git`/`build`/`node_modules`/`dist` 但 `.worktrees`/`.aria` 非内建（spike 2 实测：删子仓 `.gitignore` 后 `.worktrees`/`.aria` 被误索引）。成员增删后 SHALL 原子更新该文件并 sync/全量 index；index 后用 `codegraph files` 与负查询验证边界，发现非成员路径即失败而非静默接受。同一源码主 checkout 与 worktree 不重复索引。

#### Scenario: 排除执行 worktree
- **WHEN** 成员仓内存在 `.worktrees/**` 或构建产物
- **THEN** 索引 SHALL 排除这些路径，查询结果不命中未提交的 coding 分支内容

### Requirement: 索引快照与 freshness（REQ-IND-03）
系统 SHALL 在每次 init/sync 前后由 Cadence 采集各成员仓 `git rev-parse HEAD` 与 dirty 状态填充 `AggregateIndexMemberSnapshot`（CodeGraph 不提供 per-member revision API）；按需 sync（AI 用前检查）+ 定时兜底刷新；stale/degraded 状态由 Cadence 对比快照 revision 推断并显式呈现。

#### Scenario: 成员主 checkout 被外部更新
- **WHEN** 成员主 checkout 在索引后发生外部提交/切换
- **THEN** 系统 SHALL 标记索引为 stale（对比快照 revision），规划时明确提示「索引可能过时」；按需 sync 或定时兜底触发刷新

#### Scenario: 快照不可变证据
- **WHEN** 索引建立或刷新
- **THEN** 系统 SHALL 记录 AggregateIndexMemberSnapshot（member/checkout/revision/included/indexed_at）作为不可变证据，供规划与审计引用

### Requirement: 索引生命周期与故障恢复（REQ-IND-04）
系统 SHALL 提供聚合索引 single-writer 锁、last-known-good 保留、重建失败恢复与 stale/degraded/superseded 状态转换；规划使用冻结快照，不承诺实时。

`codegraph files --json` 输出被截断（`codegraph_files_output_truncated`）属**构建失败**路径：系统 SHALL 保留 last-known-good 并标记 degraded，MUST NOT 伪造 active，MUST NOT 因此丢弃失败事实（诊断 SHALL 保留专用机器码与上限事实）。

成员覆盖验收改走库 API 后，库 API 判定「某成员目录下无可索引文件」仍属**构建失败**路径：系统 SHALL 保留 last-known-good 并标记 degraded，MUST NOT 伪造 active，并沿用既有覆盖不足错误码点名未覆盖成员。库 API 不可用触发的回退痕迹与索引健康告警 SHALL 作为**可审计告警**随索引记录留存，MUST NOT 单独阻断激活。

#### Scenario: 索引重建失败
- **WHEN** 聚合索引重建或刷新失败
- **THEN** 系统 SHALL 保留 last-known-good 索引并标记 degraded，规划时返回可审计告警，不阻塞只读规划

#### Scenario: 并发写保护
- **WHEN** 多个 sync/rebuild 请求并发
- **THEN** 系统 SHALL 通过 single-writer 锁串行化，避免索引损坏或半写状态

#### Scenario: files 输出截断按构建失败处理

- **WHEN** 聚合索引构建因 `codegraph files --json` 输出被截断而失败
- **THEN** 系统 SHALL 保留 last-known-good 并将索引标记为 degraded；MUST NOT 伪造 active、MUST NOT 把截断当成功，MUST NOT 因此丢弃失败事实

#### Scenario: 截断失败不阻塞只读规划

- **WHEN** 存在 last-known-good 的聚合索引因文件清单截断而刷新失败
- **THEN** 系统 SHALL 继续以只读 last-known-good 支撑规划并注入可审计告警；MUST NOT 因截断把该失败改判为致命

#### Scenario: 库 API 覆盖不足按构建失败处理

- **WHEN** 聚合索引构建因库 API 判定某成员目录下无可索引文件而失败
- **THEN** 系统 SHALL 保留 last-known-good 并将索引标记为 degraded、沿用既有覆盖不足错误码并点名未覆盖成员；MUST NOT 伪造 active、MUST NOT 把覆盖不足当成功

#### Scenario: 回退痕迹与健康告警不阻断激活

- **WHEN** 库 API 不可用而平台回退 CLI 文件清单路径完成验收，或索引健康告警被记录
- **THEN** 该告警 SHALL 随索引激活记录留存为可审计事实；MUST NOT 因该告警阻断激活，也 MUST NOT 静默丢弃告警

### Requirement: 聚合索引生产触发

逻辑代码库聚合初始化 SHALL 在缺少 active aggregate index 时按冷启动依赖顺序推进：先完成可用的 identity、manifest/checkout、rules/policy 与成员 index 材料，再触发首次 aggregate index 构建。首建准入 MUST NOT 要求一个尚不存在的 active index，也 MUST NOT 通过 GET/规划只读请求隐式启动 provider 或索引。每个步骤的状态、checkpoint、失败原因和合法准备/继续/重试操作 SHALL 可查询；步骤成功后自动推进到下一步骤，最终将可验证的聚合索引置为 active/PlanningReady。

#### Scenario: 初始化后自动首建

- **WHEN** 聚合初始化成功完成
- **THEN** 自动触发首次索引构建；首建失败不回滚初始化，索引状态为 missing 附失败原因，可手动重建

#### Scenario: 首建期间漂移

- **WHEN** 首次构建期间成员 HEAD 或 dirty 状态变化
- **THEN** 该索引记录标记 Failed（而非 stale），避免出现无可读 active 索引

#### Scenario: 规划读时 stale 同步

- **WHEN** 规划上下文解析读取到 stale 索引
- **THEN** 执行按需同步后继续；degraded 索引不自动重建，仅向规划上下文注入可审计告警；degraded 记录不会被新鲜度评估误报为 active

#### Scenario: 手动重建

- **WHEN** 调用 POST /api/projects/{pid}/logical-codebase/aggregate-indexes/rebuild
- **THEN** 同步执行重建并返回最终记录（前端展示 loading）；同 project 已有重建进行中时返回 409 aggregate_index_rebuild_in_progress

#### Scenario: 构建状态可见

- **WHEN** 构建进行中查询 GET /api/projects/{pid}/logical-codebase/aggregate-indexes/active
- **THEN** 返回 state=rebuilding（Building 记录先于索引命令持久化）；状态映射覆盖 active/stale/degraded/rebuilding/missing（Failed 有 last-known-good 时呈现 degraded，否则 missing）

#### Scenario: 缺少 active index 的 LC 首建

- **WHEN** 新逻辑代码库已完成身份与成员材料但没有 active aggregate index
- **THEN** 系统 SHALL 接受首建并按缺失步骤生成成员 index 与 aggregate index，不因 active index 缺失拒绝成员准入，也不要求用户手工 seed

#### Scenario: 首建步骤中断后继续

- **WHEN** 首次 aggregate index 构建或其前置步骤失败
- **THEN** 系统 SHALL 保留已完成 checkpoint、失败状态及原因；用户从产品操作面点击允许的继续/重试后 SHALL 只执行未完成或安全可重做步骤，完成后自动推进，不重复已完成 provider turn

#### Scenario: GET 仅投影 index 状态

- **WHEN** 客户端查询 active aggregate index、成员 index 或 LC 初始化状态
- **THEN** 系统 SHALL 返回 missing/rebuilding/failed/degraded/active 等 durable 投影和允许动作，MUST NOT 因查询触发索引命令、provider、checkout 或状态突变

### Requirement: 构建写入的 single-writer 与快照

全部索引写入路径 MUST 经统一 single-writer 互斥，并 SHALL 保留命令前后成员快照证据。

#### Scenario: 首建与重建共用互斥
- **WHEN** 首次构建、重建或同步并发执行
- **THEN** 全部写入路径经统一 single-writer 互斥；跨命令前后采集成员快照，重建/同步期间漂移将新记录标 stale（不静默丢弃），active 记录引用命令前快照

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

#### Scenario: 截断码不是工具缺失码

- **WHEN** 聚合索引因 `codegraph_files_output_truncated` 失败
- **THEN** 该失败 MUST NOT 被当作工具缺失处理：MUST NOT 进入致命匹配集（该集合只含 `codegraph_missing` 与 `codegraph_version_mismatch`）；SHALL 走「有 last-known-good ⇒ degraded」的一般规则，MUST NOT 因此把冷启动判为致命

#### Scenario: 库 API 不可用不是工具缺失

- **WHEN** 聚合索引因 codegraph 库 API 不可用（SDK 未安装 / 与受支持版本不兼容）而需要回退 CLI 文件清单路径
- **THEN** 该事实 MUST NOT 被当作工具缺失处理：MUST NOT 进入致命匹配集；致命判据仍只认 codegraph CLI 本身不可用（`codegraph_missing` / `codegraph_version_mismatch`）；MUST NOT 因库 API 不可用把冷启动判为致命
