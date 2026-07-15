# Coding Provider 无响应恢复技术方案

## 1. 背景

`coding_attempt_0001` 在 Work Item Draft 4 的 Coder 返修阶段续接 Codex 会话时，Codex 在 60 秒内没有产生 Provider 进展事件，底层进程被终止并返回：

```text
Codex resume stalled before provider progress
```

当前 Coding Workspace 将该 Provider 故障直接转为终态 `failed`，随后失败清理流程检查 Issue 共享 worktree。由于 Coder 已产生未提交修改，清理流程又创建 `shared_worktree_dirty_manual_gate`，最终 Runner 把次级清理错误包装成 `coding_start_failed`。结果是：

- 原始 Provider 无响应原因被次级错误遮蔽；
- Attempt 进入前端不允许继续操作的 `failed` 状态；
- worktree 中仍保留有价值的返修修改；
- 创建出的人工 Gate 在 `failed` 状态下无法正常响应。

## 2. 目标

- Codex continuation 无进展时自动切换为新会话重试一次。
- Fresh Retry 必须使用完整 Coding Prompt，不得复用续接场景的增量 Prompt。
- 自动重试仍失败时保留 worktree 修改，并进入可操作的恢复 Gate。
- Provider 故障不得被 `shared_worktree_dirty_manual_gate` 或通用 `coding_start_failed` 遮蔽。
- 用户可从恢复 Gate 重新启动 Coder 或终止 Attempt。
- 不改变正常 Provider 成功路径、Code Reviewer 恢复路径和共享 worktree 完成门禁。

## 3. 非目标

- 不取消 Provider 无进展超时。
- 不无限自动重试。
- 不自动提交或丢弃 Coder 留下的 worktree 修改。
- 不改变 Claude Code、Tester、Internal Reviewer 的一般超时策略。
- 不重构全部 Coding Workspace 状态机。

## 4. 方案选择

### 4.1 采用：自动 Fresh Retry 加可恢复 Gate

第一次 Codex resume stall 自动使用新会话重试；第二次失败进入人工恢复 Gate。

优点：正常的短暂 continuation 故障可自动恢复，重复故障仍由用户控制，且不会丢失修改。

### 4.2 未采用：第一次失败立即进入 Gate

实现较简单，但会把可自动恢复的偶发 continuation 故障暴露给用户，频繁打断 Coding 流程。

### 4.3 未采用：延长或取消超时

不能修复已失效的 continuation，还会长期占用 Provider 进程与 Runner 资源。

## 5. 状态流转设计

### 5.1 自动 Fresh Retry

仅当以下条件同时成立时自动重试：

- 当前角色是 `Coder`；
- Provider 是 Codex；
- 本轮输入携带 `resume_provider_session_id`；
- Provider 失败消息包含稳定标记 `Codex resume stalled before provider progress`；
- 当前 Coding 节点尚未执行 Fresh Retry。

满足条件后：

1. 记录 `codex_resume_stall_fresh_retry` Role Run 事件。
2. 移除 Attempt 中 Coder 对应的 Provider conversation 引用。
3. 构造 `resume_provider_session_id = None` 的新输入。
4. 使用完整 Coding Prompt 再次启动 Codex。
5. 沿用当前 Coding timeline node 和 role run，避免把一次逻辑返修拆成两个用户可见任务。

自动 Fresh Retry 最多执行一次。

### 5.2 Fresh Retry 成功

- 正常保存新 Provider session ID。
- 完成当前 Coding timeline node 和 role run。
- 继续进入 Code Review。
- Attempt 保持正常运行状态。

### 5.3 Fresh Retry 仍失败

- 当前 Coding timeline node 标记为 `failed`，摘要保留原始 Provider 错误。
- 当前 Coder role run 标记为 `failed`，reason code 为稳定值 `coder_provider_interrupted`。
- Attempt 标记为 `blocked`，不得标记为 `failed`。
- 创建 `coder_provider_interrupted` Blocked Gate。
- Gate 提供：
  - `retry_coding`：重新启动 Coder；
  - `abort`：终止 Attempt。
- 不调用失败终态的共享 worktree 锁释放流程，因此不会创建 `shared_worktree_dirty_manual_gate`。

### 5.4 用户重新启动 Coder

处理 `retry_coding` 时：

1. 清除 Coder continuation 引用。
2. 将 Attempt 从 `blocked` 恢复为 `running`。
3. 将 stage 恢复为 `coding`。
4. 解析当前 Gate。
5. 重新启动 Coding Runner，并使用完整 Prompt 创建新 Provider 会话。

worktree 中已有修改作为当前 Draft 的工作现场保留，供新 Coder 会话继续处理。

## 6. 组件改动

### 6.1 Coding Provider Stream

Coding Coder 调用需要同时提供：

- 首次 continuation 输入：增量 Prompt 加 `resume_provider_session_id`；
- 可选 Fresh Retry 输入：完整 Prompt 且无 `resume_provider_session_id`。

Provider stream 在把失败写入终态前识别 resume stall，并在同一逻辑运行内切换 Fresh Retry。其他错误仍交给现有失败处理。

### 6.2 Provider conversation 管理

新增按 Coding 角色清除 conversation 的窄接口，只删除目标角色和 Provider 的引用，不影响 Code Reviewer、Tester 或其他 Provider 会话。

### 6.3 Coding Gate 契约

新增：

- `CodingGateActionType::RetryCoding`；
- action ID `retry_coding`；
- Role Run reason code `coder_provider_interrupted`。

前端 GatePanel 继续使用通用 action 渲染，不要求用户填写人工上下文。

### 6.4 Runner 错误传播

Runner 捕获执行错误后重新读取 Attempt：

- 若 Attempt 已进入 `blocked` 或 `waiting_for_human`，发送最新 Session State，不发送 `coding_start_failed`；
- 若 Attempt 仍处于不可恢复状态，才发送原有 Protocol Error。

这样恢复 Gate 承载用户可见的故障信息，Protocol Error 只表示真正的不可恢复失败。

### 6.5 共享 worktree 门禁

`shared_worktree_dirty_manual_gate` 继续用于完成、终止、删除或释放共享锁前的人工清理保护。Coding Provider 中断不释放共享锁，因此不得进入该门禁。

## 7. 错误与审计

- 自动 Fresh Retry 必须记录原 continuation ID、失败原因和 Fresh Retry 已触发标记，但不得额外记录敏感 Prompt 内容。
- Gate description 保留 Provider 原始错误，便于诊断。
- Fresh Retry 失败后不得用次级 worktree 状态覆盖原错误。
- 用户点击 `retry_coding` 后，新 role run 和 timeline node 按现有顺序生成，历史失败节点保留。

## 8. 测试设计

### 8.1 后端单元测试

- Codex continuation stall 后自动以无 resume ID 的完整 Prompt 重试。
- Fresh Retry 成功时 Attempt 不进入 `failed`，且不生成 Dirty Manual Gate。
- Fresh Retry 再失败时 Attempt 为 `blocked`，生成 `coder_provider_interrupted` Gate，并保留脏 worktree。
- 非 Codex、无 continuation 或非稳定 stall 标记时不触发自动 Fresh Retry。
- `retry_coding` 清除 Coder conversation，并恢复 `running / coding`。
- Coder conversation 清理不影响 Code Reviewer conversation。

### 8.2 WebSocket 与 Runner 测试

- `retry_coding` GateResponse 被当前状态允许。
- GateResponse 后重新启动 Runner。
- Attempt 已进入可恢复 Blocked 状态时不发送 `coding_start_failed`。
- 真正不可恢复错误仍发送 Protocol Error。

### 8.3 前端测试

- `retry_coding` action 可渲染为“重新启动 Coder”。
- 点击后发送正确的 GateResponse，且不要求额外上下文。

### 8.4 回归测试

- Code Reviewer 中断恢复行为保持不变。
- Work Item Group 单元完成与共享 worktree 锁转移保持不变。
- 正常 Coder continuation 仍只启动一次 Provider。

## 9. 现场恢复

代码修复与验证完成后，对 `coding_attempt_0001` 执行以下回退：

- `aria/issues/issue_0001` 恢复到 Draft 3 完成提交 `58ec0db`，清理 Draft 4 未提交修改。
- Attempt 恢复为 `running / prepare_context`。
- `current_work_item_id` 指向 Work Item Draft 4。
- `coding_unit_0004` 保持系统定义的预启动 `running` 状态。
- 时间线最后节点为 Draft 3 `code review 通过`。
- 删除 Draft 4 的 role runs、reviews、gates、chat entries、raw outputs 和 context records。
- 清空 Provider conversations，保证 Draft 4 使用新会话。
- 重启开发服务，仅执行健康与只读状态验证，等待用户手动点击“开始 Coding”。

## 10. 验收标准

- stale Codex continuation 能自动 Fresh Retry 一次。
- Provider 重复无响应后仍可通过 Gate 继续，不进入不可操作的终态。
- 页面不再显示由 Dirty Manual Gate 包装出的误导性 `coding_start_failed`。
- worktree 修改在恢复流程中不丢失、不自动提交。
- 回归测试、格式化、Clippy、Cargo Check 和相关前端测试通过。
- `coding_attempt_0001` 最终处于 Draft 4 尚未启动的可操作状态。
