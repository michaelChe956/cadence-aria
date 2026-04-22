# 横切能力文档：checkpoint_and_recovery

## 1. 能力标识

- 能力 ID：`CC05`
- 能力名称：`checkpoint_and_recovery`
- 类型：恢复 / 存储
- 适用范围：全部关键节点

## 2. 能力目的

支持 daemon 崩溃、终端退出、机器重启后的状态恢复，确保任务不因前台断开而丢失。

## 3. 触发条件

- 节点进入前后
- gate 挂起前后
- integration 前后
- daemon 启动恢复时

## 4. 前置状态与输入

- session state
- task states
- gate queue
- worktree refs
- latest artifacts

## 5. Aria 执行动作

1. 写 append-only event
2. 定期或关键节点写 checkpoint
3. daemon 启动时回放 event 并恢复 checkpoint

## 6. 状态变化与副作用

- 更新 checkpoint refs
- running task 可能转为 `recovering`

## 7. 输出与记录

- checkpoint file
- recovery report

## 8. 完成判定

checkpoint 可被重新加载，恢复后的 session 与 task 索引一致。

## 9. 失败与恢复

- checkpoint 写入失败：继续保留 event log 并转保守模式
- recovery 失败：进入人工介入

## 10. 与节点文档的关联规则

节点文档只声明何时必须打 checkpoint，不重复定义恢复算法。

