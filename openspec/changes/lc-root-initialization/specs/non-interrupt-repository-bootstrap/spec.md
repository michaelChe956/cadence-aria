# Spec Delta

## MODIFIED Requirements

### Requirement: 无中断 Claude Code 初始化命令（REQ-BOOT-01）

系统 SHALL 使逻辑代码库场景下四个无中断命令在 canonical LC 聚合根执行一次；这次 root-cwd recipe 统一生成根级规则、MCP 与相关 provider-native 配置，不再以逐成员仓确定性本地化作为 LC 前提，也不逐成员仓库启动初始化 provider。传统单仓登记 SHALL 保持现有逐仓四命令与 `git_finalize` 契约不变。

#### Scenario: 逻辑代码库聚合根执行一次

- **WHEN** 逻辑代码库场景下执行聚合初始化
- **THEN** 四个无中断命令 SHALL 在 canonical 聚合根各执行一次，recipe SHALL 记录根级产物与可审计 receipt，不启动逐成员初始化会话或向成员仓复制配置

#### Scenario: 传统单仓登记保持不变

- **WHEN** 非逻辑代码库执行传统单仓登记
- **THEN** 现有逐仓四命令、单仓 cwd、operation 及 `git_finalize` 行为 SHALL 保持原契约

## ADDED Requirements

### Requirement: 聚合根 recipe 与 readiness 事实闭环（REQ-BOOT-03）

逻辑代码库的 root-cwd recipe SHALL 复用四条无中断命令的取消、超时、输出摘要与步骤恢复语义，并将其映射到既有五步聚合初始化 operation；recipe provider SHALL 固定为 Claude Code。LC 根准入冻结 canonical root 后、Claude Code recipe operation 启动前 SHALL 完成/记录面向后续 Codex/Kimi session 的独立 trust preparation 与核验。该事实不属于五步 operation、不插入步骤之间且不新增第六步；它不是 Claude Code recipe 的执行前提，Claude recipe 按五步启动。命令成功、根级产物 receipt、根 policy/rule 摘要和 bootstrap 投影 SHALL 独立记录。recipe 完成本身 MUST NOT 被视为 `planning_ready`，直到规则/policy、成员索引和聚合索引 active 等投影事实均完成。

#### Scenario: 四命令按根顺序执行

- **WHEN** LC 根准入已确定并冻结 canonical root，trust preparation 已记录后启动新 LC 的 Claude Code 五步聚合初始化 recipe
- **THEN** 随后 SHALL 按 MachineSkills→AggregatePreflight→PreCheck→RuleAndMcpConfig→OpenspecAndExamples 原样推进；四条 `/pre-check --no-interrupt --upgrade 用大陆镜像`、`/rule-config --no-interrupt`、`/mcp-configuration --no-interrupt`、`/project-rules-examples --no-interrupt` SHALL 按固定顺序在根执行；任一命令失败、取消或输出不可审计时 SHALL 停止 recipe/后续命令并保留失败事实。Codex/Kimi trust preparation 的失败只阻断依赖该 trust 的后续 session，不阻断固定 Claude Code recipe。

#### Scenario: recipe 成功但 readiness 材料未齐

- **WHEN** 四条命令已成功但根 rule/policy receipt、MemberIndex 或 AggregateIndexActive 投影缺失或摘要漂移
- **THEN** operation MAY 为 Completed，但 `planning_ready` SHALL 保持非就绪并返回可操作的核验/继续等待项；普通 provider session SHALL 不得仅凭 recipe Completed 放行

#### Scenario: 根 recipe 不产生单仓 Git 副作用

- **WHEN** root-cwd recipe 为包含多个独立 Git 成员的 LC 执行
- **THEN** 系统 SHALL 不调用单仓登记持久化、成员 GitFinalize、成员主 checkout 写入或成员初始化 provider；成员 Git 身份与状态 SHALL 保持不变

### Requirement: 聚合初始化自举相位凭据（REQ-BOOT-04）

系统 SHALL 为唯一的“根规则尚未生成”自举例外使用内部不可伪造的 bootstrap phase credential。凭据 SHALL 由当前 durable aggregate initialization operation 的 Running 状态、当前 step、operation input digest、LC 身份和 canonical root 共同派生，并在 provider spawn 前重新核验；普通 provider session 不得自行声明该相位。

#### Scenario: 新 LC 使用有效自举凭据

- **WHEN** 新 LC 的 Running operation 正处于允许执行根 recipe 的步骤，且凭据的 LC、root、step 与 input digest 均匹配
- **THEN** admission SHALL 仅豁免“根规则尚未生成”检查，同时继续校验 authority、临时 policy、capability、gateway、cwd、target 与可用性后才允许 provider spawn

#### Scenario: 凭据缺失或漂移

- **WHEN** provider 请求缺少凭据，或 operation 已 Completed/Failed/Cancelled，或 step、input digest、LC/root 与当前 durable operation 不一致
- **THEN** provider SHALL 在 spawn 前被拒绝并生成 fail-closed 等待/诊断，系统不得以普通 session 或宽松权限替代自举路径

#### Scenario: 普通会话不可绕过根规则

- **WHEN** 普通 planning、coding 或 review session 在根规则缺失、替换或摘要漂移时启动
- **THEN** 系统 SHALL 零启动 provider，返回根规则/policy 准备或重试操作，且不得自动取得 bootstrap 例外
