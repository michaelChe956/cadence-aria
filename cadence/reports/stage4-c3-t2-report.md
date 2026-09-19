# Stage 4 C3 Task 2 报告：SC 门关门决策 typed 重承载（L0）

> 状态：**完成** · 2026-09-19 · 工作树 `.worktrees/feat-b-0808-add-monorepo` · 基线 `a92151c4`
> 计划：`cadence/plans/2026-09-19_计划文档_阶段4-C3_旧协议退役与DEF4_v1.0.md` Task 2（v1.1 双审修订版）
> 详细报告：`cadence/reports/legacy-protocol-retirement/wp2-l0-report.md`

## 交付摘要

- **wire**：`WsInMessage::AbandonHumanGate { command_id }`（wire 名 `abandon_human_gate`，serde 实测 round-trip 钉死——T4 前端可直接按 `{type:"abandon_human_gate",command_id}` 实现）。
- **引擎**：新枚举 `HumanGateCloseDecision{Approve,Abandon}`（`pub(crate)` re-export）；`handle_human_gate_termination`/`close_human_gate`/两 translate 共 4 处签名收敛；RequestChange 臂删除=结构性拒绝（SC 门 typed 面不可表达）。
- **路由**：inbound 新增 AbandonHumanGate 分发臂（command_id 边界校验）；`Confirm` SC 分流→Approve；`HumanConfirm` SC 双轨桥接→映射式（Terminate→Abandon 保留、RequestChange 显式拒绝）；legacy 流路由一字不动。
- **双审修订两处均已落地**：protocol.rs `is_message_valid_for_stage_with_flow` SC HumanConfirm 白名单显式 +`AbandonHumanGate`；`message_type` 补 wire 名 `abandon_human_gate`。红测以 through socket dispatch 形态钉住（白名单漏加时真实链路拒为 `WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID`，Stage A 实测复现该红形态）。

## 红绿证据

1. 编译红：7 errors（E0433×3 + E0599×4）。
2. 运行红（Stage A 白名单缺项）：`ProtocolError{code:"WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID", message:"message abandon_human_gate not allowed in stage human_confirm"}`。
3. 绿（Stage B）：新测 6 条全过（engine 等价 1 + dispatch 族 3 + 白名单 unit 1 + roundtrip 扩展 1）。

## 门禁（定向+全量）

- conversational_gate 90 / human_gate 26 / workspace_ws_handler 126 / single_candidate 112 / abandon_human_gate 3 —— 全绿
- `cargo test --locked` 全量全 targets 绿（cargo 顺序执行到末尾 target；12 ignored 为既有 WP2 标注项）
- `cargo fmt --check` OK；`cargo clippy --all-targets --all-features --locked -- -D warnings` OK

## 实施中发现（已处置，详见 wp2-l0-report §4）

1. **栈溢出回归**：加臂后 `campaign_stage3_advance` 族测试线程栈溢出（基线 PASS/改动必现/大栈 PASS 三点定性=状态机尺寸贴边，非逻辑递归）。处置=分发函数整体 `Box::pin` 装箱（语义零变化），族恢复 4/4 绿。
2. `campaign_stage3_interactive` 内直呼 engine 的 RequestChange 防回归锁迁移至 wire 面（typed 签名后结构性保证），原位留指针注释。

## 提交

- `git commit -m "feat(ws): SC gate close decision typed re-carriage — AbandonHumanGate command + HumanGateCloseDecision, dual-track legacy retained (REQ-RET-02 L0/REQ-CG-02/04, retire-legacy-workitem-protocol)"`（hash 见下）

## Concerns（供 controller/T4/T5）

- SC 流 `HumanConfirm{RequestChange}` 的 wire 可达错误文本归一为旧 engine RequestChange 臂原文（原 stage-guard defense-in-depth 文本仅差措辞，通道/非关门行为不变）——T5 删桥接时该路径一并消失。
- `handle_workspace_inbound_message` 公开签名改为返回 `Pin<Box<dyn Future>>`（装箱修复附带），调用面 socket.rs/测试无需改（`.await` 兼容）；T5 加/删臂时不再受栈贴边约束。
- 双轨期纪律保持：零删除，`HumanConfirmDecision` 全接受面不动；删除动作全部在 T5。
