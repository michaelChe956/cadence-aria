# WP2 L0 报告：SC 门关门决策 typed 重承载（REQ-RET-02 L0 / REQ-CG-02/04）

> change: retire-legacy-workitem-protocol · Task 2（WP2.1-2.3）
> 日期：2026-09-19 · 基线：`a92151c4`（含 F-14 修复 `4d097c1d`）
> 实施工作树：`.worktrees/feat-b-0808-add-monorepo`

## §0 OQ1 定案记录

**abandon 形态 = 显式 typed 入站命令 `WsInMessage::AbandonHumanGate { command_id: String }`**（serde tag `type="abandon_human_gate"`），内部承载新枚举 `HumanGateCloseDecision { Approve, Abandon }`（conversational_gate 域 `pub(crate)`）。定案理由（计划 Task 2 背景原文承接）：

1. 与 `HumanGateFeedback{command_id,feedback}`/`Advance{command_id}` 构成门命令族（command_id 幂等/审计键先例，inbound.rs:74-121），语义对称；
2. 不与既有全局 `Abort`（取消 active run，inbound.rs:571-575）混淆——design R4 风险消除；
3. 与 legacy `human_confirm` 通道零共用枚举（REQ-RET-02 契约）；
4. 映射：`Confirm` 变体→Approve、`AbandonHumanGate`→Abandon、双轨期 `HumanConfirm{Terminate}` SC 桥接→Abandon（L2 删除）。

## §1 红绿记录（TDD，through socket dispatch 锚）

### 红（编译期，合法红）

新增 5 条测试先于实现落盘，`cargo test --locked abandon_human_gate` 编译失败：

```
error[E0433]: cannot find type `HumanGateCloseDecision` in this scope   ×3
error[E0599]: no variant named `AbandonHumanGate` found for enum `WsInMessage` ×4
error: could not compile `cadence-aria` (lib test) due to 7 previous errors
```

### 红（运行期，白名单漏加形态复现——双审修订 k3 P2 锚）

实现分两阶段：Stage A = 类型/枚举/路由/message_type 全部就位但**不加** `is_message_valid_for_stage_with_flow` 白名单项；Stage B = 补白名单。Stage A 运行 through-dispatch 测试，真实链路红形态：

```
panicked: typed abandon must reach the gate close chain, got rejected: ProtocolError {
  code: "WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID",
  message: "message abandon_human_gate not allowed in stage human_confirm",
  context: {"flow_kind": "single_candidate", "received": "abandon_human_gate", "stage": "human_confirm"} }
```

即：白名单漏加时新变体在真实分发链（scope→stage 边界→dispatch→handler）被拒为 STAGE_INVALID——非仅编译红，双审修订「防白名单漏加不可测」诉求成立。

### 绿（Stage B 白名单补齐后）

| 测试 | 断言面 |
|---|---|
| `abandon_human_gate_command_closes_gate_without_legacy_enum`（engine 级） | typed Abandon 终态 Abandoned+durable Terminated+快照/预约清理+单条 close 事件（payload 字面量 `terminate` 不变）；in-flight turn→Busy；预算耗尽仍可 abandon；函数体内零 `HumanConfirmDecision`（类型面证明） |
| `campaign_stage3_abandon_human_gate_typed_command_closes_gate_through_socket_dispatch` | 真实 ws inbound 分发链关门：通道静默（无 protocol error）→durable Terminated+快照清理+零 turn+零 provider start |
| `campaign_stage3_abandon_human_gate_rejects_blank_command_id_without_side_effects` | 空白 command_id→`INVALID_COMMAND_ID`，session 字节零变化 |
| `campaign_stage3_legacy_request_change_is_rejected_at_sc_gate_wire_boundary` | 门开启 stage 白名单直接拒 legacy RequestChange→`WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID`，零副作用 |
| `single_candidate_human_gate_stage_whitelist_accepts_typed_close_commands`（unit） | SC HumanConfirm 白名单正放行 4 形态 + 反向钉（经 HumanConfirm 通道只放行 Terminate 桥接） |
| `conversational_gate_inbound_roundtrips_and_preserves_command_ids`（扩展） | wire 名 `abandon_human_gate` round-trip+`message_type`（T4 union/T5 负向测试契约依赖） |

## §2 等价性对照（重承载前后）

| 行为 | 承载前 | 承载后 | 判定 |
|---|---|---|---|
| approve（`Confirm` 变体） | inbound→`HumanConfirmDecision::Confirm`→close | inbound→`HumanGateCloseDecision::Approve`→close（close 体内 compile 链一字不动） | 等价（既有 90 条 conversational_gate 族+126 条 ws_handler 族零语义改通过） |
| abandon（`HumanConfirm{Terminate}` 桥接） | 原枚举 Terminate 直传 close | 桥接映射 Abandon（close Terminate 臂即 Abandon 臂，事件 payload 字面量 `terminate` 保持） | 等价 |
| abandon（新 typed 命令） | —— | `AbandonHumanGate{command_id}`→Abandon（与上同臂） | 新面，行为同上 |
| Busy{turn_id}（in-flight） | close 前 non-terminal turn 检查 | 不变 | 等价 |
| AlreadyClosed（迟到 approve 幂等） | `translate_lost_human_gate_close_race` Confirm 方向 | 同函数 Approve 方向（direction 字面量 `confirm` 保持） | 等价 |
| 迟到 abandon（durable Terminated） | 显式 `already terminated` 错误 | 同（direction 字面量 `terminate` 保持） | 等价 |
| 预算 | abandon 不退还不扣减，budget=0 仍可关门 | 不变 | 等价 |
| legacy RequestChange 经 HumanConfirm 到 SC | engine close 旧枚举 RequestChange 臂 Err（wire 可达路径为 stage guard 错误文本） | inbound SC 桥接显式 `WsOutMessage::Error`（消息文本=旧 engine RequestChange 臂原文），门开启 stage 由白名单拒为 STAGE_INVALID | 行为等价（非关门+显式错误通道不变）；差异=wire 不可达的 defense-in-depth 错误文本归一，见 §4 |
| legacy 流（非 SC）全部 HumanConfirm 决策 | `handle_human_confirm_from_handler` 路由 | 一字不动 | 等价 |
| amendment/revision/recovery 链 | 直呼 close 的测试仅构造面换枚举名 | 断言语义零改 | 等价 |

## §3 双轨期声明

- 本 Task 落地后 legacy 消息（含 `HumanConfirm` 全决策）仍接受：legacy 流路由一字未动；SC 流 `HumanConfirm{Terminate}` 桥接保留（L2/T5 删除）。
- **零删除**：`HumanConfirmDecision` 枚举、legacy 路由、`handle_human_confirm` 引擎面全部保留；删除动作全部在 T5。
- 全量门禁绿见 §5。

## §4 实施中发现与处置

1. **测试线程栈溢出回归（已修复）**：Stage B 后 `campaign_stage3_advance_confirmed_plan_is_ready_without_provider_start` SIGABRT（stack overflow）。RR-3 定性：基线（stash 后）单测 PASS；改动在场隔离复跑必现；`RUST_MIN_STACK=16MB` PASS——非逻辑递归，是巨型 `handle_workspace_inbound_message` async 状态机加臂后把贴边的深链测试（fixture→Confirm→compile 链在 #[tokio::test] 测试线程内联 poll）推过默认栈。处置：分发函数整体装箱（`Box::pin` 私有 inner，公开签名返回 `Pin<Box<dyn Future>>`），语义零变化、状态机移堆，campaign_stage3_advance 族 4/4 恢复绿。WS 入站消息频率低，一次堆分配可忽略；此后加臂不再影响调用方栈深。
2. **`handle_workspace_inbound_message` 的 AbandonHumanGate 臂内 command_id 校验**：与 Advance 同款 in-arm 校验（空白/超长拒为 `INVALID_COMMAND_ID`）；command_id 定位=幂等/审计键（wire 对称），关门幂等本身由 durable 会话单飞语义承载（先到者赢），不另设 per-command 记录。
3. **engine 测试 `campaign_stage3_interactive` 内直呼 `handle_human_gate_termination(RequestChange)` 的防回归锁**：typed 签名后无法表达（结构性保证），wire 面锁迁移至 `campaign_stage3_legacy_request_change_is_rejected_at_sc_gate_wire_boundary`（STAGE_INVALID 形态），原位留注释指针。

## §5 门禁记录（定向）

- `cargo test --locked --lib conversational_gate`：90 passed
- `cargo test --locked --lib human_gate`：26 passed
- `cargo test --locked --lib workspace_ws_handler`：126 passed
- `cargo test --locked --lib single_candidate`：112 passed
- `cargo test --locked --lib abandon_human_gate`（新测族）：3 passed（另有 legacy_request_change/whitelist/roundtrip 各 1 passed 见 §1）
- 全量/fmt/clippy：见 stage4-c3-t2-report.md（controller 面报告）

## §6 变更文件

- `src/web/workspace_ws_types/in_.rs`：+`AbandonHumanGate{command_id}` 变体（`Advance` 后、`Abort` 前，门命令族聚拢）
- `src/product/workspace_engine/conversational_gate.rs`：+`HumanGateCloseDecision` 枚举；`handle_human_gate_termination`/`close_human_gate`/`translate_human_gate_close_cas_error`/`translate_lost_human_gate_close_race` 四处签名收敛；RequestChange 臂删除（结构性拒绝）；删 `HumanConfirmDecision` 导入
- `src/product/workspace_engine/mod.rs`：re-export +`HumanGateCloseDecision`
- `src/web/workspace_ws_handler/protocol.rs`（双审修订两处）：SC HumanConfirm 白名单显式 +`AbandonHumanGate`；`message_type` 补 wire 名 `abandon_human_gate`
- `src/web/workspace_ws_handler/decisions.rs`：handler 签名/判定换 typed
- `src/web/workspace_ws_handler/decisions/inbound.rs`：+`AbandonHumanGate` 分发臂；Confirm SC 分流→Approve；HumanConfirm SC 桥接→映射式（Approve/Abandon/RequestChange 显式拒绝）；分发函数整体装箱
- 测试：`tests/conversational_gate.rs`（+1 用例+构造面适配）、`tests/conversational_gate_close.rs`、`tests/conversational_gate_amendment_real_chain.rs`、`tests/conversational_gate_revision.rs`（构造面适配）、`campaign_stage3_interactive/{harness,cases}.rs`（+探针/3 用例/锁迁移）、`tests/conversational_gate_protocol.rs`（+roundtrip 扩展+白名单 unit）；`src/product/coding_workspace_engine/tests/campaign_stage3_amendment.rs`（构造面适配）
