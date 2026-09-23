# Design: artifact-candidate-selection

> 完整设计调查与方案权衡见 `.superpowers/sdd/2026-09-23_计划文档_change实施_plan-compile-gate-visibility_v1.0/f46-fix-design.md`（design-task 产出）与 `f46-fix-oracle.md`（oracle 裁决）。本文件为契约层设计摘要。

## Context

F-46：`extract_artifact_content`（`src/product/artifact_extraction.rs:25-38` 取首个 opening、`:75-86` 取最后 closing）在弱模型输出「前置示意 block + `<thinking>` + 最终候选」形态时跨块混吞（实证 8,150 字符污染块 vs 5,498 字符干净候选），gate 双规则拒绝，kimi 无 retry 直接 failed。

## Goals / Non-Goals

- Goals：候选边界修复（唯一 gate-passing 选择）、失败可定责（有界诊断）、预防（prompt 负面清单）、全部共享调用方一致迁移
- Non-Goals：见 proposal（不剥离 `<thinking>`、不解除 kimi retry 排除、不动 gate 禁止规则、不加 UI 语义）

## Decisions

### D1 候选选择用「唯一 gate-passing」而非「末块」（oracle 预判分歧的裁决）

末块策略在两个合法候选时静默丢一个且救不回「末块污染、前块干净」的形态；唯一 gate-passing（0/多通过均 fail-closed）满足 oracle 验收底线（能救回 F-46 原始输入：示意块不过 gate、最终块唯一通过）且歧义不静默。提取层只负责枚举与元数据，gate 校验收敛在 workspace-aware selector（`artifact_constraints.rs` 同职责模块新增 `select_workspace_artifact`），避免提取器依赖 gate 的循环。

### D2 fence 匹配状态机与历史兼容

保留现有 marker 优先级（XML `<artifact>` > fenced > unclosed tag > heading fallback）；顶层扫描进入 opening 后以嵌套 fence 状态推进到匹配 closing；外层 fence 字符必须一致、closing 长度不短于 opening；四反引号外层容纳三反引号代码块的既有测试行为保持。同长度 fence 歧义/未闭合 inner fence=不可判定→fail-closed（不猜不选末块）。已有完整候选全部失败时禁止回落 heading fallback。

### D3 调用方全量迁移到单一 selector

`provider_drive.rs`（retry 判断/Completed 提取/complete_assistant_message）、`artifact_retry.rs`（阻断原因/失败摘要）、`parsers/choice.rs`（候选不再误判 choice）、`lifecycle.rs`（恢复用同一选择）、`mappings.rs::latest_artifact_from_messages` + `types.rs::from_record`（签名带 workspace type，reload 不回退旧策略）、`coding_work_item_context.rs::latest_assistant_artifact_markdown`。生产路径禁止旧首开—末闭调用；`artifact_extraction.rs` 无类型单测保留锁定底层 marker 行为。`full_output`/`full_content` 双源选择在 `complete_assistant_message` 入口统一消费，不出现前后分歧。

### D4 诊断事件形态

挂点复用 `NodeDetail.execution_events`（`emit_execution_event` upsert、稳定 event id=node+raw hash）：kind=artifact、output=None（`build_session_state_node_detail` 会清 output，原文不进公开投影）、detail=有界 JSON（diagnostic_version/workspace_type/raw chars+sha256/逐候选行号+字节范围+sha256+gate 结果/candidate_count/passing_count/selection）。写失败→失败摘要附「artifact diagnostic persistence failed」有界提示，gate 不变、不额外启动 provider。

### D5 prompt 负面清单注入点

不建第二套 builder：`src/web/workspace_context/prompts.rs::output_schema_for`（初次）+ `src/product/workspace_engine/prompts.rs::append_author_artifact_output_contract`（revision/choice followup 共享）+ `build_artifact_retry_prompt`（非 kimi retry，与共享清单对齐去重）。`author_artifact_skeleton_example` 骨架护栏保持。split JSON 流（`WorkItemSplitProviderOutput`）不注入。

## Risks / Trade-offs

- 多候选容忍面扩大——唯一通过 + 多通过 fail-closed 是不可删护栏（REQ-ACS-01 scenario 钉死）
- fence 状态机对历史三反引号正文的兼容——保留既有 inner code block/四反引号测试全绿为验收
- `latest_artifact_from_messages` 签名变化波及 session reload/coding fallback——全量迁移不留双语义（D3）
- 诊断 detail 若直接进 chat 展示可能影响体验——只 durable，UI 不新增承诺

## Migration Plan

1. extraction scanner（含元数据）→ 2. typed selector + 全调用方迁移 → 3. 诊断事件 → 4. prompt 三注入点 → 5. 跨四类型表驱动回归 → 每步既有测试全绿，最后统一走门禁。

## Open Questions

（实施首步需验证）F-46 第 270-361 行候选用当前 Rust validator 实跑的通过结果——设计阶段未运行；若实跑不通过，scanner 仍正确枚举但「救回本案」验收需以实跑结果修正。
