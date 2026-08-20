# Proposal: tolerate-explained-empty-open-items

## Why

Story Spec 生成（claude_code + glm-5.3 实测）首轮失败率超过 50%：模型已产出结构完整的 artifact，但 `## 待确认项` 写成「无待确认项。Issue 已明确需求……」这类"空标记 + 解释"形式，被 `unresolved_open_item_findings` 误判为未解决开放问题，整份 artifact 被拒后触发自动续写。未解决 cue 列表（"待确认"、"未决"、"未明确"）本身包含在否定表述（「无待确认项」「未决问题」）中，属于校验器自相矛盾的误杀。

## What Changes

- `open_item_section_is_resolved` 容错：若待确认项节内首个非空行以空标记开头（前缀匹配 `无待确认项` / `无` / `暂无` / `none` 等既有 marker 集合），整节判为已解决，不再对后续解释行执行 cue 检查。
- Story 的 `open_item_policy_hint` 补充指示：若无开放问题，`## 待确认项` 正文只写「无待确认项」，解释性内容写入其他章节。
- 不改变 artifact 输出内容要求：headings、[REQ-*]/[AC-*]、source id 等约束与重试行为全部保持不变。

## Capabilities

### New Capabilities
- `workspace-artifact-open-item-validation`: workspace Story artifact 中「待确认项」章节的已解决/未解决判定规则。

### Modified Capabilities

（无）

## Impact

- 代码：`src/product/workspace_engine/artifact_constraints.rs`（`open_item_section_is_resolved` 及空 marker 前缀判定）、同文件 Story `open_item_policy_hint` 文案。
- 测试：`artifact_constraints` 相关单测新增"空标记 + 解释"通过用例、真实失败样本回归用例；既有"真未解决待确认项仍被拒绝"用例保持通过。
- 对外接口无变化；不修改重试协议、不修改 reviewer 校验规则。
