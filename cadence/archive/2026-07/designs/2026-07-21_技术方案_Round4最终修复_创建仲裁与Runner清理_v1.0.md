# Round 4 最终修复：创建仲裁与 Runner 必达清理

## 目标

- Single 与 Group Coding Attempt 对同一 Work Item 使用同一跨进程创建仲裁，任何交错最多产生一个 active Attempt、一份 provider config 和一个有效 Issue Shared Worktree owner。
- Group 初始化 journal 使用严格 required `worktree_lease_id`，不兼容旧 journal Schema，不从其他 temporary owner 推断身份。
- Coding runner 在正常返回、早退、panic unwind 和 task cancel 时都必达 registry remove/completion。
- WebSocket Abort 等待 runner completion 时持续释放 outbound event channel 容量，避免满 1024 队列与 runner cleanup 形成死锁。
- Group replay 与显式 Delete 使用同一 initialization arbitration，删除完成后不遗留 Attempt、journal、binding 或 Unit。

## 方案比较

### 1. 创建仲裁

采用带 identity 的 `WorkItemAttemptCreationGuard`，由 Single 与 Group handler 在 Lifecycle lease 之前获取并持有到 Attempt/provider/bind 完成。`create_attempt` 保留自动获取 guard 的兼容入口，handler 使用 guard-aware 内部入口；Group ensure 必须接收同一 guard。

未采用全 issue 创建锁，因为不同 Work Item 会被无必要串行。未把 Single 全量迁移到 Group journal，因为这会扩大 Schema 与恢复范围，且不是关闭本轮竞态所必需。

### 2. Group lease 身份

`CodingGroupInitializationJournal` 新增 required `worktree_lease_id`。首次 prepare 生成并固定 lease ID；重启 replay 必须使用该 ID。由于字段没有 serde default，旧 journal 缺字段会直接反序列化失败，不提供历史兼容。

`try_acquire_issue_worktree_lock` 返回 `acquired=false` 时：

- owner 精确等于 journal Attempt ID，且 journal phase 已达到 `WorktreeBound`，视为已绑定 replay；
- owner 等于 journal `worktree_lease_id` 时，正常取得同一 temporary lease；
- 其他 temporary lease、其他 Attempt owner 或 ownerless 记录全部 fail-closed，不创建 Attempt/provider/binding/Unit。

### 3. Runner completion

每个 spawned runner task 首行构造 `CodingRunnerRegistrationGuard`。guard Drop 同步调用 registry `remove`，因此正常返回、任意早退、panic unwind 与 future drop 都会发送 completion watch。

Registry Abort 快照包含 `run_id`、command sender 与 completion receiver。如果 command send 发现 receiver 已关闭，registry 立即 remove 该 run；receiver 已关闭意味着 runner future 已释放 command receiver，不再可能继续执行 durable mutation。

### 4. WebSocket event backpressure

Abort 分支使用 `tokio::select!` 同时等待 registry Abort future 与 `OutboundEventReceiver::recv`。等待期间收到的事件暂存在有序队列，持续释放 mpsc 容量；Abort 完成后按原顺序通过既有 `send_coding_event` 写 socket并结算 delivery ACK。

若 socket 写失败，剩余缓存事件显式标记 delivery failure，随后退出 socket；runner completion 已先收敛，不依赖 socket 写成功。

### 5. Retry 与 Delete 串行

Group Delete 在完成 runner wait、取得 Attempt mutation lease并重载 Attempt 后，获取 issue 级 Group initialization arbitration，并持有到 worktree/branch cleanup、journal 与 Attempt 目录删除全部结束。Group replay 从 prepare 到 Completed 已持有同一 arbitration，因此 replay 与 Delete 线性化。

`delete_group_initialization_for_attempt` 继续严格校验 journal Attempt ID；任何 identity 不匹配都拒绝删除，不尝试兼容或猜测。

## 锁顺序与无环证明

创建链路：

1. Group 请求可先持有进程内 named async guard。
2. Group initialization 文件仲裁。
3. Work Item creation 文件仲裁。
4. Lifecycle Issue Shared Worktree 文件锁。
5. CodingAttemptStore 文件写与 Git 操作。

Single 创建从第 3 步开始，不获取 Group arbitration。没有路径在持有 Lifecycle 文件锁时再获取 Work Item creation guard。

终止链路：

1. registry mutex 仅用于内存快照/更新，不跨 `.await`。
2. 发送 Abort，并在 WS 场景同时 drain event queue，等待 runner completion。
3. Attempt mutation lease。
4. Group initialization arbitration（仅 Group Delete）。
5. Lifecycle、Git 与 CodingAttemptStore cleanup。

创建/replay 不获取 Attempt mutation lease；Abort/Delete 不获取 Work Item creation guard；runner cleanup guard 不获取文件锁。因此不存在 `group → work-item → lifecycle` 与 `mutation → group → lifecycle` 的反向边，也不存在 registry mutex 跨 await，锁图无环。

## 测试设计

- 两个独立 WebAppState/router 共享同一 `.aria` root，确定性交错：Group 持 creation guard/lease 暂停后 Single 尝试，以及 Single 暂停后 Group 尝试。
- 每个交错断言最多一个 active Attempt、一份 provider config、至多一个 Group journal、单一 lease owner；loser 不产生 ambiguity，winner 终态可正常 Abort/Delete。
- runner task 在注册后主动 panic，timeout 内 registry count 收敛为 0；command receiver 预先关闭时 Abort timeout 内返回。
- 容量为 1 的 outbound queue 先填满，runner 第二次 send 被阻塞；WS Abort drain helper 必须释放容量、收到 Abort、完成 remove 并在 timeout 内返回。
- Group restart replay 暂停期间并发 Delete；Delete 在 arbitration 等待，replay 完成后 Delete 清除 Attempt、provider、binding、Unit、journal 与 lease。
- 保留 `test_controls_enabled()` 默认 false 与既有 `/api/test/*` 条件挂载断言，不新增 HTTP test route。
