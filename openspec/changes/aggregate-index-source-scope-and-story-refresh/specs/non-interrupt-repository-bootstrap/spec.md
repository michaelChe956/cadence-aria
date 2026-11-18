## MODIFIED Requirements

### Requirement: 布局规则与七个对外稳定码的唯一登记（REQ-BOOT-10）

该来源 MUST 登记 `logical-codebase-layout-guidance` 的六条布局规则与以下七个**既有对外稳定码**：`aggregate_root_is_git` / `aggregate_root_member_outside_root` / `aggregate_root_member_symlink_escape` / `aggregate_root_nested_worktree` / `aggregate_root_overlap` / `aggregate_index_layout_unsupported` / `aggregate_index_member_coverage_failed`；MUST NOT 省略该登记。内部码 SHALL 仅作对应对外码的反查字段，不替代上述契约键。

第六条 SHALL 同源登记三个**新增补充失败码**与一个**新增告警码**：

- `aggregate_index_member_source_excluded`：源码只存在于被排除的工作树目录（`Environment`）；动作为把需索引的源码合入成员主 checkout 的普通路径后重建。
- `aggregate_index_all_members_source_less`：全部成员经确认无源码，没有任何可覆盖成员（`Environment`）；动作为加入含源码的成员仓或把实际源码放到成员主 checkout 的普通路径后重建。
- `aggregate_index_source_scan_incomplete`：因权限、超时、超限或 IO 失败**无法确认**成员是否有源码（`Environment`）；MUST NOT 宣称该成员没有源码，动作为检查主 checkout 可读性与扫描范围后重建。
- `aggregate_index_member_source_less`：**告警码**，表示成员经确认无源码、被跳过代码检索但仍是项目成员；它 MUST NOT 加入失败码集合，MUST NOT 触发失败通知，MUST NOT 以失败五字段传输。

第六条既有码 `aggregate_index_member_coverage_failed` SHALL 支持两个成品变体，按持久化失败事实的机器事实前缀选择：旧记录（`no indexable code file under included members [...]`）SHALL 沿用既有成品，只说明覆盖不足，MUST NOT 编造磁盘有源码的证据；新事实（`source found on disk; no indexed files under included members [...]`）SHALL 点名漏收成员、说明磁盘上有源码但索引未收录，并给出检查成员路径与忽略规则后重建的处置，MUST NOT 责怪用户没有放源码。两个变体 SHALL 登记在本来源的同一张码表内，MUST NOT 另建文案表；新事实 MUST NOT 渲染成旧成品，旧记录 MUST NOT 渲染成新成品。

同一来源 SHALL 登记该条的布局范围成品说明，以及 `aggregate_index_member_source_less` 告警成品在**聚合索引卡片一行**、**创建 Issue 选成员逐仓标注**、**跨仓证据未检索说明**三处的呈现口径；三处 SHALL 消费同一份成品模板，MUST NOT 各自维护同义文案。

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

### Requirement: FailureCopy 五字段合同（REQ-BOOT-11）

该来源产出的 `FailureCopy` MUST 包含五个必填、非 `Option` 字段：`title_zh: String`（问题）、`reason_zh: String`（原因）、`action_zh: String`（怎么做）、`triage: FailureTriage`（Environment / ProductDefect / RetryOnly）、`machine_line: String`（原始机器码的次要展示位）。码表每条失败 MUST 给出非空的三个中文字段；七个既有布局码及第六条的三个补充失败码 MUST 给出具体规则依据与可执行处置动作。无 `action` 字段 MUST NOT 成为另建来源、漏登记或缺失任一段的例外。DTO 及消费链 MUST 完整透传五字段，不把 `reason_zh` 改为可选。

非阻断告警码 `aggregate_index_member_source_less` SHALL 作为独立结构化告警，至少包含稳定告警码和非空中文说明，可按成员渲染不同文案（逐仓标注）或按成员列表渲染（卡片一行），MUST NOT 以 `FailureCopy` 传输或渲染，MUST NOT 分配 `FailureTriage`，MUST NOT 带失败五字段。无源码告警 MUST NOT 污染既有失败封套、健康/回退诊断及其他证据不可用的失败语义。

#### Scenario: 字段完整且跨链同构

- **WHEN** 两条链产出任一已知布局失败或第六条的补充失败
- **THEN** `title_zh`/`reason_zh`/`action_zh`/`triage`/`machine_line` 全部存在，前三字段非空，Rust DTO、HTTP `details.copy` 与 TS 字段合同同构；缺 `reason_zh` 不得按缺省降级判为通过

#### Scenario: 成功告警不冒充失败

- **WHEN** 聚合索引通过混合成员验收且存在跳过成员
- **THEN** 返回同源的独立告警字段、不生成该告警的 `failure_copy`、不标初始化步骤失败、不改变其他代码路径的失败投影

#### Scenario: 告警文案三处同源

- **WHEN** 同一逻辑代码库的跳过事实分别出现在聚合索引卡片、创建 Issue 选成员标注与跨仓证据未检索说明
- **THEN** 三处 SHALL 渲染由同一份登记模板产出的中文说明；MUST NOT 在任何一处自写同义中文正文
