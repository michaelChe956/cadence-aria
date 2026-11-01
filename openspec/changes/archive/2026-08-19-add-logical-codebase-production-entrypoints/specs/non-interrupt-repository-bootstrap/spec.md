## ADDED Requirements

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
