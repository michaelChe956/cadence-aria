# 规范增量：多仓评审范围与材料（story / design / work item 三环节）

<!-- 来源：cadence/designs/2026-10-22_技术方案_story-design-workitem评审修正_v1.2.md（commit 24fedc80）。
     本 delta 全部为 ADDED：不改 REQ-PLN-01/06（provider-desandbox-full-open、lc-root-initialization 正在改），
     不改 REQ-PLN-03/05/07（fix-planning-session-anchor-recovery 正在改），不改 REQ-PLN-09/10
     （fix-error-experience-structural-governance 新增）。 -->

## ADDED Requirements

### Requirement: 多仓评审范围与代码库形态门禁（REQ-PLN-11）

系统 SHALL 以 `IssueRecord.logical_codebase_id` 作为代码库形态的唯一判定键：`None` 为单仓库代码库，`Some(_)` 为多仓库逻辑代码库。

多仓库逻辑代码库下，Story / Design / Work Item 三环节评审的评审范围 MUST 等于该 issue 选定的真实仓库集合（1~N 个成员仓）。系统 MUST NOT 引入「主仓 / 主成员（primary）」概念，MUST NOT 回落到 manifest 首成员、`IssueRecord.repo_id` 投影或任何 primary 锚定结果，MUST NOT 因无法解析集合而静默缩小范围。

单仓库代码库（`logical_codebase_id == None`）下，评审窗口、评审目标与 cwd 解析、主成员早返回路径、基线语义、门控、容错与文案 MUST 与改造前逐项一致。

#### Scenario: 多仓评审范围取 issue 选定仓库集合

- **WHEN** 多仓 issue 选定 2 个成员仓并进入 Story / Design / Work Item 评审
- **THEN** 评审 MUST 覆盖且仅覆盖这 2 个仓，评审材料与 reviewer prompt MUST NOT 出现「主仓 / 主成员 / primary」措辞，MUST NOT 产生主成员锚定事件

#### Scenario: 多仓不得回落 manifest 首成员或 repo 投影

- **WHEN** 多仓 issue 的评审链路解析仓库集合失败
- **THEN** 系统 MUST 以可诊断错误阻断该轮评审，MUST NOT 选取 manifest 首成员或 `IssueRecord.repo_id` 投影继续

#### Scenario: 单仓形态行为不变

- **WHEN** 单仓 issue（`logical_codebase_id` 为 `None`）进入三环节评审
- **THEN** 评审窗口记录与 window id、评审目标与 cwd、窗口早返回路径、基线语义、门控与文案 MUST 与改造前一致

### Requirement: 评审材料仓库清单全量注入与超限阻断（REQ-PLN-12）

多仓三环节评审材料 MUST 全量注入该 issue 选定仓库集合的成员清单（每仓至少含 id、alias、路径、role、profile 摘要）。系统 MUST NOT 按 soft/hard 预算裁剪清单，MUST NOT 以 `omitted_member_ids`、`omitted_member_count`、`truncated` 等缺员标记呈现被省略成员。

评审材料绝对上限 MUST 为 500 KiB。渲染结果超过该上限时，系统 MUST 以明确错误 `planning_material_over_budget` 阻断该轮评审，MUST NOT 启动 provider。

#### Scenario: 全量清单无缺员

- **WHEN** 多仓 issue 选定 3 个成员仓并构建评审材料
- **THEN** 渲染材料 MUST 含全部 3 个成员仓，材料 MUST NOT 含任何 omitted 或 truncated 形式的缺员标记

#### Scenario: 超上限显式阻断

- **WHEN** 渲染后的评审材料超过 500 KiB
- **THEN** 系统 MUST 返回 `planning_material_over_budget` 并阻断本轮评审，MUST NOT 启动 provider，MUST NOT 丢弃成员后继续

#### Scenario: 单仓材料与 prompt 不变

- **WHEN** 单仓 issue 构建评审材料
- **THEN** 系统 MUST NOT 注入成员清单块，材料与 reviewer prompt MUST 与改造前字节一致

### Requirement: 评审材料稳定注入面（REQ-PLN-13）

多仓评审的仓库清单与逐仓实况 MUST 位于不随会话历史压缩丢失的稳定注入面（canonical inputs 区），MUST 在长会话多轮评审后仍被完整重放。系统 MUST NOT 让该材料只存在于消息尾部。

材料块 MUST 保持既有段边界与既有聚合 scope 标记可解析，MUST NOT 引入会截断后续材料段的形态。

#### Scenario: 长会话压缩后材料仍在

- **WHEN** 多仓评审会话经历 3 轮以上评审并触发历史压缩
- **THEN** 该轮 reviewer prompt MUST 仍包含完整仓库清单与逐仓实况

#### Scenario: 材料块不破坏既有段解析

- **WHEN** 材料块写入稳定注入面
- **THEN** 既有材料段边界与聚合 scope 标记 MUST 仍可被解析，后续材料段 MUST NOT 被截断

#### Scenario: 单仓不注入成员材料

- **WHEN** 单仓 issue 构建 reviewer prompt
- **THEN** 系统 MUST NOT 注入成员清单或逐仓实况，MUST NOT 使该会话被判定为聚合规划会话

### Requirement: 逐仓实况与逐仓基线（REQ-PLN-14）

多仓评审材料 MUST 为每个成员仓渲染固定实况行：`职责`、`关键目录`、`入口`、`构建/测试命令`、`基线` 各一行。无法识别的项 MUST 写「未识别」并保留该行，MUST NOT 省略行或整仓。

逐仓基线 MUST 取自该仓 checkout 的当前 HEAD 所在分支（`git rev-parse --abbrev-ref HEAD`），MUST NOT 猜测 `main` / `master`。单仓基线解析规则 MUST 与改造前一致，含不可解析时 fail-closed 终止该轮。

#### Scenario: 多仓逐仓实况齐全

- **WHEN** 多仓 issue 含一个 Rust 仓与一个 Vue 仓并构建评审材料
- **THEN** 材料 MUST 出现两段逐仓实况，每段含职责、关键目录、入口、构建/测试命令、基线五行；缺项 MUST 显示「未识别」

#### Scenario: detached HEAD 或空仓只影响本仓

- **WHEN** 某成员仓处于 detached HEAD 或无任何提交
- **THEN** 该仓基线行 MUST 显示「基线未识别」及诊断，其余仓 MUST 继续渲染，该轮评审 MUST 继续

#### Scenario: 单仓基线仍 fail-closed

- **WHEN** 单仓 issue 的基线分支不可解析
- **THEN** 系统 MUST 按改造前语义 fail-closed 终止该轮，MUST NOT 降级为「基线未识别」

### Requirement: 多仓评审 must-fix 必查类别（REQ-PLN-15）

多仓三环节 reviewer 的 must-fix 边界规则 MUST 追加以下四类必查项：需求漏项（需求或范围未被覆盖）、仓库选择不合理（列出但不涉及 / 涉及但未列）、验收标准不可判定、跨仓影响缺失。

单仓 reviewer 的 must-fix 规则文案 MUST 逐字不变。

#### Scenario: 四类必查出现在多仓 reviewer 规则

- **WHEN** 构建多仓 Story / Design / Work Item reviewer prompt
- **THEN** must-fix 边界规则段 MUST 含需求漏项、仓库选择不合理、验收标准不可判定、跨仓影响缺失四类

#### Scenario: 漏项夹具产出 must_fix

- **WHEN** 多仓夹具的需求漏掉某涉及仓的改动，且 reviewer 判定该漏项成立
- **THEN** 该轮结论 MUST 含 must_fix 级别的 finding

#### Scenario: 单仓文案逐字不变

- **WHEN** 构建单仓三环节 reviewer prompt
- **THEN** must-fix 边界规则段 MUST 与改造前逐字一致

### Requirement: 多仓 finding 完整性与旧轮证据保留（REQ-PLN-16）

多仓评审中，任一 finding 缺少非空 `evidence` 或缺少非空 `required_action` 时，系统 MUST NOT 将其收入 findings；该轮 MUST 判定为需要人工（`needs_human`）并携带诊断码 `malformed_findings`。

多仓评审的旧轮历史摘要 MUST 保留每个 finding 的 `message` 与 `evidence`；旧记录缺失该字段时 MUST 以显式占位值呈现，MUST NOT 静默丢弃。

单仓的 findings 解析与旧轮摘要 MUST 与改造前一致。

#### Scenario: 缺字段即不收

- **WHEN** 多仓评审输出含一条 `evidence` 为空的 finding
- **THEN** 该 finding MUST NOT 进入 findings，该轮 MUST 判为 `needs_human` 且诊断码 MUST 为 `malformed_findings`

#### Scenario: 旧轮保留 message 与 evidence

- **WHEN** 多仓评审进入第 2 轮并压缩第 1 轮历史
- **THEN** 第 1 轮摘要 MUST 含该轮每条 finding 的 message 与 evidence

#### Scenario: 单仓同输入行为不变

- **WHEN** 单仓评审输出同样缺少 `evidence` 的 finding
- **THEN** 行为 MUST 与改造前一致（照收该 finding），旧轮摘要格式 MUST 不变

### Requirement: 多仓 Work Item splitter 目录摘要与显式失败（REQ-PLN-17）

多仓 Work Item 拆分的目录摘要 MUST 分层呈现（顶层条目加一级子项，总行数受限），MUST NOT 使用单层截断 30 条。

目录读取失败时 MUST 以显式文本 `repository_structure_unavailable: <原因>` 进入材料与诊断；空仓 MUST 以 `repository_structure_empty: <路径>` 呈现，MUST NOT 以伪内容冒充空仓。

单仓拆分的目录摘要 MUST 与改造前逐字一致。

#### Scenario: 多层仓出现二级条目

- **WHEN** 多仓成员仓含多层目录并参与 Work Item 拆分
- **THEN** 目录摘要 MUST 含顶层条目与其一级子项

#### Scenario: 读取失败显式报错

- **WHEN** splitter 读取某成员仓目录失败
- **THEN** 材料 MUST 出现 `repository_structure_unavailable: <原因>`，MUST NOT 呈现为空仓伪内容

#### Scenario: 单仓摘要逐字不变

- **WHEN** 单仓 issue 执行 Work Item 拆分
- **THEN** 目录摘要 MUST 与改造前逐字一致（含既有 30 条截断与空仓文案）
