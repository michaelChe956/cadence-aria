# Task 6.2（C2）fresh sender 与 typed 结果对账 — 红/绿证据报告

- 日期：2026-10-12
- Worker：ImplAnchorC62（glm53-task）
- Plan：`cadence/plans/2026-10-11_实施计划_规划阶段会话锚点固定与漂移恢复闭环_v1.1.md` Task 6.2
- Change：`openspec/changes/fix-planning-session-anchor-recovery` 工作包 6.2（细则权威）
- 设计：获批设计 §5.1（消息与 Replay 优先级）/§6（C/D 分工）；C6.1 报告移交节第 2 条（handler/sender 对账在 hooks 写面）
- cwd：`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/web`
- 前置：C6.1 `a693764c`（store 四态重建 + typed DTO）；B4.2/B1b `6acdd4fc`（后端 wire 合同 + Replay/claim 闭包）

## Step 1–2 红（先写失败行为测试，过滤命中且断言真实失败）

命令：

```bash
npm test -- src/hooks/workspace-ws-message-handler.test.ts src/hooks/useWorkspaceWs.actions.test.tsx src/api/types.test.ts -t anchor_t20
```

输出（红，新增 11 用例命中；8 个新行为断言 FAIL，3 个既有合同回归锚 PASS）：

```
 ❯ src/hooks/workspace-ws-message-handler.test.ts (29 tests | 5 failed | 24 skipped)
   × anchor_t20 fresh sender and typed operation result > anchor_t20_fresh_sender_and_typed_operation_result
     → expected [] to have a length of 1 but got +0
   × anchor_t20 ... > replays the same operation without duplicating execution or receipts
     → expected [] to have a length of 1 but got +0
   × anchor_t20 ... > presents running, resolved and prepare-failed states while only session_state moves authority
     → expected undefined to match object { operation_state: 'running' }
   × anchor_t20 ... > keeps unknown operation state non-optimistic without generic fabrication
     → expected [] to have a length of 1 but got +0
   × anchor_t20 ... > drops result frames for a superseded or absent recovery without side effects
     → expected [ Array(1) ] to have a length of +0 but got 1   ← default 分支误记 UNRECOGNIZED_EVENT
 ❯ src/hooks/useWorkspaceWs.actions.test.tsx (36 tests | 3 failed | 32 skipped)
   × anchor_t20 ... > anchor_t20_fresh_sender_and_typed_operation_result
     → TypeError: harness.api.sendLcResumeFresh is not a function
   × anchor_t20 ... > keeps the caller-provided operation id stable and never regenerates it
   × anchor_t20 ... > returns false without queueing when the socket is not open

 Test Files  2 failed | 1 passed (3)
      Tests  8 failed | 3 passed | 73 skipped (84)
```

3 个红期即绿的用例为**既有合同回归锚**（非新行为）：types.test.ts 两条（C6.1 已交付的出站结果帧 wire 形状 pin + 普通 WsInMessage 无 fresh 意图的 @ts-expect-error 编译合同）与 actions.test.ts "keeps the plain sendStartGeneration wire free of resume recovery"（普通路径 literal None 边界）。新增行为断言（sender/handler）全部 FAIL，非 0 tests、非绕过。

## Step 3–4 绿（最小实现 + 同文件旧用例回归）

同一命令输出（绿）：

```
 ✓ src/api/types.test.ts (19 tests | 17 skipped)
 ✓ src/hooks/workspace-ws-message-handler.test.ts (29 tests | 24 skipped)
 ✓ src/hooks/useWorkspaceWs.actions.test.tsx (36 tests | 32 skipped)

 Test Files  3 passed (3)
      Tests  11 passed | 73 skipped (84)
```

三文件全量（plan 8.1 表 6.2 行范围化回归组，旧用例全保留）：

```bash
npm test -- src/hooks/workspace-ws-message-handler.test.ts src/hooks/useWorkspaceWs.actions.test.tsx src/api/types.test.ts
# Test Files  3 passed (3)   Tests  84 passed (84)
```

邻近面回归（C6.1 三个状态文件 + hooks 目录，验证 handler 消费 C6.1 store 字段零回归）：

```bash
npm test -- src/state/workspace-ws-store.snapshot.test.ts src/state/workspace-cockpit-projection.test.ts src/state/workspace-observer-store.test.ts src/hooks/
# Test Files  25 passed (25)   Tests  340 passed (340)
```

全量前端单元回归（额外 sanity，超出 plan 范围仅留证；C6.1 基线 212 文件/2076 用例，+11 新用例）：

```bash
npm test
# Test Files  212 passed (212)   Tests  2087 passed (2087)
```

编译门（`npx tsc -b`）：本包写面 0 错误；全仓仅剩 `CockpitInbox.tsx` 2 处 TS7053（C6.1 报告移交节第 1 条已记录的 D 写面预期移交，非本包引入、未扩散）。

## 新用例与断言映射（anchor_t20，fixture 复用 C6.1 anchor_t19 值族：rec-1/node-1/resume_fingerprint_mismatch/codex/legacy_draft/op-1，经真实 ws.receive/handler 路径重建 store）

- `workspace-ws-message-handler.test.ts`（typed 结果帧对账）：
  - accepted/reserved 回执逐字段镜像 wire（id 幂等键 `recovery_id:operation_id:delivery:state`、metadata 含 fresh_node_id/reason_code）；已知协议成员不落 UNRECOGNIZED_EVENT 诊断；不伪 providerStatus running、无 error 面。
  - 同帧重放幂等 upsert（回执不重复、不复活执行）；replayed/running 只读回放独立呈现。
  - 三态：fresh_running 活跃期恢复卡在场（旧 generic 出口由恢复卡替代）；resolved 同 ID 回放 completed 后卡消失且 generic stopped 卡不被永久屏蔽；prepare 失败回 waiting 不删 ID，回执呈现 failed+reason，typed 帧不改写 store 权威（session_state 全量替换唯一权威）。
  - unknown 非乐观：不标成败、无 protocolError/error、零诊断。
  - 旧 recovery_id 迟到帧与权威清空后（store 无恢复投影）的帧均静默丢弃，不复活恢复事实。
- `useWorkspaceWs.actions.test.tsx`（fresh sender）：
  - `sendLcResumeFresh("rec-1","op-1",providerConfig,true)` wire：`type=start_generation` + `resume_recovery={recovery_id:"rec-1",intent:"fresh",operation_id:"op-1"}`（与后端 in_.rs::LcResumeRecoveryIntent serde 逐字段一致）；发送后 providerStatus 仍 starting、artifact 哨兵保留（无乐观本地变更，typed authority 确认）。
  - 结果帧序列（accepted/reserved→replayed/running→completed）+ busy typed 拒收后：start_generation 帧恰 1 条、`automaticRetries===0`（无 retry_interrupted_run/advance/换新 ID 重发）——与既有 advance 重放/retry_interrupted_run 通道分离。
  - 调用方 operation_id 稳定透传（两次同 ID 发送 wire 一致，sender 不重新生成）；断链（ws 未 open）返回 false 不排队。
  - 普通 `sendStartGeneration` wire 无 resume_recovery 键（仍 None），保留既有乐观行为作对照。
- `api/types.test.ts`（wire 编译合同）：出站 `lc_resume_recovery_operation_result` 成员 snake_case JSON round-trip；普通 `WsInMessage` start_generation 携带 resume_recovery 被 @ts-expect-error 拒（显式 fresh 只经 hooks 层发送联合）。

## 交付接口（Produces）

- `LcResumeFreshIntentWire`（`web/src/hooks/useWorkspaceWs.ts` 导出）：`{recovery_id,intent:"fresh",operation_id}` snake_case 镜像；`WorkspaceWsSendMessage` 增可选 `resume_recovery` 加宽变体（普通 literal None 不变）。
- `sendLcResumeFresh(recoveryId, operationId, providerConfig, reviewerEnabled): boolean`：沿 sendJson 既有 lease/connection 守卫；零乐观本地变更；operation_id 透传不重生成；不进入 retry/advance 通道；已加入 `useWorkspaceWs` 返回对象（`WorkspaceWsApi`）。
- `handleWorkspaceWsMessage` 新 case `lc_resume_recovery_operation_result`：recovery_id 对账当前权威恢复（不匹配/无投影静默丢弃、零诊断）；Replayed 不重复执行；三态+unknown/not_started typed 回执（幂等 chat entry，metadata 全字段）；不改写 store 恢复权威、不从错误字符串推权限。
- 测试夹具（`useWorkspaceWs.test-utils.tsx`）：`lcResumeSessionStateMessage` / `lcResumeOperationResultMessage` 供 D 层复用。

## 提交

```bash
git add -- web/src/hooks/workspace-ws-message-handler.ts web/src/hooks/useWorkspaceWs.ts web/src/hooks/useWorkspaceWs.test-utils.tsx web/src/hooks/workspace-ws-message-handler.test.ts web/src/hooks/useWorkspaceWs.actions.test.tsx web/src/api/types.test.ts web/src/pages/ChatWorkspacePage.test-utils.tsx
git commit -m "fix: C2 fresh sender与typed结果对账"
```

实际提交：`ef587e8d`（7 files changed, 588 insertions(+), 2 deletions(-)）。

## 移交与已知边界

1. **写面外机械伴随（报备）**：`web/src/pages/ChatWorkspacePage.test-utils.tsx` 的 `mockWorkspaceWs` 按 `WorkspaceWsApi` 全量字面量构造，新接口键缺失导致 TS2322。已补最小 mock 成员 `sendLcResumeFresh: vi.fn(() => false)`（保守缺省未发送，需 fresh 流程的用例经 overrides 覆盖）。该文件不在 plan 6.2 列表内、亦不在 C/D/B 任何独占清单内，属 clean-cutover 调用方机械迁移，无行为变化；如 controller 判归属不符可单独摘除重派。
2. **Task 7.1（D）**：`sendLcResumeFresh` 已就绪——D 生成稳定 operation_id（`newCommandId()` 一次用户动作一次）并接线 `onStartLcResumeFresh(recoveryId)`；回执 metadata（delivery/operation_state/fresh_node_id/reason_code）可作卡面/文案数据源；CockpitInbox 的 2 处 TS7053 仍待 D 补 `lc_resume_recovery` 卡面分支。
3. observer 侧不消费结果帧（设计 §4.5：恢复走 authoritative SessionState 重基线；结果帧仅发请求连接），C6.1 移交节第 3 条边界不变。
4. 回执条目复用 `ChatEntryType="start_generation"`（关闭联合内最贴近的生成事件族类型，metadata 无 snapshot 故渲染为中性系统行）；若 D 需专属卡面/图标，可在 D 写面扩展渲染分支，metadata 契约已冻结。
