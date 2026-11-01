# Task 6.1（C1）driver 与 observer authoritative 恢复投影 — 红/绿证据报告

- 日期：2026-10-11
- Worker：ImplAnchorC61（glm53-task）
- Plan：`cadence/plans/2026-10-11_实施计划_规划阶段会话锚点固定与漂移恢复闭环_v1.1.md` Task 6.1
- Change：`openspec/changes/fix-planning-session-anchor-recovery` 工作包 6.1
- cwd：`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/web`

## Step 1–2 红（先写失败行为测试，过滤命中且断言真实失败）

命令：

```bash
npm test -- src/state/workspace-ws-store.snapshot.test.ts src/state/workspace-cockpit-projection.test.ts src/state/workspace-observer-store.test.ts -t anchor_t19
```

输出（红，新增 4 用例命中、全部 FAIL 于新行为断言，非 0 tests、非绕过）：

```
 FAIL  src/state/workspace-ws-store.snapshot.test.ts > anchor_t19 driver/observer authoritative lc resume recovery > anchor_t19_driver_observer_recovery_projection_and_resolved_clear
AssertionError: expected undefined to be "rec-1"
 ❯ src/state/workspace-ws-store.snapshot.test.ts:918:50
    expect(driver.lcResumeRecovery?.recovery_id).toBe("rec-1");

 FAIL  src/state/workspace-ws-store.snapshot.test.ts > anchor_t19 ... malformed
AssertionError: expected undefined to match object { recovery_id: 'rec-1', …(1) }

 Test Files  3 failed (3)
      Tests  4 failed | 100 skipped (104)
```

（另 2 个 FAIL 同因：observer `lcResumeRecovery` 为 undefined、inbox 无 `lc_resume_recovery` 卡。）

## Step 3–4 绿（最小实现 + 同文件旧用例回归）

同一命令输出（绿）：

```
 ✓ src/state/workspace-observer-store.test.ts (21 tests | 20 skipped)
 ✓ src/state/workspace-cockpit-projection.test.ts (63 tests | 62 skipped)
 ✓ src/state/workspace-ws-store.snapshot.test.ts (20 tests | 18 skipped)

 Test Files  3 passed (3)
      Tests  4 passed | 100 skipped (104)
```

三文件全量（范围化回归组 6.1 行，旧用例全保留）：

```bash
npm test -- src/state/workspace-ws-store.snapshot.test.ts src/state/workspace-cockpit-projection.test.ts src/state/workspace-observer-store.test.ts
# Test Files  3 passed (3)   Tests  104 passed (104)
```

全量前端单元回归（额外 sanity，超出 plan 范围仅留证）：

```bash
npm test
# Test Files  212 passed (212)   Tests  2076 passed (2076)
```

## 新用例与断言映射（anchor_t19，fixture 值与后端 anchor_t03 wire 逐字段一致：rec-1/node-1/resume_fingerprint_mismatch/codex/legacy_draft/op-1）

- `workspace-ws-store.snapshot.test.ts`：driver 全量首帧落库（`lcResumeRecovery.recovery_id === "rec-1"`、`allowed_actions` 镜像、`last_operation` typed 不 error-parse）→ 增量序列 waiting→fresh_reserved→fresh_running→resolved 最新帧胜出 → resolved 证据保留 → 字段缺席权威帧清 null → `reset()` 清空 → 切会话不串旧事实 → 重连同会话全量帧重建且 driver/observer 同 ID/action；畸形载荷（非法 state 字面量）fail-closed 保留同会话旧投影（不伪造清卡）。
- `workspace-observer-store.test.ts`：observerStateFromSessionState 显式落 `lcResumeRecovery`（不因 `as unknown` 掩码漏投影）；序列最新帧胜；resolved 证据在 store、`selectObservedInbox` 无卡；waiting 投卡；缺席=null 无卡；畸形不落半成品。
- `workspace-cockpit-projection.test.ts`：三活跃态（waiting/fresh_reserved/fresh_running）各投 `kind === "lc_resume_recovery"` 卡，卡面 `recovery` 投影字段完整（recoveryId/state/reasonCode/provider/failedNodeId/runKind/detectedAt/updatedAt/allowedActions/lastOperation），`allowedActions` 只镜像 wire（B 未接齐恒空，不从 reason_code/错误文本推导权限）；resolved 卡消失（`selectCockpitInbox(resolvedState).some(kind === "lc_resume_recovery") === false`）且 generic stopped 卡不被永久隐藏；null 不投卡。

## 交付接口（Produces）

- `LcResumeRecoveryDto`/`LcFreshOperationDto`/`LcResumeRecoveryState`/`LcFreshOperationState`/`LcResumeRecoveryDelivery` TS 镜像 + 运行时守卫 `isLcResumeRecoveryDto`（`web/src/api/types/workspace.ts`，snake_case 与后端 out.rs 逐字段一致）
- `WsOutMessage`：`session_state.lc_resume_recovery?` 字段 + 新出站变体 `lc_resume_recovery_operation_result`（delivery accepted|replayed、operation_state、fresh_node_id?、reason_code?）
- `WorkspaceWsState.lcResumeRecovery`（全量替换/reset/切会话清/畸形 fail-closed）
- `observerStateFromSessionState` 字段完整（显式映射 + 守卫窄化）
- `selectCockpitInbox` 仅 waiting/fresh_reserved/fresh_running 三态投卡；`LcResumeRecoveryInboxProjection`（`item.recovery`）供 D 层渲染

## 提交

```bash
git add -- web/src/api/types/workspace.ts web/src/state/workspace-ws-store-types.ts web/src/state/workspace-ws-store.ts web/src/state/workspace-cockpit-projection.ts web/src/state/workspace-observer-store.ts web/src/state/workspace-cockpit-projection.test-fixtures.ts web/src/state/workspace-ws-store.test-utils.ts web/src/state/workspace-ws-store.snapshot.test.ts web/src/state/workspace-cockpit-projection.test.ts web/src/state/workspace-observer-store.test.ts
git commit -m "fix: C1 driver与observer authoritative恢复投影"
```

（`workspace-ws-store.test-utils.ts` 无需改动：既有 `installWorkspaceStoreTestHooks` reset 钩子已复用；保持原样。）

实际提交：`a693764c657a62613a7d1fc6e50a0ea7edd69a48`（9 files changed, 577 insertions(+), 3 deletions(-)）。

## 移交与已知边界

1. **Task 7.1（D）**：`CockpitInbox.tsx` 的 `KIND_GLYPH`/`KIND_CLASS` 查找表需补 `lc_resume_recovery` 项并加卡面渲染分支（消费 `item.recovery` 投影）；当前 `tsc -b` 在该文件报 2 处 TS7053，属 D 写面的预期移交，非本包缺陷（本包写面内 0 编译错误）。
2. **Task 6.2（C2）**：`lc_resume_recovery_operation_result` 出站消息的 handler/sender 对账在 hooks 写面；store 侧 `last_operation` 已随 SessionState 权威全量替换。
3. observer 增量路径无独立恢复 delta 帧（设计 §4.5：RecoveryChanged 走 authoritative SessionState，不造竞态 delta）——observer 重连由 session_state 重基线帧承载。
