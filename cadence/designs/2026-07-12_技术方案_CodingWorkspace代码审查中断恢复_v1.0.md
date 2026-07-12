# Coding Workspace 代码审查中断恢复技术方案

## 1. 背景

`coding_attempt_0001` 在 Work Item 2 的 Code Review 阶段等待 Reviewer 权限响应超时。当前实现通过 `fail_provider_stream` 直接把整个 Coding Attempt 标记为 `failed`，随后尝试释放 Issue 共享 worktree 锁。由于 Work Item 2 已产生未提交修改，共享 worktree 不干净，清理逻辑又创建了 `shared_worktree_dirty_manual_gate`。

这会形成不一致状态：

- Attempt 已是终态 `failed`；
- 页面仍展示“人工继续”Gate；
- Coding WebSocket 在终态检查处拒绝所有业务消息，包括该 Gate 的 `gate_response`；
- 用户收到 `coding_message_not_allowed`，无法继续 Reviewer，也无法安全创建新 Attempt；
- Work Item 2 的代码仍保存在共享 worktree，但后续 Unit 不再执行。

## 2. 目标

- Code Review 因 Provider 断流、权限超时或进程异常中断时，不把整个 Attempt 不可逆地终止。
- 提供显式手动操作“重试代码审查”，由用户决定是否继续调用 Provider。
- 保留当前共享 worktree、未提交修改、Coding Unit 顺序和已完成 Work Item。
- 兼容已经持久化为 `failed` 的历史 Attempt，使 `coding_attempt_0001` 可以原地恢复。
- 防止重复点击启动多个 Reviewer Run。
- 不自动提交、丢弃、重置或清理共享 worktree 中的代码。

## 3. 非目标

- 不自动批准 Reviewer 的权限请求。
- 不自动重试 Provider。
- 不在本次改动中设计 Coding、Testing、Internal PR Review 的通用断线恢复协议。
- 不创建新的 Coding Attempt，也不重跑已完成的 Work Item 1。
- 不修改 Work Item 2 当前未提交代码的内容。

## 4. 方案比较

### 4.1 阶段级原地恢复（采用）

Code Review Provider 失败后将 Attempt 转为 `blocked`，创建带 `retry_review`、`send_to_coder` 和 `abort` 操作的恢复 Gate。用户点击“重试代码审查”后，系统 supersede 失败的 Reviewer Role Run，并在同一 Attempt、同一 active Unit、同一共享 worktree 上启动新的 Reviewer Run。

优点是保留完整执行上下文，并能避免后续再次出现同类死锁。代价是需要同时调整 Engine、WebSocket 合法消息判断和历史状态恢复。

### 4.2 仅修改当前持久化 JSON（不采用）

直接把 `coding_attempt_0001` 从 `failed` 改为 `blocked` 并替换 Gate。该方式能临时解锁当前数据，但未来同类 Provider 超时仍会复现，而且人工修改状态容易破坏审计一致性。

### 4.3 创建新 Attempt（不采用）

清理锁后新建 Attempt 会丢失当前 Role Run 连续性，并可能重复执行 Work Item 1 或错误处理 Work Item 2 的未提交修改，因此不采用。

## 5. 状态机设计

### 5.1 新发生的 Code Review Provider 中断

当满足以下条件时，将失败降级为可恢复阻塞：

- 当前 stage 为 `code_review`；
- 当前 Attempt 不是 `completed` 或 `aborted`；
- Provider 运行因 stream error、权限请求超时、进程异常退出或 completion 缺失而失败；
- 当前 active Unit 仍存在。

处理顺序：

1. 把当前 Code Review Timeline Node 标记为 `failed`，保留原始失败原因。
2. 把 Attempt 状态更新为 `blocked`，stage 保持 `code_review`。
3. 创建 Code Review 恢复 Gate：
   - `retry_review`，文案“重试代码审查”；
   - `send_to_coder`，文案沿用现有定义；
   - `abort`，文案沿用现有定义。
4. 不调用共享 worktree 清理和锁释放逻辑。
5. 通过 WebSocket 推送 `coding_gate_required` 和最新 SessionState。

### 5.2 用户点击“重试代码审查”

后端必须重新读取持久化 Attempt 和 Gate，不信任前端传入的 stage：

1. Gate 必须处于 open 状态，stage 为 `code_review`，action 为 `retry_review`。
2. Attempt 必须是 `blocked`，或满足第 6 节定义的可恢复历史 `failed` 状态。
3. 当前不能存在仍由运行注册表持有的 Reviewer Run。若持久化 Role Run 仍为 `running`，但它绑定的 Timeline Node 已是 `failed`，则视为旧故障路径留下的僵尸记录，允许在恢复时 supersede。
4. active Unit 必须仍是 `running`，共享 worktree 路径必须存在。
5. 将 Attempt 更新为 `running + code_review`。
6. supersede 上一个失败 Reviewer Role Run，创建 trigger 为 `retry_review` 的新 Role Run。
7. 关闭恢复 Gate并启动 Coding Runner。

重复点击时，第二次请求必须被稳定拒绝，不得创建第二个 Reviewer Run。

### 5.3 原子 Runner Reservation

`runner_count()` 只读检查不能作为并发互斥。恢复请求必须在首次持久化写入前，通过 `CodingRunRegistry` 原子取得 Attempt 级 reservation：

- 同一 Attempt 同一时刻只能存在一个 reservation 或 active Runner；
- 第二个并发请求在任何 Attempt、Gate、Role Run 写入前返回 `coding_recovery_already_active`；
- Engine 恢复失败时 reservation 自动释放；
- Engine 恢复成功后，现有 `spawn_coding_runner` 使用同一个 reservation 激活 Runner，不允许再次无条件 insert；
- socket 或任务提前退出时，未激活 reservation 必须自动释放。

### 5.4 幂等恢复 Journal

历史恢复使用 Attempt 目录内的轻量 journal 记录预期身份与阶段，不引入全仓通用事务框架。Journal 至少包含：

- attempt ID；
- gate ID；
- failed Timeline Node ID；
- stale Reviewer Role Run ID；
- 新 RetryReview Role Run ID（创建后写入）；
- 当前恢复 phase；
- created/updated timestamp。

恢复 phase 至少区分：prepared、attempt reopened、retry run prepared、attempt running、gate resolved、runner started。每个 phase 必须满足：

- 进入 phase 前按 journal 中的 expected IDs 精确校验，不重新选择“最新”Node 或 Role Run；
- phase 对应操作可重复执行；进程在操作完成、phase 更新前退出时，下次调用能够识别已完成的前缀并继续；
- 已创建的 RetryReview Run 使用稳定 recovery key，重试时复用，不创建第二个 Run；
- Gate 已 resolved 但 journal 未完成时，SessionState 仍派生恢复 Gate；
- Runner 激活后才把 journal 标为 completed/runner started；若标记失败，registry reservation/active Runner 仍阻止重复启动。

Journal 未完成时，Attempt 可以处于 Failed、Blocked 或 Running CodeReview；这些状态仅能通过匹配 journal 的 `retry_review` 继续，不能放宽普通 terminal 消息规则。

## 6. 历史 Failed Attempt 兼容恢复

历史数据只有同时满足以下条件才允许恢复：

- Attempt `status=failed`；
- Attempt `stage=code_review`；
- scope 为 `work_item_group` 或普通 Work Item Attempt；
- 存在 active Unit，且状态为 `running`；
- 最新 Code Review Timeline Node 为 `failed`；
- 不存在更新的 completed Code Review Node；
- Attempt 没有 `completed_at` 之后的新业务执行；
- 共享 worktree 存在；
- 不存在仍由运行注册表持有的 Reviewer Run；允许最新持久化 Reviewer Role Run 为 `running`，但仅限其 `node_id` 精确绑定最新失败 Code Review Node 的历史僵尸记录。

SessionState 重建时，若存在 `shared_worktree_dirty_manual_gate`，或存在未完成的匹配 recovery journal，后端输出一个派生的 Code Review 恢复 Gate，不直接修改 Attempt、Gate、Role Run 或 Unit。用户首次点击 `retry_review` 时：

1. 校验上述历史状态仍未变化；
2. 关闭或 supersede `shared_worktree_dirty_manual_gate`；
3. 清除终态完成时间；
4. 原子取得 Attempt 级 Runner reservation；
5. 创建或继续幂等 recovery journal；
6. 若旧 Reviewer Role Run 是与失败 Node 绑定的僵尸 `running` 记录，将其 supersede；
7. 将 Attempt 恢复为 `running + code_review`；
8. 创建或复用新的 RetryReview Role Run并启动 Runner；
9. Runner 激活后完成 journal。

该兼容路径只允许 Code Review 恢复，不能把任意 failed Attempt 通用地重新打开。

## 7. 当前数据恢复结果

对 `coding_attempt_0001` 的预期恢复结果：

- Work Item 1 保持 completed，提交 `b0373a0` 不变化；
- Work Item 2 保持 active/running；
- 共享 worktree `/home/michaelche/workspace/github/cadence-aria/.worktrees/aria-issues/issue_0001` 的未提交修改不变化；
- Work Item 3 至 Work Item 10 保持 pending；
- 原失败 Code Review Node 和权限超时原因保留；
- 新增关联原失败 Reviewer Run 的 retry Role Run；
- Reviewer 成功后继续现有 Group Coding 流程。

## 8. WebSocket 与前端

不新增独立消息类型，复用现有：

```json
{
  "type": "gate_response",
  "gate_id": "coding_recovery_gate_0001",
  "action_id": "retry_review"
}
```

调整点：

- `is_coding_ws_message_allowed` 对经过严格识别的历史 Code Review 恢复 Gate 允许 `gate_response`。
- 普通 terminal Attempt 仍拒绝所有业务消息。
- 页面使用现有 Gate UI 展示“重试代码审查”，不允许在该状态发送普通 Context Note 代替恢复操作。
- 点击后按钮进入 pending 状态，收到 SessionState、Runner 事件或 ProtocolError 后复位。

## 9. 错误处理

至少区分以下错误：

- 恢复 Gate 已失效；
- Attempt 状态已变化；
- active Unit 已变化或不存在；
- Reviewer Run 仍由运行注册表持有，或持久化 `running` Role Run 未绑定最新失败 Node；
- recovery journal 的 expected gate/node/run 与当前持久化身份不一致；
- 共享 worktree 不存在；
- 当前 Failed Attempt 不符合历史恢复条件。

任何校验失败都不得修改 Attempt、Unit、Gate、Role Run 或共享 worktree。

## 10. 测试设计

### 10.1 Engine

- Code Review Provider 权限超时后 Attempt 进入 `blocked + code_review`，而不是 `failed`。
- 创建的 Gate 包含 `retry_review`、`send_to_coder`、`abort`。
- 失败时不释放 active work item lock，不创建 dirty cleanup Gate。
- `retry_review` 保留 active Unit 和 worktree，supersede 旧 Reviewer Role Run。
- 新失败路径会把失败 Reviewer Role Run 持久化为 `failed`；历史恢复允许 supersede 与失败 Node 绑定的僵尸 `running` Role Run。
- 重复 `retry_review` 不创建第二个 Role Run。
- 两个并发 socket 同时恢复时只有一个能取得 reservation，另一个在首次业务写入前失败。
- 对 prepared、attempt reopened、retry run prepared、attempt running、gate resolved 等持久化前缀逐项重试，最终收敛到唯一 RetryReview Run 和 resolved Gate。

### 10.2 历史状态恢复

- `failed + code_review + running unit + failed review node` 输出恢复 Gate。
- `completed`、`aborted`、无 active Unit、存在新 completed Review、worktree 缺失时不输出恢复 Gate。
- 当前 `coding_attempt_0001` 形态可恢复，Work Item 1 和 Work Item 3 至 10 状态不变化。

### 10.3 WebSocket 与前端

- 合法历史恢复 Gate 的 `gate_response/retry_review` 可通过 stage guard。
- 普通 failed Attempt 的任意业务消息仍返回 `coding_message_not_allowed`。
- 点击“重试代码审查”发送正确 GateResponse，pending 期间防重复点击。
- ProtocolError 后按钮恢复可用。

## 11. 验收标准

- 当前页面不再展示无法执行的“人工继续”作为唯一恢复入口。
- `coding_attempt_0001` 显示“重试代码审查”。
- 点击一次后启动 Reviewer，且 Work Item 2 的未提交修改保持不变。
- Reviewer 成功后继续后续 Group Coding 流程，不重跑 Work Item 1。
- 重复点击不会启动多个 Reviewer。
- `cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo check --locked`、`cargo test --locked` 全部通过。
- `cd web && pnpm tsc -b`、`cd web && pnpm test` 全部通过。
