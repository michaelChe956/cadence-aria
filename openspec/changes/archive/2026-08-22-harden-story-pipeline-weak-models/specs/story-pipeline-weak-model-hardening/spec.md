## Purpose

约束 Story Spec 链路（author artifact 一次成功 + 全部 sentinel 消费方的结构化协议）在弱能力模型下的输出协议、prompt token 预算、severity 归并与实测 campaign 验收，使链路对弱模型可用且产物语义不变。

## ADDED Requirements

### Requirement: Story author artifact 一次成功（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

标准 Story author 的"一次成功"定义为：首次 provider turn 输出的候选产物通过 artifact gate（单一完整 artifact fence、一级标题、必需 heading、稳定 ID、追踪 token、禁止项），未触发自动 retry。

#### Scenario: author 首次通过
- **WHEN** author 首次输出即为完整合规 artifact
- **THEN** 不触发 `build_artifact_retry_prompt`，记一次 author 成功

### Requirement: sentinel 协议（全局统一，nonce 单点校验）（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

所有设置 `StructuredOutputContract` 的请求（workspace review、aggregate story/design author、coding review、image prompt iteration、work-item split、fake/test controls）统一：开始标签携带 nonce 属性；JSON 顶层含 `"nonce"` envelope 字段；结束标签为无属性标准闭合标签。nonce 校验以开始标签属性为准，JSON 内 nonce 缺失或不一致时拒绝。仓库内不得存在第二套 sentinel 解析实现（`workspace_engine/parsers.rs` 的重复实现收敛到 `cross_cutting/structured_output.rs`）。

#### Scenario: 新格式解析成功
- **WHEN** 输出 `<ARIA_STRUCTURED_OUTPUT nonce="N">{"nonce":"N",...}</ARIA_STRUCTURED_OUTPUT>`
- **THEN** 解析成功；`nonce` 作为 envelope 字段在业务 payload 反序列化前剥离

#### Scenario: JSON nonce 不一致被拒绝
- **WHEN** JSON 内 nonce 缺失或不等于 N
- **THEN** 解析失败，错误码可区分，进入既有可修复错误通道

#### Scenario: 旧格式不兼容
- **WHEN** 输出结束标签带 nonce 属性（旧双闭合格式）
- **THEN** 解析失败，报可区分错误（不尝试兼容），进入可修复错误通道

#### Scenario: 闭合标签后尾随文本
- **WHEN** 闭合标签 `</ARIA_STRUCTURED_OUTPUT>` 之后存在文本
- **THEN** 该文本剥离进 readable 输出，不影响解析；sentinel 内 JSON 与闭合标签之间仅允许空白，否则失败

#### Scenario: 消费方回归
- **WHEN** 协议切换提交
- **THEN** workspace review、aggregate author、coding review、image prompt iteration、work-item split、streaming fake 的现有测试全部通过（同一工作包内原子切换，不出现中间态）

### Requirement: JSON 受限恢复（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

仅在已定位且 nonce 匹配的 sentinel 内容内部做恢复：用 string/escape-aware 状态机提取唯一完整顶层 JSON 对象；允许候选外仅存在空白或一层完整 code fence 包裹；存在多个候选对象、嵌套异常或超过字节/深度上限时失败。恢复在 parser 内完成，不增加 provider 轮次。

资源上限（默认值，常量可配置）：`MAX_JSON_BYTES=65536`、`MAX_JSON_DEPTH=32`；超过上限即失败。

#### Scenario: fence 包裹恢复
- **WHEN** sentinel JSON 被一层 ``` 包裹
- **THEN** 恢复解析成功，不触发 retry

#### Scenario: 多候选失败
- **WHEN** sentinel 内存在两个顶层 JSON 对象
- **THEN** 解析失败（不得任选其一）

#### Scenario: 上限临界
- **WHEN** JSON 深度恰为 `MAX_JSON_DEPTH` 且结构完整，或字节数恰为 `MAX_JSON_BYTES` 且可解析
- **THEN** 解析成功；深度为 `MAX_JSON_DEPTH+1` 或字节数为 `MAX_JSON_BYTES+1` 时失败

### Requirement: few-shot 示例（防照抄）（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

author artifact 契约（初次、修订 `build_revision_full_prompt`、artifact retry 三个注入点）与 reviewer 裁决契约末尾必须包含最小同构输出示例。防照抄规则分两类：
- sentinel 消费方（reviewer 等）：示例内 nonce 使用固定占位值 `EXAMPLE_NONCE`，任何请求不派发该值，照抄示例无法通过 nonce 校验；真实 nonce 仅出现在示例之后的输出模板中。
- author artifact：示例为不含 REQ/AC 内容的结构骨架（仅一级标题 + 必需 heading 占位），照抄无法通过 artifact gate（缺稳定 ID 与追踪 token）；不引入 sentinel nonce 语义，不改动 artifact 持久化 schema。

#### Scenario: reviewer 照抄示例被拒绝
- **WHEN** reviewer 输出与示例完全一致（含 EXAMPLE_NONCE）
- **THEN** nonce 校验失败，进入可修复错误通道

#### Scenario: author 照抄骨架被拒绝
- **WHEN** author 输出与示例骨架一致（无 REQ/AC 内容）
- **THEN** artifact gate 判定失败，触发差分 retry

#### Scenario: 注入点覆盖
- **WHEN** 构建初次/修订/retry author prompt 或 reviewer prompt
- **THEN** 各自契约末尾含对应示例，reviewer 示例为完整 verdict JSON 骨架

### Requirement: severity 三档（live/回放双入口）（SHALL）

live provider payload 的 findings severity 只接受 `blocking`/`must_fix`/`suggestion`（出现旧档位值即 schema 校验失败）；持久化/历史回放读取旧 6 档并归一化，归一化结果持久化与 API/WebSocket 输出只含三档；Web 前端类型与渲染同步三档。旧档位完整映射：`blocking→blocking`、`must_fix→must_fix`、`suggestion→suggestion`、`strong_recommend_fix→must_fix`、`minor→suggestion`、`optional→suggestion`；未知旧值 SHALL 判定失败（不静默降级）。`impact` 并入规则：live schema 不再接受独立 `impact` 字段；历史回放遇到非空 `impact` 时以 `"\n影响：" + impact` 追加到 `message` 后，且同一记录 SHALL 只追加一次（幂等，重复回放不重复追加）。

#### Scenario: live 旧档位拒绝
- **WHEN** reviewer 新输出含 `strong_recommend_fix`
- **THEN** schema 校验失败并进入差分 retry

#### Scenario: 历史回放归一化
- **WHEN** 读取历史裁决含 `minor`/`optional`/`strong_recommend_fix`
- **THEN** 分别映射为 `suggestion`/`suggestion`/`must_fix`，round-trip 后不再出现旧值

#### Scenario: 全栈一致
- **WHEN** severity 变更提交
- **THEN** 后端单测、`web` 的 `pnpm test` 与 `pnpm tsc -b` 全绿

### Requirement: prompt 滑动窗口与 token 验收（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

author 增量 prompt（`build_prompt` 与 `build_revision_full_prompt`）与 reviewer prompt 的历史重放为滑动窗口：最近 2 轮原文保留；更早轮次压缩为确定性（非 LLM）结构化摘要，摘要至少含：round 标识、artifact 版本引用、review verdict、finding ID/severity/required_action、未关闭强返修项全文、choice 审计 ID 与答案及影响的 REQ/AC、retry 失败原因一行。当前最新版 artifact 与 choice 审计记录永不压缩；reviewer prompt 保留 `canonical_inputs` 全文与全部未关闭 blocking/must_fix finding 全文，中间版本 artifact 以相邻版本 diff 摘要呈现（diff 生成失败时保留全文，不静默丢弃）。摘要生成失败时 fail closed（退回全量重放）。

对 provider 原生 resume：先实测各 provider resume 的 input-token usage；若证实服务端重放历史导致窗口收益无效，则在窗口边界切换 fresh session 并携带确定性压缩上下文（最新 artifact、canonical inputs、全部未关闭强 finding、choice 审计、版本引用）。

验收指标：同一需求语料、相同轮数、相同 provider/model 与 resume 策略下，修订轮 provider 返回的 input-token usage 较基线下降 ≥40%（fresh 与 resume 分列报告）；prompt 字符数仅作单测代理指标并单独标注。

唯一判定公式：对每个 provider×strategy（fresh、resume 分列），取基线 campaign 与 gate campaign 的**一一配对**样本，计算每样本 `revised_input_tokens / baseline_input_tokens`，对均值判定 ≤0.60 为达成。配对键为 `case_id=(shape_id, repetition_id, round, strategy)`：同一 case_id 在基线（`run_kind=baseline`）与改后（`run_kind=revised`）各跑一次且各有一个非 retry 样本；每个受支持的 provider×strategy 必须采集等量配对的 baseline/revised 样本（不支持 resume 的 provider 该 strategy 不参与并在报告声明原因）；baseline 样本可复用于多个 gate 样本时，聚合规则为「同一 case_id 只计一次均值贡献」，不得混算重复贡献。`cache_read` 与 completion 不计入分子分母，cache 命中分列报告；触发 retry 的样本不进入 token 分母但必须在 manifest 中记录。

#### Scenario: 修订轮压缩
- **WHEN** 会话已有 ≥3 轮历史并触发修订 prompt
- **THEN** 早期轮次以结构化摘要出现，最近 2 轮原文保留，最新 artifact 全文保留

#### Scenario: 未关闭 finding 可定位
- **WHEN** 第 1 轮 must_fix 到第 4 轮仍未关闭
- **THEN** reviewer prompt 仍含该 finding 全文与 round/artifact 版本引用

#### Scenario: token 验收
- **WHEN** campaign 复测完成
- **THEN** 报告各组合修订轮 input-token usage 对比（≥40% 下降为 release gate），字符数单独标注

### Requirement: 产物语义等价验收（golden 规范化 diff）（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

golden 以规范化 schema 比对：必需 heading 集合、REQ/AC/NFR 稳定 ID 集合、AC↔REQ 关联、source id 覆盖、用户确认决策（choice ID、答案、绑定 REQ/AC）。允许差异：措辞、排序、非约束性说明；禁止差异：需求/验收增删或弱化、source 丢失、用户决策反转。diff 脚本输出 machine-readable pass/fail 与字段差异。golden 语料 ≥5 个不同形态需求（含待确认项、含返修轮），golden 冻结只读并带 digest，运行不得写回 golden。

#### Scenario: golden 通过
- **WHEN** campaign 产物与 golden 规范化比对
- **THEN** 无禁止差异则 pass，否则 fail 并列出差异字段

### Requirement: 成功率 campaign（release gate）（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

campaign 运行以 machine-readable manifest/schema 冻结：每个样本必含 `case_id=(shape_id, repetition_id, round, strategy)`、`run_kind=baseline|revised`、provider、model/version、provider version、issue/session id、语料形态 id、round 数、三口径结果、retry 分布、失败分类、token usage（input 与 cache_read 分列）、时间戳；golden 只读带 digest。

固定语料 ≥5 需求形态（含待确认项、含返修轮、含用户 choice），三组合 {claude code+glm-5.3, kimi+deepseek-v4-flash, pi+deepseek-v4-flash}；开发迭代期每组合 ≥5 样本；最终验收每组合 20 样本，语料配比均衡（5 形态 × 4 样本，每形态 ≥3），每次独立 issue/session。

分口径报告：author 首次通过率、reviewer 首次 syntax+schema 通过率、full-chain 一次成功率（author 与 reviewer 均未触发自动 retry），及 retry 分布与失败分类。full-chain 一次成功率 ≥95% 为 release gate（20 样本下至少 19/20），未达标时验收任务不得勾选。

#### Scenario: gate 判定
- **WHEN** 某组合 20 样本 full-chain 一次成功 <19 次
- **THEN** 验收失败，change 不得进入完成流程

#### Scenario: manifest 完整
- **WHEN** campaign 结束
- **THEN** 每个样本均在 manifest 中可追溯，含形态 id 与 usage 字段
