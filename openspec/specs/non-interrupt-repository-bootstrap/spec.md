# non-interrupt-repository-bootstrap Specification

## Purpose

以无中断模式（`--no-interrupt`）执行 Cadence-skills 的四个 Claude Code 初始化命令，禁止初始化流程等待人工输入。
## Requirements
### Requirement: 无中断 Claude Code 初始化命令（REQ-BOOT-01）
系统 SHALL 使逻辑代码库场景下四个无中断命令在聚合根执行一次（聚合模式），逐仓本地化由确定性程序完成，不逐成员仓库启动 Claude 会话；传统单仓登记保持原契约。

#### Scenario: 逻辑代码库聚合根执行一次
- **WHEN** 逻辑代码库场景下执行聚合初始化
- **THEN** 四个无中断命令 SHALL 在聚合根执行一次，逐仓本地化由确定性程序完成，不逐成员仓库启动 Claude 会话

#### Scenario: 传统单仓登记保持不变
- **WHEN** 非逻辑代码库的传统单仓登记
- **THEN** 现有逐仓四命令与 git_finalize 行为保持原契约不变

