# 横切能力文档：manual_intervention

## 1. 能力标识

- 能力 ID：`CC08`
- 能力名称：`manual_intervention`
- 类型：控制 / 终态处理
- 适用范围：全部节点

## 2. 能力目的

在系统无法安全自动继续时，显式进入人工介入终态，而不是让任务停在模糊失败状态。

## 3. 触发条件

- 无法自动决策
- 连续失败超阈值
- 恢复失败
- 集成风险过高

## 4. 前置状态与输入

- 原节点上下文
- 失败摘要
- 建议人工动作

## 5. Aria 执行动作

1. 生成 intervention record
2. 更新 task 状态为 `blocked` 或 `terminated_pending_decision`
3. 向 REPL 暴露摘要和建议
4. 写 checkpoint

## 6. 状态变化与副作用

- 暂停后继自动调度
- 等待用户人工接管

## 7. 输出与记录

- manual intervention record
- user-visible intervention summary

## 8. 完成判定

用户明确回复继续、回流、终止或人工处理完成时。

## 9. 失败与恢复

- 记录写入失败：升级为系统级失败
- daemon 恢复后：继续保留 intervention state

## 10. 与节点文档的关联规则

任何节点只要存在 `manual_intervention_required` 分支，都必须引用本能力。

