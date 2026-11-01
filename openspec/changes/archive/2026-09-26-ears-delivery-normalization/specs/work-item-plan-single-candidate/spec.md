# work-item-plan-single-candidate Delta

## ADDED Requirements

### Requirement: generate 段编译语法失败的教学重驱（REQ-WSC-09）

SC 候选编译（generate 段）失败时，系统 SHALL 在会话进入终态前提供恰好一次教学重驱：失败类别为可教学修复的 parse 语法类（至少含 missing_section、invalid_ears、invalid_work_item_id）时，以既有 code:line:message 错误回灌 author prompt 重驱一次；重驱后仍失败则按既有失败路径终态。每 candidate 至多一次额外驱动（既有上限不变）；非 parse 语法类失败不触发重驱、按既有路径处理。

#### Scenario: invalid_ears 触发一次教学重驱

- **WHEN** 候选编译产出 invalid_ears 且为该 candidate 首次编译失败
- **THEN** 系统以行号+错误信息回灌 author 重驱一次；重驱交付通过编译则继续既有链路，仍失败则终态（不再二次重驱）

#### Scenario: 重驱上限不被突破

- **WHEN** 同一 candidate 的编译失败类别先后命中两个可教学类别
- **THEN** 教学重驱总共仍至多一次，第二次失败直接按既有失败路径处理

## MODIFIED Requirements

### Requirement: markdown 编译器模型（REQ-WSC-02）

系统 SHALL 以 markdown/EARS 文档作为 work item plan 的唯一可编辑源，并提供确定性编译器将其单向编译为顶层 `PlanCandidateIr { source_revision_hash, compiler_version, items: Vec<PlanCandidateItemIr> }`；每个 item SHALL 为 `PlanCandidateItemIr { target_repository_id, contract, verification_plan: WorkItemDraftVerificationPlan, trusted_commands }`。typed IR 的 source revision hash 与 compiler version 仅位于顶层；**publish 前** hash 或版本不匹配时系统 SHALL 拒绝发布并提示重新编译，hash/version 随不可变 publication provenance 落盘。coding 段只消费已发布的 immutable runtime binding，SHALL NOT 在执行期间解析 markdown 或重新解释 compiler version；write_policy 与 trusted commands 等安全边界 SHALL 保持强类型。对 markdown 的人工或模型修改 SHALL 产生新 revision 并触发重新编译。

SC 交付（author 与人工修订两条路径）进入编译前，系统 SHALL 允许两类确定性归一化：其一为结构标题行（一级文档标题、Work Item 二级标题、三级结构 section 标题）归一化——以固定中文→英文映射表逐字映射回规范英文，仅覆盖固定词表的已知翻译变体；其二为 **EARS 关键字空白归一化**——当 statement 行内 `WHEN` 后或 `THE SYSTEM SHALL` 前缺失半角空格、或关键字邻位为 U+3000/NBSP 时，确定性补齐/归一为单个半角空格。两类归一化均只触碰上述空白与固定词表位置，正文其余内容（含 CJK 标点如全角引号）零触碰；表外未知标题 SHALL NOT 被猜测改写，SHALL 照旧交给编译器以 fail-closed 拒绝；任一归一化发生时系统 SHALL 落一条可判定该次交付被救回的诊断/事件。

#### Scenario: markdown 与 IR 漂移被拒绝

- **WHEN** 发布前 typed IR 的 source_revision_hash 与当前 markdown 源不匹配，或 compiler_version 过期
- **THEN** 系统拒绝发布，提示重新编译；已发布的 binding 不受影响

#### Scenario: 编译错误可诊断

- **WHEN** markdown 源不满足 grammar（缺 section、ID 格式错误、EARS 句式非法）
- **THEN** 编译器返回行号、字段名与一个修复示例；该错误信息可直接作为返修反馈回喂模型

#### Scenario: 固定词表中文结构标题确定性归一化后可编译

- **WHEN** provider 交付的 markdown 把固定词表结构标题翻成已知中文变体（如 `# 工作项计划`、`## 工作项 WI-001: x`、`### 身份`、`### 写入策略 (Write Policy)`），且正文其余部分满足语法
- **THEN** 系统在编译前把结构标题行逐字归一化为规范英文并编译通过，正文中文逐字保留，并记录一条归一化诊断/事件；表外未知标题（如 `### 溯源清单`）不被猜测改写，仍按既有语法契约 fail-closed 拒绝

#### Scenario: EARS 关键字缺空格确定性补齐后可编译

- **WHEN** provider 交付的 statement 条件以 CJK 标点（如全角引号 `」`）收尾后无半角空格直接接 `THE SYSTEM SHALL`，或 `WHEN` 后缺半角空格，或关键字邻位为 U+3000/NBSP
- **THEN** 系统在编译前确定性补齐/归一为单个半角空格后编译通过；条件文本与标点本身逐字保留，并记录一条归一化诊断/事件；除关键字邻位空白外任何正文内容不被改写

#### Scenario: 归一化不救非空白语法错误

- **WHEN** statement 的语法错误不是关键字空白形态（如缺 WHEN 前缀、THEN 语义缺失、顺序错误）
- **THEN** 归一化层不修改该行，编译器照旧以 fail-closed 拒绝并返回行号诊断
