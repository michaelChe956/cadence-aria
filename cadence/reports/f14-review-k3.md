# F14Fix k3 审查报告（commit 4d097c1d）

- 审查人：F14K3（k3 重点：诊断修正采信核验 + 修复面 + 循环打断 + part_02 重钉 + 越界）
- 结论：**PASS**（1 个 P3 观察项，无阻塞缺陷）
- 方法：逐 hunk 读 diff + 读全 5 文件与全部跨界消费点 + 磁盘取证复核时间线 + 2 个 scoped 测试实跑

## 1. 诊断修正采信核验（k3 重点）——全部成立

| 报告主张 | 核验结果 |
|---|---|
| resume 链没断：消费点 socket.rs:557-572 | ✅ socket.rs:557 `should_resume_runner_after_gate_response(&action_id, &current_attempt)`、:563 `spawn_coding_runner(...)`，且以 `updated.status == Running` 为门 |
| 该链自 f52c454e（2026-07-15）即存在 | ✅ `git show f52c454e` 含 14 处 `should_resume_runner_after_gate_response`；远早于部署二进制（2026-09-18 构建） |
| gate 0006 于放行后 185ms 创建 | ✅ 磁盘 gate 0006 `created_at=22:15:24.273` − 报告取证放行时刻 22:15:24.088 = 185ms，精确成立 |
| 22:21 重连 → gate 0007 同形态死亡 | ✅ gate 0007 `created_at=22:21:41.308`，5s 过期，无对应 role run |
| 22:28 取证探针 0008 自愈走通 | ✅ gate 0008 `created_at=22:28:52.520`；`coding_role_run_0006` `started_at=22:28:57.593`（gate 过期 22:28:57.520 + 92ms，与报告精确吻合），status=completed；timeline coding_node_0006/0007 completed |
| stage gate 全库唯一创建点 = gates.rs:71 | ✅ `coding_store.create_stage_gate` 全库唯一调用在 `await_stage_gate`（gates.rs:71），即 gate 创建即证明 runner 已拉起 |
| 死亡窗口 runner.rs:328-345（gate 过期→provider_for→execute 首行） | ✅ 代码结构吻合：`await_stage_gate` Ok → `provider_for(state, ..., "coding author provider")?` → `execute_coding_with_commands_outcome(...)?`，任一 `?` 抛错时 attempt 停留 Running |

**额外佐证**：commit 后 8 分钟（22:57:33.477，旧二进制仍在运行）该 attempt 再次被放行 → gate 0009 于 +249ms 创建 → 5s 过期 → 无 role_run_0007 → 僵尸再形成。与报告「修复须重建部署后生效」完全一致，属诊断的再次实证而非矛盾。

## 2. 修复面核验——正确

- **触发条件**：错误分支内二次读最新 attempt，仅 `status == Running` 时转人工恢复；Blocked/WaitingForHuman 走原快照分支（行为不变）；Aborted/取消由外层既有守卫排除。terminal/已人工恢复态不触发（后者本就幂等）。
- **store API 复用**：`transition_to_awaiting_manual_recovery`（admission.rs:180-215）在 attempt 级排他文件锁内重读+校验；幂等（已 AwaitingManualRecovery → Ok）；非活跃态拒绝；Running 时**同锁**清 `admission_ticket_consumed_at`；持久化 `manual_recovery_reason` + version+1。与 resumption B 兜底（resumption.rs:158-171）同一 API 同一语义。✅
- **fail-visible**：transition 失败仅 `warn` 不吞错；快照发送失败仅 `warn`；原 `CodingProtocolError`（code=coding_start_failed 等）无条件保留。可见性 ≥ B 兜底（后者仅 protocol error，前者快照+error）。✅
- **循环打断（跨界消费点全查）**：
  - `resumed_attempt_needs_runner`（resumption.rs:105-115）要求 `status == Running` → AwaitingManualRecovery 返回 NotNeeded，attach 零重启；✅
  - `is_coding_ws_message_allowed`（socket.rs:897-900）：AwaitingManualRecovery 仅放行 AbortAttempt → gate_response{retry_coding} 无法再触发 spawn；✅
  - `blocked_gate_is_actionable_for_attempt`（state.rs:412-417）：人工恢复态不展示可行动 gate；✅
  - dto.rs:772 HTTP 层映射既有；group.rs:72-78 编排层消费 `manual_recovery_reason` → `GroupUnitFailureOutcome::AwaitingManualRecovery` 既有分支。无任何静默丢弃。✅

## 3. 测试面核验

- **runner_recovery.rs 红测**：admitted running/coding + 空 registry（coder 指向未注册 provider）复刻「gate 过期后死于 provider 启动前」形态；断言状态/reason/admission 清空/`NotNeeded`/注册表零重启/protocol error 可见——断言面完整。**实跑通过**（`cargo test --lib runner_recovery` → 1 passed）。修前红可由代码逻辑直接推出（修前错误分支不改状态 → left: Running）。
- **part_02 重钉非放松**：原断言 `assert_eq!(status, Running)` 即时检查钉住的正是僵尸行为；新断言轮询 settle 后钉 `AwaitingManualRecovery` + reason + stage 不变（WorktreePrepare）——严格更强。fixture 前提成立：`app_with_attempt`（part_04.rs:370）= 简单 git repo + `base_branch: "main"` + 无 provider 注册 → WorktreePrepare 确定性失败。**实跑通过**（`cargo test --test it_web coding_ws_start_coding_pushes_engine_stage_and_timeline_events` → 1 passed）。
- part_02 其余 3 个 hunk 为 cargo fmt 格式化，无语义变化。

## 4. 越界核验

- commit 恰好 5 文件（stat 确认），与报告文件清单一致。✅
- 主仓误编辑还原声明成立：主仓 `src/web/coding_ws_handler/runner/task.rs` 无 F-14 标记（`coding_runner_failed_while_running` 0 命中）；主仓工作树修改清单中不含 F-14 的任何文件。✅

## 5. Findings

| # | 级别 | 内容 |
|---|---|---|
| 1 | P3 | task.rs:198-217：transition 成功后重读 attempt 失败时静默跳过人工恢复快照（无 warn），与同分支内 emit 失败路径的 warn 不对称。影响仅限可观测性（protocol error 仍发出、状态落盘正确），罕见 I/O 失败路径。建议补 `else` warn。 |

无 P0-P2 缺陷。观察面（attach 建 gate 路径、gate_response→spawn 链）确认零改动。
