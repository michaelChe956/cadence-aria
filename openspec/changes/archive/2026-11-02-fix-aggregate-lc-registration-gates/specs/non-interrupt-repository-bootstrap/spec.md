# Spec Delta: fix-aggregate-lc-registration-gates

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

### Requirement: 聚合重新初始化入口

系统 SHALL 提供聚合「重新初始化」入口：对已存在（含已初始化）的逻辑代码库，**无视初始化检测结果**强制执行完整五步（含四条无中断命令），用于 Cadence-skills 更新后重装。

#### Scenario: 已初始化根上的重新初始化

- **WHEN** 对已初始化的聚合根请求重新初始化
- **THEN** 系统 SHALL 执行完整五步并运行全部四条命令，MUST NOT 因检测到已初始化而跳过；最终生成完整 recipe 回执

#### Scenario: 重新初始化与普通初始化通道一致

- **WHEN** 重新初始化请求与普通初始化请求并发
- **THEN** 两者 MUST 遵循同一套通道/幂等/并发仲裁语义，不得出现并行双跑
