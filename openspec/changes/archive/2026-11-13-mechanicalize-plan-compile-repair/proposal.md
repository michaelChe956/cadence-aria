# Proposal: mechanicalize-plan-compile-repair

## Why

同一多仓需求在 2026-10-30~31 连续多次生成 Work Item Plan 失败，owner 2026-10-31 裁定「**这样不可控性太高了**」。现场为 4 次真失败 + 1 次基础设施假失败，其中三类与模型能力无关：

1. **可机械判定的契约被压给模型一次写对**：SC author prompt 固定教学块约 15.5K 字符、硬措辞约 63 处（实测），加模板/上下文后约 31K 字符。而 prompt 里已**逐字**写了被违反的规则（`prompts/sc_author_teaching.rs:41` 明写 `kind=single_repository` 却填 `intent_target_logical_repo` 会被拒），模型照样违反 —— 「再写一遍规则」这条路已证伪。
2. **可机械判定却未接的平台能力**：lowering 诊断 `kind: None`（`work_item_plan_compiler/lower.rs:1123`），typed 分类器无对应变体，落 `Unknown` → `UnknownStop`（`planning_failure.rs:404`）；教学重驱白名单只收 4 个 parse 码（`web/.../run/single_candidate.rs:84-100`）→ 该类失败**零自修、零教学、立即终态**（`:1301-1313`）。
3. **同一事实在两处图模型上判定不一致**：Stage1 结构门用 typed 有效图（`canonical_input` + `change_order`，`compile_support.rs:435`），Final Compile 用 `input_contracts` 推出的 `depends_on` + `change_order` 拍平图（`:1015-1037`、`:783-826`），且后者**丢失边来源**，只能拼一串人话报错。同一份计划可能「早期放行、approve 后炸」。

**根问题定性**：责任错配（把机械契约交给模型承载）+ 通道错位（机械失败唯一出口是「重写整份文档」）。

## What Changes

- **契约四层下沉**：L0 平台确定性自修 / L1 结构投影补全 / L2 精确缺口 / L3 业务取舍；模型只负责业务内容。
- **lowering typed fault**：新增独立 `LoweringFault`（实现于 `CompilerDiagnostic` 新字段，**不进** wire 枚举 `PlanningFailureKind`），在 `lower.rs:1121` 构造点赋值；教学资格从「parse 码白名单」改为「fault 在场且事实齐全」（`MODIFIED REQ-WSC-09`），并按裁决 §1.5 把四类 typed IR 诊断（`unknown_done_when_ref` 等）与确定性结构拒收纳入该唯一资格清单。
- **计划意图分支等价解析**：`kind=single_repository` 而值填在兄弟 key 时，按成员表唯一映射解析采用（**不改 source**，产 `equivalence_canonicalization` 审计）；不唯一或双命中一律不猜。
- **环诊断带来源、不删边**：typed witness 逐跳标来源；平台**不得**删依赖边消环（现场环是 `canonical_input` 逆仓序的真实数据流），只做结构化教学 → 同指纹重复则升级业务取舍并一次问清。
- **图奇偶性**：Stage1 与 Final Compile 对同一 fixture 的「有效边集合 / 成环结论 / witness 首跳」必须一致；不一致先登记缺陷单。
- **一次答完**：编译失败 verdict 必须填 `owner_question` + ≥2 `owner_options`（前端 `gate-next-step.tsx:225` 已渲染，现为 `None`）。
- **可观测性**：追加写 `candidate-attempts.jsonl`（现状是单文件覆盖写，跨尝试序列丢失，导致「偶发用例名抓不到」）。
- **prompt 体量闸门**：固定教学块字符数与硬措辞数由计量测试锁定（REQ-CRP-05），条款只在平台侧已覆盖后才允许从 prompt 删除。

## Impact

- 受影响 spec（delta）：新能力 `work-item-plan-compile-repair`（REQ-CRP-01..05，05 为 prompt 体量闸门）；`work-item-plan-single-candidate`（**MODIFIED** REQ-WSC-09；ADDED REQ-WSC-23/24/25）；`work-item-plan-conversational-gate`（ADDED REQ-CG-08/09）。
- 代码面：`work_item_plan_compiler/{types,lower}.rs`、`web/workspace_ws_handler/run/single_candidate.rs`、`workspace_engine/{compile/single_candidate,compile_failure_surface,plan_projection}.rs`、`draft_batch/compile_support.rs`（需拆文件，已 2309/2310 行）、`models/human_gate.rs`、`web/src`（仅消费既有字段）。
- 兼容：`CandidateCompileStatus` 是 `deny_unknown_fields` 单文件覆盖写，新字段一律 `default + skip_serializing_if`；`final_compile_prepare_failed` 稳定码保留一个版本周期。

## Non-Goals

- **不新增对已持久/已评审 source 的直接 patch**（见 design「与 REQ-SG-02A 的关系」）。
- 不让平台自动补 `reviewer_check_refs`（会造假验证证据）。
- 不让平台自动改 design 的 `change_order`。
- 不推翻「approve 必重跑 Final Compile」。
- 不处理 `outbound_queue_full`（会话出站可靠性，`degraded_session` 面，另开工单）。
- O1 已由 owner 裁决（2026-11-08 裁决 §2.2/§2.3）：**不改 `REQ-SG-02A`，现有 `contract_autorepair` 的内容 autorepair 迁原授权 author**（落地工作包见 tasks 第 8 节；本 change 不新增对任何 source 的内容 patch）。
