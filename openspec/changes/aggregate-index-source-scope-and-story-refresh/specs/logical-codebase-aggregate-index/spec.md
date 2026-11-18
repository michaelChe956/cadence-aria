## MODIFIED Requirements

### Requirement: 聚合索引建立（REQ-IND-01）

系统 SHALL 在非 git 聚合根建立一份统一 CodeGraph 索引（exact version 钉定 v1.5.0；本变更不调整既有版本准入）。CodeGraph 按目录递归扫描而非按 git 边界识别成员（spike 2 实测非 git 目录 `not-a-repo` 同被纳入），主路径不依赖 `includeIgnored`（D3 非 git 父目录）。索引验收 SHALL 包含成员覆盖交叉验收及非成员/`.worktrees`/`.aria`/构建产物负查询不命中，而非仅 DB 文件存在。代表性查询 SHALL 验证覆盖范围的实际可用性；覆盖多个源码仓时验证代表性跨子仓查询。不以硬编码的代表符号或至少两个被覆盖成员作为运行时激活前置。

**成员覆盖判定 SHALL 走逐成员目录级查询**：平台 SHALL 经 CodeGraph 库 API（`CodeGraph.open(root)` + 逐成员 `hasFilesUnder(member)`）判定每个成员是否被覆盖，MUST NOT 使用 `codegraph files --json` 全量清单作为常规路径（载荷随规模线性增长；用户现场 3.1 GB 索引对应约 230 MB 清单）。目录边界 MUST 按组件而非裸字符串前缀匹配：`member-al` MUST NOT 命中 `member-alpha`。排除性查询 SHALL 保持既有 CLI 机制与判定口径。

仅在 SDK 未安装或与受支持版本不兼容时 MAY 回退既有 CLI 文件清单路径，且 SHALL 留审计痕迹。识别到的 `codegraph_files_empty_index` 通知或合法空清单 SHALL 在聚合验收层视为所有成员未覆盖；工具执行失败、SDK 协议错误/缺成员、非法 JSON 或截断 MUST NOT 被视为空索引。

**磁盘交叉验收（成员未收录时才做）**：对每个**未被索引收录**的成员，平台 SHALL 在该成员的**主工作目录**内做一次**有界盘检**，用 Aria 自有的源码类型清单判定该成员是否真的没有源码。盘检 SHALL 只按后缀做元数据判定，MUST NOT 读文件内容、推断语言或符号、按仓名/标签/角色判断，也不 MUST 跟随符号链接（符号链接与非普通文件都不算源码）。Aria 源码类型清单 SHALL 由 Aria 自有且 MUST 是 CodeGraph 支持后缀的**真子集**；清单的具体后缀集合 SHALL 在实施时用真实 CodeGraph 逐后缀验证后确定，MUST NOT 依赖上游内部表或内部 API 取得该清单。`.yaml/.yml/.xml/.properties/.tf/.tfvars/.tofu` 一类配置格式与 `.json/.md/.toml/.txt/.csv/.html/.htm` 一类文档格式 SHALL NOT 计入 Aria 源码；是否产出符号 MUST NOT 作为判据。

盘检 SHALL 复用聚合索引既有的内建排除口径，MUST NOT 另写一份排除表；普通遍历 SHALL 剪枝 `.git/.aria/node_modules/target/build/dist/.worktrees`，且 MUST NOT 模拟 `.gitignore`（普通路径有源码但被忽略仍是真漏收）。`.worktrees/` SHALL 作为独立的第二段诊断：仅当普通路径查完且无源码时才检查 `.worktrees/` 子树。

盘检 SHALL 是有界的：根深度 0、上限深度 32；每成员最多访问 20,000 个目录条目，单次验收整批最多 100,000 个条目、整批 2 秒（含 `.worktrees/` 诊断，共享同一计数与同一 deadline）；命中普通路径源码即停止扩散。上限 SHALL 固化为常量并可注入时钟/取消信号以便测试，MUST NOT 依赖真实 sleep。MUST NOT 展开全量路径、读文件内容或为盘检启动 git/CodeGraph 进程。

**盘检结论 SHALL 是四值**：普通路径有源码、源码仅在 `.worktrees/`、确认无源码、**无法确认**（权限不足、IO 失败、超时、超限等证据不足）。**无法确认 MUST NOT 等同于无源码。** 盘检未完成且没有正证据时，结论 SHALL 为「无法确认」。

普通路径有源码 SHALL 报 `aggregate_index_member_coverage_failed`（点名漏收成员，说明磁盘上有源码但索引未收录，并指引检查成员路径与忽略规则后重建；MUST NOT 责怪用户没有放源码）；只在 `.worktrees/` 有源码 SHALL 报 `aggregate_index_member_source_excluded`；盘检无法确认 SHALL 报 `aggregate_index_source_scan_incomplete`（点名无法确认的成员，MUST NOT 宣称该成员没有源码）。多类同时出现时 SHALL 依此顺序选择主码并保留已观察的阻断事实。仅正常路径与 `.worktrees/` 范围均确认无源码的未覆盖成员 MAY 跳过。

**跳过与激活**：有至少一个成员被覆盖、其余未覆盖成员全部确认无源码、且其它验收（排除负查询、代表性检索）成功时，SHALL 激活并在该世代记录被跳过成员；被跳过的成员 SHALL 保留在成员快照里，SHALL NOT 被移出项目成员集合，SHALL 仍可作为 Issue 目标仓。各成员均确认无源码且无任何被覆盖成员时 SHALL 报 `aggregate_index_all_members_source_less`，MUST NOT 发布新 active，MUST NOT 把空索引报成成功。

索引健康（索引状态不完整、仍有待处理变更、需要重建、文件计数为零）SHALL 以告警留存，MUST NOT 单独阻断激活或代替覆盖证据。

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

#### Scenario: 混合 LC 可激活并记下跳过事实

- **WHEN** 一个成员可覆盖，另一个未覆盖且经有界盘检在普通路径与 `.worktrees/` 均确认无源码，其余验收通过
- **THEN** SHALL 发布 active 并在该世代记下后者的跳过事实；MUST NOT 要求第二个被覆盖的成员、把跳过成员移出项目成员集合或把它当覆盖

#### Scenario: 真实漏收不能被跳过

- **WHEN** SDK 或 CLI 均可给出未覆盖事实，且 Aria 在该成员普通路径看到源码（包含 `.gitignore` 忽略路径中的源码）
- **THEN** SHALL 报 `aggregate_index_member_coverage_failed` 并点名漏收成员、说明磁盘有源码但索引未收录并指引检查路径与忽略规则；MUST NOT 按配置仓继续，MUST NOT 说用户没有放源码

#### Scenario: 只在工作树有源码

- **WHEN** 成员未覆盖，普通路径无源码，但只在 `.worktrees/` 下发现源码
- **THEN** SHALL 报 `aggregate_index_member_source_excluded`，MUST NOT 视为无源码跳过

#### Scenario: 盘检无法确认不等于没有源码

- **WHEN** 未覆盖成员盘检因 IO/权限、深度、条目数或时间不能完成且没有普通路径源码的正证据
- **THEN** SHALL 以 `aggregate_index_source_scan_incomplete` 记录已知原因并点名该成员，MUST NOT 写跳过事实，MUST NOT 宣称该成员没有源码

#### Scenario: 空索引仍需交叉判定

- **WHEN** SDK 确认全部未覆盖，或回退 CLI 返回空 notice / 空清单
- **THEN** SHALL 逐成员交叉判定；全部确认无源码才用 `aggregate_index_all_members_source_less`，有普通路径源码用覆盖失败码；两者均不得生成 active

#### Scenario: 盘检不扩大 CodeGraph 收录范围

- **WHEN** 平台为判定成员是否有源码做盘检
- **THEN** MUST NOT 修改 `codegraph.json` 的 `exclude`、生成 `extensions` 配置、改变 CodeGraph 支持版本或索引范围；MUST NOT 为配置/文档类文件要求被索引

### Requirement: 索引生命周期与故障恢复（REQ-IND-04）

系统 SHALL 提供聚合索引 single-writer 锁、last-known-good 保留、重建失败恢复与 stale/degraded/superseded 状态转换；规划使用冻结快照，不承诺实时。

`codegraph files --json` 输出被截断（`codegraph_files_output_truncated`）属**构建失败**路径：系统 SHALL 保留 last-known-good 并标记 degraded，MUST NOT 伪造 active，MUST NOT 因此丢弃失败事实（诊断 SHALL 保留专用机器码与上限事实）。

REQ-IND-01 的真实漏收、源码仅在被排除的工作树目录、盘检无法确认、全员均确认无源码均属**构建失败**路径：SHALL 记录对应原因与成员，MUST NOT 发布新 active；无 last-known-good 的冷启动不 planning-ready，刷新失败时 SHALL 保留有用的 last-known-good 作只读降级。混合 LC 中确认无源码的成员跳过事实、SDK 不可用的回退痕迹及健康异常 SHALL 作为可审计告警随成功世代留存，MUST NOT 单独阻断激活，MUST NOT 以 `FailureCopy` 传输或渲染，MUST NOT 触发失败通知。

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

- **WHEN** 成员普通路径有源码却漏收、只在工作树有源码、盘检未完成或全员确认无源码
- **THEN** SHALL 按 REQ-IND-01 的稳定码报告且点名成员；如有 last-known-good 则保留并将其标记 degraded，MUST NOT 伪造 active 或把相应失败事实记成跳过

#### Scenario: 回退痕迹、健康告警与有覆盖的混合 LC 跳过事实不阻断激活

- **WHEN** 通过所有激活验收，且触发了合法 CLI 回退、健康告警或确认跳过无源码成员
- **THEN** SHALL 留存告警事实，MUST NOT 因这些事实阻断激活或静默丢弃

#### Scenario: 跳过事实不冒充失败面

- **WHEN** 某可读世代含被跳过的无源码成员
- **THEN** 该事实 SHALL 以独立告警呈现，MUST NOT 生成对应的 `failure_copy`、MUST NOT 把初始化步骤标为失败、MUST NOT 改变其它代码路径的失败投影

## ADDED Requirements

### Requirement: 跳过事实的世代记录与消费者边界（REQ-IND-06）

成功世代的成员快照 SHALL 保留全部登记成员的成员 ID、checkout、revision、dirty 与采集时间；快照 SHALL 携带一个可选的跳过标记，值为「已确认无源码」时表示该成员经盘检确认没有源码、本次未纳入代码检索范围；缺少该标记的记录（历史世代）SHALL 仍可读取且不产生跳过告警。成员快照的既有 `included` 字段语义 SHALL 保持不变（被跳过的成员仍在快照里，MUST NOT 用改写 `included` 的方式表达跳过）；合法组合只有「无跳过标记＝已覆盖」与「跳过标记＝已确认无源码」，其它组合 MUST NOT 被投影为跳过证据。失败或未验收世代的默认快照 MUST NOT 被投影为跳过。

被跳过的成员 SHALL 仍参与成员集合、身份、主工作目录、Git revision 与 dirty 的漂移判据，SHALL 仍参与规划指纹与引用巡检；跳过标记 MUST NOT 充当 checkout 可用性或授权判据，MUST NOT 让被跳过的成员变成不可用目标仓。索引纳入状态 MUST NOT 成为 Issue 创建、选仓或目标仓准入的门槛。

对曾被跳过的成员后来出现源码（含只出现在 `.worktrees/` 的源码），非降级世代的 freshness 评估 SHALL 发现需要重新验收并按既有同步机制重跑交叉验收；只有新验收成功后才改变该成员的覆盖事实与新世代告警。

只读投影 SHALL 使用同一逻辑代码库的已验收世代事实，MUST NOT 扫磁盘、启动 CodeGraph 或 provider 来补标注。

#### Scenario: 历史世代与混合世代可读

- **WHEN** 读取不含跳过标记的历史成功世代，或读取含被跳过成员的成功世代
- **THEN** 历史记录可正常读取且不产生跳过告警；新世代可据跳过标记生成告警，且不丢失该成员的 Git 快照

#### Scenario: 跳过成员仍在快照且 included 语义不变

- **WHEN** 一个成员经盘检确认无源码而被跳过
- **THEN** 该成员 SHALL 仍出现在该世代的成员快照里，其 `included` 取值 SHALL 与既有语义一致；MUST NOT 因跳过而移除项目成员或改写 `included` 的含义

#### Scenario: 跳过成员参与规划与引用巡检

- **WHEN** 选取已跳过但具有有效主检出的成员作为配置变更目标
- **THEN** 规划 SHALL 保留该成员的成员 ID、checkout、revision 与 dirty 指纹并保持其可用；巡检仍验证成员引用与 checkout 引用，MUST NOT 因跳过隐藏悬挂引用或把跳过本身当悬挂

#### Scenario: 检测已跳过成员新增源码

- **WHEN** 曾跳过的成员后来在普通路径增加源码，即使 revision 与 dirty 布尔值未变，或出现只在 `.worktrees/` 的源码
- **THEN** 下次非降级 freshness 评估 SHALL 发现需要重新验收并按既有同步策略重跑交叉验收；只有新验收成功后才改变该成员的覆盖事实与新世代告警

#### Scenario: 跳过不成为准入门槛

- **WHEN** 用户在被跳过的成员上创建 Issue 或把它选为焦点目标仓
- **THEN** 创建与选仓 SHALL 按既有成员资格、工作区与授权规则判定，MUST NOT 因跳过标记而拒绝、禁用或改变提交资格
