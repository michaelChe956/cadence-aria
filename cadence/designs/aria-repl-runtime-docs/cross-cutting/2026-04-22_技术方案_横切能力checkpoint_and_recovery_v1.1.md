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

## Event Log 增长管理

### Compaction 策略

- Event log 采用 append-only 写入，不做原地修改
- 每累计 500 条 event 或每 30 分钟（以先到者为准），触发一次 compaction
- Compaction 操作：
  1. 将当前 event log 中所有 `task_completed`、`task_terminated` 类型的 event 之前的所有 event 合并为一条 summary event
  2. summary event 包含：taskId、起止时间、最终状态、关键产物引用
  3. 原始 event 文件保留（不删除），但文件名追加 `.archived` 后缀
  4. 新的 event log 从 summary event 开始
- compaction 本身也需要写一条 `compaction_executed` 类型的 event

### Checkpoint 完整性校验

- 每次 checkpoint 写入完成后，立即计算 SHA-256 校验和
- 校验和存储在 checkpoint 文件同目录下的 `.checksum` 文件中
- daemon 启动恢复时：
  1. 读取最新 checkpoint
  2. 校验 checksum 是否匹配
  3. 若不匹配，尝试使用上一个 checkpoint
  4. 若所有 checkpoint 均损坏，回退到 event log 全量回放

### 回放性能优化

- daemon 启动时优先恢复最新 checkpoint，然后仅回放 checkpoint 之后的 event
- 不做全量 event log 回放（除非 checkpoint 全部损坏）
- 回放时跳过与当前 session 无关的 event（通过 sessionId 过滤）

### 存储上限

- event log 单文件上限：50MB
- 超过上限时触发 compaction
- archived event 文件保留 7 天后自动清理（可配置）
- checkpoint 保留最近 10 个版本（可配置）
