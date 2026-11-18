## MODIFIED Requirements

### Requirement: 布局规则与七个对外稳定码的唯一登记（REQ-BOOT-10）

该来源 MUST 登记 `logical-codebase-layout-guidance` 的六条布局规则与以下七个**既有对外稳定码**：`aggregate_root_is_git` / `aggregate_root_member_outside_root` / `aggregate_root_member_symlink_escape` / `aggregate_root_nested_worktree` / `aggregate_root_overlap` / `aggregate_index_layout_unsupported` / `aggregate_index_member_coverage_failed`；MUST NOT 省略该登记。内部码 SHALL 仅作对应对外码的反查字段，不替代上述契约键。

加上下文的三个补充失败码，本来源登记的布局失败码集合 MUST **恰为十个失败码**，且 MUST 与 `logical-codebase-layout-guidance` 的 `REQ-LAYOUT-04` / `REQ-LAYOUT-05` 声明的集合严格相等（增删码必须同时改这些登记点）。第六条相关的四个失败码（复用的 `aggregate_index_member_coverage_failed` 加三个补充码）的 `reason_zh` SHALL 都含第六条规则依据「聚合索引至少需要一个已覆盖成员；确认无源码的成员可以跳过」；该措辞 MUST 与 `src/product/logical_codebase/failure_copy_table.inc.rs` 的 `LAYOUT_RULES` 第六条 `rule_zh` 逐字一致，MUST NOT 两处各写一版。

第六条 SHALL 同源登记三个**新增补充失败码**与一个**新增告警码**：

- `aggregate_index_member_source_excluded`：源码只存在于被排除的工作树目录（`Environment`）；动作为把需索引的源码合入成员主 checkout 的普通路径后重建。
- `aggregate_index_all_members_source_less`：全部成员经确认无源码，没有任何可覆盖成员（`Environment`）；动作为加入含源码的成员仓或把实际源码放到成员主 checkout 的普通路径后重建。
- `aggregate_index_source_scan_incomplete`：因权限、超时、超限或 IO 失败**无法确认**成员是否有源码（`Environment`）；MUST NOT 宣称该成员没有源码，动作为检查主 checkout 可读性与扫描范围后重建。
- `aggregate_index_member_source_less`：**告警码**，表示成员经确认无源码、被跳过代码检索但仍是项目成员；它 MUST NOT 加入失败码集合，MUST NOT 触发失败通知，MUST NOT 以失败五字段传输。

第六条既有码 `aggregate_index_member_coverage_failed` SHALL 支持两个成品变体，按持久化失败事实的机器事实前缀选择：旧记录（`no indexable code file under included members [...]`）SHALL 沿用既有成品，只说明覆盖不足，MUST NOT 编造磁盘有源码的证据；新事实（`source found on disk; no indexed files under included members [...]`）SHALL 点名漏收成员、说明磁盘上有源码但索引未收录，并给出检查成员路径与忽略规则后重建的处置，MUST NOT 责怪用户没有放源码。两个变体 SHALL 登记在本来源的同一张码表内，MUST NOT 另建文案表；新事实 MUST NOT 渲染成旧成品，旧记录 MUST NOT 渲染成新成品。**新事实（漏收）只在 git 可见范围内成立**：被 `.gitignore` 忽略的文件对 CodeGraph 与 git 同时不可见，MUST NOT 产生该变体的事实；被忽略的文件按「确认无源码」的可跳过成员处理（见 `REQ-IND-01`）。成品文案 SHALL 沿用本来源既有登记文本，本变更不新增第二份文案、不改写已登记的中文成品。

同一来源 SHALL 登记该条的布局范围成品说明，以及 `aggregate_index_member_source_less` 告警成品在**聚合索引卡片一行**、**创建 Issue 选成员逐仓标注**、**跨仓证据未检索说明**三处的呈现口径；三处 SHALL 消费同一份成品模板，MUST NOT 各自维护同义文案。告警码的成品 SHALL 支持按成员渲染（逐仓标注用单成员文案）与按成员列表渲染（卡片一行点名全部被跳过成员）两种形态，两种形态都由本来源的同一模板产出。

**两出口同一解析入口**：把持久化失败封套（`aggregate_index_failed:<code>: <message>` 或 `aggregate_index_degraded:<code>: <message>`）解析成「内层稳定码 + 原始事实 + 三段成品」的能力 SHALL 只有一个共用入口，且该入口 MUST 位于产品层（`src/product/logical_codebase/failure_copy`），使 HTTP 聚合索引投影与冷启动 Bootstrap 步骤投影都能调用；MUST NOT 只把它留在 HTTP handler 里、再由冷启动另写一份判断。冷启动投影 MUST NOT 用裸消息前缀（`warning.contains("<code>")`）特判已登记码：它拿到的是完整封套，前缀判断会把三个补充失败码与复用的覆盖失败码落成通用 `aggregate_index_failed`。两出口对同一封套 SHALL 输出同一稳定码与同一份三段成品，`machine_line` SHALL 写原始稳定码。

同一稳定码的**同一张表内** SHALL 允许按持久化机器事实前缀选择不同成品变体（覆盖失败码的旧记录/新事实两变体即是此机制）；变体选择 MUST NOT 替换 canonical 契约键——归因与码表查找仍用 canonical 码，`machine_line` 仍写原始稳定码，变体只决定 `title_zh` / `reason_zh` / `action_zh` 的模板文本。

#### Scenario: 无 action 的布局失败同源

- **WHEN** 索引/登记布局事实不带 action
- **THEN** 七个既有对外码与第六条的三个补充失败码均命中已知模板、三段完整，不降级为通用重试

#### Scenario: 单点修改覆盖两出口

- **WHEN** 修改一个布局模板
- **THEN** Bootstrap 投影/通知和登记 HTTP copy 同步消费，不改块3中文字面量；聚合索引 HTTP 复用相同成品

#### Scenario: 同码区分历史记录和交叉验收的事实

- **WHEN** 读取旧版覆盖失败，或读取新验收确认普通路径有源码但索引漏收的失败
- **THEN** 两者保持 `aggregate_index_member_coverage_failed` 与具体点名；新文案说明漏收并给检查路径与忽略规则的处置，旧文案只说明覆盖不足、MUST NOT 编造磁盘证据

#### Scenario: 新增码在唯一来源登记且不混用

- **WHEN** 第六条出现源码仅在工作树、盘检无法确认、全员无源码三类事实，或出现确认无源码的可跳过成员
- **THEN** 前三类 SHALL 命中本来源登记的补充失败码并给出完整三段与可执行处置；可跳过成员 SHALL 命中登记的告警码并以独立轻量成品呈现，MUST NOT 以失败三段渲染或触发失败通知

#### Scenario: 冷启动出口也给出补充码的三段

- **WHEN** 冷启动 Bootstrap 步骤读取到的失败封套内层码是第六条的任一补充失败码
- **THEN** 该步骤失败面 SHALL 呈现该稳定码与同源三段成品（经两出口共用的解析入口）；MUST NOT 落成通用 `aggregate_index_failed`，MUST NOT 依赖裸消息前缀判断

#### Scenario: 成品变体不替换契约键

- **WHEN** 覆盖失败码按机器事实前缀选中新事实变体或旧记录变体
- **THEN** 对外稳定码 SHALL 仍是 `aggregate_index_member_coverage_failed`，`machine_line` SHALL 写原始稳定码；变体只影响三段模板文本，MUST NOT 产生新契约键或改变归因分支

### Requirement: FailureCopy 五字段合同（REQ-BOOT-11）

该来源产出的 `FailureCopy` MUST 包含五个必填、非 `Option` 字段：`title_zh: String`（问题）、`reason_zh: String`（原因）、`action_zh: String`（怎么做）、`triage: FailureTriage`（Environment / ProductDefect / RetryOnly）、`machine_line: String`（原始机器码的次要展示位）。码表每条失败 MUST 给出非空的三个中文字段；七个既有布局码及第六条的三个补充失败码 MUST 给出具体规则依据与可执行处置动作。无 `action` 字段 MUST NOT 成为另建来源、漏登记或缺失任一段的例外。DTO 及消费链 MUST 完整透传五字段，不把 `reason_zh` 改为可选。

非阻断告警码 `aggregate_index_member_source_less` SHALL 作为独立**结构化告警对象** `{code, message_zh}` 传输与渲染：`code` 为稳定告警码，`message_zh` 为非空中文说明；可按成员渲染（逐仓标注）或按成员列表渲染（卡片一行）。该对象形状 SHALL 与证据面「未检索」的 `not_searched: {code, message_zh}` 同构，MUST NOT 使用裸中文字符串、只给成员名或另造一套字段名。它 MUST NOT 以 `FailureCopy` 传输或渲染，MUST NOT 分配 `FailureTriage`，MUST NOT 带失败五字段。无跳过事实时该字段 SHALL 省略（不返回空串或空对象）。无源码告警 MUST NOT 污染既有失败封套、健康/回退诊断及其他证据不可用的失败语义。

#### Scenario: 字段完整且跨链同构

- **WHEN** 两条链产出任一已知布局失败或第六条的补充失败
- **THEN** `title_zh`/`reason_zh`/`action_zh`/`triage`/`machine_line` 全部存在，前三字段非空，Rust DTO、HTTP `details.copy` 与 TS 字段合同同构；缺 `reason_zh` 不得按缺省降级判为通过

#### Scenario: 成功告警不冒充失败

- **WHEN** 聚合索引通过混合成员验收且存在跳过成员
- **THEN** 返回同源的独立告警字段、不生成该告警的 `failure_copy`、不标初始化步骤失败、不改变其他代码路径的失败投影

#### Scenario: 告警文案三处同源

- **WHEN** 同一逻辑代码库的跳过事实分别出现在聚合索引卡片、创建 Issue 选成员标注与跨仓证据未检索说明
- **THEN** 三处 SHALL 渲染由同一份登记模板产出的中文说明；MUST NOT 在任何一处自写同义中文正文

#### Scenario: 告警与未检索都是结构化对象

- **WHEN** 客户端读取聚合索引卡片、成员列表或跨仓证据的未检索结果
- **THEN** 跳过提示 SHALL 是 `{code, message_zh}` 对象，未检索说明 SHALL 是 `not_searched: {code, message_zh}`；两处 MUST NOT 返回裸中文字符串，也 MUST NOT 用不同字段名承载同一含义
