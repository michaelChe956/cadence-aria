# 节点文档：N01 intake_capture

## 1. 节点标识

- 节点 ID：`N01`
- 节点类型：混合
- 主执行者：REPL + Aria daemon
- 所属链路：需求入口
- 版本：v1.0

## 2. 节点目的

捕获用户原始需求、补充上下文和会话入口信息，并生成后续所有上游节点都要消费的 `intake_brief`。

## 3. 进入条件

- `N00` 已完成
- 当前 session 可用

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| user raw request | REPL | 是 | 原始文本 | 不进入节点 |
| sessionId | `N00` | 是 | session ref | 回退 `N00` |
| repo context | 本地仓库 | 否 | repo 名称、分支 | 缺失则用空上下文 |

## 5. Aria 驱动动作

1. 读取用户原始需求。
2. 建立初始会话上下文。
3. 生成 `intake_brief` 初稿。
4. 为后续 EpicTask 分配预留 task key。
5. 写 snapshot 和 checkpoint。

## 6. Provider 执行契约

无。该节点不调用外部 provider。

## 7. 输出产物

- `intake_brief`
- `runtime_snapshot:N01.intake_capture`

## 8. 输出产物最小格式

`intake_brief` 至少包含：

- `request_summary`
- `raw_user_request`
- `repo_context`
- `initial_constraints`
- `requested_goal`

## 9. 完成判定

`intake_brief` 已生成并可被 `N02` 消费。

## 10. 失败与回流

- 用户输入为空：停留在当前节点等待补充
- brief 生成失败：回退 REPL 重新提交输入

## 11. 交接到下一节点

将 `intake_brief`、`sessionId`、初始 repo 上下文交给 `N02 epic_task_create`。

## 12. 关联横切能力

- `checkpoint_and_recovery`
- `artifact_validate`

