## MODIFIED Requirements

### Requirement: 聚合索引建立（REQ-IND-01）

系统 SHALL 在非 git 聚合根建立一份统一 CodeGraph 索引（exact version 钉定 v1.5.0；本变更不调整既有版本准入）。CodeGraph 按目录递归扫描而非按 git 边界识别成员（spike 2 实测非 git 目录 `not-a-repo` 同被纳入），主路径不依赖 `includeIgnored`（D3 非 git 父目录）。索引验收 SHALL 包含成员覆盖交叉验收及非成员/`.worktrees`/`.aria`/构建产物负查询不命中，而非仅 DB 文件存在。代表性查询 SHALL 验证覆盖范围的实际可用性；覆盖多个源码仓时验证代表性跨子仓查询。不以硬编码的代表符号或至少两个被覆盖成员作为运行时激活前置。

**成员覆盖判定 SHALL 走逐成员目录级查询**：平台 SHALL 经 CodeGraph 库 API（`CodeGraph.open(root)` + 逐成员 `hasFilesUnder(member)`）判定每个成员是否被覆盖，MUST NOT 使用 `codegraph files --json` 全量清单作为常规路径（载荷随规模线性增长；用户现场 3.1 GB 索引对应约 230 MB 清单）。目录边界 MUST 按组件而非裸字符串前缀匹配：`member-al` MUST NOT 命中 `member-alpha`。排除性查询 SHALL 保持既有 CLI 机制与判定口径。

仅在 SDK 未安装或与受支持版本不兼容时 MAY 回退既有 CLI 文件清单路径，且 SHALL 留审计痕迹。识别到的 `codegraph_files_empty_index` 通知或合法空清单 SHALL 在聚合验收层视为所有成员未覆盖；工具执行失败、SDK 协议错误/缺成员、非法 JSON 或截断 MUST NOT 被视为空索引。

**磁盘交叉验收**：已覆盖成员不需要盘检；对未覆盖成员 SHALL 使用经现行布局校验的主 checkout 根与 Aria 自有正向源码清单，以不跟随符号链接的有界元数据扫描核实。扩展名清单 SHALL 是受支持 CodeGraph 口径的真子集，不通过上游内部 API/内部表取值，不改索引配置；`.yaml/.yml/.xml/.properties/.tf` 不算 Aria 的盘检源码，不据此否定已确认的上游覆盖。扫描 SHALL 共用内建排除定义，不用 `.gitignore` 把正常位置有源码的未覆盖成员当成空仓；`.worktrees/` SHALL 作为独立的有界诊断范围，其他内建排除位置不纳入源码口径。

普通位置有源码 SHALL 报 `aggregate_index_member_coverage_failed`；只在 `.worktrees/` 有源码 SHALL 报 `aggregate_index_member_source_excluded`；没有正证据且扫描未完成（包括 IO/权限/超限）SHALL 报 `aggregate_index_source_scan_incomplete`。多类同时出现时 SHALL 依此顺序选择主码并保留已观察的阻断事实。仅正常和 worktree 范围均确认无源码的未覆盖成员 MAY 跳过。有至少一个已覆盖成员、所有其余成员均确认跳过且其他验收成功才 SHALL 激活；各成员均确认无源码且无覆盖成员时 SHALL 报 `aggregate_index_all_members_source_less`，MUST NOT 发布新 active。

索引健康（索引状态不完整、仍有待处理变更、需要重建、文件计数为零）SHALL 以告警留存，MUST NOT 单独阻断激活或代替逐成员覆盖判据。

`codegraph files --json` SHALL 使用有文档上限的专用 stdout 抓取配额（64 MiB）；其余命令 stdout 与全部命令 stderr SHALL 保持既有上限。stdout 截断 SHALL 在 JSON 解析前检查，MUST 以 `codegraph_files_output_truncated` 报告并说明截断及上限字节数，MUST NOT 当合法清单解析或伪装为 `codegraph_files_invalid_json`。调用 SHALL 只下发 `files --json` 这一形态，MUST NOT 附加未经实测证明可降低载荷的旗标，也不得靠运行时去旗标重试掩盖失败。

#### Scenario: 非 git 聚合根建索引

- **WHEN** 完成成员登记并在非 git 聚合根初始化索引
- **THEN** 聚合根产生一份 `.codegraph/codegraph.db`；验收覆盖成员及非覆盖成员的交叉判定、代表性检索与负查询，不以数据库存在冒充成功

#### Scenario: 文件清单走专用抓取配额

- **WHEN** 平台执行 `codegraph files --json`
- **THEN** 该调用的 stdout 抓取配额 SHALL 为 64 MiB 的专用上限；MUST NOT 与其余 codegraph 命令共用同一抓取上限

#### Scenario: 截断以专用机器码报告

- **WHEN** `codegraph files --json` 的 stdout 被抓取上限截断（即使保留片段恰可解析）
- **THEN** 平台 SHALL 报 `codegraph_files_output_truncated`、说明截断并给出上限字节数；MUST NOT 报 `codegraph_files_invalid_json` 或使用残缺清单验收

#### Scenario: 未截断的完整载荷照常解析

- **WHEN** `codegraph files --json` 的 stdout 未被截断且为合法清单
- **THEN** 平台 SHALL 正常解析清单并按成员覆盖交叉验收建立索引；MUST NOT 因增加抓取配额而改变解析口径或引入失败判据

#### Scenario: 抓取配额不扩散到其它命令

- **WHEN** 平台执行 files 之外的 CodeGraph 命令（版本探测、query、status、init、sync 等）
- **THEN** 这些调用的 stdout 与所有命令的 stderr SHALL 保持既有上限；既有超时、取消和退出码检查语义 MUST NOT 改变

#### Scenario: 文件清单调用不带额外旗标

- **WHEN** 平台执行 `codegraph files --json`
- **THEN** 只有 `--json` 旗标；MUST NOT 附加未经实测证明可降低载荷的旗标，也不得靠去旗标重试掩盖上游不支持错误

#### Scenario: 真非法 JSON 和已识别空索引不混淆

- **WHEN** stdout 未截断但确实非法，或上游输出已知空索引提示
- **THEN** CLI 层 SHALL 分别沿用 `codegraph_files_invalid_json` 和 `codegraph_files_empty_index`；仅后者进入聚合层全员未覆盖的交叉验收，MUST NOT 归为截断

#### Scenario: 成员覆盖走库 API 的逐成员目录级查询

- **WHEN** 平台执行成员覆盖验收
- **THEN** SHALL 以 `CodeGraph.open(root)` + 逐成员 `hasFilesUnder(member)` 为主路径，MUST NOT 以 `codegraph files --json` 全量清单为常规路径

#### Scenario: 目录边界按目录语义判定

- **WHEN** 成员名为 `member-al`，而索引中有 `member-alpha` 下的文件
- **THEN** SDK 和 CLI 两条路径 SHALL 判 `member-al` 未覆盖；MUST NOT 因裸前缀把另一个成员算进去

#### Scenario: 排除性查询保持既有 CLI 机制

- **WHEN** 平台验证非成员目录 / `.worktrees` / `.aria` / 构建产物未被索引
- **THEN** SHALL 保持既有 CLI 查询机制与判定口径，MUST NOT 随成员覆盖调整而改写或跳过排除验收

#### Scenario: 库 API 不可用时回退并留痕

- **WHEN** 定位不到 CodeGraph 库 API 或库 API 与受支持版本不兼容
- **THEN** MAY 回退既有 CLI 文件清单，SHALL 在成功世代留可审计的回退痕迹，MUST NOT 静默切换

#### Scenario: 回退不是常规路径

- **WHEN** 库 API 可用且受支持
- **THEN** 成员覆盖验收 SHALL 走库 API 路径；MUST NOT 把 CLI 全量清单当作常规验收

#### Scenario: 索引健康异常只告警不阻断

- **WHEN** 索引状态异常、仍有待处理变更、需要重建或文件计数为零
- **THEN** SHALL 留存健康告警，MUST NOT 仅因健康异常阻断激活；若逐成员覆盖也未通过，仍按覆盖规则判定失败

#### Scenario: 回退仍受专用配额与截断码约束

- **WHEN** CLI 文件清单回退路径 stdout 被截断
- **THEN** SHALL 报 `codegraph_files_output_truncated`，MUST NOT 误报非法 JSON 或擅自判为协议不符

#### Scenario: 混合 LC 可激活

- **WHEN** 一个成员可覆盖，另一个未覆盖且经有界扫描确认无源码，且排除检查通过
- **THEN** SHALL 发布 active 并记录后者的跳过事实；MUST NOT 要求第二个被覆盖的成员或把跳过成员当覆盖

#### Scenario: 真实漏收不能被跳过

- **WHEN** SDK 或 CLI 均可给出未覆盖事实，且 Aria 在该成员普通路径看到源码（包含 `.gitignore` 忽略路径中的源码）
- **THEN** SHALL 报既有码 `aggregate_index_member_coverage_failed` 并点名漏收成员；MUST NOT 按配置仓继续

#### Scenario: 只在工作树有源码

- **WHEN** 成员未覆盖，普通路径无源码，但只在 `.worktrees/` 下发现源码
- **THEN** SHALL 报 `aggregate_index_member_source_excluded`，MUST NOT 视为无源码跳过

#### Scenario: 扫描未完成不等于没有源码

- **WHEN** 未覆盖成员盘检因 IO/权限、深度、条目数或时间不能完成且没有正常路径源码的正证据
- **THEN** SHALL 以 `aggregate_index_source_scan_incomplete` 记录已知原因，MUST NOT 写跳过事实

#### Scenario: 空索引仍需交叉判定

- **WHEN** SDK 确认全部未覆盖，或回退 CLI 返回空 notice / 空清单
- **THEN** SHALL 逐成员交叉判定；全部确认无源码才用 `aggregate_index_all_members_source_less`，有正常路径源码用覆盖失败码；两者均不得生成 active

### Requirement: 索引生命周期与故障恢复（REQ-IND-04）

系统 SHALL 提供聚合索引 single-writer 锁、last-known-good 保留、重建失败恢复与 stale/degraded/superseded 状态转换；规划使用冻结快照，不承诺实时。

`codegraph files --json` 输出被截断（`codegraph_files_output_truncated`）属**构建失败**路径：系统 SHALL 保留 last-known-good 并标记 degraded，MUST NOT 伪造 active，MUST NOT 因此丢弃失败事实（诊断 SHALL 保留专用机器码与上限事实）。

REQ-IND-01 的真实漏收、源码仅在被排除的工作树目录、扫描未完成、全员均确认无源码均属**构建失败**路径：SHALL 记录对应原因与成员，MUST NOT 发布新 active；无 last-known-good 的冷启动不 planning-ready，刷新失败时 SHALL 保留有用的 last-known-good 作只读降级。混合 LC 中确认无源码的成员跳过事实、SDK 不可用的回退痕迹及健康异常 SHALL 作为可审计告警随成功世代留存，MUST NOT 单独阻断激活。

#### Scenario: 索引重建失败

- **WHEN** 聚合索引重建或刷新失败
- **THEN** SHALL 保留 last-known-good 索引并标记 degraded，规划返回可审计告警，不阻塞只读规划

#### Scenario: 并发写保护

- **WHEN** 多个 sync/rebuild 请求并发
- **THEN** SHALL 通过 single-writer 锁串行化，避免索引损坏或半写状态

#### Scenario: files 输出截断按构建失败处理

- **WHEN** 构建因 `codegraph files --json` 输出截断失败
- **THEN** SHALL 保留 last-known-good 并将其标为 degraded；MUST NOT 伪造 active、将截断当成功或丢弃失败事实

#### Scenario: 截断失败不阻塞只读规划

- **WHEN** 存在 last-known-good 的聚合索引因文件清单截断而刷新失败
- **THEN** SHALL 继续以只读 last-known-good 支持规划并注入可审计告警，MUST NOT 因截断改判为致命

#### Scenario: 真实覆盖失败仍走失败与降级路径

- **WHEN** 成员正常路径有源码却漏收、只在工作树有源码、盘检未完成或全员确认无源码
- **THEN** SHALL 按 REQ-IND-01 的稳定码报告且点名成员；如有 last-known-good 则保留并将其标记 degraded，MUST NOT 伪造 active 或把相应失败事实记成跳过

#### Scenario: 回退痕迹、健康告警与有覆盖的混合 LC 跳过事实不阻断激活

- **WHEN** 通过所有激活验收，且触发了合法 CLI 回退、健康告警或确认跳过无源码成员
- **THEN** SHALL 留存告警事实，MUST NOT 因这些事实阻断激活或静默丢弃

## ADDED Requirements

### Requirement: 成员跳过事实与消费者边界（REQ-IND-06）

成功世代的命令前后快照 SHALL 保留全部登记成员的成员 ID、checkout、revision、dirty 和采集时间；`included: true` 且没有 `skip_reason` 表示覆盖，`included: false` 且 `skip_reason: no_source` 表示验收确认的无源码跳过事实。缺少 `skip_reason` 的历史记录 SHALL 仍可读取；失败或未验收记录的默认快照、未知/不一致的字段组合 MUST NOT 被投影为跳过。只读投影 SHALL 使用同一 LC 的验收世代事实；MUST NOT 扫磁盘、启动 CodeGraph 或 provider 来补标注。

跳过成员仍参与 Issue 授权范围内的规划指纹、revision/dirty 与 checkout/成员漂移和引用巡检。索引纳入标志 MUST NOT 充当 checkout 可用性或授权判据。对曾跳过成员发现新的候选源码或盘检未得结论 SHALL 使非降级世代在 freshness 评估中成为 stale，并按既有同步机制重新验收；MUST NOT 因为当前 Git 快照不含跳过标志而判漂移。保持无可读 LC 索引的 fail-closed 行为。

在鉴权与既有 ACL 通过后，若钉住的可读世代确认目标 checkout 为跳过成员，跨仓证据请求 SHALL 给出显式未检索结果，MUST NOT 发起 CodeGraph 查询或把未检索冒充查询无命中；配置/文档由 provider 在已授权目标工作区直接操作。其他目标查询 SHALL 继续遵守既有 ACL，且 MUST NOT 从钉住世代的跳过成员返回索引证据；不得扩大查询授权。

#### Scenario: 历史与混合世代可读

- **WHEN** 读取历史成功世代或含无源码成员的成功世代
- **THEN** 无 `skip_reason` 的历史记录可正常读取且无跳过告警；新世代可据 `false + no_source` 生成告警且不丢失该成员的 Git 快照

#### Scenario: 跳过成员参与规划与引用巡检

- **WHEN** 选取已跳过但具有有效 checkout 的成员作为配置变更目标
- **THEN** 规划 SHALL 保留该成员的可用 checkout 指纹；巡检仍验证成员引用及 checkout 引用，MUST NOT 因 `included=false` 隐藏悬挂或把跳过本身当悬挂

#### Scenario: 检测已跳过成员新增源码

- **WHEN** 曾跳过的成员后来在普通路径增加源码，即使 revision 和 dirty 布尔值未变，或出现只在 `.worktrees/` 的源码
- **THEN** 下次非降级 freshness 评估 SHALL 发现 stale 并按既有同步策略重新验收；只有新验收成功后才改变该成员的覆盖事实和新世代告警

#### Scenario: 配置目标使用工作区而非索引证据

- **WHEN** 已授权 attempt 对钉住世代的已跳过目标仓请求跨仓索引证据
- **THEN** SHALL 返回显式未检索且无命中，以 `aggregate_index_member_source_less` 解释；MUST NOT 访问 CodeGraph；目标仍可由 provider 按原工作区规则操作
