# Design: harden-single-repo-design-weak-models

## Context

Story change（已归档）完成的全局协议层对 Design 自动生效；本 change 只补 Design 业务侧缺口。五份分析/评审材料为设计依据：`cadence/analysis-docs/2026-08-22_Design链路加固需求分析.md` 与四份 subagent 报告（代码审计、campaign 规划、判例设计、双评审交叉收敛）。关键既有事实：

- Design review 唯一走 `build_review_input`（`prompts/review.rs:15`）；`reviewer_output_contract` 其余 5 处调用均属 WorkItemPlan 家族。
- 用户反馈返修唯一生产入口 `revision.rs:47`，命中条件为「有 pending_revision_context 且无 reviewer verdict」。
- repair 链路：`is_repairable` 仅被 workspace reviewer 双 driver 消费（`drive.rs:62` 直连 / `drive.rs:270` gateway）；整块照抄示例当前即不可修复（NonceMismatch 恒带空载荷）；混合照抄（真 nonce + 示例载荷）今天被「示例载荷非 schema-valid」挡住——判例若带封装会自造可利用路径。
- Design 生成硬依赖已确认 Story Spec（lifecycle 校验），campaign 必须种入冻结上游 fixture。

## Goals / Non-Goals

Goals：单仓 Design 的边界判定可靠性（判例）、用户反馈返修契约完整性、结构契约回归矩阵、与 story 同级的实测证据链。

Non-goals 见 proposal（aggregate 分支、共享层重写、关键词 pre-gate、grammar 收紧、双重重放去重、跨 workspace 反馈入口统一、delta prompt 补全）。

## 架构决策

### D1 判例注入：单点拼接而非共享签名增参

在 `build_review_input` 内、`reviewer_output_contract(...)` 结果**之前**追加判例串（Design 时；与修改清单及 opus 评审推荐一致，判例先于输出格式/nonce 模板语义更连贯），而非给 `reviewer_output_contract` 增参改 6 处调用。理由：Design 只有这一个 review 入口；WorkItemPlan 家族传空串是无谓的签名污染；单点注入使「判例位置」测试锚定在一个 builder 上，并配排序锚定断言（判例串出现在实际输出模板之前）。

判例物理形态：纯文本三段式（产物状态摘录 + 正确判定 + 错误判定），**不含 sentinel/nonce/完整 JSON**——这是安全设计而非风格选择：带封装的判例会把「照抄载荷 + 正确 nonce」从不可能变为可能（B-1 主路径），且任何 repair 层防护都拦不住该路径。固定 ID 组（DEC-001/CMP-002/API-002/REQ-003）兼作照抄指纹。

### D2 repair 纵深：内容指纹而非 nonce 判据

`observed_nonce == "EXAMPLE_NONCE"` 判据对 `MissingJsonNonce` 无效（该码 observed_nonce 恒 None）。采用载荷指纹判据：`recoverable_value` 序列化后包含全部四个示例 ID 组合 → 不可修复 → UserTriage。误伤真实 finding 的理论风险以「降级人工可见」兜底（不静默丢弃）。同时删除 `is_repairable` 中 NonceMismatch 死分支（全仓唯一构造点恒传空载荷），附 contract 测试锁定。repair prompt 两处配套：显式 nonce 排除提示；回灌原文改剥离 sentinel 后 readable 文本（消除照抄块二次进入上下文）。

### D3 反馈入口注入清单与 compact_history 暂缓

注入 schema/fence 契约/skeleton/notes 四项；输入侧当前产物围栏三反引号改四反引号（Design artifact 含代码块是现实场景，part_04 已证）。compact_history 暂缓：resume 会话已有服务端历史，无条件注入构成双重重放；且压缩无硬 token 上限（fail-closed 全量回放），不能推导预算安全。实现上由调用方传入 fresh/resume 标记（`resume_provider_session_id.is_none()`），不在 prompt builder 内自行猜测——为将来启用留接口。

### D4 回归矩阵的单失败原因基准

表驱动负例以完整合法 Design artifact 为基准每次只动一处，保证 finding 归因稳定；「测试越界」类负例**不进入** deterministic 矩阵（validator 本就不查关键词），由 WP-1 的 candidate→finding 测试与 campaign 边界样本覆盖——两条线用测试锁定「不加关键词 pre-gate」的现状契约。

### D5 golden 不比 ID 集合相等

设计方案合法多样：DEC/CMP/API 数量与内容可比 golden 多。规范化比较限定为：heading 完整性、上游 REQ/AC 引用不丢失（禁丢不禁增）、dec_req_links 不丢失、source 覆盖不丢失、用户决策不丢失/不反转/不改绑。decision 双章节抽取（设计决策∪追踪关系）+ 键双形态锚定（author-decision-* 或 dec_id 等价映射）。

### D6 manifest 字段必填化与 author/reviewer 分离

design 版 validator 不复制 story 弱版：author/reviewer 各自记录 provider/model/version；strategy/usage/retry/超时分类/choice 必填；digest 校验 corpus 与 golden；`--paired` 实现配对校验供 baseline↔revised 对比。story 的 gate-manifest.json 因历史数据缺字段只能过 warning 版校验器，两版并存（story 版已回填，见 5bf2a31a）。

### D7 Choice followup 第四入口契约（会审 A 级补充）

用户回答 author 确认问题后的续写 prompt 当前零契约注入（`lifecycle.rs:615-678` 裸文本 → `provider_drive.rs:30-45` DeltaOnly → `prompts.rs:214-216` 原样透传）：弱模型以对话体作答将 gate 失败且决策不落章，直接击穿 golden「用户决策不丢失」。修法：在 choice followup 的 Design 分支复用 WP-2 注入件（artifact 输出 fence 契约 + Design skeleton + 结构化决策落章 contract），Story 分支字节不变负例同款；滑窗/入口覆盖测试补第四入口断言。

### D8 边界观测策略：主 gate + mini-campaign 分离

主 gate 维持 12 样本/组合（6 形态 × 2）；边界分类（抽象追踪假阳/测试越界假阴）另立 mini-campaign：D04/D05 各 5 重复 × 3 provider = 各 15 观测（0/15 的 95% 上界 <20%），只跑单轮 review 判定不跑全链，可与主 gate 分时跑。所有成功率类结论附样本量与置信上界，不以零失败直接声称可用率百分比。

## 失败处理

- 判例指纹误判真实 finding → 降级 UserTriage（人工可见），绝不静默丢弃或自动返修。
- campaign 单样本超时 → 按 story 先例区分 driver-timeout 与模型失败，600s 上限复跑一次。
- kimi 登录过期 → 重登录后全量重跑该组合。
- usage 缺失（provider 不暴露）→ 记录 `usage_unavailable` 并注明原因，不以 0 冒充、不入分母。

## 迁移策略

无持久化 schema 变更；prompt 变更仅影响新生成内容。Story/WorkItem 行为零变化由显式负例测试锁定。baseline 先于 M3/M4 prompt 改造采集，保证收益对比成立。
