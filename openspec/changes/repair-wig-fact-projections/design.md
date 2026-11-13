# 设计：把事实送到正确的决策点（WIG 事实投影修复）

> 权威上游：owner 批准（2026-11-13）；`cadence/designs-reviews/2026-11-09_裁决_WIG四change审阅问题处置_v1.0.md`（§2.1 描述段门禁 / §2.2 共享面顺序 / §2.3 命名域归属）；`cadence/analysis-docs/2026-11-10_分析报告_WIG基线树事实注入与前轮状态复算_复测_v1.0.md`（§9.1 复测判决）；`cadence/analysis-docs/2026-11-11_问题简报_WIG三缺陷待修_v1.0.md`（35 run 面板与残余失败归因）；`cadence/analysis-docs/2026-11-11_调研_WIG三缺陷的业界做法_v1.0.md` §4.1（三条可迁移做法）。
> 本设计与两份设计分析（`/tmp/wig-new-plan/max.md`、`/tmp/wig-new-plan/orch.md`）一致；**与旧序 4→5→6→7 串行链的差别是：删除未证实的契约依赖与重复表达，不削减任何质量门禁。**

## 1. 三个预算口径（不得混用）

| 口径 | 真实含义 | 本 change 的处置 |
|---|---|---|
| **262,144 B** | 生产作者 prompt 硬兜底（`prompts.rs:39,362`） | 不调整 |
| **23,500 B** | 现有**短动态 fixture** 的全 prompt 质量界线（`#[cfg(test)]` 下）；具体余量（如 52 B / 18~40 B）是**运行期 `eprintln!` 打印值**，不同 fixture 与批次可能不同 ⇒ **一律以最近一次绿色输出的实测值为准**，静态不可测 | **不提额**，继续守 `<23_500` |
| **32,000 B** | `confirmed_context` 动态上下文总额（`sc_baseline_tree.rs:13`）；story/design 各 14,400、structure **3,200**、baseline 段 ≤8,000 且受实际余量夹紧 | **不扩大**；成员事实**替换** structure 的 3,200，不追加第五段 |

**教学面当前没有字节闸**：`sc_compile_teaching.rs` 与 `build_typed_compile_reredrive_prompt`（`single_candidate_compile_preflight.rs:809-820`）均无 MAX/LIMIT 常量。A1 **新增**的边界仅是「新增的 parse 诊断事实块 ≤32,000 B」，**不是**假称整个教学 prompt 已有该闸。

## 2. M1 · A1：预算内事实不丢字段

### 接地事实

`CompilerDiagnostic`（`work_item_plan_compiler/types.rs:63-88`）已有 `code / line / field / message / repair_example`；`parse.rs:405-419` 已为缺字段写入精确 `field` 与 `- {field}: value`；而 `single_candidate_compile_preflight.rs:71-79` 只保留 `code:line:message`；`sc_compile_teaching.rs:90-113` 照此渲染。**同文件 lowering/IR 两类是完整 typed 载荷** ⇒ parse 类是唯一有损的一类。

### 机制

保留 `TeachingPayload.parse_reasons: Vec<String>` 与既有 builder 签名（**不引入新持久 schema、不引入 patch 协议**）。每条 parse 诊断块：
1. 首行**逐字**保留原 `code:line:message` 三段；
2. 附**精确 `field`**；
3. 附**精确 `repair_example`**。

不解析 message 猜字段、不填业务值；示例只表达**形状**，`value` 不得照抄为业务值。**仅** `unknown_structured_key` 分支按需给 `grammar.rs:59` 的 40 个合法 `STRUCTURED_KEYS`（既有补救只给了 section 两组）——**不复发**旧 A2 的完整结构契约大包。

### 新增边界（parse 事实块 ≤32,000 B）

按 UTF-8 字节计量；**只保留完整诊断块，不切半字段、半例子**；单条超大**整条省略**；显式显示总数 / 呈现数 / **省略数**；顺序稳定；**不新增失败、不新增模型调用、不新增重试**。**旧 lowering / IR / registered IDs 不在本 Task 扩修、不截断**（IR 记录当前只有 `detail`，无法完整回取 `legal_refs` ⇒ 不得截子集后宣称「全审计」）。计量表同时列三项：完整教学 prompt 字节 / parse 事实块字节 / 固定措辞字节。

### 审计与身份中性

`single_candidate_structure_reredrive.rs:531` 的 `record_generate_segment_compile_failure(engine, lifecycle, diagnostics: &[CompilerDiagnostic], detail: &str)` **已带 engine + lifecycle + 完整诊断切片**。

- **A1 拥有**完整五字段审计写入逻辑（落在 `single_candidate_structure_reredrive.rs`，现 679 行）。
- **C2 拥有** runner 调用点接入（`run/single_candidate.rs:850-874` 附近，同处已有完整 `diagnostics` + `payload` + `reredrive_prompt` + 记录调用）⇒ 该文件 **1200/1200 顶格 ⇒ 必须净零**。
- **身份 detail 与审计文本必须分开**：`failure_id = sha256(series / stage / code / detail)`（`:506-508`）**哈希了 `detail`** ⇒ ⇒ **完整五字段绝不能拼进 `detail`**（否则 durable 身份变更、影响跨 attempt 重复检测）⇒ 走独立审计文本（可放既有 `source_missing_reason` 的明确标记分区，保留原缺失说明）。**`failure_id`、attempt 身份、终态 code/message、fingerprint 逐字不变。**
- 审计写失败沿既有 `candidate_attempt_log_write_failed` 可见降级，**不伪称可取回**。

### A1 → C2 显式依赖

A1 单独交付时可证「预算内每条带齐 field/example + 省略条目数被显式记账 + `failure_id` 逐字不变」；**「被省略条目的完整五字段可从 durable 记录完整取回」要等 C2 接入后才成立** ⇒ ⇒ **C2 接入前，任何文档不得声称后者。**

### 今日不做的旧教学项

- 旧 A1 五段教学 → 由**逐条定点诊断目标**替代。
- 旧 A2 结构契约大包 → **不做**（与 A1 重叠；合法集单一来源已在校验器侧）。
- 旧 A3 完整失败 source → **明日**（依赖 A4 提供的可靠 ref）。
- 旧 A4 终局修复包 → **明日**（SC 真接点在 `run/single_candidate.rs` 与 `structure_reredrive` 的现有记录函数，**不是** `provider_drive.rs:1032`，后者是通用 artifact 选择失败路径）。

## 3. M2 · B1–B4：SC 会话有效投影（窄域建议矛盾消解）

### 接地事实

`classify.rs:124-139` 先看 `is_actionable` 再采 `class_hint`，**`severity` 不参与判定**；`ReviewFinding::is_actionable` 只看 `disposition`，且**缺失即 actionable**（fail-safe）。留存证据（`retest_harvest_output.txt:25-31` + 同批复测报告）：3 条误必关闭形状 = `severity=Suggestion` + `class_hint=Repairable` + `disposition=None` + category ∈ {Completeness, VerificationUnattributable}。原 /tmp JSON 已消失 ⇒ **证据限仓库留存派生输出，不外推全部 3/8 run**。

### 镜像对称性（本机制的正当性核心）

主规格 REQ-WSC-12 既有场景「**模型把 advisory 误标为 must_fix 也不阻断**」已确立一条轴：**模型自报 `severity=must_fix` 而 `disposition=advisory` 时，平台以 `disposition` 压过 `severity`**，判定在平台侧、不依赖模型自觉标对。

本 change 的窄例外是它的**镜像**：模型自报 `severity=Suggestion` 而 `class_hint=Repairable` 时，平台**以 `severity` 压过 `class_hint`**。

⇒ ⇒ **这不是新放松，而是把「平台在两轴冲突时以 `disposition` 为准」的对称面补全**：两个方向上都由平台裁定，而不是由模型更严的那一轴单方面决定。⇒ 契约类（`contract_gap` / `self_contradiction`）**必须保持 fail-closed** —— 主规格 REQ-WSC-17 已明文「`contract_gap` / `self_contradiction` MUST NOT 仅凭模型标 `advisory` 获得免阻断」，本 change 不触碰该义务。

### 描述段压缩依据（裁决 §2.1）

REQ-WSC-12 的描述段由主规格现值压缩到 **489 字符**（门禁 ≤500）：把**已由既有场景承载**的细则下沉到场景（`值非法 MUST 按既有 invalid_finding_field 判不合规` 已有专属场景「非法 disposition 值判不合规」；`历史记录 MUST 照旧可解析` 已有场景「缺失 disposition 的历史记录仍阻断」；`actionable / advisory` 的语义注解已有场景「本 Work Item 的验收标准缺口仍是 actionable」），**保留全部 MUST/MAY 义务**（含 `finding MAY 携带 target_work_item_id`）与**活的交叉引用**（`契约类缺口的平台加固见 REQ-WSC-17` —— 主规格 REQ-WSC-17 确含「`contract_gap` / `self_contradiction` MUST NOT 仅凭模型标 `advisory` 获得免阻断」，该引用**不失效**、**必须保留**）。窄例外的**边界**由新增场景「窄例外的边界（契约类与强严重度仍按既有语义）」承载（场景是 requirement 的规范组成部分）。**交叉引用收窄声明**：主规格 REQ-WSC-12 原描述段为「**返修触发路径与**契约类缺口的平台加固见 REQ-WSC-17」，本 delta 收窄为「契约类缺口的平台加固见 REQ-WSC-17」——**无 MUST 被削弱**（返修触发路径的义务由 REQ-WSC-17 自身承载，主规格 `spec.md:651-653`），仅去掉一处重复指引。

### 统一命名

```rust
is_sc_non_contract_suggestion(verdict: &ReviewVerdict, finding: &ReviewFinding) -> bool   // 私有谓词
apply_sc_effective_review_verdict(verdict: ReviewVerdict) -> ReviewVerdict                 // 会话方法
```

**不使用 `requires_closure`**：`close-contract-findings-before-pass` 已占用该词且是**跨作用域全口径**谓词；本谓词是 **SC 会话有效投影下的窄口径**，二者并存不冲突，序6 落地时应把本谓词纳入其输入之一。

### 匹配条件（须同时全部成立）

1. 服务端可信会话确认为 **WorkItemPlan + SingleCandidate**；
2. `severity = Suggestion`；
3. `class_hint = Repairable`；
4. category **显式**属于 `Completeness` / `VerificationUnattributable` / `Other`（**缺失或未知不纳入**）；
5. verdict 为 `Pass` / `Revise`，且**无** `structured_output_diagnostic`；
6. finding **无** `machine_repair` / `owner_question` / `owner_options`。

原 disposition 为 `Actionable` 或 `None` **均覆盖**；原 `Advisory` 保持。命中只把**内存有效视图**的 disposition 投影为 `Advisory`；**severity / message / evidence / action / finding_key / raw durable 均不改**，**durable 不回写**。

### 优先级的三方裁定（owner 批准）

平台在三条轴上会互相作用，必须定出唯一顺序：

> **平台契约强制（`with_platform_disposition_rules`：契约类 advisory ⇒ actionable）＞ severity 降级（本次窄例外）＞ 未来的 `requires_closure` 全口径谓词。**

⇒ ⇒ **severity 降级只对非契约类生效**（否则会推翻平台对真缺口的强制）；契约类（`contract_gap` / `self_contradiction`）**必须保持 fail-closed**。

### 五个实际消费入口（必须统一）

| 入口 | 落点 | 要求 |
|---|---|---|
| live | `finalize_review_verdict` | 存原始后用会话视图 |
| 历史刷新 | `latest_review_verdict_for_session` | 在原反序列化/历史兼容写回**之后**投影；**不得**把新 Advisory 写进 durable |
| 前轮 reviewer | `review_context.rs` | 先用同一会话视图再 classify；**保留 D6 内容身份求交** |
| 重复闸门 | `review/routing_scope.rs` | 前后两轮均用同视图；**顶格文件替换调用须净零**，不改集合比较算法 |
| 作者返修 | `sc_revision.rs` | 消费有效 view，结构化 actionable 为**唯一执行清单**；**散文不再复活建议**（raw 散文保留在原审计，作者仅见非执行计数说明）；真必修 action/evidence 不丢 |

### 路由末端与耗尽分支（不变）

`routing.rs:503-511` 的 `cycle.initial_count == 1 && cycle.verification_count >= 1 ⇒ HumanRequired{reason: RepairBudgetExhausted}` 分支**一字不改**（`history_delta: Default::default()`）；`evaluate.rs:235-273` 的状态/scope/transition 检查**继续执行**，**不提前 return Pass**、不增加 provider 调用。`:533-549` 的原生 NeedsHuman 覆盖保留。

**第 5 条验收的作用域**：**限耗尽 / 诊断 / 真人工上下文** —— 「耗尽 cycle 即使有效 findings 全为 Advisory 也仍 HumanRequired，非静默 pass」；**fresh-cycle 明确不承诺**非通过态（现有语义「Revise 全 Advisory ⇒ Pass」「evaluate 空集 ⇒ Valid」保持不变；强造非通过态需新增状态路由，超出本 change）。

### 不动清单

契约类 / 缺或未知 category / must_fix·blocking / 机械 Error / 原生 NeedsHuman / 产品问句 / 异常 scope / 状态损坏 / 预算终止 ⇒ **一律保持原语义**。

## 4. M3 · C1–C2：首轮成员事实保底（替换 structure）

### 接地事实

`context.rs:202-247` 把点文件与成员仓目录混排后 `truncate(12)`、再 `truncate(40)`；SC 真实调用在 `run/single_candidate.rs:544-551`（`prompts.rs` 的四个调用点是 legacy split/outline/revision，**不是**这条线）。成员**身份**其实已由 `confirmed_member_order` 教学段列全 ⇒ 缺的是成员仓**内容**在 `structure:` 里的位置。

### 机制

保留旧 `summarize_repository_structure(path, logical)` 供四个 legacy 调用（**单仓路径逐字不变**）；**新增 SC 专用的成员感知装配/renderer**（新接口，非现有符号），只消费本轮 `member_targets` 与**同源**前缀交集（`draft_batch/member_checkout_prefixes::plan_member_checkout_prefixes`：物理 ID → 权威聚合根相对前缀；检查 active/Main 唯一/域内；**alias 不能当已验证 checkout**）。`member_targets` 为 None 时保持原单仓摘要。

- **所有成员身份头先保留**，再轮转每仓子项；12 项 / 40 行**只约束可轮转细节**。
- 根前缀为空串时显示 `.`；缺权威映射按既有错误面**显式失败**，**绝不猜目录**。
- 3,200 B 内按**完整 UTF-8 行**装配（含截断说明）；上层 budget 函数**不再切成半行**。全部身份头本身超限时显式 `member_inventory_budget_exceeded` + 总数/所需字节 ⇒ **不假称全员在场**（此为新材料失败边界，须在 change 中明示并测）。
- 可选事实只含：权威成员相对根、`member_reality::detect_entrypoints` 探到的**根下清单文件存在性**、一级目录。**不采纳** package.json ⇒ npm test / pom ⇒ mvn test 之类启发式命令；**不新增**任意源码/README/环境探测；**实际 scripts 直供**独立后续。**不把「构建入口文件」吹成业务源入口或可执行命令。**
- 首轮固定阅读纪律**等长或净减**替换：「已给事实不重复全仓探测；缺事实才针对性查证」——**不禁止必要查证、不禁止产出真实验证命令**。
- 聚合根取 **`lc_plan_launch.working_dir`**，**不是** `repository.path`（focus 可能是成员 checkout）。

## 5. M4 · D1：reviewer 无损紧凑化

`review_context.rs` 用 `to_string_pretty`；单候选投影目标 `single_candidate_input.inc.rs`（现 319 行）既有 canonical 又复制 Cross-Item 供需值（确有重复，但**既有测试未覆盖其阅读形状**，不能用「现有测试不红」批准删除）。

**今天只做**：在 SC 投影目标内增加**局部 compact 序列化 helper** 并替换 SC 本体 JSON 段输出，**保留 section 顺序、所有键值、数组顺序、绑定 refs、结构审计与 D6**；helper **局部于该 include 目标**，不改通用 pretty helper（避免与 B2 共写 `review_context.rs`）；**不产生第二个全局序列化策略**；**Cross-Item 段及其值完整保留**。

## 6. 共享面与排序（对称登记，裁决 §2.2）

本 change 与下列包**共写同一文件**，相对次序如下（裁决 §2.2 的全局串行顺序：0 决策六 → 1 mechanicalize → 2 quota-pool → 3 bound → 4 effort → 5 isolate → 6 close）：

| 文件 | 本 change 内的写入者 | 与本 change 的其它写入者的次序 |
|---|---|---|
| `web/workspace_ws_handler/run/single_candidate.rs`（1200/1200） | **C2** | 独占（A1 不碰此文件）；与 A1 审计接入的调用点同处，净零 |
| `web/workspace_ws_handler/run/single_candidate_structure_reredrive.rs` | **A1** | 独占（C2 只在 runner 侧接入） |
| `web/workspace_ws_handler/run/single_candidate_compile_preflight.rs` | **A1** | 独占 |
| `product/workspace_engine/prompts/sc_compile_teaching.rs` | **A1** | 独占 |
| `product/workspace_engine/review/disposition_override.rs` | **B1** | 独占 |
| `product/workspace_engine/session_state_parts/historical_review.inc.rs` | **B1** | 独占 |
| `product/workspace_engine/prompts/review_context.rs` | **B2** | 独占（D1 的 helper 局部在 include 目标，**不共写**） |
| `product/workspace_engine/review/routing.rs` / `review/routing_scope.rs`（1200/1200） | **B3** | 独占（序 1 已归档、**序 6 已 `--skip-specs` 归档（未实施）** ⇒ 本 change 合法写入 `review/` 家族） |
| `product/work_item_split_engine/prompts/sc_revision.rs` | **B4** | 独占 |
| `product/work_item_split_engine/context.rs` / `prompts.rs`（1162） | **C1** | 独占（`prompts.rs` 的四个 legacy 调用不改） |
| `product/workspace_engine/prompts/review_parts/single_candidate_input.inc.rs` | **D1** | 独占 |

⇒ **路径约定**：本文件中的路径一律省略 `src/` 前缀（`web/...`、`product/...`、`tests/...` 均在 `src/` 下）；
⇒ **本 change 不改** `work_item_plan_policy/classify.rs`（裁决 §2.3：isolate → close 串行）⇒ ⇒ **会话投影改 disposition 后，现有 `classify` 的 advisory 优先逻辑自动生效** ⇒ ⇒ 这就是「不改中央 classifier、只在会话层投影」的实现依据。
⇒ **本 change 不新增**任何 change 的 requirement 所有权冲突：REQ-WSC-06（**最新态经 `deliver-d5-d6-fact-injection` 同步进主规格**，本 change 对其为 MODIFIED）、REQ-WSC-32（承接自 `teach-sc-compile-reredrive-feedback`）、REQ-WSC-12（MODIFIED）三者**互不相同**，且无其它 active change 声明。

## 7. 容量与波次

- **8 个 Task × 5 阶段 = 35 个编号工作包**（W1 反例/不变量测试 · W2 实现 · W3 定向红/绿证据 · W4 独立只读评审与修正 · W5 固定提交与回执；每个 Task 的末条合并承载 W3/W4/W5）。**不是**把文档/准备伪装成代码 Task。
- 每 Task ≤3 个文件且 <4 h。**首波 A1 / B1 / C1 / D1**；**第二波 B2 / B3 / B4 / C2**（B2/B3/B4 依赖 B1 的谓词接口但文件闭集独立 ⇒ 可并行；C2 依赖 C1；C2 另承接 A1 的审计接入 ⇒ A1 必须先于 C2）。
- 容量三档（含完整 8 次串行真机）：乐观 7.7 h / **现实 9.5 h** / 保守 12.2 h。若 owner 给 10 h 硬终点：今天做 8 Task + 全部确定性门禁 + 整审，**真机与 owner 验收归档明日**。

## 8. 效果口径

**能说**（代码 + 确定性门禁通过即可证）：parse 诊断在预算内不丢 `field`/`repair_example`；省略条目数显式记账、完整五字段可从 durable 取回（C2 接入后）；`failure_id` 逐字不变；被精确命中的普通建议不再单独驱返修、不进必关闭清单，五个入口同口径；契约类等不动清单全部保持原语义；预算内所有成员仓身份头都在首轮 prompt；超 3,200 B 显式报缺；单仓路径逐字不变；reviewer 同语义对象字节严格减少且 deep-equal；**未新增任何模型调用/重试/预算，未放宽写范围或基线校验**。

**不能说**：首 token 必然更快（17 次往返 / 275.4 s 需真机同口径测）；教学失败归零；人工介入率显著下降；token 减半；产出率提高；界面上的「需要你决定一件事」裁决块消失（web 呈现层未动）。

## 9. 未闭合项（登记格式：事实 → 现状 → 归谁 → 建议落点）

1. **web 呈现层未镜像 disposition 过滤** — 事实：`web/src/components/chat-workspace/finding-list.tsx:63` 的 `reviewOwnerGuidance` 遍历全部 findings、不按 `disposition` 过滤 ⇒ advisory finding 的 `owner_question` 仍被渲染为裁决块；现状：后端决定面已闭合、**前端未镜像**；归谁：前端；落点：补 `disposition != "advisory"` 过滤或等价镜像（**今天不做**）。
2. **web 呈现层是第六个消费入口** — 事实：B 组只改平台五入口；现状：平台降级后 UI 呈现不变 ⇒ 「人工介入」的界面路径今天不动；归谁：前端；落点：同第 1 项。
3. **A3 完整失败 source 自含教学** — 事实：`failure_source_ref` 现恒 None；现状：未做；归谁：集成 owner；落点：明日 A4 → A3。
4. **A4 终局失败修复包** — 事实：SC 真接点在 `run/single_candidate.rs` + `structure_reredrive` 的现有记录函数（非 `provider_drive.rs:1032`）；现状：未做；归谁：集成 owner；落点：明日，且**将再次触碰 `single_candidate_structure_reredrive.rs`**（开工需重读现状）。
5. **IR record 缺 `legal_refs` 的完整回取** — 事实：IR 记录当前只有 `detail`；现状：未做且**不在本 change 范围**（不得截子集后宣称全审计）；归谁：集成 owner；落点：后续新 change。
6. **返修轮逐字节重发首轮全文** — 事实：`sc_revision.rs` 仍以首轮完整 prompt 为底；现状：未做（B4 只改「作者收哪些条目」，不改上下文构造）；归谁：集成 owner；落点：后续新 change。
7. **跨仓写范围冲突假阳性（29 条里 27 条）** — 现状：未做；归谁：集成 owner；落点：后续新 change 的机械命名域。
8. **多成员基线树只覆盖一个仓** — 事实：66 个真实 IR 误判率 = 0（4 条引用全为真越界）⇒ 构造可达的潜在缺陷；现状：未做；归谁：集成 owner；落点：`plan-baseline-multi-member`（链尾，四端全迁）。
9. **8.12 第二轮三臂测量脚手架拆除** — 事实：`Cargo.toml` 的 3 个 default feature + 5 处生产 `cfg!` 门控仍在；现状：未做；归谁：**集成 owner**（C1 的文件闭集**不含** `validate.rs` ⇒ 本 change 不承接；拆除需改 `Cargo.toml`，属**需 owner 裁决的纪律例外**）；落点：**拆除前必须补 feature-off 回归**（`cfg!` 是运行期分支，feature-off 路径从未单独验证）；**需 owner 对「改 `Cargo.toml`」的纪律例外裁决**。
10. **真机效果复测** — 事实：8 次串行 ≈3.5 h，同 proxy 并发被判不可归因；现状：今日不做；归谁：owner/集成 owner；落点：明日固定二进制、同装置、串行交替。
11. **序4 effort 冻结策略** — 事实：T1 半成品已 park 于 `wip/wig-seq4-t1`（`1a1dd521`）；现状：未接续；归谁：集成 owner；落点：下一批（**预计 ≤10 处字面量构造点**（序4 计划书 `…WIG作者努力档位显式化_v1.0.md:148`「字面量构造点预计 ≤10 处，多为 `#[cfg(test)]` 夹具」）；补字段后**将出现** `E0063`，现树中并不存在该错误；`lifecycle_store/workspace.rs` 已 1730/1730 ⇒ 计划书的「净增 +2」必须改**净零**）。
