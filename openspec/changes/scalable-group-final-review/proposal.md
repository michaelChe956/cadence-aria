# Scalable Group Final Review

## Why

Group Final Review 目前把一个 WorkItemGroup 内所有 Work Item 的完整审查材料放进单次 Provider 调用，Prompt 体积随 Work Item 数线性增长，超出 Provider 可靠遵循末尾输出契约的范围。

实测证据：

| 场景 | Prompt 大小 | 结果 |
|---|---:|---|
| 单项 Code Review | 16,967 B | 输出结论 JSON |
| 单项 Code Review | 20,976 B | 输出结论 JSON |
| 组级审查第 1 次 | 82,497 B | 未输出结论 JSON |
| 组级审查第 2 次 | 83,159 B | 未输出结论 JSON |
| 组级审查第 3 次 | 84,676 B | 未输出结论 JSON |

第 3 次失败时 Provider 已产生 Turn Completed 与 MessageComplete，并输出完整自然语言审查结论，只遗漏最终 JSON。因此故障不是 Provider 中断、上游不可用或用户取消，而是单次输入过大导致末尾输出契约失效。

两项 Work Item 的 84,676 B 构成为：两份完整 Reviewer Projection 31,186 B、重复的 EvaluationContextPack 27,257 B、完整变更 diff 15,777 B、重复公共契约 5,250 B、其他上下文 5,206 B。其中 EvaluationContextPack 内嵌的 Reviewer Projection 与前段材料逐值相同。

按当前构成外推，20 个 Work Item 的单次 Prompt 约 754 KB。即使删除重复内容、公共契约去重并把 diff 限制到 12 KB，仍约 332 KB，是已验证可靠区间的十余倍。单纯减载无法满足容量目标。

## What Changes

将组级审查从"单次全量调用"改为"Rust 侧确定性编译 + 分片语义审查 + 全局归约"。

1. 新增组级审查材料编译能力：从权威 Binding 编译不可变材料快照，并在 Rust 侧完成契约匹配、写入范围、Commit 与证据一致性、Requirement 聚合等确定性检查。
2. 新增分片编排能力：按依赖与共享写入范围亲和性把 Work Item 分片，每片最多 4 个，逐片执行语义审查。
3. 新增全局归约能力：最终调用只接收全局单位摘要、各片结论、跨片可疑关系与跨界变更片段，产出最终结论与交付叙事。
4. 新增单次 Prompt 字节预算门禁：超过硬上限时不调用 Provider，改为可诊断的材料溢出门禁。
5. 新增 Unit 审查结论身份快照，使组级聚合不再依赖缺少 Work Item 身份的既有报告记录。
6. 扩展失败语义：区分单片失败与归约失败，支持仅重试失败环节。

## Impact

- 受影响 capability：
  - 新增 `group-review-sharding`
  - 修改 `group-final-review-triage`
- 受影响行为：组级审查的材料组织方式、Provider 调用次数、失败与重试粒度、组级审查产物的持久化结构。
- 不受影响行为：单项 Code Review 的材料与结论、最终用户可见的 InternalPrReview 结论形态、既有权威 Finding Target 校验规则。

## Non-goals

1. 不修改既有单项 Reviewer Projection 的渲染文本，不改变已持久化的 Execution Context Hash，不通过提升 Renderer Version 绕过既有身份绑定校验。
2. 不让 Provider 读取工作树之外的审查材料文件。
3. 不在本变更接入 Provider 原生 JSON Schema 或 Function Call 能力。
4. 不引入 Provider 特判；三家 Provider 使用同一材料协议。
5. 本变更容量目标为单组最多 20 个 Work Item；更大规模的树形归约不在范围内。
6. 不以放宽字节硬上限的方式容纳材料增长。
