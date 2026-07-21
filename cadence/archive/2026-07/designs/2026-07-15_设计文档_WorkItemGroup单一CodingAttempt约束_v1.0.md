# Work Item Group 单一 Coding Attempt 约束设计

## 1. 背景

当前 Coding Workspace 前端仅把 `created`、`running`、`waiting_for_human`、`blocked` 状态的 Group Coding Attempt 识别为“已有 Attempt”。当关联 Attempt 已经 `completed` 时，Work Item Group 抽屉错误显示“开始 Coding”，再次点击会向后端申请新的 Attempt。

后端创建 Group Coding Attempt 的接口也只阻止新的活跃 Attempt，没有保证同一个 Work Item Group 只能关联一份 Attempt。因此，前端判断遗漏、重复点击或重复请求都可能产生第二份 Group Attempt。

## 2. 目标约束

- 一个 Work Item Group 在其关联 Coding Attempt 记录存在期间，只能关联一个 Coding Attempt。
- Attempt 状态不影响关联关系；`created`、`running`、`waiting_for_human`、`blocked`、`completed`、`failed`、`aborted` 都是已有 Attempt。
- 重复调用 Group Coding Attempt 创建接口时返回原 Attempt，不创建新 Attempt、Coding Unit 或 Provider 配置。
- 不改变普通单 Work Item Coding Attempt 的现有行为。
- 不改变不同 Work Item Group 之间的关联关系。
- 若已有 Attempt 被明确删除，其关联关系随记录删除，之后允许重新创建。

## 3. 方案

### 3.1 前端入口

`resolveGroupCodingAttempt` 按 `work_item_group_id` 查找关联 Attempt，不再用状态集合过滤。只要存在关联记录，抽屉就显示“进入 Coding Workspace”，点击后直接进入原 Attempt。

### 3.2 后端接口幂等

Group Coding Attempt 创建处理器在准备共享 worktree、获取 Provider 配置和创建 Unit 之前查询该 Group 的既有 Attempt：

- 存在：直接返回既有 Attempt DTO。
- 不存在：继续执行当前创建流程。

处理器对同一 Group 的创建过程使用进程内互斥，避免同一后端实例中的并发请求同时通过查询。

### 3.3 存储层唯一性兜底

`CodingAttemptStore` 提供按 `project_id + issue_id + work_item_group_id` 查询 Attempt 的方法。`create_group_attempt` 在写入前再次检查；若发现已有记录，返回包含原 Attempt ID 的明确冲突错误。处理器捕获该冲突后读取并返回原 Attempt，从而保持 API 幂等。

该兜底同时保护绕过 Web Handler 的内部调用，避免唯一性只依赖前端或路由层。

### 3.4 历史异常数据

本次不自动删除历史重复 Attempt。若读取到遗留重复数据，按最早创建的 Attempt（`attempt_no`、`attempt_id` 稳定排序）作为原关联记录，并阻止继续创建。历史数据清理应使用独立的数据修复流程。

## 4. 测试设计

- 前端单元测试：已完成的 Group Attempt 显示“进入 Coding Workspace”，点击后不发送创建请求。
- 前端单元测试：`failed`、`aborted` 等非活跃状态仍解析为已有 Attempt。
- Store 单元测试：同一 Group 已有 completed Attempt 时拒绝第二次创建，并返回原 Attempt ID。
- API 集成测试：对同一 Group 重复 POST 返回相同 Attempt ID，且 Coding Unit 数量不增加。
- API 集成测试：不同 Group 在没有活跃 Attempt 冲突时仍可各自创建自己的 Attempt。

## 5. 非目标

- 不修改 Coding Workspace 内部执行、Review 或 Final Review 流程。
- 不修改 Coding worktree 中的业务代码或 Attempt 数据。
- 不引入 E2E、Playwright 或浏览器自动化测试。
- 不调整普通单 Work Item Attempt 是否允许多次创建的现有规则。
