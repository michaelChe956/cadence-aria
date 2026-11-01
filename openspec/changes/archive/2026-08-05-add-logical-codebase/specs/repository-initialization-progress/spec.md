# repository-initialization-progress Specification

## Purpose

（主规范沿用）为逻辑代码库场景扩展：初始化从旧 6 步逐仓执行改为聚合 5 步（机器一次 + 聚合根一次 + 逐仓确定性本地化），聚合初始化提供可轮询的独立 operation，不再向成员仓执行 git 提交推送。

## MODIFIED Requirements

### Requirement: 可轮询的代码库初始化操作（REQ-INIT-01）
系统 SHALL 在逻辑代码库场景提供可轮询、可恢复的聚合初始化操作（manifest revision、成员状态机、幂等 key、取消语义），并暴露与单仓 operation 一致的状态查询；不得复用固定六步单仓 operation 伪装为聚合。

#### Scenario: 聚合初始化操作可轮询
- **WHEN** 逻辑代码库场景下用户触发聚合初始化
- **THEN** 系统 SHALL 提供独立的聚合初始化 operation 并暴露可轮询状态

### Requirement: 聚合初始化真实状态机（REQ-REG-05）
系统 SHALL 使聚合初始化步骤状态机表达聚合级步骤（CadenceSkills/全局工具一次、聚合规则/MCP/OpenSpec/示例生成、成员预检），而非逐成员重复固定步骤;聚合模式步骤数由 design.md §7.2 spike 定值的 5 个稳定 step ID 表达。

#### Scenario: 逻辑代码库聚合步骤
- **WHEN** 逻辑代码库场景下执行聚合初始化
- **THEN** 步骤状态机 SHALL 表达聚合级步骤，而非逐成员重复五步

### Requirement: 聚合初始化不向成员仓提交推送（REQ-REG-06）
系统 SHALL NOT 在逻辑代码库聚合初始化完成时对任何成员仓执行 `git add -A`/commit/push；逐仓提交推送的 GitFinalize 仅保留给传统单仓登记路径。

#### Scenario: 聚合初始化不向成员仓提交推送
- **WHEN** 逻辑代码库场景下聚合初始化完成
- **THEN** 系统 SHALL NOT 对任何成员仓执行 git 提交推送；成员仓内不得残留聚合初始化的本地化资产
