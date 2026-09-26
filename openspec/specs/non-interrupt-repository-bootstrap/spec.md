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

### Requirement: 聚合初始化的生产执行

聚合初始化端点创建的 operation 必须被真正执行，而非仅创建可轮询记录。

#### Scenario: 提交即执行
- **WHEN** POST /api/projects/{pid}/logical-codebase/initializations 成功
- **THEN** 服务端后台执行初始化流程（machine skills → 聚合预检 → provider turns → 完成），使用生产级依赖（真实 skills 准备与聚合根预检，非空实现）；operation 最终到达 Completed 或 Failed

#### Scenario: 取消在步骤边界生效
- **WHEN** 初始化执行中调用 cancel
- **THEN** 正在执行的步骤完成后不再推进，operation 进入 Cancelled；取消触发不依赖轮询

#### Scenario: 重启恢复语义
- **WHEN** 服务重启后查询 Running 且无活跃执行租约的 operation
- **THEN** 标记为 Failed 附 interrupted 原因；人工重触发创建新 operation（新幂等键），不复活旧记录

#### Scenario: 执行租约共享
- **WHEN** 任意 handler 查询或取消初始化
- **THEN** 通过进程级共享的执行注册表（含取消令牌）判定活跃状态，不因 handler 实例隔离而误判中断
