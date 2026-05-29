# CodingWorkspace 二期 P2：Stage Gate 机制

## 文档信息

- 文档类型：计划文档
- 分支：`product-workbench-issue-lifecycle`
- 制定日期：2026-05-28
- 版本：v1.0
- 前置：P1（兼容式角色模型与 WS 协议扩展）
- 产出：AttemptRunner 执行模型 + Gate 状态持久化 + StageGate 倒计时交互 + Provider 运行时切换
- 设计文档：`cadence/designs/2026-05-28_技术方案_CodingWorkspace二期改造_v1.0.md` §2.2, §7.2
- 设计评审：`cadence/designs-reviews/2026-05-28_设计评审_CodingWorkspace二期改造_v1.0.md`

---

## 一、目标

先完成 Stage Gate 所需的执行模型重构，再实现倒计时交互。当前 WebSocket handler 在 `StartCoding` 后同步执行完整流程，无法在执行期间继续接收 `StageGateConfirm` / `ProviderSelect`。因此 P2 必须先做 runner 化和 Gate 状态持久化。

1. WebSocket handler 与执行流解耦
2. 后台 `AttemptRunner` 承载 CodingWorkspace 执行
3. 客户端消息通过 command channel 进入 runner
4. Gate 状态持久化并进入 session state
5. 每个 LLM 阶段开始前支持倒计时确认和 Provider 切换

---

## 二、任务清单

### 2.1 AttemptRunner 执行模型（src/product/coding_workspace_runner.rs）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 1.1 | 新增 `AttemptRunner`：接收 attempt_id、store、provider registry、event_tx、command_rx | 单元测试 | runner 可驱动现有 happy path |
| 1.2 | `handle_coding_socket` 收到 StartCoding 后启动 runner task，不阻塞 receive loop | WS 集成测试 | StartCoding 后仍可接收 AbortAttempt |
| 1.3 | 定义 `CodingRunnerCommand`：ProviderSelect / StageGateConfirm / AbortAttempt | 单元测试 | 命令可序列化/路由 |
| 1.4 | AbortAttempt 经 command channel 中止 runner 并更新 attempt | WS 集成测试 | 执行中可中止 |

### 2.2 Gate 状态持久化（src/product/coding_attempt_store.rs）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 2.1 | 新增 `CodingStageGateState` 模型：gate_id / stage / role / expires_at / provider_snapshot / status | 序列化测试 | 状态可持久化 |
| 2.2 | store 支持创建、查询、确认、取消 Gate | 单元测试 | 重连可恢复 open gate |
| 2.3 | `CodingSessionState.pending_gates` 返回持久化 Gate | WS 集成测试 | 重连后可见 |

### 2.3 Provider 运行时切换（src/web/coding_ws_handler.rs + runner）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 3.1 | 非 Gate 期间 ProviderSelect 更新非当前阶段角色配置 | 单元测试 | 非当前阶段可切换 |
| 3.2 | 当前运行阶段 Provider 切换拒绝：返回 CodingProtocolError | 单元测试 | 当前阶段返回错误 |
| 3.3 | Gate 期间 ProviderSelect 经 runner 更新配置并刷新 Gate expires_at | 单元测试 | 倒计时重置 |
| 3.4 | 切换成功后广播 `CodingProviderConfigUpdated` | 单元测试 | 客户端收到更新通知 |

### 2.4 StageGate 倒计时逻辑（src/product/coding_workspace_runner.rs）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 4.1 | 在每个 LLM 阶段开始前创建 open Gate 并发送 `CodingStageGate` | 单元测试 | Gate 事件正确发送 |
| 4.2 | Gate 超时自动确认：5s 后进入下一阶段 | tokio time 测试 | 自动继续 |
| 4.3 | `StageGateConfirm` 经 command channel 立即确认 Gate | 单元测试 | 确认后立即开始 |
| 4.4 | Gate 期间 `AbortAttempt` 取消 Gate 并中止 attempt | WS 集成测试 | 中止正确执行 |

### 2.5 前端 StageGateEntry 组件（web/src/components/coding-workspace/）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 5.1 | 新建 `StageGateEntry.tsx`：展示阶段名称 + 当前 Provider + 倒计时进度条 | — | UI 正确渲染 |
| 5.2 | 倒计时逻辑：前端本地 setInterval 倒计时，到 0 后变为"已自动确认"静态卡片 | — | 倒计时流畅 |
| 5.3 | "立即开始"按钮：发送 StageGateConfirm 消息 | — | 点击后 Gate 消失 |
| 5.4 | "中止"按钮：发送 AbortAttempt 消息 | — | 点击后 attempt 中止 |

### 2.6 前端 CodingProviderConfigPanel（web/src/components/coding-workspace/）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 6.1 | 新建 `CodingProviderConfigPanel.tsx`：展示 5 角色 Provider 配置列表 | — | 5 个角色均展示 |
| 6.2 | 当前阶段 Provider 显示锁定状态（灰色 + 锁图标） | — | 不可点击 |
| 6.3 | 非当前阶段 Provider 可点击打开选择器 → 发送 ProviderSelect | — | 切换后 UI 更新 |
| 6.4 | 监听 `CodingProviderConfigUpdated` 消息更新本地状态 | — | 实时同步 |

---

## 三、验收标准

1. `cargo test` 全部通过
2. 手动测试：StartCoding 后执行流运行期间仍可处理 AbortAttempt
3. 手动测试：Coding 完成后 → 出现 Testing Gate 卡片 → 5s 倒计时 → 自动进入 Testing
4. 手动测试：Gate 期间点击"立即开始" → 立即进入下一阶段
5. 手动测试：Gate 期间切换 Tester Provider → 倒计时重置 → 新 Provider 生效
6. 手动测试：Gate 期间切换当前阶段 Provider → 收到错误提示

---

## 四、不做的事

- Test Agent Loop 的实际执行（P3）
- Analyst 判定逻辑（P4）
- 前端 ChatEntryList 复用（P5）
