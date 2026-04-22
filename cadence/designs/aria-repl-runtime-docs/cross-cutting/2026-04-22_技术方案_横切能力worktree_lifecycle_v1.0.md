# 横切能力文档：worktree_lifecycle

## 1. 能力标识

- 能力 ID：`CC06`
- 能力名称：`worktree_lifecycle`
- 类型：隔离 / 调度
- 适用范围：全部 WorkTask 执行节点

## 2. 能力目的

确保每个任务拥有独立代码隔离空间，避免并发任务相互污染。

## 3. 触发条件

- WorkTask 注册完成
- 准备进入 coding/testing/review
- 集成完成后回收

## 4. 前置状态与输入

- `workTaskId`
- base ref
- repo root

## 5. Aria 执行动作

1. 申请 `WorktreeLease`
2. 创建或复用任务 worktree
3. 记录 branch、path、base ref
4. 完成后回收或保留现场

## 6. 状态变化与副作用

- 新增或更新 `WorktreeLease`
- 影响 provider 调用工作目录

## 7. 输出与记录

- worktree ready snapshot
- worktree cleanup snapshot

## 8. 完成判定

worktree 可用、路径存在、分支映射正确。

## 9. 失败与恢复

- 创建失败：任务不能进入执行节点
- worktree 污染：退回人工处理或重新创建

## 10. 与节点文档的关联规则

凡是依赖代码执行环境的节点都必须声明对本能力的依赖。

