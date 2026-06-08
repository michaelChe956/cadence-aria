# Workspace Review Gate 与消息按需加载内存治理技术方案

## 背景

真实 E2E 测试中暴露两个相关问题：

1. Story Spec、Design Spec、Work Item 的 reviewer 容易输出过多返修意见。现有 prompt 把 High/Medium、建议改动、可执行返修项都导向 `revise`，导致非阻塞建议也会把用户卡在返修决策。
2. 已落盘 workspace 刷新后，消息气泡存在内容展示不全的问题；同时多轮 review 会让前端持有大量 prompt、output、artifact 文本，内存可涨到约 500MB。

本方案保持 review 轮次机制不变，不限制 review 次数；改造重点是收紧 review 返修触发标准，并让前端按需获取完整内容、限制大文本缓存。

## 目标

- Review 可以继续提出建议，但只有真正影响当前产物进入下一阶段可用性的 findings 才能阻止用户确认当前版本。
- 非阻塞 review 结果必须允许用户确认当前 Story、Design 或 Work Item 为最终方案。
- 刷新已落盘 workspace 后，消息气泡应能通过 API 自动或按需补全完整内容，而不是长期展示不全。
- 前端不再无限缓存大文本，避免多轮 review 后内存持续膨胀。
- Story Spec、Design Spec、Work Item 三类 workspace 共享同一套 review gate 和内容 hydration 规则。

## 非目标

- 不取消 reviewer，也不降低用户配置的 review 轮次数。
- 不让前端绕过后端状态机单方面确认阻塞性 review。
- 不在本轮重构 Coding Workspace 的独立 review 流程，除非发现共享类型或 API 必须同步兼容。

## 方案总览

采用“结构化 Review Gate + 按需内容 Hydration + 有预算缓存”的组合方案。

```text
Reviewer Output
      |
      v
Structured Review Verdict + Findings
      |
      v
Review Gate Classifier
      |
      +-- requires_revision ------> review_decision
      |
      +-- user_confirm_allowed --> human_confirm

Workspace UI
      |
      +-- review_decision: 返修决策按钮
      |
      +-- human_confirm: 确认当前版本 / 发送修改意见 / 终止

Large Content
      |
      +-- session_state 只带摘要和 content_ref
      |
      +-- 可见消息、选中 timeline node、最新节点按需拉完整内容
      |
      +-- byte-budget LRU 缓存
```

## Review Gate 分级

新增 finding 严重性分级：

| 分级 | 含义 | 是否阻止用户确认当前版本 |
| --- | --- | --- |
| `blocking` | 阻塞性问题，当前版本无法进入下一阶段 | 是 |
| `must_fix` | 必须解决，不解决会破坏后续质量或执行 | 是 |
| `strong_recommend_fix` | 强烈建议解决，系统默认应进入返修 | 是 |
| `suggestion` | 普通建议 | 否 |
| `minor` | 轻微问题 | 否 |
| `optional` | 可选增强 | 否 |

`strong_recommend_fix` 按强返修处理，和 `blocking`、`must_fix` 一样进入返修决策。这是用户已确认的产品规则。

Review Gate 计算规则：

```text
如果存在 blocking/must_fix/strong_recommend_fix:
  review_gate = requires_revision

如果只有 suggestion/minor/optional:
  review_gate = user_confirm_allowed

如果 verdict = pass:
  review_gate = user_confirm_allowed

如果 verdict = needs_human:
  review_gate = user_confirm_allowed

如果 reviewer 输出无法解析:
  review_gate = user_confirm_allowed
  verdict = needs_human
  summary = 需要人工确认
```

## Prompt 规则

Reviewer prompt 应明确要求：

- 只把影响下一阶段可用性的缺陷标记为 `blocking`、`must_fix` 或 `strong_recommend_fix`。
- 风格、措辞、文档美化、未来扩展、非必要补充不得触发强返修，只能标记为 `suggestion`、`minor` 或 `optional`。
- 每个强返修 finding 必须包含 evidence、impact、required_action。
- 没有强返修 finding 时，必须允许用户确认当前版本。
- 第二轮及后续 review 只复核上一轮强返修项是否关闭，除非 revision 新引入了真正阻塞问题，不得重新发散普通建议。

Review 输出 JSON 合约：

```json
{
  "verdict": "pass|revise|needs_human",
  "summary": "一句话摘要",
  "findings": [
    {
      "severity": "blocking|must_fix|strong_recommend_fix|suggestion|minor|optional",
      "message": "问题描述",
      "evidence": "当前产物中的具体证据",
      "impact": "为什么影响或不影响下一阶段",
      "required_action": "需要作者执行的最小动作"
    }
  ]
}
```

向后兼容：

- 若旧 reviewer 只输出 `verdict` 和 `summary`，后端继续兼容。
- `verdict=revise` 但没有 findings 时，为避免非结构化文本误伤，不直接因为“建议修改”等自然语言关键词强制返修；应进入 `needs_human/user_confirm_allowed`，让用户决定是否要求修改。
- 后端不再把 High/Medium、建议改动、可执行返修项一概升级为 `revise`。

## 后端状态机设计

`complete_review()` 从只看 `ReviewVerdictType` 改为使用 `ReviewGate`：

```text
ReviewGate::RequiresRevision:
  - 记录 review verdict 和 findings
  - 标记 artifact review_verdict = revise
  - 进入 review_decision
  - 创建 ReviewDecision paused timeline node

ReviewGate::UserConfirmAllowed:
  - 记录 review verdict 和 findings
  - 标记 artifact review_verdict = pass 或 needs_human
  - 进入 human_confirm
  - 创建 HumanConfirm timeline node
```

持久化内容：

- `NodeDetail.verdict` 保留完整 reviewer JSON，包括 findings 和 review_gate。
- `ArtifactVersion.review_verdict` 仍保持现有 `pass|revise|needs_human` 对外兼容。
- `ReviewComplete` WebSocket 事件可增加可选字段 `findings`、`review_gate`，旧前端忽略即可。

## 前端交互设计

ReviewVerdictEntry 展示分为两组：

```text
需要解决
- blocking
- must_fix
- strong_recommend_fix

可选建议
- suggestion
- minor
- optional
```

交互规则：

- `review_gate=requires_revision` 或当前 stage 为 `review_decision`：
  - 显示 `接受修订建议`
  - 显示 `补充上下文后修订`
  - 显示 `跳过，人工处理`
- `review_gate=user_confirm_allowed` 或当前 stage 为 `human_confirm`：
  - 显示 `确认使用当前版本`
  - 保留输入框发送修改意见
  - 显示 `终止`

文案规则：

- 非阻塞建议不使用“必须返修”“建议返修”作为主标题。
- 强返修结果标题使用“需要解决后再继续”。
- 可确认结果标题使用“可确认当前版本”。

## 消息按需 Hydration

刷新恢复时，`session_state` 保持轻量，只传：

- timeline nodes
- node summary
- chat skeleton
- content_ref
- artifact version metadata

完整内容来源：

| 内容类型 | API |
| --- | --- |
| NodeDetail | `/api/workspace-sessions/:sessionId/timeline-node-details/:nodeId` |
| Provider Prompt | `/api/workspace-sessions/:sessionId/timeline-node-details/:nodeId/prompt` |
| Execution Output | `/api/workspace-sessions/:sessionId/timeline-node-details/:nodeId/events/:eventId/output` |
| Artifact Version | `/api/workspace-sessions/:sessionId/artifact-versions/:version` |

自动 hydration 触发条件：

- 当前选中的 timeline node。
- 最新 active/paused node。
- 当前视口内可见的 chat entry。
- 用户展开的 prompt、output、artifact。

展示规则：

- 未加载完整内容时显示摘要和加载状态。
- 加载失败时保留摘要，并提供重试入口。
- 内容加载成功后替换气泡中的不完整展示。

## 前端内存预算

禁止把大文本同时长期存放在多个位置。

数据归属：

- `chatEntries`：只存摘要、metadata、content_ref，不存大文本。
- `nodeDetails`：存结构化轻量 detail；大 prompt/output 不常驻。
- `contentCache`：存按需加载的大文本，但必须有 byte budget。
- 组件 state：只持有当前展示必要内容，不复制长期缓存。

缓存策略：

- 使用 session 级 byte-budget LRU。
- 建议初始预算 30MB；可按实际测试调整到 50MB。
- cache key 使用已有 `content_ref` 语义。
- 超出预算时淘汰最久未访问的大文本。
- 切换 session 时清空旧 session 缓存。

## 测试方案

后端测试：

- `story/design/work_item` 中，只有 `suggestion/minor/optional` findings 时进入 `human_confirm`。
- `story/design/work_item` 中，存在 `blocking/must_fix/strong_recommend_fix` 时进入 `review_decision`。
- `verdict=revise` 但缺少结构化 findings 时进入 `needs_human/user_confirm_allowed`。
- 第二轮 review prompt 包含“只复核上一轮强返修项”的约束。

前端测试：

- ReviewVerdictEntry 按“需要解决/可选建议”分组展示。
- `human_confirm` 下可确认当前版本，即使 review 有可选建议。
- `review_decision` 下仍展示返修决策按钮。
- 刷新恢复后的 chat skeleton 能按 `content_ref` 拉取完整内容。
- LRU 缓存超出预算后淘汰旧内容。

真实 E2E 验证：

- Story、Design、Work Item 各跑一次真实 provider。
- reviewer 只给普通建议时，用户能直接确认当前版本。
- reviewer 给强返修项时，页面进入返修决策。
- 多轮 review 后刷新页面，消息气泡可补全完整内容。
- 多轮 review 后前端内存不再持续增长到约 500MB。

## 风险与约束

- Provider 可能不遵守 JSON 合约，因此后端必须保留容错路径。
- `strong_recommend_fix` 被定义为强返修，会减少用户直接确认的机会；这是当前产品选择。
- 前端按需加载会增加 API 请求数量，需要避免滚动时重复请求同一内容。
- 如果旧落盘数据没有 findings，只能按兼容规则进入人工确认，不能可靠恢复强返修分类。

## 推进顺序

1. 后端扩展 review JSON 解析、ReviewGate 分类和状态分流。
2. 修改 reviewer/revision prompt。
3. 前端展示 findings 分组和确认路径。
4. 前端内容 hydration 与 byte-budget LRU。
5. 补齐三类 workspace 回归测试和真实 E2E 检查清单。
