# 设计加固评审 U01–U24 落地情况核实 v1.0

核验基准：当前 worktree 的代码/报告与提交 `49a394001e45591ef79de3a5af721e734c16f95d`。判定“已落地”要求清单所述的实质缺口有完整证据覆盖；仅存在相邻功能、campaign 产物或题目被纳入计划，不等于问题全解决。“部分落地”表示有可确认的子项实现，但仍有清单明确的缺口/验收证据未补齐。

## 判定摘要

- **已落地：3 条**（U12、U16、U22）
- **部分落地：7 条**（U01、U02、U11、U13、U17、U18、U24）
- **仍未落地：14 条**（U03–U10、U14、U15、U19–U21、U23）
- **建议从待办剔除：U12、U16、U22。**
- `49a3940` 的主题和变更范围是多仓 Work Item Group；没有证据显示它直接修复以下 Design author/reviewer prompt、validator、campaign 语义缺口。该提交的提交说明可核验为“feat: 多仓 work item group 全链（判定一致 + 结构门 typed 失败 + 作者重驱 + 通过率可读 + CI 修绿）”，对应报告目录变更也只新增设计评审记录与 campaign，而不等于它实现了 U01–U24。

## 逐条判定

### U01 — Design reviewer 边界判定缺少真实对照判例

**部分落地。** Design reviewer 有 workspace 类型特定边界规则，且 Design campaign 包含抽象追踪正例、测试越界负例并实测多 provider；然而没有在已核对的 production 测试中找到真实 candidate→prompt→review 语义判例回归矩阵及插入顺序锚定测试，campaign 的性能/实测结果也不能替代可重复的分类回归。

> `src/product/workspace_engine/prompts/review.rs:39-41`：“prompt.push_str(&reviewer_boundary_rules_for(&self.session.workspace_type));”
>
> `cadence/reports/design-weak-model-campaign/corpus/README.md:10-11`：“04-abstract-traceability.md：抽象追踪正例……”；“05-test-boundary-violation.md：测试越界反例……”
>
> `cadence/reports/design-weak-model-campaign/gate-report-final.md:11-14`：D04 误判均为 0，判例送达 2/2（各 provider）；但报告只记录样本结果，并非真实 candidate→review 的自动化分类回归矩阵。

### U02 — Design artifact 结构契约缺少 candidate→finding 回归矩阵

**部分落地。** campaign golden normalizer 对 Design heading、DEC/CMP/API、REQ/AC、source、决策与 fenced code 有语义回归；生产 canonical validator 对 Design 只核验基本非空及必需 heading，并未证实所述七 heading/三类 ID/source/禁止项到 `ArtifactValidationReport` finding 的完整矩阵。

> `cadence/reports/design-weak-model-campaign/task-2-report.md:19-23`：“规范化输出固定 `schema_version: 1` 与 `workspace_type: "design"`，并输出 `heading_set`、DEC/CMP/API ID……”；“fenced code block 在解析前整体排除……”
>
> `cadence/reports/design-weak-model-campaign/task-2-report.md:88-98`：列明 7 项 golden 测试及结果。
>
> `src/cross_cutting/artifact_validate/canonical.rs:52-71`：生产 Design canonical 检查只以“设计决策”heading 为必需项；该路径不是 campaign normalizer 的全 finding 矩阵。

### U03 — 用户反馈 revision prompt 缺 Design 输出契约

**仍未落地。** 当前作者反馈 prompt 仍是通用增量修订指令与当前 artifact/反馈；没有看到 Design 分支 schema、skeleton、decision contract 或 missing-context 注入。

> `src/product/workspace_engine/prompts/author_revision.rs:15-24`：“请作为 author 基于用户反馈对当前 Workspace 产物做增量修订。”；之后仅写“修订规则”“当前产物”“用户反馈”。

### U04 — 无 resume 时缺上游 Story 上下文，历史压缩策略未定

**仍未落地。** campaign 的最终 manifest 样本均标 `strategy: fresh`，`resume_available: false`；实际 fresh/resume 策略缺口仍未通过对照数据与明确压缩方案解决。此结论不表示要无条件重放历史。

> `cadence/reports/design-weak-model-campaign/gate-manifest-final.json:21-22`：“strategy”: “fresh”；“resume_available”: false。
>
> `cadence/reports/design-weak-model-campaign/gate-report-final.md:33-37`：“遗留跟踪：kimi per-turn usage 采集”；报告另明确 usage 机制仍缺（第20行）。

### U05 — choice followup DeltaOnly 缺输出契约

**仍未落地。** pending-choice prompt 仍把问题、选择和补充回答拼成普通文本；未发现其 Design 路径在该点复用 schema/fence/skeleton/decision contract。Campaign 中观察到选择及 D03 落章不是针对这条路径的自动化契约回归。

> `src/product/workspace_engine/lifecycle.rs:592-607`：“用户回答了 author 的确认问题”；末尾仅要求“请基于该回答继续生成完整候选产物……”。
>
> `cadence/reports/design-weak-model-campaign/gate-report-final.md:14`：“D03落章 2/2（含 driver bug 修复后重跑）”；这说明 campaign 样本结果，不构成 DeltaOnly prompt 的回归测试。

### U06 — Kimi/Pi 不支持 artifact retry

**仍未落地。** 已核查的 campaign 结果与 Story 结果不能证明弱模型 provider 已获得 artifact retry，也没有在这两份 campaign 的结论中找到限制已实现/解除的证据。原问题还明确指出可选择记录为 residual risk；未见本核查范围内相应交付结论。

> `cadence/reports/design-weak-model-campaign/gate-report-final.md:18-20`：“结构化错误：1/48 初测失败……修复后通过”及“token：usage 采集机制仍缺”；报告没有声称 Kimi/Pi artifact retry 能力已补。
>
> `src/product/workspace_engine/prompts.rs:150-188` 展示 artifact retry prompt 的通用构造，但不证明 provider retry 路由允许 Pi/Kimi；该具体能力仍待其调用/策略实现证据。

### U07 — Design heading/ID/source validator 接受宽松占位格式

**仍未落地。** Golden normalizer 具有 Design ID 抽取、source 覆盖及 fenced-code 排除，但它是 campaign 语义比较器，不是生产 artifact validator；生产 Design canonical 检查仍只要求“设计决策”heading，未见二级 heading/严格 ID grammar/仅正文计数约束。

> `cadence/reports/design-weak-model-campaign/task-2-report.md:19-23`：明确描述的是 `design_golden_diff.py` 的规范化与比较。
>
> `src/cross_cutting/artifact_validate/canonical.rs:52-71`：“ArtifactKind::Design => ("设计决策", …)”且按行 `starts_with('#')`、标题包含别名即通过；没有在此生产 validator 中要求 `##`。

### U08 — Design skeleton anti-copy 提示误用 REQ/AC

**仍未落地。** 当前 author skeleton/output contract 是 workspace-type 分支，但在核验到的生产约束中仍能看到共享 anti-copy 文案（具体该条所述 Design ID 文案未被 campaign 语料/报告证明已替换）；缺少同步后的共享测试证据。

> `src/product/workspace_engine/prompts.rs:181-187`：artifact retry 会按 workspace type 追加 `author_artifact_skeleton_example(workspace_type)` 与 decision contract，说明应检查共享骨架的实际文案；本次在相关报告/测试中未找到该文案改成 Design DEC/CMP/API/source 的证据。
>
> `cadence/reports/design-weak-model-campaign/task-2-report.md:88-98` 的 7 个用例不包含 skeleton anti-copy 文案断言。

### U09 — Design 多轮历史压缩 fixture 缺失及 finding 重复重放

**仍未落地。** 已核对的冻结语料是六个单链形态及上游 Story fixture；没有 Design 多轮 revision/choice/最近两轮历史 fixture。两处强 finding 重放状态传递也未在 campaign 产物中解决或证伪。

> `cadence/reports/design-weak-model-campaign/corpus/README.md:3`：“六个冻结的设计链路 Issue 描述……+ 六份冻结上游已确认 Story Spec fixture”。
>
> 同文件 `:7-12` 列出的六类形态，不包括多轮历史压缩。
>
> `src/product/workspace_engine/prompts/review.rs:43-50` 仅显示按会话消息组织 review 上下文及补充缺失上下文；没有这里所需的 compaction 已含 finding 状态证据。

### U10 — Review pass 后单仓 Design 人工确认端到端测试

**仍未落地。** 存在 Story/Design 通用人工确认实现及 Design campaign 真机链路，但已核读测试是 Story 专用或非该生命周期的 planning-chain smoke；没有所要求的单仓 Design pass→AuthorConfirm→AcceptFinalize→Confirmed 生命周期测试，并核对 structured metadata None / 不进入 aggregate。

> `src/product/workspace_engine/tests/author_revision_loop.rs:140-156`：测试 fixture 明确设 `WorkspaceType::Story`。
>
> `src/product/workspace_engine/tests/author_revision_loop.rs:676-698`：虽覆盖 Story/Design legacy accept finalizes，但只有“接受即定稿”的局部决策测试，非 reviewer pass 生命周期端到端测试。
>
> `cadence/reports/design-weak-model-campaign/gate-manifest-final.json:88-112`：真实样本出现 reviewer 后 author_confirm“人工确认定稿”，是 campaign 运行记录，不替代指定的 lifecycle-backed 自动回归。

### U11 — Design 弱模型冻结语料与实测 campaign 缺失

**部分落地。** 核心 campaign 已落地：冻结语料/golden/validator、真实模型样本与最终报告均存在，且报告为 48/48。未完全满足原清单的可信证据要求：最终报告注明 usage 采集仍缺、boundary 样本量有不足（见 U13），也未覆盖 resume cohort。

> `cadence/reports/design-weak-model-campaign/gate-report-final.md:8-20`：“48/48（100%），0 errors manifest 校验通过”；但“token：usage 采集机制仍缺”。
>
> `cadence/reports/design-weak-model-campaign/corpus/README.md:3,18`：冻结语料及 digest 规则。
>
> `cadence/reports/design-weak-model-campaign/gate-report-final.md:33-37`：kimi per-turn usage 仍为遗留跟踪。

### U12 — D05 预期返修与 full-chain 成功口径冲突

**已落地。** 冻结 corpus 规则明确 D05 的成功是首轮正确 must_fix 后返修、review pass 并走完全链，而非首轮 pass；最终 gate 报告也单独指出原 D05 题型反转/判定现象，矛盾口径已消除。

> `cadence/reports/design-weak-model-campaign/corpus/README.md:22-25`：“D05 特例口径：‘成功 = 含一次正确 must_fix 返修的全链走通且首轮边界判定正确’，不按‘首轮 pass’计。”
>
> `cadence/reports/design-weak-model-campaign/gate-report-final.md:22-25`：逐项复盘旧结论并说明 D05 判例送达、author/reviewer 判定结果。

### U13 — 边界分类样本量不足以支持可靠性/release gate

**部分落地。** 样本口径已提升到 corpus README 规定的每形态 15 次边界观测，并加入零失败置信上界计算与报告限制；但最终报告实际展示 D04 每 provider 2/2、D05 每 provider 2/2（四 provider合计每形态8次），未达到各 provider 5 次/总计15 的要求，因此不足以宣称边界可靠或满足严格 gate。

> `cadence/reports/design-weak-model-campaign/corpus/README.md:24-25`：“D04/D05 各 5 重复 × 3 provider = 各 15 观测”；成功率结论须附样本量与上界。
>
> `cadence/reports/design-weak-model-campaign/gate-report-final.md:9-15`：每 provider 的 D04/D05 实际计数为 2/2；最终报告第31行总体结论另以 48 样本上界说明，不能代替边界子集样本量。
>
> `cadence/reports/design-weak-model-campaign/validate_manifest.py:59-69`：实现 rule-of-three；`task-3-report.md:60-65` 说明边界统计语义。

### U14 — 正式判例注入位置契约矛盾、缺顺序测试

**仍未落地。** campaign 报告/测试集中在结果、golden、manifest；没有看到 reviewer prompt 判例插入顺序锚定测试（判例位于实际输出模板之前），也没有证明不含 sentinel/nonce 的断言。

> `src/product/workspace_engine/prompts/review.rs:63-75`：reviewer output contract 在 prompt 末端构造；本片段无正式 Design 判例及顺序锚点。
>
> `cadence/reports/design-weak-model-campaign/task-2-report.md:88-98`：7 项测试不含 prompt 插入顺序。

### U15 — sentinel/完整 JSON 可照抄，repair 风险/nonce 拦截需校正

**仍未落地。** production review repair 仍原样回灌 `completion.full_output`，没有显式禁止示例 nonce；结构输出修复也继续接收可恢复 JSON。Campaign 的审查分类成功不能证明 nonce 完整照抄防御已实现。

> `src/product/workspace_engine/prompts/review_repair.rs:16-20`：取得 `recoverable_value` 并序列化。
>
> 同文件 `:26-40`：“已恢复的原业务 JSON（必须逐字段保持语义一致）”并将 `completion.full_output` 填入“原始输出”。
>
> `src/product/workspace_engine/prompts.rs:292-299`：另一个 structured-output contract 的示例明确包含 `EXAMPLE_NONCE` 占位值及实际 nonce 模板；review repair 未见对应排除规则。

### U16 — manifest validator/paired/digest 与 author-reviewer 分离不足

**已落地。** Design campaign 提供严格 JSON schema、实际校验器、真实 paired baseline/revised 校验、digest 检查、usage 必填/明确 unavailable、author/reviewer 独立身份字段及单测。关键缺口已按 U16 提出的规则实现；kimi usage unavailable 是有明示状态而非伪造为零。

> `cadence/reports/design-weak-model-campaign/task-3-report.md:26-36`：必填 model/version、author/reviewer 分离、usage 二选一且缺失为 error。
>
> 同文件 `:48-58`：paired 整组剔除及 corpus/golden digest 复核。
>
> 同文件 `:103-128`：7/7 校验器测试通过，包括 usage 缺失、digest mismatch、paired 缺边/策略冲突。
>
> `cadence/reports/design-weak-model-campaign/manifest.schema.json:17-32,117-126`：role_provider 的 author/reviewer 身份与 usage/usage_unavailable 互斥结构。

### U17 — Design→work item 衔接未验证、验收措辞可能过度承诺

**部分落地。** 当前服务端会要求 Design 已确认才允许生成 work items，解决最基本的 gate；但校验函数只检查 ID 存在和 `Confirmed`，未重验 heading/ID/source/findings；本次没有发现确认 Design 到 Work Item 生成的所述端到端 smoke。Campaign 的报告主动限定为“单仓 Design 链路可用”，并未证明下游衔接。

> `src/web/handlers/lifecycle.rs:719-732`：逐项找 design，并仅检查 `confirmation_status != Confirmed` 时拒绝。
>
> `cadence/reports/design-weak-model-campaign/gate-report-final.md:31`：“单仓 Design 链路……可用性”；范围明确是单仓 Design 链路。

### U18 — manifest/决策/样本设计开放项未收敛

**部分落地。** 记录中的 validator 字段、fixture/digest、DEC 双键映射、forbidden_added 范围和 D05/边界样本设计已部分明确；但 boundary 数量在最终结果中仍不足，且 fresh/resume 和其他样本维度仍没有完整实现/验证。

> `cadence/reports/design-weak-model-campaign/task-2-report.md:19-23`：normalizer 包含 decision 键映射、source 覆盖，并允许新增 DEC/CMP/API。
>
> `cadence/reports/design-weak-model-campaign/task-2-report.md:261-264`：明确 `forbidden_added` 仅针对非 Design heading，新增 Design ID 不报错。
>
> `cadence/reports/design-weak-model-campaign/corpus/README.md:22-25`：明确主 gate、D05 特例和 boundary 目标；但 `gate-report-final.md:9-15` 的实测 boundary 观测仍未达到目标。

### U19 — 未关闭强 finding 重复重放造成 token 冗余

**仍未落地。** 已核查的 review 输入仍会追加缺失上下文；没有证据表明 compaction 输出显式标注 finding 是否已包含并据此去重。Campaign usage 也尚不可用，不能以实测 token 结论认为风险已消失。

> `src/product/workspace_engine/prompts/review.rs:43-50`：遍历会话上下文后调用 `append_missing_context_notes_to_prompt`。
>
> `cadence/reports/design-weak-model-campaign/gate-report-final.md:20`：“token：usage 采集机制仍缺”。

### U20 — Review 修复 prompt 回灌原输出/虚构载荷、nonce 排除策略未定

**仍未落地。** 当前 repair prompt 直接带入 completion 原始输出，并要求恢复业务 JSON 保持语义；未见显式排除 `EXAMPLE_NONCE`、剥离 sentinel 的 readable-only 输入或对应 nonce 负例测试。

> `src/product/workspace_engine/prompts/review_repair.rs:26-40`：`原始输出：{}` 使用 `completion.full_output`，并要求原 JSON 语义不变。
>
> `src/cross_cutting/structured_output.rs:58-76`：parser 按请求 nonce 查找 start tag；这不是 prompt 层排除示例 nonce 的实现。

### U21 — author feedback 当前 artifact 三反引号输入边界可被击穿

**仍未落地。** author revision prompt 仍以三个反引号包当前 markdown；未看到四反引号/无 fence 输入区段，也未看到嵌套代码块回归 fixture。

> `src/product/workspace_engine/prompts/author_revision.rs:21-23`：“## 当前产物\n\n```\n”后插入原 artifact，再追加“\n```\n\n## 用户反馈”。

### U22 — normalizer/manifest 语义校验不足，不应把弱模型输出等同 canonical

**已落地。** Design campaign normalizer 采用 frozen golden 做定向语义差异核验：检查既有必需约束是否丢失、链接/source/decision 是否破坏，同时明确允许新增 DEC/CMP/API，不要求其集合与单一 canonical 完全一致。对应测试验证新增 DEC 通过、决策改写/链接丢失失败。

> `cadence/reports/design-weak-model-campaign/task-2-report.md:19-23`：“新增 DEC/CMP/API 不会报错”；既有引用/source/决策丢失分别产生语义差异。
>
> 同文件 `:88-98`：7 个回归用例包含“新增一个 DEC 不产生差异”及“改写用户决策答案”失败。
>
> `cadence/reports/design-weak-model-campaign/test_design_golden_diff.py:76-90`：决策答案改写触发 `forbidden_decision_reversed`，新增 DEC 比较为空。

### U23 — reviewer few-shot 共享签名/注入范围及实施配套需收敛

**仍未落地。** 当前 `build_review_input` 仍通过 workspace type 边界规则和通用 reviewer output contract 构造；没有核验到正式 Design 判例专用注入及其范围/位置测试。不能仅凭 `49a3940` 有多仓 work item group 实现就视为 Design reviewer 的 few-shot 实施完成。

> `src/product/workspace_engine/prompts/review.rs:39-41,63-75`：review 输入注入 boundary rules，随后构造通用 reviewer output contract；该处没有已核实的 Design 专属判例注入。
>
> `49a3940` 提交说明：“feat: 多仓 work item group 全链……”，且该提交对 campaign 目录的变更为 campaign 产物而非上述 prompt 实现。

### U24 — campaign 样本/版本维度与 author/reviewer 归因不完整

**部分落地。** manifest 已增加 model/version、author/reviewer 双角色身份和 fresh/resume 字段，具备可表达性；但最终样本 usage unavailable、最终报告未提供 resume cohort 的 usage 数据，且边界样本量未达设计目标，故不能认为完整实现原有采样与归因要求。

> `cadence/reports/design-weak-model-campaign/manifest.schema.json:17-32,60-115`：包含主模型与角色模型身份、策略、resume 可用性及 usage 声明。
>
> `cadence/reports/design-weak-model-campaign/task-3-report.md:28-34`：明确 author/reviewer 独立身份和 fresh/resume 字段。
>
> `cadence/reports/design-weak-model-campaign/gate-report-final.md:20,33-37`：token usage 采集仍缺；kimi per-turn usage 为遗留跟踪。
>
> `gate-report-final.md:9-14`：D04/D05 实际观测为每 provider 两次，低于 README 的目标。

## `49a3940` 与 campaign 覆盖归类

- **可从待办剔除（全项解决）：U12、U16、U22。** U12 的 D05 成功口径已写入冻结语料；U16 的 schema/validator/paired/digest/role provider 及测试均已有可核验证据；U22 的语义 golden normalizer 和新增 DEC/决策改写回归已交付。
- **不宜从待办整体剔除，但已做部分应注明：U01、U02、U11、U13、U17、U18、U24。** 这些均有 campaign/生产代码部分交付，尚有各自未达到的验收条件。
- `49a3940` **自身没有证据证明直接完成 U01–U24 中任何一项**：提交主题指向多仓 Work Item Group；Design campaign 和记录文件虽纳入同一提交，但“提交中包含 campaign”不等于生产链路问题已全修。以上三条剔除依据主要来自 campaign 实施/报告，非该多仓功能提交。

## 检索范围与方法

- 问题源：`cadence/notes/design-harden-reviews/_sweep-未修问题汇总_v1.0.md`（逐项 U01–U24）。
- 提交：核对 `git show --stat 49a3940`、`git show --format=fuller --no-patch 49a3940`，以及 `git log --since=2026-10-31 -- cadence/reports`。近期 reports 目录记录到的提交为 `49a3940`；没有检索到包含 `2026-10-31`/`2026-11-01`/`2026-11-02` 日期字符串的报告正文。
- Design campaign：实际打开核对 `gate-report-final.md`、`baseline-sanity.md`、`corpus/README.md`、`task-2-report.md`、`task-3-report.md`、`manifest.schema.json`、`validate_manifest.py`、`test_validate_manifest.py`、`test_design_golden_diff.py`、`gate-manifest-final.json`；并确认目录中 corpus、golden、runner、测试文件实际存在。对存在性判断未仅凭文件名。
- Story campaign：实际打开 `gate-report.md`；枚举其目录中的 corpus、fixtures、golden、schema、validator、tests，作为对比证据，不将 Story 结果冒充 Design 修复。
- 代码：以 CodeGraph 查询 Design reviewer/input、author revision、choice followup、review repair、Design author/revision runtime、Design confirmed work-item 检查及调用关系；精读返回的 `src/product/workspace_engine/prompts/review.rs`、`author_revision.rs`、`review_repair.rs`、`lifecycle.rs`、`src/cross_cutting/artifact_validate/canonical.rs`、`src/web/handlers/lifecycle.rs` 与测试片段。对测试及路径范围再用文本检索核查，检索 `src/`、`tests/`、`web/src/` 中 Design、AuthorConfirm、must_fix、nonce、compaction、retry 相关内容。
- 未执行测试、构建或修改生产代码；本次只写入本核验报告，不执行 git add。
