# Coding Workspace 重复代码审查中断恢复 Journal 轮换技术方案

## 1. 背景

`2026-07-12_技术方案_CodingWorkspace代码审查中断恢复_v1.0.md` 已为 Code Review Provider 中断建立原地恢复机制：保留 Coder 代码、失败 Review 节点和 active Unit，通过 `retry_review` 创建新的 Reviewer Role Run，并使用 Attempt 目录下的 `failed-code-review-recovery.json` 保证恢复过程可重入和幂等。

当前实现隐含了“一个 Coding Attempt 最多发生一次 Code Review 中断恢复”的假设。实际流程中，同一 Attempt 可以跨多个 Work Item 执行多次 Review，也可能在一次恢复后的 Reviewer 中再次发生权限超时或 Provider 中断。

当前 `coding_attempt_0001` 已出现第二次中断：

- Attempt 处于 `blocked + code_review`；
- 当前 Work Item 的 Code Review Gate、失败 Timeline Node 和失败 Reviewer Role Run 均存在；
- Attempt 目录仍保留第一次恢复产生的 `completed` journal；
- 第二次点击“重试代码审查”时，恢复识别优先读取旧 journal，发现其身份不匹配当前 Gate 后直接判定为不可恢复；
- WebSocket 请求因此进入普通 Gate 路径，并被 `coding_failed_review_recovery_requires_reservation` 拒绝；
- 前端最终展示通用错误 `coding_gate_response_failed`。

该问题属于 Coding Workspace 后端恢复状态机缺陷，不是用户重复点击、页面刷新或前端按钮本身导致。

## 2. 目标

- 同一 Coding Attempt 可以按时间顺序完成多次 Code Review 中断恢复。
- 保留每一次已完成 recovery journal 的审计记录。
- 继续使用单一当前 journal 驱动崩溃恢复和幂等推进。
- 新的恢复不得被旧的 `completed` journal 阻断。
- 未完成 journal 仍必须排斥不同 recovery identity，避免两个恢复过程互相覆盖。
- 保留现有 Attempt、active Unit、Coder 工作区修改、失败 Review 节点和 Role Run 历史。
- 并发刷新、重复点击和两个 WebSocket 同时提交时，仍只创建一个 Retry Reviewer Role Run 和一个 Runner。
- 修复当前 `coding_attempt_0001` 数据，使用户能够从当前 Work Item 的 Code Review 阶段继续，不重跑 Coding。

## 3. 非目标

- 不把 journal 机制抽象为所有 Coding Stage 的通用事务框架。
- 不修改 Coding、Tester 或其他 Provider 中断恢复语义。
- 不删除失败 Review 节点、失败原因或历史 Role Run。
- 不重新执行当前 Work Item 的 Coder。
- 不重置、提交或清理业务共享 worktree 中的代码。
- 不新增 Playwright、浏览器或端到端测试。
- 不修改 Story Spec、Design Spec、Work Item 产物 Workspace 的审核状态机。
- 不在本次修复中扩展前端 ProtocolError 展示协议；`coding_gate_response_failed` 的错误可观测性可作为独立后续优化。

## 4. 根因

### 4.1 恢复识别被旧 journal 短路

`recoverable_failed_code_review` 当前只要发现 `failed-code-review-recovery.json`，就只验证该 journal：

- journal 未完成时，验证恢复前缀；
- journal 已完成时，验证 Retry Reviewer 是否仍处于“Runner 已启动但尚未绑定节点”的短暂状态；
- 验证失败便直接返回 `None`，不会继续检查当前新的 interrupted Review Gate。

因此，第一次恢复留下的 completed journal 会遮蔽第二次中断。

### 4.2 Store 不允许 journal identity 轮换

`prepare_failed_code_review_recovery_journal` 发现已有 journal 时，只允许 gate、failed node、stale role run 三个 expected ID 完全相同。不同 identity 一律返回 `coding_recovery_state_changed`。

该限制对未完成 journal 是正确的并发保护，但错误地应用到了已经完成且不再代表当前恢复事务的 journal。

### 4.3 普通 Gate 路径按设计拒绝 interrupted Review

Code Review Provider 中断必须先取得 recovery reservation，再修改 Attempt、Role Run 和 Gate。普通 Gate 路径拒绝 `retry_review` 是正确防线，不应删除或放宽。

永久修复应让第二次中断正确进入专用 recovery admission，而不是绕开 reservation。

## 5. 采用方案

采用“单一当前 journal + completed 历史归档”的惰性轮换方案。

Attempt 目录结构调整为：

```text
coding_attempt_0001/
├── failed-code-review-recovery.json
└── failed-code-review-recoveries/
    └── completed/
        ├── coding_blocked_gate_0001.json
        └── coding_blocked_gate_0007.json
```

其中：

- `failed-code-review-recovery.json` 始终表示当前正在推进或刚启动 Runner、尚需完成交接验证的恢复事务；
- `failed-code-review-recoveries/completed/<gate_id>.json` 保存已经被后续恢复轮换出去的历史 journal；
- 使用 `gate_id` 作为归档文件名，因为 Gate ID 在 Attempt 内唯一、已通过相对路径 ID 校验，并且比包含冒号的 `recovery_key` 更适合跨平台文件名；
- completed journal 不在完成瞬间立即归档，而是在出现下一次不同 identity 的恢复时惰性归档。

惰性归档保留了现有“completed journal 在 Retry Reviewer 绑定 Timeline Node 前继续参与恢复判定”的语义，避免新增额外清理时机和状态。

## 6. 恢复识别规则

`recoverable_failed_code_review` 保持只读，不在 SessionState 重建或 WebSocket admission 阶段移动文件。

读取当前 journal 后按以下顺序判断：

1. journal 未完成：
   - 恢复前缀有效时，返回该 journal 对应的 recovery identity；
   - 恢复前缀无效时，返回不可恢复，不允许绕过未完成事务去选择其他 Gate。
2. journal 已完成，且 Retry Reviewer 仍处于 Runner 已启动但尚未绑定 Timeline Node 的交接窗口：
   - 返回旧 journal 对应的 recovery identity；
   - 重复请求继续收敛到同一 Runner，不创建新 Run。
3. journal 已完成，且交接窗口已经结束：
   - 将其视为历史记录；
   - 不返回旧 identity；
   - 继续使用现有严格条件检查当前 Attempt 是否存在唯一的 interrupted Review Gate、失败 Node 和失败 Reviewer Role Run。
4. 当前形态满足恢复条件时，返回新的 recovery identity，供 WebSocket admission 取得 Attempt 级 reservation。

这一区分保证 completed 历史不会遮蔽新恢复，同时未完成恢复仍具有排他性。

## 7. Journal 轮换规则

在取得 `CodingRunRegistry` Attempt 级 recovery reservation 后，Engine 使用 Store 的 prepare-or-rotate 语义创建当前 journal。

### 7.1 相同 identity

若当前 journal 的 gate、failed node、stale role run 与请求完全相同：

- 返回现有 journal；
- 不创建新文件；
- 继续现有 phase 幂等推进。

### 7.2 不同 identity，当前 journal 未完成

- 返回恢复状态变化错误；
- 不移动或覆盖文件；
- 不修改 Attempt、Gate、Role Run、Timeline Node 或 worktree。

### 7.3 不同 identity，当前 journal 已完成

依次执行：

1. 校验旧 journal 的 Attempt 身份、completed phase、`completed_at` 和 expected ID 字段完整；
2. 计算归档路径 `failed-code-review-recoveries/completed/<old_gate_id>.json`；
3. 若归档文件不存在，在同一 Attempt 文件系统内将当前 journal 原子重命名到归档路径；
4. 若归档文件已存在，读取并确认它与当前旧 journal 是同一 recovery identity；一致时删除重复的当前旧文件，不一致时拒绝继续；
5. 使用现有原子 JSON 写入方式创建新 identity 的 `failed-code-review-recovery.json`；
6. 从 `Prepared` phase 继续现有恢复流程。

轮换不得直接覆盖已有但内容不同的历史归档。

## 8. 崩溃恢复与幂等性

归档与新 journal 写入是两个文件操作，无法组成跨文件单一原子事务，因此必须显式支持中间状态。

### 8.1 归档前崩溃

旧 completed journal 仍是当前文件。下次请求重新识别当前 interrupted Gate，取得 reservation 后再次执行轮换。

### 8.2 归档后、新 journal 写入前崩溃

当前 journal 不存在，但当前 interrupted Gate、失败 Node 和失败 Role Run 仍存在。下次请求通过无 journal 的正常识别路径重建新 journal。

### 8.3 新 journal 写入后崩溃

现有 recovery phase 幂等逻辑继续推进，不重复创建 Retry Reviewer Role Run。

### 8.4 历史归档已存在时重试

归档内容与旧 journal identity 相同则视为已完成的前缀；内容不同则返回状态变化错误，防止静默覆盖审计记录。

## 9. Reviewer 与 Attempt 状态语义

第二次及后续恢复继续复用原有业务语义：

1. 保留失败 Code Review Timeline Node 及其错误原因；
2. 该 Node 对应的 Reviewer Role Run 已是 `failed` 时保留失败状态并记录新的 `superseded_by_run_id`；只有历史僵尸 `running` Run 才转为 `superseded`；
3. 创建 trigger 为 `retry_review` 的新 Reviewer Role Run；
4. `supersedes_run_id` 只指向本次 journal 精确记录的 stale Role Run，不按“最新 Run”模糊选择；
5. Attempt 从 `blocked + code_review` 恢复为 `running + code_review`；
6. 解析并关闭本次 interrupted Review Gate；
7. 启动 Reviewer Runner；
8. 不创建 Coder Run，不重跑 Coding，不改变 active Unit；
9. Review 成功后继续现有 Work Item 或 Group 流程。

## 10. WebSocket 与并发

- `failed_code_review_recovery_request` 在旧 completed journal 已结束交接后，应识别当前新 Gate；
- 专用 recovery admission 必须在任何持久化修改前取得 Attempt 级 reservation；
- reservation 存在时，其他 socket 的业务消息继续被拒绝；
- 两个 socket 同时点击同一 Gate，只有一个能够进入 prepare-or-rotate 和 Runner 启动；
- 普通 Gate 处理中的 `coding_failed_review_recovery_requires_reservation` 防线保持不变；
- 旧 Gate 的 `retry_review` 请求不得因历史归档存在而重新变得可接受；
- 页面刷新只重建 SessionState，不触发 journal 轮换或其他持久化写入。

## 11. 当前数据修复

平台代码验证通过后，对当前 `coding_attempt_0001` 执行与新 Store 语义一致的数据修复：

1. 读取并备份 Attempt 目录中的旧 completed journal；
2. 校验它对应旧 Gate、旧失败 Node 和旧 stale Role Run；
3. 将其移动到 `failed-code-review-recoveries/completed/<old_gate_id>.json`；
4. 保留当前 Attempt 的 `blocked + code_review` 状态；
5. 保留当前 active Unit、当前 interrupted Review Gate、失败 Timeline Node 和失败 Reviewer Role Run；
6. 不预创建新的 Reviewer Run，不提前关闭 Gate；
7. 不修改业务 worktree 中的 Coder 文件；
8. 用户刷新页面并点击“重试代码审查”后，由平台正常创建新 journal 和 Retry Reviewer Run。

数据修复前后的 Attempt、Unit、Gate、Node、Role Run 和业务 worktree 状态必须分别记录并比对。备份保留规则沿用现有数据恢复操作规范。

## 12. 测试设计

本修复只使用 Rust 单元测试和现有 Coding WebSocket/Engine 测试，不使用 Playwright，不增加或运行 E2E 测试。

### 12.1 Store 单元测试

- 相同 identity 的 prepare 保持幂等；
- 不同 identity 遇到未完成 journal 时拒绝且无写入；
- 不同 identity 遇到 completed journal 时归档旧 journal并创建新 journal；
- 已存在相同历史归档时重复轮换可收敛；
- 已存在不同内容的同名历史归档时拒绝覆盖；
- 模拟归档完成但新 journal 未写入的状态，可以重新创建当前 journal；
- 归档路径使用经过验证的 Gate ID，拒绝路径逃逸。

### 12.2 Engine 与恢复识别测试

- 第一次 interrupted Review 恢复完成并形成 completed journal；
- Retry Reviewer 已绑定节点后，completed journal 不再遮蔽新的 interrupted Gate；
- 第二次恢复创建新的 Retry Reviewer Run；
- 第二次 stale Role Run 被准确 supersede，第一次恢复的 Role Run 历史不被修改；
- Attempt、active Unit 和执行指纹保持不变；
- 当前 Gate 被 resolved，旧 Gate 请求被拒绝；
- 未完成旧 journal 仍阻止不同 identity 的恢复。

### 12.3 WebSocket 并发测试

- 刷新重连后，SessionState 展示当前第二次中断的恢复 Gate；
- 两个 socket 并发提交 `retry_review`，只产生一个新 journal、Role Run 和 Runner；
- 重复点击同一 Gate 保持幂等或稳定拒绝，不产生重复运行；
- reservation 获取失败时不归档 journal、不修改业务状态；
- 普通 Gate 路径仍拒绝绕过 reservation 的 interrupted Review 重试。

### 12.4 验证命令

定向快反馈使用：

```bash
cargo test --locked --lib failed_review_recovery
```

最终验证使用仓库标准命令：

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
```

若实现不修改前端，则不要求额外前端测试；若实现过程中发现必须调整共享前端状态处理，需另行说明范围并运行 `cd web && pnpm tsc -b` 与 `cd web && pnpm test`，但仍不得引入 E2E。

## 13. 影响范围

主要修改范围：

- `src/product/coding_workspace_engine/failed_review_recovery.rs`
- `src/product/coding_attempt_store/recovery.rs`
- `src/web/coding_ws_handler/socket/admission.rs` 的既有行为回归验证
- `src/web/coding_ws_handler/tests/failed_review_recovery/**`

不应修改：

- Coder 业务 worktree 文件；
- Work Item 内容和 Work Item Draft；
- Reviewer Prompt；
- E2E、Playwright 或浏览器配置；
- Story Spec、Design Spec、Work Item 产物 Workspace Engine。

本问题位于 `coding_workspace_engine`，与 `workspace_engine` 中 Story、Design、Work Item 产物审核链路代码路径不同，因此产物 Workspace 三模块不受影响。本次回归测试聚焦 Coding Attempt、Coding Unit、Gate、Role Run 和 WebSocket reservation。

## 14. 验收标准

- 同一 Attempt 至少连续完成两次 interrupted Review 恢复；
- 第二次恢复不再返回 `coding_gate_response_failed`；
- 每次恢复只产生一个 Retry Reviewer Role Run 和一个 Runner；
- 未完成 journal 的并发保护没有被放宽；
- 旧 completed journal 可在历史目录读取，且不会被覆盖或删除；
- 失败 Review 节点、错误原因和 Role Run supersede 链完整；
- 当前 Coder 修改、active Unit 和 Work Item 执行顺序不变；
- 当前 `coding_attempt_0001` 修复后可从 Code Review 继续；
- 不新增、不运行 Playwright 或 E2E；
- Rust 定向与全量验证全部通过。

## 15. 实施顺序

1. 先以“同一 Attempt 第二次 Review 中断”为场景添加失败回归测试；
2. 实现 completed journal 的历史路径、只读识别分支和 prepare-or-rotate 语义；
3. 补充崩溃窗口、历史冲突和 WebSocket 并发测试；
4. 完成 Rust 定向与全量验证；
5. 备份并修复当前 Attempt 数据；
6. 重建 SessionState，确认页面可显示并提交当前 Review 重试；
7. 推送 `feat-b-0709` 分支。
