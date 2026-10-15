# Design: batch1-coding-autopilot-mainline

## Context

本 change 是批 1「coding 主流程自动化」的契约层。方案设计已完成并经 owner 全部裁定、controller 一致性裁决（X1/X2/X3）与接地审查（6 条 concern 全部吸收）。**本文件不复制设计内容**，设计与结论以下列文档为唯一权威，实施时按引用路径读取：

| 权威文档（相对仓库根） | 职责 |
|---|---|
| `cadence/designs/2026-10-15_方案设计_批量1总览_v1.0.md` | 七条主流程→包映射、17 包依赖与 7 波、C 区 19 项共享文件串行顺序、8 条未验证假设、统一验收 7 点 |
| `cadence/designs/2026-10-15_方案设计_批量1块1_聚合确认与总览_v1.0.md` | D0 plan 级延迟聚合确认（P1-a/b/c/d）、只读总览、仪表盘 attempt 状态 |
| `cadence/designs/2026-10-15_方案设计_批量1块2_分诊与choice页面_v1.0.md` | 三按钮分诊卡、`coding_operational_blocker` 门、`retry_coding_with_context`（A4 细化）、triage_summary、choice REST 逐题提交（D-1~D-5 全 A） |
| `cadence/designs/2026-10-15_方案设计_批量1块3_跨仓上游交付注入_v1.0.md` | `ResolvedUpstreamDelivery` prompt 注入、Kimi terminal 只读挂载、完成登记 refs 空回填 output |
| `cadence/designs/2026-10-15_方案设计_批量1块4_挂起检测与通知_v1.0.md` | idle watchdog（300/600/1800 秒）、`provider_idle_stall` 一次重试、A1 durable 事实投影与全局消费 |
| `.superpowers/sdd/batch1-batch2-interface.md` | 批 1/批 2 契约 A1~A6（CONFIRMED）与 C 区文件面串行顺序 |
| `.superpowers/sdd/batch1-ground-review.md` | 接地审查：17 包文件/符号全量实测、A1~A6 可行性、CONCERN 1~6 |
| `cadence/plans/2026-10-15_实施计划_批量1主流程_v1.0.md` | 实施计划：17 包×7 波顺序、红测清单、并行规则、统一验收命令 |

## Goals / Non-Goals

**Goals**：把 owner 已裁定的七条主流程落成可验收契约（specs/ delta）与可派工任务包（tasks.md），不新增范围、不重开已裁决问题。

**Non-Goals**：proposal「10 条不做清单」全部适用。设计层补充：不改 `handle_final_confirm` 引擎函数与 scheduler 判据；不改 `validate_plan_defect_finding` 本体（放宽只经 `execution_plan_defect_flow_decision` 注入校验闭包）；不改 handoff/envelope/projection/hash 与 `coding_work_item_context.rs`；不新增持久模型（triage_summary/attention 均为运行期投影）；不动批 2 的 plan 阶段对话窗口通道（coder 门与 plan 对话两通道严格分离）。

## Decisions（裁决转录，不复制论证）

### X1：P2-f 暂不做（owner 19:00/19:35）

choice 无 runner 时只显示人话提示、无「恢复运行」按钮；RecoverChoice/作废重跑/socket/message_policy 改动全部降为第二档。块 2 §3.6 末段与块 4 相关恢复引用按总览 §1 口径覆盖。

### X2：旧分诊门两动作（controller 19:25，owner 19:35 确认）

旧门 `coding_output_human_triage` 的动作集保持 `retry_coding`+`abort`（既有断言 `verification_triage_gates.rs:325` 不变），大白话新卡展示、无第三动作时退化两按钮；三按钮仅出现在新门 `coding_operational_blocker` 与块 4 `coder_provider_stalled` 门。块 2 §3.4 中旧门渲染三按钮的表述作废。

### X3：DTO 位置与 coding.ts 归属（controller 19:25）

A1 Rust DTO 放置由实施 worker 按现有风格定（P4-c 落 `src/web/coding_attention_info.rs` 同处或 `types.rs`，按 C 区 #4「P4-c DTO→P1-a 引用」分段串行）；`web/src/api/types/coding.ts` 补入 P2-e 独占清单。

### 接地审查修正摘要（C1~C6，全部吸收进 tasks.md 与实施计划，不改设计结论）

- **C1（P2-c 隐藏依赖）**：`CreateBlockedGateInput` 无 `Default` 且全仓 34 处字面量（抽样 20 处无 `..` 展开），直接加字段会波及约 20 个清单外文件，其中 `provider_failure.rs`（P4-a）、`group_review_orchestrator.rs`（P4-b）、`rework.rs`（P2-b/P3-a）为其他包独占文件。**采纳收敛方案（a）**：不给 `CreateBlockedGateInput` 加字段，`triage_summary` 经 `coding_attempt_store` 独立入口（新建门后写入的 store 函数）落盘，改动收回 P2-c 独占文件；`CodingGateRequired` 仍加 `Option` 字段（serde default），其 4 处字面量补 `None`（均在 P2-c 清单内）。选项（b）（登记 20 文件跨包补丁）因破坏独占纪律被弃。
- **C2（P2-e 隐藏依赖）**：`tests/it_product/product_coding_workspace_engine/part_04.rs:538` 是无 `..` 的穷尽 struct pattern，`CodingChoiceRequest` 加 `questions` 触发 E0027；该文件显式补入 P2-e 改动清单（仅该 pattern 补 `..`）。
- **C3（P4-a 假时钟不可实现）**：tokio 未启用 `test-util`、全仓零 `time::pause/advance`；红测改用 env 覆写（`ARIA_CODING_*_IDLE_SECS` 短值，先例 `coding_workspace_engine/tests/group_review_budget.rs:159`）或 `#[cfg(test)]` 短常量（先例 `workspace_engine/provider_drive/watchdog.rs:22-58`）；配置须在 runner/stream 启动时一次读取。
- **C4（P4-a 复用先例）**：复用 `src/product/workspace_engine/provider_drive/watchdog.rs` 与 `tests/provider_idle_watchdog.rs` 的同语义范式（真实活动重置 + choice/permission 暂停 + 耗尽收口），照抄不重造；该先例面向 workspace_engine，coding 流仍需新建。
- **C5（已裁决）**：挂起门复用块 2 三按钮卡（CoderTriageCard/summary/A4），块 4 不另做 UI；`isTriageCardGate` 的 reason_code 集合由 P4-e 纳入 `coder_provider_stalled`（C 区 #1：P2-d→P4-e→批 2）。
- **C6（文档行号精度）**：行号以实测为准；17 包清单内文件存在性已由接地审查全量实测确认（§1.1 无一缺失）；块 1 §3.2 `coding_workspace_engine_with_dummy_events` 实际在 `src/web/handlers/support.rs:487`（P1-a 仅调用不需改）；块 2 P2-c 两处「字面量补丁点」实为函数返回类型，真正字面量 4 处。

## Risks / Trade-offs

- 总览 §5 的 8 条未验证假设（owner worktree 寿命、冻结 output 历史空值、provider 题号/交付变体、adapter 取消边界、非原子确认、GET 后 git 变化、在线通知联调）均为已知风险，批 1 不处理、不阻塞主流程验收。
- 结构体/枚举加字段（`CodingGateActionType`、`CodingGateRequired`、`PlanCodingProgressDto`、`IssueLifecycleResponse`、`TerminalIsolation::Bubblewrap`、`ProviderInvocationOutcome`、WS `CodingChoiceRequest`）的编译面经接地审查逐项确认均被对应包清单覆盖（含 C1/C2 收敛后）。
- additive 回退风险：旧版前端遇到新 reason_code 按通用 blocked gate 渲染、新动作点击得 `coding_gate_action_not_allowed`（显式拒绝，非静默）；缓解=新动作只由新门下发（块 2 F-12）。

## Migration Plan

按实施计划 7 波顺序合入（波 1：P2-a/P2-b/P3-b/P3-c → 波 2：P2-c/P3-a → 波 3：P2-d/P4-a/P1-a/P4-c(3* 分段) → 波 4：P2-e/P4-b/P4-d → 波 5：P1-b → 波 6：P1-c/P1-d → 波 7：P4-e）；C 区 19 项共享文件按总览 §4 串行；全部合入后主 agent 统一验收（四条 cargo + web 三绿 + 单仓冒烟 + 主流程 7 点）。

## Open Questions

无阻塞项。owner 已确认全部决策（块 2 D-1~D-5 全 A、块 4 D-3=A、总览页列表+单详情、P2-f 暂停、旧门两动作）；X1/X2/X3 已裁决；接地审查 6 条 concern 已按上述方案吸收。
