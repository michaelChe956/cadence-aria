# Spec Delta

## MODIFIED Requirements

### Requirement: 无中断 Claude Code 初始化命令（REQ-BOOT-01）

系统 SHALL 使逻辑代码库场景下四个无中断命令在聚合根执行一次（聚合模式），逐仓本地化由确定性程序完成，不逐成员仓库启动 Claude 会话；传统单仓登记保持原契约。

**已初始化的聚合根 SHALL 跳过这四个命令**：当聚合根同时存在 `AGENTS.md` 与 `.claude/rules/language.md`（即 Cadence-skills 初始化已完成）时，聚合初始化 MUST 跳过 `PreCheck` / `RuleAndMcpConfig` / `OpenspecAndExamples` 三个 provider-turn 步骤（对应四条命令），只执行确定性步骤并合成、发布聚合政策。

跳过 MUST NOT 是静默忽略：operation MUST 记录跳过判定（含两个判定文件的摘要），且判定与三个 `skipped` 步骤 MUST 在同一次持久化写入中落盘。**一旦该 operation 已存在任何命令审计事实，判定窗口即关闭，MUST NOT 再改判为跳过。** 跳过式回执与「四命令全部审计通过」的完整回执互斥。

#### Scenario: 逻辑代码库聚合根执行一次

- **WHEN** 逻辑代码库场景下执行聚合初始化（聚合根未初始化）
- **THEN** 四个无中断命令 SHALL 在聚合根执行一次，逐仓本地化由确定性程序完成，不逐成员仓库启动 Claude 会话

#### Scenario: 传统单仓登记保持不变

- **WHEN** 执行传统单仓登记
- **THEN** 现有逐仓四命令与 git_finalize 行为保持原契约不变

#### Scenario: 已初始化的聚合根跳过四命令

- **WHEN** 对已存在 `AGENTS.md` 与 `.claude/rules/language.md` 的聚合根执行聚合初始化
- **THEN** 系统 MUST 不启动任何 provider turn；三个 provider-turn 步骤标记为 `skipped` 并记录判定依据；聚合政策仍 SHALL 被合成与发布，operation 最终到达 Completed

#### Scenario: 已执行过命令的 operation 不改判跳过

- **WHEN** 某 operation 已有命令审计事实（例如旧版本中断后续跑），即便判定文件齐备
- **THEN** 系统 MUST NOT 将其改判为跳过，SHALL 继续执行剩余命令

#### Scenario: 跳过但无政策发布路径

- **WHEN** 判定为跳过，但该环境没有可用的聚合政策发布通道
- **THEN** operation MUST fail-closed 为 Failed，MUST NOT 以"跳过且未发布"收口


## ADDED Requirements

### Requirement: 聚合根 recipe 与 readiness 事实闭环（REQ-BOOT-03）

逻辑代码库的 root-cwd recipe SHALL 复用四条无中断命令的取消、超时、输出摘要与步骤恢复语义，并将其映射到既有五步聚合初始化 operation；recipe provider SHALL 固定为 Claude Code。LC 根准入冻结 canonical root 后、Claude Code recipe operation 启动前 SHALL 完成所选 Codex/Kimi trust 的独立前置登记与核验。该登记不属于五步 operation、不插入步骤之间且不新增第六步；它是五步 recipe 的硬前置门，任一 trust 未 Ready 时五步 operation 不创建/不启动并进入可重试 fail-closed waiting。全部 trust Ready 后才启动五步 recipe。命令成功、根级产物 receipt、根 policy/rule 摘要和 bootstrap 投影 SHALL 独立记录。recipe 完成本身 MUST NOT 被视为 `planning_ready`，直到规则/policy、成员索引和聚合索引 active 等投影事实均完成。

#### Scenario: 四命令按根顺序执行

- **WHEN** LC 根准入已确定并冻结 canonical root，所选 Codex/Kimi trust gate 已登记并核验后启动新 LC 的 Claude Code 五步聚合初始化 recipe
- **THEN** SHALL 按 MachineSkills→AggregatePreflight→PreCheck→RuleAndMcpConfig→OpenspecAndExamples 原样推进；四条 `/pre-check --no-interrupt --upgrade 用大陆镜像`、`/rule-config --no-interrupt`、`/mcp-configuration --no-interrupt`、`/project-rules-examples --no-interrupt` SHALL 按固定顺序在根执行；任一 trust 登记失败时五步 recipe 不启动并保留可重试等待事实，任一命令失败、取消或输出不可审计时 SHALL 停止 recipe/后续命令并保留失败事实。

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
