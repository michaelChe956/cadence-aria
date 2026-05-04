# N07 Design Prompt 缺失完整 Spec 内容问题分析

## 现象

N07 (design authoring) 产出的 design 文档是通用 Hexagonal 架构模板（Subject CRUD、REST API、Repository、鉴权、审计），与 `fibonacciSquareSum(n)` 函数需求完全无关。

N07 design 明确声明：
> "canonical_inputs 中 spec 文本为占位符（仅给出 REQ-001 编号，未给出验收文本）"

## 根因分析

### 1. Prompt 模板变量不包含 canonical_inputs 完整内容

`provider_context_builder.rs` 的 `prompt_variables()` 函数（第 226 行）构建模板变量时，**没有把 `input.canonical_inputs` 的完整 JSON 序列化到 prompt 中**。

它只放入了 `canonical_input_summary`（一个纯文本摘要字符串）：

| 变量名 | 内容（N07 为例） |
|--------|-----------------|
| `canonical_input_summary` | `"spec, spec gate decision, and requirement constraints"` |
| `constraint_summary` | `"requirement_ids=FR-001,FR-002,FR-003,FR-004"` |
| `projection_summary` | `"planning chain projection summary"`（固定占位符） |

**没有任何变量包含 `spec_markdown` 的实际文本内容。**

### 2. N07 Prompt 模板渲染结果

`prompt_template_registry.rs` 第 77 行：

```
[canonical_inputs]
输入 spec 与 spec_gate_decision：
{{canonical_input_summary}}
```

渲染后变为：

```
[canonical_inputs]
输入 spec 与 spec_gate_decision：
spec, spec gate decision, and requirement constraints
```

Provider 看到的 prompt 中只有摘要字符串，没有 spec 的具体内容。

### 3. 为什么 N04/N05/N06 似乎能工作？

因为它们的 `constraint_summary` 使用的是 `proposal_constraint_summary()`，其中包含了完整的 `business_intent`（即原始请求文本）：

```rust
pub fn proposal_constraint_summary(bundle: &OpenSpecConstraintBundle) -> String {
    format!(
        "proposal business_intent={} scope={}",
        bundle.proposal_constraints.business_intent.join(" | "),
        bundle.proposal_constraints.scope.join(" | ")
    )
}
```

N04/N05/N06 的 provider 通过 `constraint_summary` 看到了原始请求，所以能产出相关内容。

但 N07 使用的是 `requirement_constraint_summary()`，**只输出 requirement IDs，没有内容**：

```rust
pub fn requirement_constraint_summary(bundle: &OpenSpecConstraintBundle) -> String {
    format!(
        "requirement_ids={}",
        bundle.requirement_constraints.requirement_ids.join(",")
    )
}
```

### 4. 关键代码路径

**`design_authoring.rs` 第 64-77 行：**

```rust
let output = run_provider_node(
    state,
    provider,
    "N07",
    json!({
        "spec": spec_markdown,              // <-- 完整的 spec 文本在这里
        "spec_gate_decision": spec_gate_decision,
        "constraint_bundle_ref": state.current_bundle.constraint_bundle_id,
    }),
    "spec, spec gate decision, and requirement constraints",  // <-- 但只传了摘要
    vec![spec_projection_ref],
    requirement_constraint_summary(&state.current_bundle),    // <-- 只有 IDs
    Vec::new(),
)?;
```

**`provider_context_builder.rs` 第 252-291 行：**

```rust
Ok(BTreeMap::from([
    ("node_id".to_string(), input.node_id.clone()),
    // ... 其他变量 ...
    ("canonical_input_summary".to_string(), input.canonical_input_summary.clone()),
    ("projection_summary".to_string(), input.projection_summary.clone()),
    ("constraint_summary".to_string(), input.constraint_summary.clone()),
    // <-- 缺少 canonical_inputs 的完整 JSON
]))
```

## 影响范围

此问题不仅影响 N07，所有 planning 节点（N04-N12）的 prompt 都存在同样的问题：**canonical_inputs 的完整内容没有进入 prompt**。只是其他节点碰巧通过 `constraint_summary` 或 worktree 文件获得了足够信息。

受影响节点：
- N07 design：看不到 spec 内容 → 产出通用模板
- N08 design review：可能通过 worktree 文件看到 artifact，但 prompt 中仍不完整
- N09 design revision：revision prompt 中可能缺少 design review findings 的完整内容
- N10 readiness：缺少 spec/design 完整内容
- N11 plan：缺少 spec/design/readiness 完整内容
- N12 dispatch：缺少 plan 完整内容

## 修复方向

在 `provider_context_builder.rs` 的 `prompt_variables()` 中添加 `canonical_inputs` 的完整序列化：

```rust
let canonical_inputs_json = serde_json::to_string_pretty(&input.canonical_inputs)
    .map_err(|error| ProviderContextBuildError::Serialization(error.to_string()))?;
```

然后在 prompt 模板中添加对应的变量引用，例如 N07：

```
[canonical_inputs]
{{canonical_inputs_json}}
```

或者更精细地，在模板中直接引用具体字段：

```
[canonical_inputs]
spec:
```json
{{spec}}
```

spec_gate_decision:
```json
{{spec_gate_decision}}
```
```

## 验证方法

1. 修改 `prompt_variables()` 添加 `canonical_inputs_json` 变量
2. 修改 N07 prompt 模板使用该变量
3. 运行 fake provider 测试，验证 prompt 包含完整的 spec_markdown
4. 重跑真实 E2E，验证 N07 产出与 spec 匹配的 design

## 关联问题

- 此问题与 N16 的 prompt 上下文不足是同一类问题（canonical_inputs 未完整传递到 prompt），只是 N16 已通过 `coding.rs` 的 `canonical_inputs_for_node()` 修复，而 planning 节点（N04-N12）仍缺失。
