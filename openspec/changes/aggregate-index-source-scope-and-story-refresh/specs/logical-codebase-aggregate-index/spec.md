## MODIFIED Requirements

### Requirement: 聚合索引建立（REQ-IND-01）

系统 SHALL 在非 git 聚合根建立一份统一 CodeGraph 索引；既有版本准入不变（现行为 `>=1.6.0,<2.0.0`）。CodeGraph 按目录递归扫描而非按 git 边界识别成员（spike 2 实测非 git 目录 `not-a-repo` 同被纳入），主路径不依赖 `includeIgnored`（D3 非 git 父目录）。索引验收 SHALL 包含成员覆盖交叉验收及非成员/`.worktrees`/`.aria`/构建产物负查询不命中，而非仅 DB 文件存在。代表性查询 SHALL 验证覆盖范围的实际可用性；覆盖多个源码仓时验证代表性跨子仓查询。不以硬编码的代表符号或至少两个被覆盖成员作为运行时激活前置。

**成员覆盖判定 SHALL 走逐成员目录级查询**：平台 SHALL 经 CodeGraph 库 API（`CodeGraph.open(root)` + 逐成员 `hasFilesUnder(member)`）判定每个成员是否被覆盖，MUST NOT 使用 `codegraph files --json` 全量清单作为常规路径（载荷随规模线性增长；用户现场 3.1 GB 索引对应约 230 MB 清单）。目录边界 MUST 按组件而非裸字符串前缀匹配：`member-al` MUST NOT 命中 `member-alpha`。排除性查询 SHALL 保持既有 CLI 机制与判定口径。

仅在 SDK 未安装或与受支持版本不兼容时 MAY 回退既有 CLI 文件清单路径，且 SHALL 留审计痕迹。识别到的 `codegraph_files_empty_index` 通知或合法空清单 SHALL 在聚合验收层视为所有成员未覆盖；工具执行失败、SDK 协议错误/缺成员、非法 JSON 或截断 MUST NOT 被视为空索引。

**磁盘交叉验收（成员未收录时才做）**：对每个**未被索引收录**的成员，平台 SHALL 在该成员的**主工作目录**内做一次**有界盘检**，用 Aria 自有的源码类型清单判定该成员是否真的没有源码。盘检 SHALL 只按后缀做元数据判定，MUST NOT 读文件内容、推断语言或符号、按仓名/标签/角色判断，也不 MUST 跟随符号链接（符号链接与非普通文件都不算源码）。**Aria 源码类型清单 SHALL 是代码内的静态常量**（发布时固化），由 Aria 自有且 MUST 是 CodeGraph 支持后缀的**真子集**；清单的具体后缀集合 SHALL 按**当前机器已安装的 codegraph 1.6.3** 实测校准后确定，MUST NOT 依赖上游内部表或内部 API 取得该清单。**校准方法（可复现）**：临时目录里每个候选后缀各放一个该语言的**最小有效样本** → `codegraph init .` → `codegraph files --json` → 出现的后缀 = 1.6.3 会收录的后缀；Aria 清单取其**子集**（只含「我们希望当源码看待」的常见语言）。清单处 SHALL 写明：① 校准基线版本 = **1.6.3**；② 上述**可复现测量命令**；③ **复校准步骤**（换 codegraph 版本后重跑测量、更新清单与基线注释）。**校准 SHALL 是实施期一次性动作**，产出固化成静态清单 + 基线注释；MUST NOT 作为运行期流程或开工门禁。系统 MUST NOT 做机器级 durable 能力记录、运行期测量、版本键缓存或自动重校准。**版本漂移只记警告**：已安装 codegraph 版本 ≠ 清单校准基线（1.6.3）⇒ 系统 SHALL 记一条**可审计 warning**，MUST NOT 改判定、MUST NOT 阻断索引构建；**已知风险与触发条件**：新版若不再收录清单内某个后缀，该类仓会被判成「漏收」而失败，届时 SHALL 按复校准步骤更新清单与基线注释（MUST NOT 自动重校准，MUST NOT 把版本漂移升级为失败）。**可见范围口径（G5）**：同一次校准动作 SHALL 记录「被 `.gitignore` 忽略的文件不被 CodeGraph 收录、git 可见范围内的文件（含未跟踪但未被忽略的）被收录」这一条正反两组，使校准结论与扫描口径一致。`.yaml/.yml/.xml/.properties/.tf/.tfvars/.tofu` 一类配置格式与 `.json/.md/.toml/.txt/.csv/.html/.htm` 一类文档格式 SHALL NOT 计入 Aria 源码（`.yaml/.yml/.xml` 虽被 1.6.3 收录也不算）；是否产出符号 MUST NOT 作为判据。

盘检 SHALL 复用聚合索引既有的内建排除口径，MUST NOT 另写一份排除表；`.worktrees/` 第二段诊断的物理遍历 SHALL 剪枝 `.git/.aria/node_modules/target/build/dist/.worktrees`。**盘检的源码候选 SHALL 只取 git 可见范围**：在该成员**主工作目录**内执行 `git ls-files --cached --others --exclude-standard`（已跟踪的文件 + 未跟踪且未被忽略的文件），与 Aria 源码后缀集求交，再扣除 Aria 自身排除（`build/`、`dist/`、`node_modules/`、`.worktrees/`、`.aria/`、`.env*`、`*credential*`、`*secret*` 等既有内建排除项，与内建排除规则表同源，MUST NOT 另写副本）。**被 `.gitignore` 忽略的文件 SHALL NOT 计入源码**：MUST NOT 判「漏收」失败，该成员按「确认无源码」走跳过分支；「源码存在却未被收录」只在 **git 可见范围内**成立，判定时 MUST 能区分「不在范围（被忽略 / Aria 自身排除）」与「在范围却漏收」。git 侧异常（主工作目录不是 git 仓、git 不可用、命令失败或超时）SHALL 归「无法确认」，MUST NOT 当成无源码。11 条内建规则里的 4 条非目录固定名模式（`**/.env`、`**/.env.*`、`**/*credential*`、`**/*secret*`）**目录级与文件级都要扣除**：命中这些模式的**目录条目** SHALL 被剪枝（例如 `credentials/`、`secret-notes/` 这类目录下的文件不进入候选），命中这些模式的**文件条目**同样 SHALL NOT 被算作源码候选（例如 `credentials.rs`、`secret_manager.ts`）。**理由**：这 4 条写进 `codegraph.json` 的 `exclude` 后按路径 glob 生效、能命中文件，CodeGraph 会把这些文件排除出索引；若盘检因后缀在清单里就把它们算作「盘上有源码」，会造成**永久假漏收**、该仓永远建不出索引（同步多少次都没用）。故系统 MUST NOT 只在目录级过滤而放过文件级命中，MUST NOT 为它另建第二份排除表（**Aria 排除集 = codegraph 排除集，同源**）。`.worktrees/` SHALL 作为独立的第二段诊断：仅当普通路径查完且无源码时才检查 `.worktrees/` 子树；该入口本身是符号链接时 SHALL 视为「该段无候选」而不跟随，入口不存在时同样 SHALL 视为「该段已完成且无候选」，两种情形都 MUST NOT 被记成盘检失败。

盘检 SHALL 是有界的：根深度 0、上限深度 32（深度上限约束物理扫描段，即 `.worktrees/` 第二段诊断）；可见清单的枚举与后缀过滤同样计入预算——每成员最多 20,000 个条目，单次验收整批最多 100,000 个条目、整批 2 秒（含 `.worktrees/` 诊断，共享同一计数与同一 deadline）；命中普通路径源码即停止扩散。上限 SHALL 固化为常量并可注入时钟/取消信号以便测试，MUST NOT 依赖真实 sleep。MUST NOT 展开全量路径、读文件内容或为盘检启动 CodeGraph 进程；盘检 MAY 且 MUST 调用只读的 `git ls-files --cached --others --exclude-standard` 取可见范围，MUST NOT 写 git 状态或改动工作区。

**盘检结论 SHALL 是四值**：普通路径**在 git 可见范围内**有源码、源码仅在 `.worktrees/`、确认无源码（可见范围内无源码）、**无法确认**（权限不足、IO 失败、超时、超限、git 侧失败等证据不足）。**无法确认 MUST NOT 等同于无源码。** 盘检未完成且没有正证据时，结论 SHALL 为「无法确认」。

普通路径**在 git 可见范围内**有源码 SHALL 报 `aggregate_index_member_coverage_failed`（点名漏收成员，说明磁盘上有源码但索引未收录，并指引检查成员路径与忽略规则后重建；MUST NOT 责怪用户没有放源码）；只在 `.worktrees/` 有源码 SHALL 报 `aggregate_index_member_source_excluded`；盘检无法确认 SHALL 报 `aggregate_index_source_scan_incomplete`（点名无法确认的成员，MUST NOT 宣称该成员没有源码）。多类同时出现时 SHALL 依此顺序选择主码并保留已观察的阻断事实。仅正常路径（git 可见范围内）与 `.worktrees/` 范围均确认无源码的未覆盖成员 MAY 跳过。

**跳过与激活**：有至少一个成员被覆盖、其余未覆盖成员全部确认无源码、且其它验收（排除负查询、代表性检索）成功时，SHALL 激活并在该世代记录被跳过成员；被跳过的成员 SHALL 保留在成员快照里，SHALL NOT 被移出项目成员集合，SHALL 仍可作为 Issue 目标仓。各成员均确认无源码且无任何被覆盖成员时 SHALL 报 `aggregate_index_all_members_source_less`，MUST NOT 发布新 active，MUST NOT 把空索引报成成功。成员**未出现在覆盖判定结果里**（缺该成员的覆盖键）MUST NOT 被当作「已确认无源码」：结论只允许来自逐成员覆盖判定与盘检正证据齐全的成员，缺证据的成员 SHALL 按「无法确认」处理。

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

#### Scenario: 真非法 JSON 与空索引不改码

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

#### Scenario: 回退路径仍受专用配额与专用截断码约束

- **WHEN** CLI 文件清单回退路径 stdout 被截断
- **THEN** SHALL 报 `codegraph_files_output_truncated`，MUST NOT 误报非法 JSON 或擅自判为协议不符

#### Scenario: 混合 LC 可激活并记下跳过事实

- **WHEN** 一个成员可覆盖，另一个未覆盖且经有界盘检在普通路径与 `.worktrees/` 均确认无源码，其余验收通过
- **THEN** SHALL 发布 active 并在该世代记下后者的跳过事实；MUST NOT 要求第二个被覆盖的成员、把跳过成员移出项目成员集合或把它当覆盖

#### Scenario: 真实漏收不能被跳过

- **WHEN** SDK 或 CLI 均可给出未覆盖事实，且 Aria 在该成员普通路径**git 可见范围内**看到源码
- **THEN** SHALL 报 `aggregate_index_member_coverage_failed` 并点名漏收成员、说明磁盘有源码但索引未收录并指引检查路径与忽略规则；MUST NOT 按配置仓继续，MUST NOT 说用户没有放源码

#### Scenario: 被忽略的文件不算源码

- **WHEN** 某成员普通路径只有被 `.gitignore` 忽略的源码文件（`git ls-files --cached --others --exclude-standard` 看不到它们），成员因此未被索引收录
- **THEN** 该成员 SHALL 按「确认无源码」处理（可跳过并告警），MUST NOT 报 `aggregate_index_member_coverage_failed`；理由是 CodeGraph 与 git 都看不到这些文件（增量同步也收录不到），判定 MUST NOT 产生「永远建不出索引」的死锁

#### Scenario: 同仓内被忽略源码与可见范围内漏收并存

- **WHEN** 同一成员仓既有被 `.gitignore` 忽略的源码文件，另有**在 git 可见范围内**却未被索引收录的源码文件
- **THEN** SHALL 只对**可见范围内漏收**的那部分报 `aggregate_index_member_coverage_failed` 并点名该成员；被忽略的部分 MUST NOT 被算作漏收、不得单独触发失败，MUST NOT 把两类文件混为一谈（判定 MUST 能区分「不在范围（被忽略 / Aria 自身排除）」与「在范围却漏收」）

#### Scenario: 只在工作树有源码

- **WHEN** 成员未覆盖，普通路径无源码，但只在 `.worktrees/` 下发现源码
- **THEN** SHALL 报 `aggregate_index_member_source_excluded`，MUST NOT 视为无源码跳过

#### Scenario: 盘检无法确认不等于没有源码

- **WHEN** 未覆盖成员盘检因 IO/权限、深度、条目数或时间不能完成且没有普通路径源码的正证据
- **THEN** SHALL 以 `aggregate_index_source_scan_incomplete` 记录已知原因并点名该成员，MUST NOT 写跳过事实，MUST NOT 宣称该成员没有源码

#### Scenario: 空索引仍需交叉判定

- **WHEN** SDK 确认全部未覆盖，或回退 CLI 返回空 notice / 空清单
- **THEN** SHALL 逐成员交叉判定；全部确认无源码才用 `aggregate_index_all_members_source_less`，有普通路径源码用覆盖失败码；两者均不得生成 active

#### Scenario: 文件命中非目录内建模式不算源码

- **WHEN** 未覆盖成员的 git 可见范围里只有命中 `**/*credential*` / `**/*secret*` / `**/.env*` 的**文件**（如 `credentials.rs`、`secret_manager.ts`），没有其它源码
- **THEN** 盘检 SHALL NOT 把它们算作源码候选（CodeGraph 会把这些文件排除出索引），该成员 SHALL 按「确认无源码」处理并走跳过分支；MUST NOT 报 `aggregate_index_member_coverage_failed`，也 MUST NOT 只在目录级过滤而放过文件级命中

#### Scenario: 版本漂移只记告警不阻断

- **WHEN** 已安装 codegraph 版本 ≠ 清单校准基线 1.6.3，其余验收全部通过
- **THEN** 系统 SHALL 在成功世代留一条可审计 warning；判定与激活口径 MUST NOT 改变，MUST NOT 因此阻断索引构建或把版本漂移报成失败；更新清单只能按复校准步骤人工完成，MUST NOT 自动重校准

#### Scenario: 盘检不扩大 CodeGraph 收录范围

- **WHEN** 平台为判定成员是否有源码做盘检
- **THEN** MUST NOT 修改 `codegraph.json` 的 `exclude`、生成 `extensions` 配置、改变 CodeGraph 支持版本或索引范围；MUST NOT 为配置/文档类文件要求被索引

### Requirement: 索引快照与 freshness（REQ-IND-03）

系统 SHALL 在每次 init/sync 前后由 Cadence 采集各成员仓 `git rev-parse HEAD` 与 dirty 状态填充 `AggregateIndexMemberSnapshot`（CodeGraph 不提供 per-member revision API）；按需 sync（AI 用前检查）+ 定时兜底刷新；stale/degraded 状态由 Cadence 对比快照 revision 推断并显式呈现。

**dirty 的口径**：成员 dirty 状态 SHALL 以该成员主工作目录的 `git status --porcelain` 为准（既有快照采集即用 `git status --porcelain=v1`，`src/product/logical_codebase/aggregate_index/snapshot.rs`）；**被 `.gitignore` 忽略的文件不参与 dirty 判定**，因此忽略文件既不会让 dirty 成立、也不会掩盖可见范围内的改动。

**本变更不改变全局漂移判据。** 既有成员漂移判定（成员集合长度、逐成员 revision 与 dirty 布尔；`src/product/logical_codebase/aggregate_index/freshness.rs` 的 `member_evidence_drifted`）SHALL 保持原义：MUST NOT 为 Story 前刷新把它全局改成「任何 dirty 都判 stale」，也 MUST NOT 在这条判据里加入磁盘或源码扫描。Story 生成前的「能确认没变 / 不能确认没变」是**该路径自有**的分支决策，归 `REQ-PLN-18` 拥有；Design、WorkItemPlan 等其它只读规划仍按既有 drift 判据与 last-known-good 语义工作。freshness 评估 MUST NOT 执行源码盘检，MUST NOT 为「跳过成员后来出现源码」做后台轮询。

#### Scenario: 成员主 checkout 被外部更新

- **WHEN** 成员主 checkout 在索引后发生外部提交/切换
- **THEN** 系统 SHALL 标记索引为 stale（对比快照 revision），规划时明确提示「索引可能过时」；按需 sync 或定时兜底触发刷新

#### Scenario: 快照不可变证据

- **WHEN** 索引建立或刷新
- **THEN** 系统 SHALL 记录 AggregateIndexMemberSnapshot（member/checkout/revision/included/indexed_at）作为不可变证据，供规划与审计引用

#### Scenario: 全局漂移判据不被 Story 路径改写

- **WHEN** 某成员工作区存在未提交修改，而本次不是 Story 生成（例如打开 Design 或 WorkItemPlan 视图）
- **THEN** 漂移判定 SHALL 按既有 revision/dirty 口径执行、degraded 仍以 last-known-good 加告警支撑该次规划；MUST NOT 因 Story 路径的「不能确认没变」规则而让这条全局判据新增磁盘扫描或强制停机

### Requirement: 索引生命周期与故障恢复（REQ-IND-04）

系统 SHALL 提供聚合索引 single-writer 锁、last-known-good 保留、重建失败恢复与 stale/degraded/superseded 状态转换；规划使用冻结快照，不承诺实时。

`codegraph files --json` 输出被截断（`codegraph_files_output_truncated`）属**构建失败**路径：系统 SHALL 保留 last-known-good 并标记 degraded，MUST NOT 伪造 active，MUST NOT 因此丢弃失败事实（诊断 SHALL 保留专用机器码与上限事实）。

REQ-IND-01 的真实漏收、源码仅在被排除的工作树目录、盘检无法确认、全员均确认无源码均属**构建失败**路径：SHALL 记录对应原因与成员，MUST NOT 发布新 active；无 last-known-good 的冷启动不 planning-ready，刷新失败时 SHALL 保留有用的 last-known-good 作只读降级。混合 LC 中确认无源码的成员跳过事实、SDK 不可用的回退痕迹及健康异常 SHALL 作为可审计告警随成功世代留存，MUST NOT 单独阻断激活，MUST NOT 以 `FailureCopy` 传输或渲染，MUST NOT 触发失败通知。

**degraded 的分界（与 `REQ-PLN-18` 一致）**：除 Story 生成之外的**只读规划**（Design、WorkItemPlan、工作区上下文读取等）SHALL 继续以 last-known-good 加可审计告警支撑，MUST NOT 因 degraded 停机，也 MUST NOT 因读取触发自动重建。「必须是最新索引」这条更严的要求**只属于** `REQ-PLN-18` 的 Story 生成前刷新：存在可恢复的 last-known-good 但刷新后仍非 Active 时，Story 生成 SHALL 停在 AI 启动之前（失败原因与恢复动作按 `REQ-PLN-18`）。本变更 MUST NOT 把这条停机语义扩散到其它只读规划路径。

**失败封套的解析唯一出口**：持久化失败封套（`aggregate_index_failed:<code>: <message>` / `aggregate_index_degraded:<code>: <message>`）到稳定码与中文成品的解析 SHALL 只有一个共用入口，HTTP 投影与冷启动 Bootstrap 投影 SHALL 都经它取「内层稳定码 + 原始事实」，MUST NOT 各自用裸消息前缀（如 `warning.contains("<code>")`）判断，MUST NOT 把已登记的失败码（含第六条的三个补充失败码）落成通用 `aggregate_index_failed`。同一封套在两出口 SHALL 得到同一稳定码与同一份三段成品。

#### Scenario: 索引重建失败

- **WHEN** 聚合索引重建或刷新失败
- **THEN** SHALL 保留 last-known-good 索引并标记 degraded，规划返回可审计告警，不阻塞只读规划

#### Scenario: 只读规划不因 degraded 停机而 Story 生成停机

- **WHEN** 同一逻辑代码库存在 degraded 的 last-known-good，一次请求打开 Design / WorkItemPlan 视图，另一次请求生成 Story
- **THEN** 前者 SHALL 用 last-known-good 加告警继续；后者 SHALL 按 `REQ-PLN-18` 先尝试恢复并验收，仍非 Active 就停在 AI 启动之前；两处 MUST NOT 混用对方的停机/放行口径

#### Scenario: 冷启动与 HTTP 出口同码同文案

- **WHEN** 一次失败世代的封套内层码是 `aggregate_index_member_source_excluded`、`aggregate_index_source_scan_incomplete` 或 `aggregate_index_all_members_source_less`
- **THEN** 冷启动 Bootstrap 步骤的失败面 SHALL 给出该内层码与同源三段成品；HTTP 投影对同一封套 SHALL 给出同一码与同一文案；两处 MUST NOT 借裸消息前缀判断，MUST NOT 把该码改写成 `aggregate_index_failed`

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

#### Scenario: 被忽略源码不报漏收、可见范围内漏收才报

- **WHEN** 同一逻辑代码库内某成员既有被 `.gitignore` 忽略的源码文件，另有**在 git 可见范围内**却未被索引收录的源码文件
- **THEN** 构建失败事实 SHALL 只来自**可见范围内漏收**的成员，并点名该成员；被忽略的源码 MUST NOT 被记成漏收、不得单独触发失败或降级，MUST NOT 让「被忽略的文件」与「在范围却漏收」混为一谈

#### Scenario: 库 API 覆盖不足按构建失败处理

- **WHEN** 库 API 判定某成员目录下没有被索引的文件
- **THEN** 该成员 SHALL 进入 REQ-IND-01 的磁盘交叉验收：确认无源码则按跳过处理并告警；普通路径有源码/仅工作树有源码/无法确认则按对应稳定码失败；MUST NOT 把「未收录」直接当成功，也 MUST NOT 跳过盘检就判「没有源码」

#### Scenario: 回退痕迹与健康告警不阻断激活

- **WHEN** 通过所有激活验收，且触发了合法 CLI 回退、健康告警，或存在确认跳过的无源码成员
- **THEN** SHALL 留存告警事实，MUST NOT 因这些事实阻断激活或静默丢弃

#### Scenario: 跳过事实不冒充失败面

- **WHEN** 某可读世代含被跳过的无源码成员
- **THEN** 该事实 SHALL 以独立告警呈现，MUST NOT 生成对应的 `failure_copy`、MUST NOT 把初始化步骤标为失败、MUST NOT 改变其它代码路径的失败投影

### Requirement: 聚合索引生产触发

逻辑代码库聚合初始化 SHALL 在缺少 active aggregate index 时按冷启动依赖顺序推进：先完成可用的 identity、manifest/checkout、rules/policy 与成员 index 材料，再触发首次 aggregate index 构建。首建准入 MUST NOT 要求一个尚不存在的 active index，也 MUST NOT 通过 GET/规划只读请求隐式启动 provider 或索引。每个步骤的状态、checkpoint、失败原因和合法准备/继续/重试操作 SHALL 可查询；步骤成功后自动推进到下一步骤，最终将可验证的聚合索引置为 active/PlanningReady。

**degraded carve-out**：degraded（有可恢复的 last-known-good）SHALL NOT 被任何只读请求自动重建，只读规划继续以该代加可审计告警支撑。唯一例外是 `REQ-PLN-18` 的 Story 生成前刷新：它 SHALL 先尝试恢复并验收该版本，仍非 Active 就停在 AI 启动之前，不得以「只读投影不阻塞」为由放行 Story 生成。

#### Scenario: 初始化后自动首建

- **WHEN** 聚合初始化成功完成
- **THEN** 自动触发首次索引构建；首建失败不回滚初始化，索引状态为 missing 附失败原因，可手动重建

#### Scenario: 首建期间漂移

- **WHEN** 首次构建期间成员 HEAD 或 dirty 状态变化
- **THEN** 该索引记录标记 Failed（而非 stale），避免出现无可读 active 索引

#### Scenario: 规划读时 stale 同步

- **WHEN** 规划上下文解析读取到 stale 索引
- **THEN** 执行按需同步后继续；degraded 索引不自动重建，仅向规划上下文注入可审计告警；degraded 记录不会被新鲜度评估误报为 active。该条只覆盖只读规划：Story 生成前的刷新按 `REQ-PLN-18` 另行处理，恢复验收后仍非 Active 时停机

#### Scenario: Story 前刷新不改写只读触发边界

- **WHEN** 某 LC 的索引处于 degraded，其它只读规划路径与 Story 生成先后请求
- **THEN** 只读路径 SHALL 不触发重建、不停机；Story 路径 SHALL 走 `REQ-PLN-18` 的恢复/验收/停机语义；MUST NOT 因 Story 路径的存在让只读路径也被迫重建或停摆

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

## ADDED Requirements

### Requirement: 跳过事实的世代记录与消费者边界（REQ-IND-06）

成功世代的成员快照 SHALL 保留全部登记成员的成员 ID、checkout、revision、dirty 与采集时间；快照 SHALL 携带一个可选的跳过标记，值为「已确认无源码」时表示该成员经盘检确认没有源码、本次未纳入代码检索范围；缺少该标记的记录（历史世代）SHALL 仍可读取且不产生跳过告警。成员快照的既有 `included` 字段语义 SHALL 保持不变（被跳过的成员仍在快照里，MUST NOT 用改写 `included` 的方式表达跳过）；合法组合只有「无跳过标记＝已覆盖」与「跳过标记＝已确认无源码」，其它组合 MUST NOT 被投影为跳过证据。失败或未验收世代的默认快照 MUST NOT 被投影为跳过。

被跳过的成员 SHALL 仍参与成员集合、身份、主工作目录、Git revision 与 dirty 的漂移判据，SHALL 仍参与规划指纹与引用巡检；跳过标记 MUST NOT 充当 checkout 可用性或授权判据，MUST NOT 让被跳过的成员变成不可用目标仓。索引纳入状态 MUST NOT 成为 Issue 创建、选仓或目标仓准入的门槛。

对曾被跳过的成员后来出现源码（含只出现在 `.worktrees/` 的源码），本能力 **MUST NOT 提供自动发现**：重新判断只发生在**下一次验收/重建**时，SHALL NOT 引入后台扫描、SHALL NOT 在 freshness 评估里盘检、SHALL NOT 轮询磁盘。跳过事实只属于它所在的那一代；下一次验收 SHALL 对每个未覆盖成员重新判定结论，MUST NOT 沿用上一代的跳过事实当作本次结论。触发重新验收的既有通路只有：成员集合或 `membership_revision` 变化、成员 revision/dirty 漂移、手动重建、以及 `REQ-PLN-18` 的 Story 前刷新落到「有变化或不能确认没变」分支（触发条件以 `REQ-PLN-18` 为准，**不含**「当前世代存在跳过事实」）。两次验收之间新增的源码要等下一次触发才被发现，这是本能力已知并接受的时间窗。

只读投影 SHALL 使用同一逻辑代码库的已验收世代事实，MUST NOT 扫磁盘、启动 CodeGraph 或 provider 来补标注。

**提示的来源与匹配口径**：卡片与成员行的跳过提示 SHALL 只从**本次实际被投影的那一份记录**派生（与该响应的 `state` 同源；例如响应投影 degraded 世代，就必须用这个被投影的世代取跳过事实，MUST NOT 另外再选一遍记录）。成员与提示的关联 SHALL 按 **(成员 ID, 当前 Main checkout ID)** 匹配：成员 ID 取快照的 `logical_repository_id`，主 checkout ID 取该成员当前可用 `CheckoutKind::Main` 检出记录的 `checkout_id`（`src/product/logical_codebase/aggregate_index/types.rs` 的 `AggregateIndexMemberSnapshot` 与 `RepositoryCheckoutRecord`）。MUST NOT 按显示名、别名或裸成员 ID 单独匹配。主 checkout 已更换、世代不同、记录缺失或对不上 ⇒ 该成员 SHALL 不出提示，MUST NOT 用旧记录或其它逻辑代码库的记录顶替。

**告警的形状**：卡片与成员列表的跳过提示 SHALL 是结构化对象 `{code, message_zh}`——`code` 为已登记的告警码 `aggregate_index_member_source_less`，`message_zh` 为同源成品文本（卡片按成员列表渲染，成员行按单个成员渲染）。MUST NOT 返回裸中文字符串或只返回成员名。无跳过事实时该字段 SHALL 省略，旧响应（无该字段）MUST NOT 产生提示。

#### Scenario: 历史世代与混合世代可读

- **WHEN** 读取不含跳过标记的历史成功世代，或读取含被跳过成员的成功世代
- **THEN** 历史记录可正常读取且不产生跳过告警；新世代可据跳过标记生成告警，且不丢失该成员的 Git 快照

#### Scenario: 跳过成员仍在快照且 included 语义不变

- **WHEN** 一个成员经盘检确认无源码而被跳过
- **THEN** 该成员 SHALL 仍出现在该世代的成员快照里，其 `included` 取值 SHALL 与既有语义一致；MUST NOT 因跳过而移除项目成员或改写 `included` 的含义

#### Scenario: 跳过成员参与规划与引用巡检

- **WHEN** 选取已跳过但具有有效主检出的成员作为配置变更目标
- **THEN** 规划 SHALL 保留该成员的成员 ID、checkout、revision 与 dirty 指纹并保持其可用；巡检仍验证成员引用与 checkout 引用，MUST NOT 因跳过隐藏悬挂引用或把跳过本身当悬挂

#### Scenario: 下次验收重新判断而不是自动发现

- **WHEN** 曾跳过的成员后来在普通路径增加源码，即使 revision 与 dirty 布尔值未变；或出现只在 `.worktrees/` 的源码
- **THEN** 下一次验收/重建（含 Story 前刷新落到同步分支）SHALL 对该成员重新盘检并给出新结论（普通路径源码 ⇒ 覆盖失败码；仅 `.worktrees/` ⇒ 源码被排除码）；只有新验收成功后才改变覆盖事实与告警；本能力 MUST NOT 声称已通过后台扫描或 freshness 盘检即时发现

#### Scenario: 跳过事实不被下一代继承

- **WHEN** 一次新的验收运行，且上一代把某成员记为跳过
- **THEN** 本次结论 SHALL 来自本次逐成员覆盖判定与本次盘检；MUST NOT 直接沿用上一代的跳过事实作为本次结果

#### Scenario: 提示取自被投影的那一代

- **WHEN** 同一 LC 同时存在 degraded 的 last-known-good 与更新的失败世代，响应因此投影 degraded
- **THEN** 跳过提示 SHALL 来自那个被投影的 degraded 世代；MUST NOT 来自失败世代、其它世代或另选一遍记录

#### Scenario: 主 checkout 已更换时不沿用旧提示

- **WHEN** 某成员当前可用的 Main checkout 与快照记录的 `checkout_id` 不同，或当前没有可用 Main checkout
- **THEN** 该成员 SHALL 不出跳过提示；MUST NOT 按显示名、别名或裸成员 ID 把旧世代的提示贴到新 checkout 上

#### Scenario: 提示是结构化告警对象

- **WHEN** 卡片或成员列表返回跳过提示
- **THEN** 该字段 SHALL 是 `{code, message_zh}` 结构化对象（`code` 为 `aggregate_index_member_source_less`，`message_zh` 非空）；MUST NOT 只返回裸中文串；无跳过事实时字段 SHALL 省略

#### Scenario: 跳过不成为准入门槛

- **WHEN** 用户在被跳过的成员上创建 Issue 或把它选为焦点目标仓
- **THEN** 创建与选仓 SHALL 按既有成员资格、工作区与授权规则判定，MUST NOT 因跳过标记而拒绝、禁用或改变提交资格
