---
name: 分析报告：planning 节点 prompt 修复完整覆盖性确认
description: 调研 N06/N08/N09/N10/N12 是否漏修 canonical_inputs_json，结论为通过 generic 兜底模板已完整覆盖，无漏改
type: project
---

# Planning 节点 prompt 修复完整覆盖性确认

## 起因

2026-05-04 完成两个修复并验证后，《2026-05-04_分析报告_N07_design_prompt缺失完整spec内容_v1.0.md》的"影响范围"小节列出 N04-N12 全部 planning 节点的 prompt 都缺失 canonical_inputs 完整内容。修复说明（《2026-05-04_状态记录_Aria_Fibonacci_E2E_两问题修复完成_v1.0.md》）只显式提到改了 N04/N05/N07/N11/generic 模板，因此需要确认 N06/N08/N09/N10/N12 是否漏改。

## 调研路径

1. 读取 `src/runtime_units/prompt_template_registry.rs`，对照 `prompt_template_for_node` 的 match 分支与各模板 `[canonical_inputs]` 章节占位符。
2. 读取 `src/cross_cutting/provider_context_builder.rs::prompt_variables`，确认 `canonical_inputs_json` 变量是否已注册。
3. 读取各节点 authoring 模块（`spec_gate_review.rs` / `design_review.rs` / `design_revision.rs` / `plan_dispatch.rs`），对照 `run_provider_node` 调用处的 `canonical_inputs` JSON 实际字段。
4. 对照 commit `83137bb` 的 diff，确认 generic_sections 在本次修复中的改动。

## 关键证据

### 1. 模板分发只列了 4 个专用节点，其余走 generic

`prompt_template_for_node`（`prompt_template_registry.rs:24`）：

```rust
let sections = match node_id {
    "N04" => n04_sections(),
    "N05" => n05_sections(),
    "N07" => n07_sections(),
    "N11" => n11_sections(),
    _ => generic_sections(system_delta(node_id), artifact_kind(node_id)),
};
```

N06/N08/N09/N10/N12 全部命中 `_` 分支，走 `generic_sections` 兜底。

### 2. generic_sections 在本次 commit 一并修复

`git show 83137bb -- src/runtime_units/prompt_template_registry.rs`：

```diff
-        "[canonical_inputs]\n{{canonical_input_summary}}",
+        "[canonical_inputs]\n{{canonical_input_summary}}\n\n完整 canonical_inputs（JSON）：\n{{canonical_inputs_json}}",
```

generic_sections 同步追加 `{{canonical_inputs_json}}` 占位符。所有走 generic 兜底的节点自动获得 canonical_inputs_json 注入。

### 3. prompt_variables 已注册变量

`provider_context_builder.rs:251-295` 新增：

```rust
let canonical_inputs_json = serde_json::to_string(&input.canonical_inputs)
    .map_err(|error| ProviderContextBuildError::Serialization(error.to_string()))?;

Ok(BTreeMap::from([
    // ... 其他变量 ...
    ("canonical_inputs_json".to_string(), canonical_inputs_json),
]))
```

模板渲染时变量可正确填充。

### 4. authoring 模块均传入完整 canonical_inputs

| 节点 | `run_provider_node` 中 canonical_inputs JSON 关键字段 |
|------|---------------------------------------------------|
| N06 spec_gate_review | `spec`（markdown 全文）+ `clarification_record`（完整对象） |
| N07 design_authoring | `spec`（markdown 全文）+ `spec_gate_decision` |
| N08 design_review | `spec_projection_payload` + `design_markdown`（全文）+ `design_projection_payload` |
| N09 design_revision | `spec_projection_payload` + `design_markdown` + `design_review` |
| N10 readiness | `spec_projection_payload` + `design_projection_payload` |
| N11 plan_authoring | `spec_projection_payload` + `design_projection_payload` |
| N12 dispatch_authoring | `plan_projection_payload` |

均传入完整内容（markdown 全文 / projection 完整 payload / 决策对象），不只是 ref 字符串。配合 generic 模板的 `{{canonical_inputs_json}}` 占位符，provider 渲染时可见完整业务内容。

## 结论

1. commit `83137bb` 通过修改 generic_sections 兜底模板，一并覆盖所有走 generic 的节点：
   - Planning 阶段：N06 / N08 / N09 / N10 / N12
   - Execution 阶段：N16 / N17 / N18 / N19 / N20 / N24
   - Closeout 阶段：N25 / N26 / N27
2. **没有任何节点漏修**。原计划"补齐 N06/N08/N09/N10/N12 prompt 修复"无须执行。
3. authoring 数据流层面亦已完整传入，无需补充。

## 含义

prompt 模板采用"专用模板 + generic 兜底"的分层结构，修复时同时覆盖两层即可让下游节点自动受益。本次修复在 generic_sections 上的统一改动是低成本、广覆盖的实现方式。

## 后续建议（非必须）

若担心 generic_sections 后续重构时丢失 `{{canonical_inputs_json}}`，可补 1-2 个 generic 节点（如 N08 或 N12）的 prompt 渲染断言测试，与现有 `context_builder_includes_canonical_inputs_json_in_prompt`（仅覆盖 N07）形成互补防护。
