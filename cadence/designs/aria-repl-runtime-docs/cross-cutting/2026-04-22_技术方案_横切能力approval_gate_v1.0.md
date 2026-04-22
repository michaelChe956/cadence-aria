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

