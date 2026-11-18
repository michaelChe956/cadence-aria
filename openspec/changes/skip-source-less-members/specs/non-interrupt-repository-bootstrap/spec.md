## MODIFIED Requirements

### Requirement: 布局规则与七个对外稳定码的唯一登记（REQ-BOOT-10）

该来源 MUST 登记 `logical-codebase-layout-guidance` 的六条布局规则与以下七个**既有对外稳定码**：`aggregate_root_is_git` / `aggregate_root_member_outside_root` / `aggregate_root_member_symlink_escape` / `aggregate_root_nested_worktree` / `aggregate_root_overlap` / `aggregate_index_layout_unsupported` / `aggregate_index_member_coverage_failed`；MUST NOT 省略该登记。内部码 SHALL 仅作对应对外码的反查字段，不替代上述契约键。

第六条的 `aggregate_index_member_coverage_failed` 对**有磁盘证据的新记录**表示普通路径有源码但索引漏收，中文 SHALL 点名未覆盖成员、说明漏收事实并给出检查忽略规则/重建指引（`ProductDefect`）；旧记录不得被推断为已确认有源码。第六条 SHALL 同源登记 `aggregate_index_member_source_excluded`（仅 `.worktrees/` 有源码；将源码合入主 checkout 普通路径）、`aggregate_index_all_members_source_less`（全部成员经确认无源码；补入含源码成员或在正常路径放实际源码）、`aggregate_index_source_scan_incomplete`（证据不足；检查可读性与扫描范围），三者为 `Environment` 并给出重建指引。

同一来源 SHALL 登记该条的布局范围成品说明，以及非阻断码 `aggregate_index_member_source_less` 的卡片、选仓及证据未检索告警成品；告警 MUST NOT 加入失败码或触发失败通知。

#### Scenario: 无 action 的布局失败同源

- **WHEN** 索引/登记布局事实不带 action
- **THEN** 七个既有对外码与第六条的补充失败码均命中已知模板、三段完整，不降级为通用重试

#### Scenario: 单点修改覆盖两出口

- **WHEN** 修改一个布局模板
- **THEN** Bootstrap 投影/通知和登记 HTTP copy 同步消费，不改块3中文字面量；聚合索引 HTTP 复用相同成品

#### Scenario: 同码区分历史记录和交叉验收的事实

- **WHEN** 读取旧版覆盖失败，或读取新验收确认普通路径有源码但索引漏收的失败
- **THEN** 两者保持 `aggregate_index_member_coverage_failed` 与具体点名；新文案说明漏收，旧文案只说明覆盖不足、MUST NOT 编造磁盘证据

### Requirement: FailureCopy 五字段合同（REQ-BOOT-11）

该来源产出的 `FailureCopy` MUST 包含五个必填、非 `Option` 字段：`title_zh: String`（问题）、`reason_zh: String`（原因）、`action_zh: String`（怎么做）、`triage: FailureTriage`（Environment / ProductDefect / RetryOnly）、`machine_line: String`（原始机器码的次要展示位）。码表每条失败 MUST 给出非空的三个中文字段；七个既有布局码及第六条补充失败码 MUST 给出具体规则依据与可执行处置动作。无 `action` 字段 MUST NOT 成为另建来源、漏登记或缺失任一段的例外。DTO 及消费链 MUST 完整透传五字段，不把 `reason_zh` 改为可选。

非阻断 `aggregate_index_member_source_less` SHALL 作为独立结构化告警，至少包含稳定告警码和非空 `message_zh`，MUST NOT 以 `FailureCopy` 传输或渲染，不分配 `FailureTriage`。无源码告警 MUST NOT 污染既有失败封套、健康/回退诊断及其他证据不可用的失败语义。

#### Scenario: 字段完整且跨链同构

- **WHEN** 两条链产出任一已知布局失败或第六条的补充失败
- **THEN** `title_zh`/`reason_zh`/`action_zh`/`triage`/`machine_line` 全部存在，前三字段非空，Rust DTO、HTTP `details.copy` 与 TS 字段合同同构；缺 `reason_zh` 不得按缺省降级判为通过

#### Scenario: 成功告警不冒充失败

- **WHEN** 聚合索引通过混合成员验收且存在跳过成员
- **THEN** 返回同源的独立告警字段、不生成该告警的 `failure_copy`、不标初始化步骤失败、不改变其他代码路径的失败投影
