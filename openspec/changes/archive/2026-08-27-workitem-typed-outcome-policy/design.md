## Context

见 proposal.md - Why。代码现状（双审核已逐项核实）：

- `src/web/workspace_ws_types/review.rs:21-28` 的 `ReviewFinding` 只有 severity/message/evidence/required_action，缺机器可读分类字段；
- `src/product/workspace_engine/review/routing.rs:100-191` 由 verdict 直接驱动阶段跳转；`NeedsHuman → enter_human_confirm`，自动路径没有独立 stopped 终态（rep1 死循环实证）；
- session 持久化链路为 `models/workspace.rs::WorkspaceSessionRecord`、`lifecycle_store/inputs.rs::CreateWorkspaceSessionInput`、`web/handlers/lifecycle.rs::prepare_work_item_plan`、`PrepareWorkItemPlanRequest`；
- 既有 `HumanConfirmDecision::{Confirm, RequestChange, Terminate}` 可复用；阶段 1 不修改人工门产品 UI；
- `sha2 = "0.10.9"` 已在 Cargo.toml；Unicode NFC 归一化须新增 `unicode-normalization` 并锁定 Cargo.lock。

## Goals / Non-Goals

**Goals：**

- 每个 workitem 运行必达 `completed`、`awaiting_human`、`stopped_needs_human` 或 `failed` 之一；
- 自动返修最多一次；初评最多一次、复评最多一次；相同指纹绝不自动再试；
- 运行历史、服务端生成的 review invocation scope、reservation、diagnostic 与人工门快照均 durable，可断线恢复；
- 历史 session JSON 按 serde default 兼容；flow_kind 在创建时固定；
- campaign 无人值守时从 durable SessionState 得到终态、计数和启动次数，不依赖 stage 猜测。

**Non-Goals：**

- markdown 编译器、单候选流程合并（阶段 2，`rearch-workitem-plan-pipeline`）；
- 对话流式人工门产品 UI 与最小 WS 决策协议（阶段 3）；
- 删除旧协议、多仓支持、claude/kimi 扩展验证（阶段 4）；
- coding 段、`work_item_split_validator` 校验语义与 compile 事务语义改动。

允许为新 durable 字段修改 Rust WS DTO、TypeScript type 与 store 的**透传/兼容**；这不构成修改人工门产品 UI，既有字段必须保持兼容。

## Decisions

### P1-D1: finding schema、显式 parser adapter 与唯一错误链路

领域 `ReviewFinding` 新增三个可选字段：

```rust
pub category: Option<ReviewFindingCategory>,
pub class_hint: Option<FindingClassHint>,
pub contract_field: Option<String>,
```

三个字段缺失时恢复为 `None`，`ReviewFinding` 继续 `deny_unknown_fields` 以拒绝未知字段名。不能把 `Option<Enum>` 的 serde 默认行为当成 unknown-value 处理：它只适合缺字段，会对未知 variant 给出通用 serde 错误，无法得到稳定 diagnostic。

因此唯一入口必须是显式 adapter：

```rust
pub fn parse_review_envelope(
    value: &serde_json::Value,
) -> Result<ParsedReviewEnvelope, ReviewStructuredOutputError>;
```

adapter 先解析字段原始字符串或预检 JSON，再转换为领域枚举。未知 category 必须返回 `ReviewStructuredOutputError::UnknownFindingCategory(raw)`；未知 class_hint 返回 `UnknownFindingClassHint(raw)`；类型错误返回 `InvalidFindingField`。不允许 fallback 到 `NeedsHuman` 或 `HumanRequired`。

`ParsedReviewEnvelope` 固定为：

```rust
pub struct ParsedReviewEnvelope {
    pub raw_verdict: ReviewVerdictType,
    pub normalized_gate: ReviewGate,
    pub findings: Vec<ReviewFinding>,
}
```

`raw_verdict` 是唯一分类输入；`normalized_gate` 只用于展示/旧协议兼容。错误名称和映射不得有别名或“对应协议错误”分支：

```text
ReviewStructuredOutputError::UnknownFindingCategory (parser)
  → ClassificationError::UnknownCategory
  → FatalReason::UnknownCategory
  → durable failed + diagnostic code `unknown_finding_category`

ReviewStructuredOutputError::UnknownFindingClassHint (parser)
  → ClassificationError::UnknownClassHint
  → FatalReason::UnknownClassHint
  → durable failed + diagnostic code `unknown_class_hint`
```

`InvalidFindingField` 映射 `ClassificationError::InvalidFinding` → `FatalReason::InvalidStructuredOutput`；scope 违例映射 `ClassificationError::VerificationScopeViolation` → `FatalReason::ProtocolViolation`。所有映射由显式函数或 `From` 实现并测试，绝不进入人工门。

### P1-D2: 分类器先于纯评估器

`classify_review` 在 `evaluate` 之前存在，且只接受已解析 envelope 与服务端 scope：

```rust
pub fn classify_review(
    envelope: &ParsedReviewEnvelope,
    invocation: &ReviewInvocationScope,
) -> Result<Vec<ClassifiedFinding>, ClassificationError>;
```

规则固定：合法 `class_hint` 优先；无 hint 时 `needs_human → HumanRequired`、`pass + findings → Advisory`、`revise + (contract_gap | self_contradiction) + contract_field → Repairable`，其他 `revise → HumanRequired`。无法分类即 Err 并按 P1-D1 失败关闭。

纯评估器不得读 store、创建 durable reservation、启动 provider 或递增成功 transition 计数；它只输出 outcome 与 history delta。只有 durable route handler 在已知 session/run owner 后才创建 reservation。持久化 CAS 协议属于 P1-D6/P1-D7，避免 Task 3 evaluator 与 session 持久化形成隐含环依赖。

### P1-D3: 完整 typed outcome、reason、delta 与 reservation 契约

策略层仅有四类 outcome：

```rust
pub enum PlanOutcome {
    Valid,
    Repairable { findings: Vec<ClassifiedFinding> },
    HumanRequired {
        findings: Vec<ClassifiedFinding>,
        repeated_fingerprints: Vec<FindingFingerprint>,
        reason: HumanReason,
    },
    Fatal { reason: FatalReason, diagnostics: Vec<PolicyDiagnostic> },
}

#[serde(rename_all = "snake_case")]
pub enum FatalReason {
    TransitionBudgetExhausted,
    UnknownCategory,
    UnknownClassHint,
    InvalidStructuredOutput,
    StateCorruption,
    ProtocolViolation,
    PersistenceFailure,
    SafetyInvariantViolation,
}

#[serde(rename_all = "snake_case")]
pub enum HumanReason {
    NativeHumanRequired,
    RepeatedFingerprint,
    VerificationNewFindings,
    RepairBudgetExhausted,
}
```

`FatalReason` 实现 `Display`，只输出稳定 snake_case variant code；原始错误详情只放 `PolicyDiagnostic { code, message, field: Option<String> }`。`HumanReason` 的来源包括原生/兜底 human_required、非 advisory 的已见指纹、复评 scope 外新 finding（`VerificationNewFindings`），以及 repairable 无自动预算（含复评未解决）。

`RunHistoryDelta` 至少具有 set union 的 `seen_fingerprints_to_add` 以及 repairs/manual_repairs/transitions/initial_review/verification_review 五个 `u32` 增量。合并规则是集合并集、计数 checked_add；持久化历史本已超预算、加法溢出或合并后超预算都为 `FatalReason::StateCorruption`。advisory 不进入 seen；纯 evaluator 的 transition delta 恒为 0，只有成功持久化 stage transition 的 route handler 才能置 1。

`RepairReservation` 至少持久化 `token`、`owner_session_id`、`owner_run_id`、`provider_start_idempotency_key`、`state`、`commit_id`。状态机为 `Reserved → ProviderStarted → Committed` 或 `Reserved → Released`。token 与 owner/key 唯一绑定：Reserved CAS 不加 `repairs_used`；成功记录 provider 启动才加一次 repairs 并转 ProviderStarted；启动失败按 token 幂等转 Released 并记录 infra diagnostic。重启时，未见启动账本的 Reserved 必须释放；ProviderStarted/Committed 必须复用 key 查询/恢复，不得二次启动。

### P1-D4: 终态矩阵与预算边界

| 条件 | Interactive | AutoIfValid |
|---|---|---|
| 首次 aggregate repairable（机械与 review 共用一次自动返修预算） | 自动返修一次 | 自动返修一次 |
| 复评未解决、重复指纹、repair budget 耗尽 | AwaitingHuman | StoppedNeedsHuman |
| 原生 human_required | AwaitingHuman | StoppedNeedsHuman |
| transition 预算耗尽、状态损坏、未知协议、持久化失败、安全不变量破坏 | Fatal | Fatal |

固定边界如下：

- `max_repairs=0` 遇 repairable 立即 `HumanRequired(RepairBudgetExhausted)`；不 fatal；
- `max_transitions=0`、或 `transitions_used >= max_transitions` 立即 `Fatal(TransitionBudgetExhausted)`；
- `max_manual_repairs=0` 或已耗尽时，人工门只保留批准/终止，返修反馈返回 protocol error；
- 持久化计数任一超预算、overflow 或不变量不一致均为 `StateCorruption`；
- `transitions_used` 仅在 stage transition 成功 durable commit 后 +1；非法 event 不计数。

### P1-D5: 两阶段 review scope 的可验证契约

**review 周期（cycle）定义**：「初评 ≤1 + 复评 ≤1」预算以**被审产物为锚**，不以 session 为锚。一个 cycle = 同一被审产物（outline 节点 / 某 draft 的 outline_id / 某 batch_id）从首次 review 到 pass 或终态。一个 session 合法包含多个 cycle（outline cycle → 各 draft cycle → batch cycle），cycle 间互不消耗预算。session 级 review 总计数仅作指标与诊断展示，不做门控。phase 判定：cycle 内首次 review = Initial；该 cycle 返修后的下一次 review = Verification；同 cycle 第二次复评请求 → 按终态矩阵进人工门/停止。

机械校验先行；机械错误与初评 repairable finding 共用 `max_repairs=1`（按 cycle 计）。cycle 内初评最多一次；只有预算未消费且有 repairable 才调度一次聚合返修。返修与人工返修后的复评最多一次（按 cycle 计），只判定原指纹是否重现，并重跑该 invocation 的机械校验；复评绝不开放发现新问题，也不再自动返修。

scope 必须消除初评“不适用字段”的空字符串：

```rust
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReviewInvocationScope {
    Initial {
        initial_revision_id: String,
        scope_digest: String,
    },
    Verification {
        original_fingerprints: BTreeSet<FindingFingerprint>,
        repaired_revision_id: String,
        mechanical_report_ref: String,
        scope_digest: String,
    },
}
```

服务端生成并与 provider invocation 同步 durable 写入 scope；prompt builder 只消费持久化 scope。`ReviewFinding` 不新增 path/region/affected_paths，因不存在可确定性验证的字段。相应地 Verification 只允许原指纹重现判定和本 scope 的机械报告重跑；删除 changed-path 归因分支，也不得以 `changed_paths` 做校验或展示为校验依据。

scope digest 的 canonical input 是 UTF-8 的固定版本串 `review-invocation-scope/v1`、enum tag、按固定字典序的字段名、每个 scalar 的十进制长度前缀和字节；`BTreeSet` 指纹按稳定迭代序逐个编码。对该输入 SHA-256，最终形状固定为 `review_scope_v1:<64 位 lowercase hex>`；前缀、长度、hex、重算不一致均为 protocol violation。

### P1-D6: session durable state、CAS 与 wire compatibility

`WorkspaceSessionRecord` durable 字段为 `flow_kind`、`run_policy`、`run_history`、`review_invocation_scope`、`human_gate_snapshot`、`repair_reservation`、`policy_diagnostics` 与 `provider_start_ledger`。`RunHistory` 必须为：

```rust
#[serde(default, deny_unknown_fields)]
pub struct RunHistory { /* 指纹集与五个计数 */ }
```

其 `Default` 是空集合、五个 0 计数；**struct 级** `serde(default)` 是旧 JSON 缺任意 history 字段恢复的必要条件，仍拒绝未知字段。`FindingFingerprint` transparent serde 为 64 位小写 SHA-256 hex，构造和 Deserialize 均校验。

`WorkspaceSessionStatus` 增加 stable serde 值 `stopped_needs_human` 和 `failed`；不得复用 `WaitingForHuman`。`CreateWorkspaceSessionInput` 使用 `Option<WorkItemPlanSessionOptions>`；仅 WorkItemPlan 可为 Some，其他 workspace 类型显式失败；prepare 缺 run_policy 为 interactive，rollout 在创建时快照后不可改。

route handler 接收 expected revision、evaluation decision 与 gate context，并在一个 CAS 中合并 delta、reservation、next state/status、scope、snapshot、diagnostic 与 provider-start ledger。CAS 冲突必须重新读取并重评，不得重放旧 delta。CAS 成功后才能启动 provider；成功 stage transition 的同一 commit 才递增 transition 计数。

`WsOutMessage::SessionState` 允许新增兼容字段，但 UI 不改。字段 JSON path 固定为：`$.session_status`、`$.flow_kind`、`$.run_policy`、`$.run_history`、`$.review_invocation_scope`、`$.human_gate_snapshot`、`$.policy_diagnostics`、`$.provider_start_ledger`。`build_session_state` 必须从 durable record 投影，不得从 engine 内存补造 gate。

### P1-D7: 人工门快照、路由与恢复

finding fingerprint 的结构化输入固定为 `category + contract_field`，不包含 class 或自由文本 message；category 缺失的旧 finding 才回退到归一化 `class + message + contract_field` 算法。这样 reviewer 改写同类同字段问题的措辞时身份保持稳定。

复评 finding 的指纹不在原 invocation scope 集合时，不再作为 `ProtocolViolation` 失败关闭；策略评估器产出 `HumanRequired(VerificationNewFindings)`，由 `route_outcome` 按模式进入 awaiting_human 或 stopped_needs_human。scope digest 不符、机械报告缺失或不符等结构性 scope 违例仍为 `Fatal(ProtocolViolation)`。

`GateSnapshotContext` 必须提供 history、budgets、invocation、findings、repeated_fingerprints 与 trigger；`route_outcome` 是纯映射，不能再从 store 补 finding：

```rust
pub struct HumanGateSnapshot {
    pub findings: Vec<ClassifiedFinding>,
    pub repeated_fingerprints: Vec<FindingFingerprint>,
    pub attempts_used: u32,
    pub manual_repairs_remaining: u32,
    pub trigger: HumanReason,
    pub resumable: bool,
}
```

`attempts_used = repairs_used + manual_repairs_used`；`manual_repairs_remaining = max_manual_repairs.saturating_sub(manual_repairs_used)`。resumable 仅适用于 `AutoIfValid` 的 `StoppedNeedsHuman` 且无 fatal/persistence diagnostic；AwaitingHuman 原 session 可继续，completed/failed 无 gate。

恢复测试必须包含 awaiting_human 重连、stopped 显式 takeover、completed/failed 重放、generate/repair reservation 中断恢复。provider 启动次数只从 durable `provider_start_ledger` 的已启动 idempotency key 去重计数；不从 WebSocket event 或 revision 猜测。takeover 新建 interactive session，原 stopped session 的终态、历史、事件不可篡改。

### P1-D8: campaign driver protocol contract

driver prepare 显式传 `run_policy`；首条 `session_state` 校验 `$.flow_kind` 与 `$.run_policy`。计数只读取 `$.run_history.repairs_used`、`$.run_history.manual_repairs_used`、`$.run_history.initial_review_count`、`$.run_history.verification_review_count`。它只接受 `$.session_status = "stopped_needs_human" | "failed"` 作为相应终态，不能按 stage 名推断；diagnostic 只从 `$.policy_diagnostics[*].code` 读取；启动次数只从 `$.provider_start_ledger[*].provider_start_idempotency_key` 去重读取。

## Risks / Trade-offs

| 风险 | 缓解 |
|---|---|
| reviewer 不输出新 schema 字段 | 三字段 Optional + 确定性兜底；阶段 2 prompt few-shot |
| unknown enum 被 serde 通用错误吞没 | Raw adapter 显式转换并以稳定 code 测试 |
| 复评找新问题 | 无 path 猜测；仅原指纹重现 + 机械报告重跑 |
| CAS/provider 故障重复消费预算 | token、启动账本、幂等释放和恢复测试 |
| 旧路径回归 | legacy parity 测试；默认 Legacy |
| 人工返修循环 | 默认 manual 预算 3，耗尽仅批准/终止/新运行 |

## Open Questions

（无——本 change 的阶段 1 契约已经固定；阶段 2/3/4 范围由相邻 change 约束。）
