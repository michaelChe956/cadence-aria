# 提案：把事实送到正确的决策点（WIG 事实投影修复）

> 状态：草案 —— **owner 已批准实施**（2026-11-13）。本 change 是 `feat-wig-optimize-11-08` 分支上 WIG 优化链的**新一环**，取代此前「扩建旧串行链」的路径。

## Why

用户的原始诉求是三条现象：**① work item plan 生成慢**（启动一次 5–10 分钟才出 token、输出很久）；**② IR 校验失败 → 教学重驱 → 再生成**；**③ reviewer 失败 → 再生成 → 重复多次直到额度耗尽、人工介入**。

逐条核查代码后，三条现象各自有一个**已被定位、且尚未被修掉**的机制缺陷。它们不是「功能不够」，而是**平台把自己已经算好的事实丢掉**、或**平台在模型自报的两个轴矛盾时只听更严的那一轴**：

1. **教学投影有损**：`CompilerDiagnostic` 自带 `field`（缺失字段名）与 `repair_example`（正确形状 `- {field}: value`），`parse.rs` 也确实填了；但 `single_candidate_compile_preflight.rs:71-79` 的教学载荷只保留 `code:line:message` ⇒ 重驱作者轮**工具调用 0 次、纯盲改** ⇒ 结构类失败终局 `failed`、plan 停在 draft、**无人接手**（实测 `missing_section` 类 3/8）。同文件 lowering/IR 两类走的是**完整 typed 载荷** ⇒ parse 类是唯一有损的一类；`sc_compile_teaching.rs:100-113` 已为 `unknown_structured_key` 做过同构补救（附合法集与动作，注释原文「错误码单行不足以让 AI 自纠」）。
2. **建议矛盾被当成必关闭**：`classify.rs:124-139` 让 advisory 优先**只认 disposition**，`severity` 完全不参与判定；`is_actionable()` 也只看 disposition（缺失即 actionable，fail-safe）⇒ 模型报 `severity=suggestion` 却同时报 `class_hint=repairable` ⇒ 落 `Repairable` ⇒ 进必关闭清单 ⇒ 触发返修、消耗预算、耗尽后升级人工。**权威证据**（`2026-11-10_分析报告_WIG基线树事实注入与前轮状态复算_复测_v1.0.md:138-148`）：唯一一次 `stopped_needs_human` 的第二轮 6 条 finding **全部 `severity=suggestion`**，其中 **3 条 `class_hint=repairable` 被判必关闭 ⇒ 升级人工**；同批另 2 条模型自报 `class_hint=advisory` ⇒ 净效果正好是去掉这 3 条误升级。
3. **成员仓被点文件挤出摘要**：`context.rs:202-247` 把点文件与成员仓目录混在**同一字典序**里 `sort()` 后 `truncate(12)`、再 `truncate(40)` ⇒ 名字靠后的成员仓**整个消失** ⇒ 作者看不见成员 ⇒ 只能自助探索（实测首轮 17 次串行工具往返、探索期 275.4 s）。

外加一处输入冗余：reviewer 侧 input 实测 **61,051 token**，投影两段 16,084 字符 / 18,362 字节，`review_context.rs` 用 `to_string_pretty`。

## What Changes

本 change 落地**四个短闭环**，每个都只改**一处信息丢失或一处判定错位**：

| 闭环 | 内容 | 命中的现象 |
|---|---|---|
| **A1 预算内事实不丢字段** | 教学载荷保留原 `code:line:message` 三段，**附**精确 `field` 与 `repair_example`；新增 parse 事实块的有界投影（≤32,000 B，按完整 UTF-8 行、含省略计数）；超限整条省略并**显式记账**，完整五字段走**独立审计**（不进 prompt）；`failure_id` 逐字不变 | ② |
| **B1–B4 SC 会话有效投影** | 一个私有谓词 + 一个会话级投影方法，统一**五个平台消费入口**（live / 历史刷新 / 前轮 reviewer / 重复闸门 / 作者返修）：`severity=Suggestion` 且 category **明确非契约** 且 `class_hint=Repairable` 的 finding，在**内存有效视图**里投影为 Advisory ⇒ 不进必关闭清单、不驱返修；契约类 / 缺 category / must_fix·blocking / 机械 Error / 原生 NeedsHuman / 异常 scope / 状态损坏 / 预算终止 **一律保持原语义** | ③ |
| **C1–C2 首轮成员事实保底** | **替换** `structure:` 段（不追加第五段）：成员仓**身份头先全保留**，再轮转子项；12 项 / 40 行只约束可轮转细节；预算内**所有成员仓身份头都在**；超 3,200 B 显式 `member_inventory_budget_exceeded` + 总数/所需字节（**不假称全员在场、不新增模型调用**）；聚合根取 `lc_plan_launch.working_dir`；首轮纪律句**等长或净减**替换 | ① |
| **D1 reviewer 无损紧凑化** | 只在单候选投影目标内增加**局部 compact 序列化** helper 替换 `to_string_pretty`；**所有 section、所有键值、数组顺序、绑定 refs、D6 内容全保留**；今天**不删** Cross-Item 段 | ③ 输入面 / token |

**边界**：本 change **不新增**模型调用、重试次数、依赖、运行时 env 开关；**不放宽**写范围校验、基线校验、不可变规格面（23,500 B）与上下文面（32,000 B）；**不引入**命名域迁移、闭合资格全口径引擎、patch 协议、检索工具或约束解码。

## 取代关系

| 旧 change（已归档） | 处置 | 本 change 的承接 |
|---|---|---|
| `rework-prompt-input-surface`（`--skip-specs` 归档） | 其 MODIFIED REQ-WSC-06 的 45 场景不整体合并；已实施且已复测的 8 场景由 `deliver-d5-d6-fact-injection` 带同步进主规格 | **C1/C2** 承接「成员仓摘要不得被点条目挤掉」；**D1** 承接 reviewer 投影的字节面（**不删语义**） |
| `teach-sc-compile-reredrive-feedback`（`--skip-specs` 归档） | ADDED REQ-WSC-32 未合并 | **REQ-WSC-32 所有权移交本 change**：**A1** 纳入该 requirement（预算内事实不丢字段）；**A3**（完整失败 source 自含教学）/ **A4**（终局失败修复包）明日独立续做，次序 A4 先 → A3 再 |
| `close-contract-findings-before-pass`（`--skip-specs` 归档） | MODIFIED REQ-WSC-17 / ADDED REQ-TOP-07 未合并 | **D9 的意图**由 **B1–B4 以窄口径**承接（落在 **REQ-WSC-12** 的显式例外）；全口径七面闭合留待后续 |
| `isolate-plan-criterion-namespace` / `explicit-sc-author-effort` / `plan-baseline-multi-member`（`--skip-specs` 归档） | 未实施 | 不承接；见各归档件的处置声明 |

## 受影响 capability

- **SC 作者轮（生成段）**：诊断事实的呈现完整度；首轮成员事实的在场保证。
- **SC 评审与返修链**：建议级 finding 的必关闭判定；作者返修输入的执行清单。
- **SC reviewer 输入**：序列化字节（语义不变）。

## 非目标

- 不修「教学耗尽后无人接手」的**完整**问题（A3/A4 明日）。
- 不修「返修轮逐字节重发首轮全文」（§3 留待后续）。
- 不修「跨仓写范围冲突假阳性（29 条里 27 条）」（机械命名域留待后续）。
- 不修「多成员基线树只覆盖一个仓」（`plan-baseline-multi-member` 留待后续）。
- 不修 **web 呈现层未镜像 disposition 过滤**（`finding-list.tsx:63`）—— 后端决定面已闭合，**界面呈现仍可能出现「需要你决定一件事」裁决块**；今天登记为未闭合项。
- 不承诺首 token 延迟下降、不承诺人工介入率下降、不承诺 token 同比下降（这些需真机复测）。
