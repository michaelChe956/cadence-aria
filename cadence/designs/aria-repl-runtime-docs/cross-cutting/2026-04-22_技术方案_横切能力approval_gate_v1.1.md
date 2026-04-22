# 横切能力文档：approval_gate

## 1. 能力标识

- 能力 ID：`CC01`
- 能力名称：`approval_gate`
- 类型：交互 / 控制
- 适用范围：允许人工确认的全部节点

## 2. 能力目的

在节点无法自动继续或策略要求人工确认时，显式创建闸门对象、冻结后继调度，并等待用户通过 REPL 恢复。

## 3. 触发条件

- 信息缺失
- 高风险动作
- 设计冲突
- 集成冲突
- 连续失败超阈值

## 4. 前置状态与输入

- 当前 `taskId`
- 当前 `nodeId`
- gate reason
- 建议动作
- 当前交接包

## 5. Aria 执行动作

1. 生成 `gateId`
2. 写入 `ApprovalGate`
3. 将 task 状态切换为 `blocked`
4. 发送 REPL 可见摘要
5. 写 event log 和 checkpoint

## 6. 状态变化与副作用

- 新增 `ApprovalGate`
- task 状态变为 `blocked`
- 暂停原节点后继调度

## 7. 输出与记录

- gate record
- user-visible gate summary

## 8. 完成判定

当用户执行 `approve/reject/reply` 且 `approval_gate_resume` 完成处理时，本能力视为完成。

## 9. 失败与恢复

- gate 写入失败：节点不能进入阻塞态，必须退回原节点失败
- daemon 重启：通过 checkpoint 恢复全部未关闭 gate

## 10. 与节点文档的关联规则

允许挂 gate 的节点必须引用本能力，并明确触发条件和恢复后回到哪个节点。

## Gate 超时机制

### TTL 配置

- 每个 ApprovalGate 支持可选的 TTL（Time To Live）
- 默认 TTL：无限（不超时，等待用户主动响应）
- 支持阶段级策略覆写 TTL 值
- TTL 取值范围：`null`（无限）或正整数（单位：分钟）
- 常见配置示例：
  - clarification 阶段 gate：默认无限（等待用户回复）
  - design review gate：默认无限
  - integration gate：默认 60 分钟（避免长时间阻塞集成队列）

### 超时处理

- gate 创建时记录 `createdAt` 时间戳
- daemon 定期检查（每次 event loop 迭代）未关闭的 gate 是否超时
- 超时判定：`current_time - createdAt > ttl`
- 超时后处理：
  1. 更新 gate 状态为 `expired`
  2. 任务状态从 `blocked` 变更为进入 X08（manual_intervention_hold）
  3. 生成 intervention record，注明 `gate_expired`
  4. intervention record 包含原始 gate 的摘要和建议动作
  5. 写 checkpoint
  6. 向 REPL 发送超时通知

### 超时与策略的关系

| 策略模式 | 默认 TTL 行为 |
|---------|-------------|
| `conservative` | 无限（永不超时） |
| `balanced` | 默认 120 分钟 |
| `aggressive` | 默认 30 分钟 |

阶段级覆写优先于策略默认值。

### 多 gate 并存规则

- 同一 WorkTask 同一时间只允许存在一个 active gate
- 如果已有 active gate 时需要创建新 gate，Aria 先将旧 gate 标记为 `superseded`，再创建新 gate
- 同一 EpicTask 下的不同 WorkTask 可以各自拥有独立的 gate
- 所有 active gate 在 REPL 中以列表形式展示给用户

### Gate 记录存储

- gate record 存储在 `.aria/runtime/gates/` 目录
- 文件命名格式：`<gateId>.json`
- gate 关闭后文件保留，不删除（供审计追溯）
