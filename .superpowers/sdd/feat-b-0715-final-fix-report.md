# feat-b-0715 最终修复报告

## 交付摘要

- 分支：`feat-b-0715`。
- 提交策略：单个原子 fix commit，不 amend，不 push。
- 范围：Coding Attempt 创建 TOCTOU、前端 Plan Repair timeline 乱序、Child Workspace 确认 Amendment 后未进入真实 Coding WS 发布/应用/runner 恢复链路，以及相关失败重试。
- 明确未运行：E2E、Playwright、browser、真实网络 Codex/Claude CLI。

## 修复内容

### 1. Coding Attempt 创建 TOCTOU

- 单 Work Item Attempt 创建改为在跨 Store、跨进程共享的文件锁内完成 active-attempt 检查、attempt number 分配、Attempt 写入和 provider config 写入。
- 并发失败方返回结构化 `ProductStoreError::Conflict`，Web API 保持稳定的 `active_coding_attempt_exists` 错误码。
- provider config 写入失败时回滚已写入的 Attempt，避免留下半成品记录。
- 新增两个独立 `CodingAttemptStore` 实例并发创建的确定性回归测试，证明最多只有一个成功记录与一份 provider config。

### 2. Plan Repair timeline 乱序保护

- 前端聚合状态新增 authoritative snapshot `timelineWatermark`。
- 拒绝水位之前或等于水位的未知迟到 create。
- 已存在节点拒绝更旧覆盖；terminal 节点拒绝回退到 non-terminal 或其他状态。
- snapshot 合并保留水位之后的合法 live 节点，同时避免旧 snapshot 重新引入已淘汰节点。
- Coding Plan Repair aggregator 明确只接受 `plan_repair` relation；Story/Design amendment child timeline 保持隔离。

### 3. Child Confirm 到真实 Coding WS 的生产链路

- `WorkspaceEngine::confirm_and_publish_plan_amendment` 统一完成权威 Child snapshot 刷新、确认、candidate package 重建、review attestation 校验、Amendment 发布与 Child Published snapshot 持久化。
- `PlanRepairEngine::load_prepared_amendment` 从 durable candidate package 和权威 revision artifacts 重建 `PreparedPlanAmendment`，不依赖进程内临时对象。
- 新增 per-Attempt Coding socket registry：支持同 Attempt 多 socket token，选择最新未关闭 sender，并在 socket 退出时按 token 注销。
- Child handler 发布成功后取得 Attempt 级锁与 runner reservation，通过 Amendment 专用 reserved spawn 启动既有 Coding runner。
- runner 首步继续复用既有 `recover_plan_amendment`、跨进程 arbitration、application journal、stable event ID、socket-write ACK、ApplyFailed 与 retry 逻辑；Child handler 不复制 apply 实现。
- 重复 Confirm 通过 Attempt guard、reservation 与 runner registry 幂等收敛，不产生双 runner。

### 4. 部分提交与失败重试

- 首次 runner activation 因 Coding WS 尚未连接而失败时，Amendment 发布保持 durable；连接 Coding WS 后重复 Confirm 可恢复为单 runner、单 application journal。
- publication journal 已存在时，重复 Confirm 不再重新执行 awaiting-package base 校验；恢复会复用 journal 中首次持久化的 confirmation、`confirmed_at` 与 review attestation，继续完成同一 publication。
- 修复了 `JournalPlanPublished` 之后 active revision 已推进、request 尚未标记 Published 时重复 Confirm 报 `AmendmentConflict` 的真实部分提交缺陷。
- socket 已写出事件但 delivery mark 失败时，重复 Confirm 使用同一稳定 `event_id` 重发并收敛到 `Delivered`；application journal 与 runner 数量保持唯一。

## 关键 RED → GREEN

| 场景 | RED | GREEN |
| --- | --- | --- |
| Attempt TOCTOU | 两个 Store 可同时通过 active 检查 | 共享文件锁串行化，1 成功、1 typed conflict |
| Timeline 乱序 | 迟到 active/paused 可覆盖 completed/failed | watermark、terminal 单调与时间比较拒绝回退 |
| 真实 Child Confirm | Child WS 确认成功，但 Coding WS 3 秒内收不到 `plan_amendment_updated` | 真实双 WS 收到事件，binding/delivery/Child Completed/runner 均收敛 |
| Publication 部分提交 | `JournalPlanPublished` 失败后重复 Confirm 报 base `AmendmentConflict` | 识别既有 journal，复用首次 confirmation/attestation 后完成 Published |
| Runner activation 重试 | 未连接 Coding WS 时 activation 失败 | 后续连接并重复 Confirm，单 journal、单 runner |
| Delivery mark 重试 | socket write 成功后 mark 失败，Attempt 为 ApplyFailed、marker 为 Pending | 重复 Confirm 使用同一 event ID，恢复 Running/Delivered |

## 验证结果

### 定向验证

- `cargo test --locked --lib single_work_item_attempt_creation_is_serialized_across_store_instances -- --nocapture`：1 passed。
- `pnpm exec vitest --run src/state/plan-repair-session.test.ts src/hooks/useWorkspaceWs.plan-repair.test.tsx`：2 files、34 tests passed。
- `cargo test --locked --test it_web child_confirmation_publishes_applies_and_restarts_through_real_websockets -- --nocapture`：1 passed。
- `cargo test --locked --test it_web child_confirmation_retries_activation_after_coding_socket_connects -- --nocapture`：1 passed。
- `cargo test --locked --lib plan_repair_confirm_and_publish_recovers_partial_publication_with_same_attestation -- --nocapture`：1 passed。
- `cargo test --locked --lib repeated_confirmation_recovers_delivery_mark_failure_without_duplicate_application -- --nocapture`：1 passed。

### 最终门禁

- `cargo fmt --check`：PASS。
- `cargo check --locked`：PASS。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：PASS，0 warnings。
- `cargo test --locked`：PASS，exit 0；lib、integration 与 doc tests 均无失败。
- `cd web && pnpm tsc -b`：PASS。
- `cd web && pnpm test`：89 files、721 tests passed。
- `cd web && pnpm build`：PASS；仅保留 Vite 的既有大 chunk 提示。
- `git diff --check`：PASS。
- 35 个代码改动文件均不超过 800 行；最大文件为 `src/web/workspace_ws_handler/decisions/inbound.rs`，795 行。

## Minor：`src/web/test_controls` 生产边界决策

- 本轮未对 `PlanRepairFixtureRuntime` 做 feature-gate 或迁移。
- 原因：现有大量 integration tests 通过 `cadence_aria::web::test_controls` 使用该 fixture；integration test 编译库时不带 `cfg(test)`，当前 Cargo 也没有能由标准 `cargo test --locked` 自动启用的 self-feature。直接 gate 会破坏标准测试命令，迁移到 `tests/support` 则会形成大范围、高风险的测试基础设施重构。
- 新增能力只是 fixture 方法，不是 HTTP route。现有 `/api/test/*` 仍只在 `ARIA_E2E_TEST_CONTROLS=1` 时注册，默认生产 router 不暴露这些入口。
- 该边界作为已知 Minor 保留，后续如要彻底隔离，应单独设计 test-support crate/feature 与统一的 integration test 启动方式。

## 自审结论与边界

- Attempt 创建、Amendment publication/application/delivery 的权威身份与并发边界均 fail-closed。
- 重复 Confirm 可以恢复 publication、activation、apply/delivery 失败，不宣称客户端 exactly-once；delivery 语义仍是 stable event ID 的 socket-write-confirmed at-least-once。
- Story/Design amendment 未接入 Coding Plan Repair aggregator。
- 未新增生产测试路由，未运行被禁止的 E2E/浏览器/真实 provider 网络流程。
- worktree 保留；完成后仅创建单个原子提交，不 push。
