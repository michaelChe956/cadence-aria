# Proposal

## Why

F-46 实证缺陷：`kimi_code` author turn 正常结束（两次结构化选择成功、`end_turn`、usage 落盘），但 Story Spec 因 artifact extraction 的「首开—末闭」跨块区间策略失败——provider 原文含一个**前置示意 artifact block**（自检推演）与一个**真实最终候选 block**，提取器把两者连同中间 `<thinking>` 文本拼成一个约 8,150 字符的污染 artifact（真实候选仅 5,498 字符且自身零污染），被 workspace artifact gate 双规则拒绝（nested fence + `<thinking>`）；且 kimi 被排除在自动 artifact retry 之外，节点直接 failed。

该缺陷不是 kimi 特有：提取器与 gate 为 Story/Design/Work Item/legacy Work Item Plan 四类共享，啰嗦型弱模型输出（过程示例 + 最终候选并存）全体暴露；F-46 是首次真实命中。

## What Changes

1. **共享提取层改为顶层候选枚举 + workspace-aware 唯一选择**：不再使用「首个 opening → 最后 closing」跨块区间；枚举顶层完整 artifact 候选，对每个候选运行当前 workspace type 的既有 gate，**恰好一个通过才选中**；零通过或多个通过均 fail-closed（歧义失败，不静默选末块/最长块）。gate 硬拒绝语义零放宽：所选正文内的禁止 token 仍硬拒绝。
2. **失败诊断落盘**：候选选择/失败时在 timeline node 写有界结构化诊断事件（候选行号/字节范围/SHA-256、逐候选 gate 结果、最终 selection=unique/none/ambiguous），不复制完整原文（原文已持久化于 session assistant message/streaming content）。诊断写失败可见但不放宽 gate。
3. **弱模型 prompt 负面清单**：在既有三注入点（初次 output schema、共享 author output contract、artifact retry contract）统一注入「单一最终顶层 block、思考过程在 fence 外、正文需代码块时外层用四反引号」教学，覆盖四 workspace 类型；split JSON 的 Work Item Plan 流不动。
4. **全部共享调用方迁移到同一 selector**（provider drive、artifact retry、choice 检测、lifecycle 恢复、session reload 映射、coding context），禁止生产路径残留旧首开—末闭提取。

## Capabilities

### New Capabilities

- `artifact-candidate-selection`: 共享 artifact 候选枚举与唯一 gate-passing 选择的提取边界契约（含失败诊断与 prompt 负面清单教学面）

### Modified Capabilities

- `story-pipeline-weak-model-hardening`: 「一次成功」判据中「单一完整 artifact fence」的 raw 输入边界改为「唯一 gate-passing 候选」——前置无效候选 block 不再使整轮失败，两个有效候选仍失败
- `kimi-code-provider-integration`: **不修改**（kimi 不自动 artifact retry 的 SHALL 保持；此处仅显式登记非目标，防止误读）

## Non-Goals

- 不解除 kimi 的 artifact retry 排除（主 spec SHALL；未来须另立 change 并以 resume 稳定性实证为前置）
- 不做 `<thinking>` 剥离层（无诊断锚点达不到 F-41 精度门槛；会把提取边界错误信号静默抹掉）
- 不改 AskUserQuestion 桥接与 auto permission 语义（F-46 已证无辜）
- 不改 artifact gate 的禁止规则本身（fail-closed 契约不动）
- 不为诊断新增 WebSocket/UI 公开语义（只做 durable 可查）
