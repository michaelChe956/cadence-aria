# Workspace 产品工作台优化 - 实施计划总览（v1.0）

## 总览信息

- 分支：`product-workbench-issue-lifecycle`
- 工作区：`.worktrees/product-workbench-issue-lifecycle`
- 制定日期：2026-05-20
- 版本：v1.0
- 依据设计：`cadence/designs/2026-05-20_技术方案_Workspace产品工作台优化_v1.0.md`

## 拆分原则

- **每个 plan 在一个 256k 上下文窗口内可执行完毕**（含 TDD 单测 / 集成测 / 必要 E2E）
- **尽可能独立可完成**（每个 plan 完结后独立可用，独立可验证）
- **必须串联的依赖明确写在 plan 头部**（前置 plan + 输出 + 后置 plan 消费点）

## 7 个 Plan 与依赖关系

```
                          ┌─→ P2 PrepareContext UI 重构 ──┐
                          │                                  ↓
P1 协议层 + Timeline 持久化 ─┼─→ P4 Running/审核/确认 UI
（基础设施，必须最先）         │
                          ├─→ P5 断开策略 + WebSocket 重连
                          │
                          └─→ P6 Permission 链路修复（弱依赖：仅用 NodeDetail.permission_events 字段）

P3 看板侧滑详情面板（完全独立，可与任意 plan 并行）

P7 E2E 测试升级（依赖 P1-P6 全部完成，作为收尾验收）
```

## Plan 列表

| Plan | 文件 | 范围 | 前置 | 估算任务数 |
|---|---|---|---|---|
| P1 | `2026-05-20_计划文档_Workspace优化_P1_协议层与Timeline持久化_v1.0.md` | §1 + §2 | — | ~18 |
| P2 | `2026-05-20_计划文档_Workspace优化_P2_PrepareContext阶段UI_v1.0.md` | §3 | P1 | ~10 |
| P3 | `2026-05-20_计划文档_Workspace优化_P3_看板侧滑详情面板_v1.0.md` | §4 | — | ~12 |
| P4 | `2026-05-20_计划文档_Workspace优化_P4_Running审核确认UI_v1.0.md` | §5 | P1, P2 | ~14 |
| P5 | `2026-05-20_计划文档_Workspace优化_P5_断开策略与WebSocket重连_v1.0.md` | §6 + §7 | P1 | ~12 |
| P6 | `2026-05-20_计划文档_Workspace优化_P6_Permission链路修复_v1.0.md` | §8 | 弱依赖 P1 | ~10 |
| P7 | `2026-05-20_计划文档_Workspace优化_P7_E2E测试升级_v1.0.md` | §9 | P1-P6 | ~14 |

## 推荐执行顺序

```
1. P1（基础设施）            必须最先
   ↓
2. P3（看板侧滑）            可与 P2/P5/P6 并行
   P2（PrepareContext UI）   
   P5（断开+重连）           
   P6（Permission 修复）     
   ↓
3. P4（Running/审核/确认 UI）需要 P1+P2 完成
   ↓
4. P7（E2E 测试升级）        收尾
```

## 串联交付物（plan 间契约）

### P1 输出（其他 plan 消费）

后端 Rust 类型：
- `WsInMessage` 新增：`ContextNote { content }`、`StartGeneration { provider_config, reviewer_enabled }`、`Hello { session_id, last_seen_node_id }`、`Ping`
- `WsOutMessage` 新增：`ProtocolError { code, message, context }`、`ProviderLocked { snapshot, locked_at }`、`Pong`
- `TimelineNodeType` 新增：`ContextNote`、`StartGeneration`、`AuthorRun`（替代 `Generation`）、`ReviewerRun`（替代 `Review`）、`AbortedByDisconnect`、`ProtocolError`
- `NodeDetail` 结构 + `SessionState.timeline_node_details: HashMap<String, NodeDetail>` + `SessionState.active_run_id: Option<String>`

前端 TS：
- `useWorkspaceWs.sendContextNote(content)` / `sendStartGeneration(snapshot, reviewerEnabled)` / `sendHello(sessionId, lastSeenNodeId)` / `sendPing()`
- `workspace-ws-store` snapshot 应用时灌入 `nodeDetails[node_id] = snapshot.timeline_node_details[node_id]`
- 新增 selector `selectNodeDetail(nodeId)`

### P2 消费 P1
- 调 `sendContextNote` 替换原 `sendMessage`
- 调 `sendStartGeneration` 替换原"统一输入提交触发生成"路径
- 监听 `provider_locked` 事件切换 Header 状态

### P4 消费 P1 + P2
- 使用 `selectNodeDetail` 渲染 5 tab
- 复用 P2 `useStageUI` hook

### P5 消费 P1
- 后端 socket close handler 调 `engine.append_aborted_by_disconnect_node`（P1 已暴露）
- 前端处理 `ws.send({ type: "hello", ... })`（P1 已定义）

### P6 弱依赖 P1
- `NodeDetail.permission_events` 字段在 P1 已定义
- 防御性修复主要在 `src/cross_cutting/approval_bridge.rs`，不依赖 P1 协议变更

### P7 消费 P1-P6
- 利用 P1 新协议、P2/P4 新 UI、P5 自动重连 + 断开拦截、P6 Permission 修复路径写 7 闭环 E2E 用例

## Plan 内格式约定

每个 plan 文档包含：
1. **Header**：Goal / Architecture / Tech Stack
2. **前置依赖**：明确指出依赖哪些 plan 的产物
3. **后续 plan 消费点**：本 plan 输出哪些被后续消费
4. **File Structure**：会新建/修改的文件清单（含责任）
5. **Tasks**：每个 task 用 TDD（写测 → 跑测 → 实现 → 跑测 → commit）
6. **验收清单**：本 plan 完结后需通过哪些断言

## 测试与覆盖率

- 单元 / 集成测试沿用项目默认 80% 阈值
- E2E：本轮新增用例在 P7 集中实现，单 plan 内可写小规模冒烟 E2E（被 P7 整合）
- 回归基线：

```bash
cargo test --workspace
pnpm --filter web test
pnpm --filter web test:e2e
```

## 风险与重要提示

- **P1 是地基**：任何对协议、Timeline 类型、SessionState、NodeDetail 的改动必须在 P1 内完成。后续 plan **不得**新增 WsInMessage / WsOutMessage 变体，否则会破坏独立性
- **P3 完全独立**：lifecycle 看板与 Workspace 共享 store，但本轮 P3 只动 `lifecycle-workbench-store.ts` 中新增字段（不动 workspace-ws-store）
- **过渡期兼容**：P1 内 `user_message` 保留软兼容（按 context_note 语义处理 + warning log），帮助 P2/P4 在切换期不破坏既有 E2E
- **冲突区**：P2 / P4 都会改 `WorkspacePage.tsx`。建议顺序执行（P2 先把"输入区+按钮"拆出去，P4 再重构 Header + 节点详情 + 阶段面板）

## 验收链

完整链路验收（在 P7 中实施）：

```
A. 输入语义解耦      （依赖 P1 + P2）
B. Timeline 审计     （依赖 P1）
C. 看板侧滑详情      （依赖 P3）
D. 阶段化 UI         （依赖 P1 + P2 + P4）
E. 断开策略          （依赖 P1 + P5）
F. 自动重连          （依赖 P1 + P5）
G. Permission 链路   （依赖 P1 + P6）
```

每一项在 P7 内组织 E2E case。
