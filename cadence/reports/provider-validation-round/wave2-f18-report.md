# wave2 F-18 修复报告：story 会话 terminate 通路补齐（author_confirm 门 typed abandon）

- 工作树：`.worktrees/feat-b-0808-add-monorepo`
- 输入证据：`wave2-2c/story-terminate-v2-evidence.json`（story 会话 0472，flow_kind=legacy，
  stage=author_confirm：WS `abandon_human_gate` → `INVALID_MESSAGE_FOR_STAGE`；WS `confirm` → 同拒；
  HTTP confirm ✔（approve 设计通路）；HTTP terminate 端点不存在；门卡 UI 不渲染——
  `selectGateProjection` 对 author_confirm 返回 null）
- 缺口定性：story 会话 approve 有通路、terminate 零通路。

## 1. 修法选择：a（WS typed abandon 扩展放行 + 引擎 story 门关门接线）

对比 b（HTTP terminate 端点，镜像 confirm 端点）：

| 面 | a（选定） | b |
|---|---|---|
| 消息面 | 复用 T2 既有 `AbandonHumanGate` typed 命令（矩阵加一臂） | 新增路由+handler+DTO |
| 语义 | 引擎关门：durable Terminated + Completed 阶段 + 「流程终止」节点 + `human_gate_closed` 出站事件（WS 连接实时可见关门） | 记录级写状态，绕过引擎（与 HTTP confirm 同形）：无时间线节点、无关门事件 |
| 方向一致性 | 与 C3（REQ-RET-02）「关门决策 typed 化」同向 | 新开 HTTP 决策面 |

选 a：改动面 = 矩阵一臂 + 引擎一个分流函数（分发链 `inbound.rs`→
`handle_human_gate_termination_from_handler` 已存在，零改动），且语义完整（时间线/事件/终态）。

### 改动点

1. `src/web/workspace_ws_handler/protocol.rs` —— stage 矩阵 `AuthorConfirm` 臂放行
   `AbandonHumanGate`（与 `Abort` 并列；`Confirm` 帧继续不放行——approve 仍走 HTTP confirm 端点）。
2. `src/product/workspace_engine/conversational_gate.rs` ——
   `handle_human_gate_termination` 对 **story/design + abandon** 分流到新函数
   `terminate_story_author_gate`（WorkItemPlan 一律仍走 SC `close_human_gate`）：
   - durable `WaitingForHuman`（author_confirm 的 status 映射）+ 内存 `AuthorConfirm` 门开 →
     `update_workspace_session_status(Terminated)` + `Completed` 阶段 + `complete_active_node("已终止")`
     + 「流程终止」Completed 节点 + 恰一条 `HumanGateClosed{terminate, completed}` 事件
     （终态形状承接 C3 删除的 legacy `Terminate` 分支）。
   - durable 已 `Terminated` → 幂等 no-op（`AlreadyClosed`，不依赖内存 stage——迟到 worker
     内存可能已 Completed）。
   - durable 已 `Confirmed`（HTTP approve 先行）或非门开态 → fail-closed 明确报错，不改写。
   - 写入走通用 status 更新（与 HTTP confirm 端点同形），不占用 SC 门 close CAS。

## 2. C3 契约核对（bug 修复论证，不改契约文本）

- **REQ-CG-02/04**（SC 门消息面边界/门关闭决定）：仅约束 SC interactive 与 amendment 门
  （human_confirm 阶段）。SC 矩阵臂与引擎 SC close 路径零改动；红测
  `conversational_gate_stage.rs` 既有断言（legacy 流 `AbandonHumanGate` 在 **human_confirm**
  不放行）保持原样通过。author_confirm 放行不触碰该边界。
- **REQ-RET-03**（存量与历史记录处置）：其「在途 legacy 会话决策拒绝」约束的是**已删除的
  legacy 决策消息**（parse 面 `LEGACY_MESSAGE_RETIRED`，不变）与 durable 只读保留（不变）。
  `abandon_human_gate` 是 T2 现役 typed 命令而非退役消息；story/design 会话是现行产品面
  （wp5 归因表曾把其 author 决策通道整体终止折入该登记限制），本修复以 typed 命令补
  terminate 通路=解除该折入限制的功能缺口，不复活任何已删符号（`AuthorDecision`/
  `human_confirm` 消息族仍 parse 面拒收）。与 F7 修复（f6c2833e）同款 bug-fix 框架，不动 spec。
- **spec-design-dialog-revision**（story AuthorConfirm 契约）：契约面只钉反馈修订循环与
  确认双出口；「推倒重来出口被移除」（Reject→引导错误）场景不涉及（无 AuthorDecision 复活）。
  增补 terminate 出站命令不与任何既有场景冲突。
- WorkItemPlan 语义边界：引擎分流以 `workspace_type ∈ {Story, Design}` 判据，WIP 会话
  （SC 或 legacy 流）在 author_confirm 发 abandon 仍落 SC close 守卫错误（红测
  `work_item_plan_author_confirm_abandon_keeps_sc_close_boundary` 钉死）。

## 3. TDD 红绿

红（修复前实测，失败原因与 W2c 现场证据同因）：

- 矩阵：`is_message_valid_for_stage_with_flow(Legacy, abandon, AuthorConfirm)` = false
- 引擎：story@AuthorConfirm abandon →
  `"human gate close is only available for a single-candidate work-item plan in human_confirm"`
- WS 分发：`handle_workspace_inbound_message(AbandonHumanGate)` → 出站 Error 帧、durable 停留
  waiting_for_human

绿（新增 6 测全过）：

- `src/web/workspace_ws_handler/tests/conversational_gate_stage.rs`：矩阵 author_confirm 放行
  abandon/Abort、继续拒 Confirm 帧
- `src/product/workspace_engine/tests/story_author_gate.rs`（新）：
  story/design abandon 关门（durable Terminated + Completed + 「流程终止」节点 + 恰一条
  terminate close 事件）；重复 abandon 幂等 AlreadyClosed；Confirmed 不被改写（fail-closed）；
  WIP@AuthorConfirm 保持 SC 边界报错
- `src/web/workspace_ws_handler/tests/conversational_gate_protocol.rs`：story 门 abandon 经真实
  inbound 分发链关门（无 Error 出站帧 + durable Terminated + close 事件）

定向回归（全绿）：

- `web::workspace_ws_handler` 118 passed
- `product::workspace_engine` 671 passed
- `web::handlers`（HTTP confirm 端点等）103 passed

## 4. HTTP/WS 双面黑盒验证（真实服务器，新二进制，隔离工作区 :4399）

种子：克隆 W2c 素材 issue（story 会话 + author_confirm active 时间线，status 重置
waiting_for_human），两个会话分跑双面。证据：`wave2-f18/smoke-e2e-result.json`、
`wave2-f18/smoke-drive.cjs`。

| 面 | 操作 | 结果 |
|---|---|---|
| HTTP | `POST /api/workspace-sessions/{id}/confirm` | 200，durable `confirmed`（approve 通路不受影响） |
| WS | 门开态（session_state: author_confirm/waiting_for_human）单发 `abandon_human_gate` | 收到出站 `human_gate_closed{decision:"terminate",stage:"completed"}`，durable `terminated`；时间线落 node_003 completed + 「流程终止」node_004（首轮实测留痕） |
| WS（关门后） | 再发 abandon | `INVALID_MESSAGE_FOR_STAGE ... stage completed`（矩阵在终态正确拒绝，零副作用） |

修复前同驱动（W2c 原始证据）：abandon 14ms 即拒 `INVALID_MESSAGE_FOR_STAGE`，会话停留
waiting_for_human。

## 5. 不变项与遗留观察

- HTTP terminate 端点**刻意未加**（修法 a 的最小面选择；terminate 统一走 WS typed 命令）。
- WS `confirm` 帧在 author_confirm 继续不放行（story approve 设计通路=HTTP 端点，维持现状）。
- 前端观察项（不在本修复范围）：`selectGateProjection` 对 author_confirm 仍返回 null，
  SC 门卡形态不投影 story 门——驾驶端/脚本已可经 WS typed 命令 terminate；story 页面级
  terminate 按钮属 UI 面，留待 UI 波次。
- HTTP confirm 端点对已 Terminated 会话仍是记录级无条件写 Confirmed（既有形态，本修复不改；
  引擎侧 terminate 已 fail-closed 反向防护）。

## 5a. Fix round 1（k3 审 1×P2）：终态写非原子 → CAS 单飞

**问题**：round 0 的 `terminate_story_author_gate` 终态写走非原子
`update_workspace_session_status(Terminated)`——HTTP confirm 端点不持锁无条件写
Confirmed，若落在引擎 durable 读与终态写之间，Terminated 覆盖 Confirmed：durable
矛盾（confirm 方已收 200）且 terminate 伪报成功。

**修法**（k3 给定）：

- 终态写改 `compare_and_update_workspace_session_status(&durable, Terminated)` CAS
  （amendment.rs:733 同款先例：独占锁内比对 expected 快照后置终态）。
- CAS `IdentityMismatch` 经新 `translate_lost_story_terminate_race` 重读翻译（与 SC
  `translate_lost_human_gate_close_race` 同族）：durable 已 Terminated（另一 terminate
  先到）→ 幂等 `AlreadyClosed`（同步内存态、不重复 close 语义/事件）；其余漂移（含
  HTTP confirm 先行的 Confirmed）→ 明确错误上抛，绝不覆盖先到者。
- 测试注入口：`#[cfg(test)]` drift-hook 注册表（session id 键，生产构建空内联），
  在「durable 读 → CAS 写」窗口注入一次外部写，确定性复现竞态。

**TDD**：

- 红（round 0 代码实测）：竞态窗口注入 Confirmed → terminate 仍报 `Ok(Abandoned)` 且
  durable 被**覆盖成 Terminated**（k3 指出的矛盾现场）；注入 Terminated → 重复报
  成功+重复 close 事件。
- 绿（3 新测）：confirm 先行 → Err 点名 race+Confirmed、durable 保持 Confirmed、零
  close 事件；另一 terminate 先行 → `AlreadyClosed` 幂等、无重复事件、不本地关门；
  store 级 CAS 钉测（drifted expected → IdentityMismatch、零写入）。
- fixture 强化：session id 进程内唯一（drift-hook 注册表全局键防并行串扰）。

**回归**：`story_author_gate` 8/8、`product::workspace_engine` 674、
`web::workspace_ws_handler` 118 全绿；rustfmt+clippy 干净。

## 6. Commit 文件清单

- `src/web/workspace_ws_handler/protocol.rs`
- `src/product/workspace_engine/conversational_gate.rs`
- `src/web/workspace_ws_handler/tests/conversational_gate_stage.rs`
- `src/web/workspace_ws_handler/tests/conversational_gate_protocol.rs`
- `src/product/workspace_engine/tests/story_author_gate.rs`（新）
- `src/product/workspace_engine/tests.rs`
- `cadence/reports/provider-validation-round/wave2-f18-report.md`（本报告）
- `cadence/reports/provider-validation-round/wave2-f18/smoke-e2e-result.json`
- `cadence/reports/provider-validation-round/wave2-f18/smoke-drive.cjs`
