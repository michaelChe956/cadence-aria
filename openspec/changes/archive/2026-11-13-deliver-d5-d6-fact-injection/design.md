# 设计：交付 D5/D6 已实施的事实注入与前轮状态复算（REQ-WSC-06 拆分件）

## 1. 本件是什么

一份**只做规范同步**的交付件：把 `rework-prompt-input-surface` 中**已实施且已复测**的 8 个场景与其两项 SHALL 义务，从该 change 的 45 场景 delta 中析出，落进主规格。**不改任何产品代码。**

## 2. 已交付的三组行为（实施面）

| 组 | 行为 | 实现落点（已实施） | 对应已交付场景 |
|---|---|---|---|
| **D5 事后** | 越界报错 `acceptance_path_not_in_baseline` 的 message 附「基线树中同目录最近可用路径」；候选不得臆造，路径 MUST 取自同一棵基线树 | `work_item_plan_compiler/validate.rs` + 新增 `baseline_candidates.rs` | 越界路径回灌最近候选 |
| **D5 事前** | 作者轮启动前装载**校验器同源**的 plan 基线树，作为 L1 扩展注入 `[confirmed_context]` 的 `baseline:` 段（走上下文面预算；额度为 0 ⇒ 整段缺席） | `workspace_engine/plan_preflight.rs` + `work_item_split_engine/prompts/sc_baseline_tree.rs` | 基线树事实直供 / 基线树清单不得占用固定规范面预算 |
| **D6** | 前轮 findings 从「待办清单」改为「**带本轮机械复算状态的分区事实清单**」（`unresolved_findings` / `resolved_findings` / `duplicate_count` / `mechanical_report_available`）；求交键为**内容身份**（`contract_field`），同身份折叠为一条并计数 | `workspace_engine/prompts/review_context.rs` + `work_item_plan_policy/types.rs` | 注入前轮结构化清单 / 重复判定不靠记忆 / 前轮 finding 状态求交按内容身份 / 前轮 finding 分区注入 / 前轮清单同身份去重 |

## 3. 证据（已复测，非本件新增）

累积 **35 个有效 run、跨 4 个时段窗口**（`cadence/analysis-docs/2026-11-11_问题简报_WIG三缺陷待修_v1.0.md` §1）：

| 指标 | 改前 | 改后 | p |
|---|---|---|---|
| 走到第二轮评审的 run 中**出现重复报**的比例 | 8/9（89%） | **0/11（0%）** | **0.00131** |
| 计划产出（confirmed） | 4/15（27%） | 6/20（30%） | 0.179 |
| 编译通过 | 1/8 | 2/8 | — |

复测判决（`cadence/analysis-docs/2026-11-10_分析报告_WIG基线树事实注入与前轮状态复算_复测_v1.0.md` §9.1）：**判决一**「未产出计划」类失败为**环境性**（同时段基线臂同样失败）⇒ **D5 事前保留**；**判决二**「重复报归零」缺同窗口阳性对照 ⇒ **D6 因果效果待重测**。

⇒ ⇒ **本件的规范口径只声明「已实施且已复测的行为」，不声明效果比例**。上表 p=0.0013 是**既有资产**，不计入本件成果。

## 4. 拆分口径与描述段压缩

| 项 | rework 整份 delta | 本拆分件 |
|---|---|---|
| 场景数 | 45 | **36**（主规格原有 28 **逐字保留** + 已交付 8） |
| 描述段 | 499 字符（含 3 项**未交付**义务） | **244 字符**（基线规则 + 2 项已交付义务的 SHALL 强度） |
| 未交付义务 | L2 依赖图事实直供 / reviewer 投影最小内容（gaps-only 等）/ 返修轮 delta-only 重发 | **全部移除** |

**压缩纪律**（承接 `rework-prompt-input-surface` 工作包 8.2 / 8.11）：描述段压缩 **MUST NOT** 削弱「基线树事实直供」与「前轮 finding 状态由平台机械复算」两项义务的 SHALL 强度 —— 二者在本件描述段中**逐字在位**（见 `specs/work-item-plan-single-candidate/spec.md` 描述段）；「写范围越界诊断精度」「逐仓实况事实直供」两项**未交付**义务不在本件，故随主体归档件留存并登记未闭合。

## 5. 未实施部分的处置（不在本件）

以下场景**未实施**，其规范义务随 `rework-prompt-input-surface` 归档件留存，并逐条登记「事实 → 现状 → 归谁 → 建议落点」：

| 场景簇 | 状态 | 落点 |
|---|---|---|
| 成员仓摘要不得被点条目挤掉 / author 上下文分级供给（L1/L2） | 未实施 | L1 与名额分配由 `repair-wig-fact-projections` 承接（C1/C2）；L2 依赖图事实直供留待后续新 change |
| 返修轮不重发首轮全文 / 返修轮保留本轮修复所需上下文 | 未实施 | 留待后续新 change（与 `repair-wig-fact-projections` 的 B4 无重叠：B4 只做「作者只收有效必关闭条目」，不改返修上下文构造） |
| 投影最小内容为缺口与消费闭环 / 写范围冲突事实按同仓限定 | 未实施 | 跨仓 27/29 假阳性的机械命名域，留待后续新 change |
| 写范围越界诊断区分绝对路径与仓内相对写法 / 写范围教学反例精度 | 未实施 | `WIG写范围诊断与成员清单注入增补` 的 Task2（诊断小闭环），留待后续 |
| 逐仓实况事实直供 / 逐仓实况成员缺席如实说明 | 未实施 | 同上增补的 Task4/Task5，留待后续 |

## 6. 共享面与排序

本件**不写任何产品文件** ⇒ 与任何 change **无共享面冲突**，排序上**无前置**。它必须在 `rework-prompt-input-surface` 归档**之前**完成（因为它的 MODIFIED REQ-WSC-06 与后者争同一 requirement）⇒ ⇒ **本件先归档（带 spec 同步），主体再以 `--skip-specs` 归档。**

## 7. 验收

1. `openspec validate deliver-d5-d6-fact-injection --strict` exit 0。
2. 归档后 `openspec validate --specs --strict` 全绿；主规格 REQ-WSC-06 描述段实测 ≤500 字符。
3. 主规格 REQ-WSC-06 场景数 = **36**（28 原有逐字 + 8 已交付）；主规格**原有 28 个场景标题与正文逐字未变**（脚本比对）。
4. 主规格 REQ-WSC-06 描述段**不含**三处未交付义务（脚本断言零命中）。
5. `git diff --stat -- openspec/specs/` 仅 REQ-WSC-06 一处改动。
