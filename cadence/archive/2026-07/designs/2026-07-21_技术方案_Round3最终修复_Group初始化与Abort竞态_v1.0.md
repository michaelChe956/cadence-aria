# Round 3 最终修复：Group 初始化与 Abort 竞态

## 目标

- Group Attempt 初始化在 Attempt 持久化后、owner bind 后、部分 Unit 物化后崩溃时，新进程能仅按首次请求的权威身份幂等恢复。
- Abort/Delete 返回后，旧 runner 已完全退出，不得继续修改 Git、Attempt、Unit、Run、handoff、timeline、event 或 lease。

## 方案比较

1. **journal-first replay + runner retirement/wait（采用）**
   - Group 先写持久化 journal，再幂等 ensure Attempt、owner、binding 和 units。
   - Abort 将 Attempt 标记为 retired，撤销 reservation，发送中止命令并等待所有已注册 runner 真正 remove。
   - 优点：延续现有 JSON Store 与 registry 模式，修改边界清晰，可用确定性 seam 验证。
2. **Attempt envelope 单文件 + generation CAS**
   - 把初始化数据嵌入 Attempt envelope，runner 每次写入携带 generation。
   - 缺点：涉及大量现有 Attempt 读写路径和每个 durable transition，超出本轮两个 finding 的最小修复范围。
3. **staging 目录完整物化后 rename publish**
   - 先在隔离目录写完 Attempt 全部数据，再一次性发布。
   - 缺点：Lifecycle owner 与 CodingAttemptStore 跨 Store，目录 rename 不能原子覆盖 lease bind，仍需 journal，复杂度更高。

## Group 初始化设计

`CodingGroupInitializationJournal` 保存首次请求的完整身份：project/issue/plan、Attempt 记录、provider snapshot、base/branch/worktree、current Work Item、plan revision、有序 Unit 记录和 phase。journal 存在 issue 范围的 Group initialization 目录，可在 Attempt 文件尚未存在时被新进程发现。

初始化持有 issue 级文件仲裁锁，步骤为：

1. 校验当前权威 plan 和 provider 身份，写入或复用 journal。
2. 获取 issue-worktree 临时 lease。
3. 幂等 ensure Attempt 和 provider config。
4. 幂等 bind owner 到 Attempt。
5. 幂等 ensure plan binding。
6. 按 journal 中的固定 ID 和顺序幂等 ensure units，并修复 active/current pointer。
7. 校验 integrity，把 phase 推进到 completed。

每个 ensure 对已存在记录要求精确身份相等；当前 plan revision、ordered bindings、provider、branch/base 或 Work Item 与 journal 不一致时，在任何新写入前 fail-closed。不兼容历史无 journal Attempt：发现“Attempt 存在、journal 缺失”时直接返回 incomplete。

## Abort/runner 设计

Registry 为每个 run 保存 completion signal，并为 Attempt 保存 retired tombstone。`abort_attempt` 在不持有 registry 同步锁的情况下：

1. 标记 retired，撤销未激活 reservation，拒绝后续 insert/reserve。
2. 复制 runner sender 和 completion handle后释放 registry 锁。
3. 发送 AbortAttempt。
4. 等待每个 runner 调用 `remove`，确认任务已退出 durable mutation 区域。

HTTP Abort/Delete 然后获取现有 Attempt mutation lease，重载最新 Attempt 再执行终态操作。`handle_abort` 对已 Aborted 状态幂等返回。owner 校验在 shared record 存在时必须精确匹配 work item 和 owner，`(None, None)` 不再视为合法 runner owner。Group completion 只允许 persisted `Running + ReviewRequest`；provider failure 在写入前重载 Attempt，要求 status 仍 active 且 stage 未漂移。

## 锁顺序

1. registry 同步锁仅用于取快照或更新内存状态，不跨 `.await`。
2. 等待 runner completion。
3. Attempt mutation lease。
4. Lifecycle issue-worktree 文件锁。
5. CodingAttemptStore 文件锁和 Git await。

Group 初始化另外持有 group named guard，再持有 issue 级 initialization 文件仲裁锁；其内按 Lifecycle 再 CodingAttemptStore 的顺序执行，不与 runner registry 锁交叉。

## 测试设计

- Group 默认关闭的 TestControls checkpoint：persist-before-bind、bind-before-binding、partial-units。每条首次请求被中断，然后用同一 workspace root 的新 Store、新 WebAppState 和新 router 重试。
- ownerless record 直接验证严格 owner preflight 零写失败。
- Running completion、CompletedRetry、provider failure 在 owner preflight 后用默认关闭的 pause seam 停住，并发 Abort；证明 Abort 在 runner remove 前不返回，返回后全部 durable/Git/lease 快照稳定。
