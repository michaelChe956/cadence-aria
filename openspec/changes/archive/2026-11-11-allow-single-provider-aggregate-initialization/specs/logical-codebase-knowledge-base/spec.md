# Spec Delta

## ADDED Requirements

### Requirement: 聚合初始化的 provider 依赖只覆盖被选中的那一家
聚合初始化 SHALL 允许仅安装一家 provider 即完成：provider 选择器 MUST 按 owner 顺序选中第一家已核验且可用的 provider（Pi > Claude Code > Codex），系统 MUST NOT 要求四家 provider 全部安装。Cadence-skills 四条命令的链接层校验 MUST 只覆盖**被选中 provider 对应的那一层**（Pi → shared、Claude Code → claude、Codex → codex）；未被选中、且其命令行未安装的 provider 的层缺失 MUST NOT 阻断本次聚合初始化。被选中 provider 那一层缺命令时，系统 MUST 以 `skills_commands_missing` 失败。

#### Scenario: 只装 Pi 也能通过链接层校验
- **WHEN** 机器上只安装了 Pi，shared 层具备四条 Cadence-skills 命令，而 codex 层与 claude 层缺失
- **THEN** 聚合初始化的链接层校验通过，不因 codex / claude 层缺失而失败

#### Scenario: 选中 provider 的层缺失仍被拒绝
- **WHEN** 被选中的 provider 对应的那一层缺少某条 Cadence-skills 命令
- **THEN** 聚合初始化以 `skills_commands_missing` 失败，且不把任何 step 标为失败

### Requirement: 知识库操作的无人值守执行
知识库 operation SHALL 作为无人值守任务执行：provider turn 的 prompt MUST 附带无人值守指令（不要发起提问或授权请求、不要等待回答），遇到需要用户决策的分支按 skill 规定的非破坏性默认路径继续，无法安全继续时停止并在产物中记录。provider 发起提问或请求授权时，系统 MUST 以 `knowledge_base_provider_interaction_required` 失败，MUST NOT 归因为通用的「provider 执行失败」，且 MUST 把 provider 问了什么写入失败诊断。

#### Scenario: provider 发问有独立稳定码与可执行文案
- **WHEN** 知识库 operation 的 provider turn 发起提问或请求授权
- **THEN** 失败记录的稳定码为 `knowledge_base_provider_interaction_required`，诊断里含 provider 的提问原文，界面文案说明「无人值守执行无法回答」并给出补输入或换 provider 的下一步

#### Scenario: prompt 带无人值守指令
- **WHEN** 知识库 operation 向 provider 发起 turn
- **THEN** prompt 由命令与无人值守指令组成，指令明确要求不要提问、不要请求授权、不要等待回答
