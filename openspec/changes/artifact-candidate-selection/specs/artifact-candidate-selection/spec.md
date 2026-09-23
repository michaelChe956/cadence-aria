# artifact-candidate-selection Delta

## Purpose

为 Story/Design/Work Item/legacy Work Item Plan 四类共享的 Markdown artifact 提取建立「顶层候选枚举 + workspace-aware 唯一 gate-passing 选择」契约：弱模型输出的前置示意 block、过程思考不再污染最终候选的提取边界；零通过或多个通过保持失败关闭；选择与失败均带可定责的有界诊断。

## ADDED Requirements

### Requirement: 顶层候选枚举与唯一选择（REQ-ACS-01）

artifact 提取 SHALL 枚举 provider 输出中的**顶层完整** fenced artifact 候选（含既有 `<artifact>` XML marker 优先级与 fallback 兼容），SHALL NOT 再以「首个 opening 至最后 closing」的跨块区间作为候选。对每个完整候选，系统 SHALL 以当前 workspace type 的既有 artifact gate 逐个校验；**恰好一个候选通过时** SHALL 选中该候选作为 author 产物。通过数为零或大于一时 SHALL fail-closed 失败：零通过保留全部既有阻断原因并附候选位置上下文；多个通过代表语义歧义，MUST NOT 静默选择「最后一个」或「最长一个」。存在完整候选但全部失败时 SHALL NOT 回落 heading fallback 猜测产物。嵌套 fence 边界不可判定（同长度 fence 歧义、未闭合 inner fence）时 SHALL fail-closed，不猜测候选边界。既有 gate 对所选正文的禁止 token（nested artifact fence、`<thinking>` 等）SHALL 保持硬拒绝零放宽。

#### Scenario: 前置示意 block 加唯一有效候选可恢复

- **WHEN** provider 输出含一个缺必需 heading 的前置示意 artifact block、块间 `<thinking>` 过程文本、以及一个完整合规的最终候选 block
- **THEN** 系统枚举两个顶层候选，示意块被 gate 拒绝、最终块唯一通过并被选中为产物；过程文本不进入产物，节点成功不触发 retry

#### Scenario: 两个有效候选歧义失败

- **WHEN** provider 输出含两个各自完整通过 gate 的候选 block
- **THEN** 系统不选择任一候选，节点以候选歧义失败关闭，诊断记录两个候选的位置与 hash

#### Scenario: 零通过不回落 heading 猜测

- **WHEN** 存在一个或多个完整候选但全部未通过 gate
- **THEN** 系统失败关闭并保留逐候选阻断原因，不从候选外文本或 heading fallback 生成产物

#### Scenario: 同长度嵌套 fence 歧义失败关闭

- **WHEN** 候选正文内存在与外层同长度的 fence 且无法判定配对边界
- **THEN** 系统不猜测边界，该候选按不可判定失败关闭

#### Scenario: 所选正文污染仍硬拒绝

- **WHEN** 唯一被选中的候选正文内部含 `<thinking>` 或 nested artifact fence
- **THEN** 既有 gate 硬拒绝该候选，系统失败关闭，提取边界修复不构成放行

### Requirement: 候选选择诊断落盘（REQ-ACS-02）

artifact 候选选择发生或失败时，系统 SHALL 在当前 timeline node 的 execution events 写入一条有界结构化诊断事件（稳定 event id、kind=artifact）：包含 workspace type、provider 完整输出的字符数与 SHA-256、每个候选的 opening/closing 行号与字节范围与 SHA-256、逐候选 gate 通过性与阻断原因、candidate_count/passing_count 与最终 selection（unique/none/ambiguous）。诊断 SHALL NOT 复制完整 provider 原文（原文已持久化于 session assistant message/streaming content）。诊断持久化失败 SHALL 可见（失败摘要附有界提示）但 SHALL NOT 改变 gate 判定或触发额外 provider 启动。

#### Scenario: 失败可定责

- **WHEN** author 产物提取失败（零通过或歧义）
- **THEN** 调查者可从该诊断事件的候选边界与逐候选 gate 结果区分「模型输出污染」与「候选选择行为」，无需读取完整原文即可定位首个被拒候选的位置

#### Scenario: 诊断写失败不放宽 gate

- **WHEN** 诊断事件持久化本身失败
- **THEN** gate 判定与失败关闭行为不变，失败摘要携带「artifact diagnostic persistence failed」有界提示，不静默吞掉

### Requirement: 弱模型输出负面清单教学（REQ-ACS-03）

Story/Design/Work Item/legacy Work Item Plan 的 author prompt（初次 output schema、共享 author output contract、非 kimi 的 artifact retry contract）SHALL 注入一致的负面清单教学：最终响应只生成一个顶层完整 artifact block；过程说明与思考必须在最终 fence 之外且不输出 `<thinking>` 标签；正文内代码块用四反引号外层；prompt 中的骨架/示例不得作为候选回显。该教学为预防层，SHALL NOT 改变 REQ-ACS-01 的 gate 判定语义。Work Item Plan 的 split JSON 流 SHALL NOT 注入该 Markdown 教学。

#### Scenario: 负面清单覆盖初次与返修

- **WHEN** 构建任一 workspace type 的 author 初次 prompt 或 revision/retry prompt
- **THEN** 输出契约包含该负面清单，且各注入点文案语义一致、无第二套独立实现

#### Scenario: split JSON 流不注入

- **WHEN** 构建 Work Item Plan split/outline 的 JSON 结构化输出 prompt
- **THEN** 该 prompt 不含 Markdown artifact 负面清单，JSON validator 契约不变
