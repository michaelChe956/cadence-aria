# WIG 方案片段 · prompt 质量（SC author 生成 prompt + review prompt）

> 作者：DesignPromptQuality｜基准树：`.worktrees/feat-wig-optimize-11-08/src/`
> 证据基线：`cadence/analysis-docs/2026-11-08_WIG实测证据基线_v1.0.md`（§1~§13 已全部读完）
> 范围：生成 prompt（SC author）与 review prompt 的内容结构、职责划分、准确性提升空间。
> **不负责**：性能/探索行为控制、重驱机制与预算（DesignPerfToken / DesignConvergence）——本文只标注交互点。

---

## 0. 实测基线（本片段自算，与基线 §4 对齐并修正一处口径）

作者 prompt 实测（`timeline_node_001.json` 的 `prompt` 字段，32,609 **字符** / 52,355 **字节**）。
注意：**基线 §4 的「字符」口径与 Rust `prompt.len()` 的「字节」口径不同**（中文 3 字节）。
预算常量 `WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES = 23_500` 是**字节**口径。

| 段 | 字符 | 字节 | 判定 |
|---|---|---|---|
| `[confirmed_context]`（story+design+structure） | 14,835 | 24,211 | 动态，不可压 |
| `[cross_reference_discipline]` | **1,802** | 2,822 | ⚠️ **基线 §4 记为 6,803 是错的**——它把 `[weak_model_precision]`+`[markdown_grammar]` 一并吞进去了（我用 `^\s*\[` 精确切分才分得开，见下文 §1.1） |
| `[weak_model_precision]` | 1,321 | 1,975 | 可判定规则 |
| `[markdown_grammar]` | 3,679 | 5,389 | 混合：机械边界 + 重复表述 |
| `[real_finding_few_shot]`（11 判例） | 4,701 | 8,393 | 价值高但与 minimum_legal_source 重叠 |
| `[minimum_legal_source]` | 1,212 | 1,252 | 最小合法形状 |
| `[multi_source_traceability_example]` | 369 | 429 | 与 grammar/CRD 重叠 |
| `[confirmed_member_order]` | 1,614 | 2,460 | LC 专属，与本次问题无关 |
| `[plan_options_mirror]` | 387 | 553 | 条件渲染 |
| `[source_boundary]` | 523 | 975 | 与 grammar 重叠 |
| `[project_rule_priority]`+`[code_usage_summary]`+`[code_reading_summary]`+`[cadence_rule_read_gate]` | 1,239 | 2,243 | 语言/纪律面，与本问题无关 |
| `[issue]`+`[routing_reference]`+`[cadence_project_rules]`+`[design_requirements]`+`[format_clamp]`+`[output]` | 1,146 | 2,278 | 少量可省 |

**静态规范面实测 = grammar(3,679) + CRD(1,802) + WMP(1,321) + few_shot(4,701) + min_source(1,212) + trace_example(369) = 13,084 字符 / 20,259 字节**
（基线说「约 15,000 字符」——它把 `[project_rule_priority]` 等也计入了。我的精确口径是 13,084。）

---

## 1. 第 1 题：逐段审计（保留 / 合并 / 删除 / 改写）

### 1.1 先纠正一处测量口径（影响后续所有比例）

基线 §4 用 `^\[[a-z_]+\]`（行首无空白）切分首轮 prompt。但 `[weak_model_precision]` 与 `[markdown_grammar]` 在 `sc_author_teaching.rs:122/34` 里是以**缩进续行**形式嵌入 `format!` 字符串的，落盘后是 `\n         [weak_model_precision]`（9 空格缩进）。行首锚点匹配不到 → 这两段被并入上一段 `[cross_reference_discipline]`，得出「6,803」。

用 `^\s*\[[a-z_]+\]` 重切：CRD=1,802 / WMP=1,321 / GRM=3,679，合计 6,802 —— 与基线的 6,803 差 1（换行归属）。

**影响**：基线结论「cross_reference_discipline 是单一最大教学段、内部有冗余」→ **该段本身只有 1,802 字符，是三段中最小的**。真正最大的是 `[markdown_grammar]`(3,679)。而「15,000 字符静态规范面」的真实值是 13,084。设计结论不变（都要压），但**优化的靶子从 CRD 转到 GRM + few_shot**。

### 1.2 逐段结论表

| 段 | 字符 | 判定 | 理由（可执行 / 重复 / 隐式负担） |
|---|---|---|---|
| `[markdown_grammar]` | 3,679 | **改写（压缩 ~900 字符）** | 25 行里只有 11 行是「模型能据此自检的判定规则」；其余是**重复表述**或**隐式负担**。见 §1.3 |
| `[cross_reference_discipline]` | 1,802 | **改写（+381 字符）+ 删 2 行** | 20 行里 **3 行是判定规则**、**4 行是重复**、**2 行是隐式负担**（要求模型同时记 ID 域 + 配对关系）。见 §1.4 |
| `[weak_model_precision]` | 1,321 | **改写（-55 字符）** | 4 条里 **2 条可执行**、**2 条重复**。核心缺陷：**「引用纪律」把 spec 域与 plan 域混为一谈**（基线 §9.3 根因） |
| `[real_finding_few_shot]` | 4,701 | **合并（-~1,400 字符）** | 见第 2 题 |
| `[minimum_legal_source]` | 1,212 | **保留 + 改一处** | 结构示例本身必要，但**必须换 ID 形态**（见第 3 题） |
| `[multi_source_traceability_example]` | 369 | **删除（-369）** | 与 GRM 的「key 白名单 + Traceability 三元组」及 CRD 的「Traceability 登记行」重复；其唯一增量信息「多 source_type 成组」已可由 grammar 的「Traceability section 必填 source_type/source_id/requirement_id」表达 |
| `[source_boundary]` | 523 | **删 2 行（-~190）** | 「Verification.command 直接声明…」与 GRM 第 6 行逐字重复（基线 §14 批注称已删过，实测仍在）；「不要 JSON、私有协议…」与 GRM 第 1 行重叠 |
| `[design_requirements]` | 121 | **保留 + 改标签** | 内容必须保留（唯一合法编号清单），但**必须改成两个清单**（见第 3 题） |
| `[format_clamp]` | 173 | **保留** | 已是压缩形态（与 GRM 第 2 行部分重叠，但位置在末尾，是「最后一口气」的高价值锚点，保留） |
| `[confirmed_member_order]` | 1,614 | **不动** | LC 多仓专属，与本次三个问题无关；动它会扩大爆炸半径 |
| `[plan_options_mirror]` | 387 | **不动** | 条件渲染，净增有上限 |
| `[project_rule_priority]` 等 4 段 | 1,239 | **不动** | 语言/纪律面，不在本问题域 |
| `[confirmed_context]` | 14,835 | **不动（但见 §3.4 交互点）** | 动态上下文；是否注入 spec 原文属性能角色决策，我只标注它与本方案的交互 |

### 1.3 `[markdown_grammar]` 内部结构（25 行逐行归类）

| 行 | 内容摘要 | 归类 | 结论 |
|---|---|---|---|
| 1 | 第一行必须精确为 `# Work Item Plan` | 判定规则 | 保留 |
| 2 | 标题逐字英文、14 个 section 名之一 | 判定规则 | 保留（与 format_clamp 部分重叠，保留此处因它是首次出现） |
| 3 | 每个 statement 一句、不重复 | 判定规则 | 保留 |
| 4 | section 按序各一次 + 14 名字单 | 判定规则 | 保留 |
| 5 | Blockers 空 section 规则 | 判定规则 | 保留 |
| 6 | Verification.command 直接声明 | **重复**（source_boundary 逐字重复） | **删除此行**（保留 source_boundary 版），-79 字符 |
| 7 | 行形态 / ID 行 / EARS 模板 | 判定规则 | 保留 |
| 8 | task_id/criterion_id/check_id 全局唯一且全局递增 | **隐式负担**（要求模型跨 WI 维护一个全局计数器，且与「criterion_id 是 WI 本地」直接冲突） | **改写**（见 §1.5） |
| 9 | CJK 空格规则（含 `」` 判例） | 判定规则 | 保留 |
| 10-16 | Inputs 四行 + 示例 4 行 + 空 section | 判定规则（可机械自检） | 保留 |
| 17 | provider_logical_work_item_id 反例 | 判定规则 | 保留 |
| 18-20 | trusted command 唯一 + 正反例 | 判定规则 | 保留 |
| 21 | key 白名单（627 字符） | 判定规则，但**隐式负担**：40 个 key 一次性列出 | **保留**（与 grammar.rs 单源，改动风险大；压缩需另议） |
| 22 | 值域（483 字符） | 判定规则 | 保留 |
| 23 | Plan Intent 分支必填 | 判定规则 | 保留 |
| 24 | fail-closed 诊断码 | 判定规则 | 保留 |

**结论**：GRM 本身**不是冗余大户**——24 行里 21 行是可执行判定规则。真正的重复只有第 6 行（-79 字符）。**这与基线 §4「cross_reference_discipline 是单一最大教学段」的判断相反。**

### 1.4 `[cross_reference_discipline]` 内部结构（20 行逐行归类）

| 行 | 内容 | 归类 | 结论 |
|---|---|---|---|
| 1 | `done_when_refs 仅引用先定义 criterion_id` | **隐式负担 + 规则缺失**（"先定义"= 文档顺序义；无 WI 本地限定 → 基线 §9.3 的直接病灶） | **改写**（见 §3.2 草案） |
| 2 | `target_contract_refs` 逐字引用 | 判定规则 | 保留 |
| 3 | `requirement_refs 仅引用清单` | **规则不完整**（校验器要的是「本 WI Traceability 登记行」，不是全局清单；清单只是必要条件） | **改写** |
| 4 | reviewer_check_refs ≡ 本 item AC 集合 | 判定规则（但与 WMP 第 1 行重复） | 保留此行，删 WMP 重复行 |
| 5 | 同 contract_id 覆盖关系 | 判定规则（与 WMP 第 3 行重复） | 保留此行，压缩 WMP |
| 6-8 | 能力原子化 / require_all 精确匹配 / require_all-vs-any 语义 | **三行讲同一件事**（原子化是因，精确匹配是果，require_all/any 是场景） | **合并为 1 行**（草案见 §1.4.1），-~150 字符 |
| 9 | 端点须显式声明 | 判定规则 | 保留 |
| 10-11 | 能力覆盖反例/正例 | 判定规则（可自检） | 保留 |
| 12 | canonical fail-closed | **重复**（第 8 行已说「缺一项→required_capability_missing」） | **删除**，-58 字符 |
| 13-14 | Handoff Schema 三字段必填 + 只允许三 key | 判定规则 | 保留 |
| 15 | Outputs 与 Handoff 独立 | 判定规则 | 保留 |
| 16 | provided_contract_refs 定义（216 字符） | 判定规则但**过长** | 压缩措辞，-~60 字符 |
| 17-18 | provided_contract_refs 正反例 | 判定规则 | 保留 |
| 19 | requirement_refs 必须在**本 item** Traceability 有登记行 | **判定规则，且是本段唯一救回 run1 的那一条**（基线 §9.3 反证） | **保留 + 升级为「本 item」强调句** |
| — | — | — | — |

#### 1.4.1 合并后的 §1.4 三合一草案（CRD 第 6-8 行）

**原文（3 行，297 字符）**：
```
能力必须原子化：每个 `- capabilities:` 与 `- required_capabilities:` 字段一行只写一个精确能力字符串；禁止用「A + B」、逗号、斜杠或自然语言把多个能力合并为一个字符串。
`require_all` 按字符串逐字精确匹配；将两个能力合并成一个字符串会制造假性 `required_capability_missing`，即使语义上看似都已提供。
require_all=全部覆盖（缺一项→required_capability_missing）；require_any=至少一项相交（交集空→required_capability_missing）。
```
**改写（1 行，172 字符）**：
```
能力原子化：每个 `- capabilities:` / `- required_capabilities:` 一行只写一个精确能力串，禁止「A + B」、逗号、斜杠或自然语言合并；require_all=全部逐字覆盖（缺一项→required_capability_missing），require_any=至少一项逐字相交（交集空→required_capability_missing），两策略都按字符串逐字精确匹配——合并两个能力会制造假性 required_capability_missing。
```
**收益**：-125 字符 / -~190 字节。**约束力损失评估**：无。三句的判定语义（原子化 / require_all 语义 / require_any 语义 / 假性 missing 的成因）全部保留；「即使语义上看似都已提供」这半句被「两策略都按字符串逐字精确匹配」覆盖。

### 1.5 GRM 第 8 行的「全局递增」表述（隐式负担）

**原文**（169 字符）：
```
task_id、criterion_id、check_id 在整份文档内全局唯一且全局递增——第二个 Work Item 的任务从 TASK-004、验收从 AC-004 继续（假设前一 item 用了 TASK-001~003），不得在每个 item 内重新从 001 编号；contract_id 同理在整份文档内全局唯一，不得重复。
```
**问题**：它要求模型维护**一个跨 WI 的全局计数器**。这与「`criterion_id` 的语义作用域是 WI 本地」（校验器口径）形成认知夹缝——模型刚被教「AC 编号全局递增」，紧接着被教「done_when 只能引本 WI 的 AC」，两者需要模型同时在脑内维护「全局计数器」和「本 WI 子集」两个结构。

**改写草案**（**注意：这是 §3 命名空间方案的产物**，若采纳 `WAC-` 前缀则本行需同步改；此处给出与 §3.1 独立、仅去隐式负担的版本）：
```
task_id、criterion_id、check_id、contract_id 在整份文档内全局唯一：第 N 个 Work Item 继续用上一 item 的下一个编号（TASK-004、AC-004……），不得在每个 item 内从 001 重新开始；新增编号一律接在已分配的最大序号之后，不得重排已有编号。
```
**差异**：「全局递增」→「全局唯一 + 接在最大序号之后」。这与 DesignConvergence 的收敛结论一致（返修时新增 criterion 用最大已分配序号+1，不要求文档顺序严格递增，否则前 WI 补 AC 会逼后 WI 重编号）。
**约束力损失**：原文「不得在每个 item 内重新从 001 编号」被保留；新增「不得重排已有编号」是**加强**不是削弱（防 run2 的 CT 编号重排 / AC 合并，对应基线 §9.4）。
**字节**：+21 字符 / +~30 字节。
**风险**：`tests/prompt_contract.rs:820` 与 `:896` 两处断言逐字包含旧串 → 需同步改断言。

---

## 2. 第 2 题：`few_shot` 与 `minimum_legal_source` 的重叠与合并

### 2.1 重叠的精确位置（实测，不是泛泛而谈）

`few_shot` 4,701 字符的构成（逐行统计）：
- 错误模式 1,365 字符（11 条）
- **证据 1,925 字符（11 条）← 重灾区**
- 修正原则 1,043 字符（11 条）
- 段头/段尾 308 字符（含末尾「标题正反例」）

`minimum_legal_source` 1,212 字符 = 一份完整 14-section 的最小 plan 骨架（53 行）。

**重叠的确切内容**：11 条判例中有 **5 条**（rep2-f1 / rep2-f2 / rep2-f4 / rep2-f5 / rep2-f6）的「证据」字段逐字引用了 **`minimum_legal_source` 里同一个骨架的字段名与形状**：

| 判例 | 证据里引用的骨架元素 | minimum_legal_source 对应行 |
|---|---|---|
| rep2-f1 | `handoff_contract.required_fields` / `write_policy` / `input_contracts` | `### Handoff Schema` / `### Write Policy` / `### Inputs` |
| rep2-f2 | `output_contract.oc_frontend_page_entry.capabilities` | `### Outputs` 的 `capabilities:` |
| rep2-f4 | `oc_levels_api_contract.capabilities` | `### Outputs` 的 `capabilities:` |
| rep2-f5 | `handoff_contract.required_fields` / `actual_listen_address` | `### Handoff Schema` 的 `required_fields:` |
| rep2-f6 | `criterion_backend_api` / `criterion_frontend_error_handling` | `### Acceptance Criteria` 的 `criterion_id:` |

而且**这 5 条的证据里用的字段名根本不是 SC markdown 的 key 名**（`handoff_contract.required_fields`、`oc_levels_api_contract.capabilities`、`criterion_backend_api` 都是 legacy JSON DTO 的名字），SC 路径根本没有 `oc_*` / `criterion_*` / `draft_00*` 这些 key（`grammar::STRUCTURED_KEYS` 40 个 key 里一个都没有）。

→ **这 1,925 字符的「证据」对 SC author 有害**：它教模型一个「看起来像证据、但在本 grammar 下不存在的字段路径」。模型被教「证据应该长这样」，于是 run1 的 Traceability 就伪造出了 `requirement_id: AC-003` 这种「编号对得上」的形式（基线 §10.2）。

### 2.2 合并方案

**保留判例的「错误模式 → 修正原则」学习价值** + **结构示例的「最小合法形状」价值**，去掉「重复的完整 plan 骨架」。

具体做法（三步，净收益可算）：

**步骤 1：判例的「证据」字段全部改写为 SC grammar 内的合法 key 路径**（不是删除）。
把 `handoff_contract.required_fields` → `Handoff Schema.required_fields`；`oc_levels_api_contract.capabilities` → `Outputs.capabilities`；`criterion_backend_api` → `Acceptance Criteria.criterion_id`。
- 收益：**不省字节**，但把「学到的证据形态」从 legacy DTO 换成 SC 合法 key → 直接抑制基线 §10.2 的「编号巧合冒充溯源」。

**步骤 2：判例中与 `minimum_legal_source` 重复的骨架描述删除**，只留「语义缺口」。
例：rep2-f1 的证据 `backend input_contracts 为空，且 write_policy 禁止依赖 web/**` → 保留（这是语义冲突，不是骨架重复）；但 `wi_backend_levels_api 的 task_backend_contract_tests 要求使用 tests/backend/fixtures/**` 里的 DTO 路径改写成 `Tasks.statement`。
- 收益：**-~600~800 字符**（11 条 × ~60 字符的 DTO 路径噪音）。

**步骤 3：段尾「标题正反例」（~110 字符）删除**——它与 `[format_clamp]`（173 字符，「结构标题必须逐字照抄…禁止翻译、改写或加中文括号」）和 `[markdown_grammar]` 第 2 行完全重复三遍。
- 收益：**-110 字符 / -~160 字节**。

**合并后字数估算**：
| | 现在 | 合并后 |
|---|---|---|
| few_shot | 4,701 字符 / 8,393 字节 | **~3,800 字符 / ~6,900 字节**（-900 字符 / -1,490 字节） |
| minimum_legal_source | 1,212 / 1,252 | 1,212 / 1,252（改 ID 形态后 ±0，见 §3.3） |
| 合计 | 5,913 / 9,645 | ~5,012 / ~8,152 |

### 2.3 约束力损失评估（逐条）

| 被删/被改的内容 | 只存在于哪一份？ | 约束力损失 |
|---|---|---|
| 标题正反例（`❌ # 工作项计划 / ✅ # Work Item Plan`） | few_shot 段尾 | **无**。`[format_clamp]`（173 字符）+ GRM 第 2 行已覆盖同一约束，且 format_clamp 出现在 prompt 更靠后位置（近输出指令，模型注意力权重更高） |
| 判例「证据」里的 DTO 字段路径（`oc_*`/`criterion_*`/`draft_00*`/`handoff_contract.*`） | 只在 few_shot | **无（反而是净收益）**。这些路径在 SC grammar 下不存在，保留它们教的是错误形态。改为 SC 合法 key 路径后**约束力净增加** |
| 判例的「错误模式」（1,365 字符） | 只在 few_shot | **无**，全部保留。这是判例的核心学习价值 |
| 判例的「修正原则」（1,043 字符） | 只在 few_shot | **无**，全部保留 |
| plan 骨架形状（section 顺序 / key 顺序 / 必填字段） | minimum_legal_source | **无**，全部保留 |

**结论：本次合并的约束力损失为零，且净收益为正**（去掉错误形态的教学、去掉三重复述）。

### 2.4 一个副作用（必须记录）

`tests/prompt_contract.rs:972-1000` 断言 few_shot 的 11 条判例的 `message`/`evidence`/`required_action` **逐字**出现在 prompt 中。若按步骤 1/2 改写「证据」字段，**该断言必然失败**。

处置：`golden_findings.json` 是 **fixture**（`src/product/work_item_plan_policy/fixtures/golden_findings.json`），它的 `source_kind: provider_raw` 标注「这是真实 provider 原始 finding」。改写它 = 篡改「真实证据」。

**推荐**：**不改 fixture**，改 prompt 渲染层。
- 渲染时对 11 条判例的「证据」做**字段路径规范化**（DTO 路径 → SC 合法 key 路径），在 `work_item_plan_real_few_shot()`（`prompts.rs:255`）内实现一张 `legacy_field_path → sc_field_path` 映射表。
- 测试断言改为：fixture 原文逐字仍在 prompt 中（保持 `provider_raw` 不篡改），**新增**断言「prompt 中不出现 `oc_` / `criterion_backend` / `draft_00` 等 SC 不存在的字段路径」。
- 代价：`prompts.rs` 需要一张映射表（~15 条 × ~40 字符 = 600 字符源码，非 prompt 字节）。**这是代码量增加，但 prompt 字节净减。**

---

## 3. 第 3 题：准确性提升（命名空间物理隔离 vs 保留前缀）

### 3.1 三层方案与推荐

基线 §9 + §10 揭示的是**两个独立缺陷**，必须分开治：

| 缺陷 | 现象 | 治法 |
|---|---|---|
| **D1：作用域矛盾**（§9） | 15 条 `unknown_done_when_ref`，全为跨 WI 引用 | **改 prompt 规则**（§3.2）。**命名空间隔离解决不了这个**——隔离后跨 WI 引用 `WAC-002` 依然越界 |
| **D2：溯源造假**（§10） | plan 域 `AC-00N` 冒充 spec 域 `AC-00N`，6 条登记无一精确匹配 | **命名空间物理隔离**（§3.3） |

→ **推荐两层都做**：D1 用 prompt 规则修正（低成本、直接消除 run1 的 15 条错误）；D2 用命名空间隔离（结构性、防止复发）。

### 3.2 D1 治法：把两个编号域在 prompt 里显式分开（源头规则修正）

#### 3.2.1 `[design_requirements]` 段改造：拆成两个清单

**现状**（实测 121 字符）：
```
[design_requirements] AC-001、AC-002、AC-003、AC-004、AC-005、AC-006、REQ-001、REQ-002、REQ-003、REQ-004、REQ-005、REQ-006、REQ-007
```
**问题**：这是**一个混合清单**（spec 域的 AC + REQ），它同时是 `requirement_refs` 的合法集（对）和 `done_when_refs` 的合法集（错，校验器要的是 WI 本地）。

**改后**（保持单段、单源，字数几乎不变）：
```
[design_requirements]
spec 域编号（全局，只能用于 requirement_refs 与 Traceability.requirement_id）：AC-001、AC-002、AC-003、AC-004、AC-005、AC-006、REQ-001、REQ-002、REQ-003、REQ-004、REQ-005、REQ-006、REQ-007
plan 域编号（WI 本地，由你在各 WI 的 Acceptance Criteria 段自行分配）：AC-001 起全局唯一递增，只用于本 WI 的 done_when_refs 与 reviewer_check_refs
```
**字节**：+~120 字符 / +~230 字节。

⚠️ **注意**：即使加了标签，`AC-001` 在两个清单里同名仍是 §10 的病灶。**这一段改造只解决 D1（作用域），不解决 D2（造假）**。D2 必须靠 §3.3。

#### 3.2.2 `[cross_reference_discipline]` 首行改写（D1 的核心）

**现状**（第 1 行，35 字符）：`done_when_refs 仅引用先定义 criterion_id。`
→ "先定义" = 文档顺序义；模型的 run1 行为正是照它做的（跨 WI 引用已在别处定义过的 AC）。

**改后草案**（替换 CRD 第 1、3 行，并新增 3 行域定义，共 +~380 字符 / +~525 字节）：
```
[cross_reference_discipline]
编号域纪律（本 plan 有两个互不相通的编号域，不得混用）：
- spec 域（全局）：[design_requirements] 清单里的 REQ-*/NFR-*/AC-*，只能用于 requirement_refs 与 Traceability 的 requirement_id。
- plan 域（本 WI 局部）：本 Work Item 自己的 Acceptance Criteria 段里声明的 criterion_id，只能用于本 WI 的 done_when_refs 与 Handoff Schema 的 reviewer_check_refs。
done_when_refs 只能引用本 WI Acceptance Criteria 段已声明的 criterion_id；引用其他 WI 的 AC → unknown_done_when_ref 拒绝。
requirement_refs 只能引用本 WI Traceability 段登记的 requirement_id；清单外 REQ-* → unknown_requirement_ref 拒绝。
```
**约束力**：把「隐式负担」（模型须自行推断域）变成**显式二分 + 逐域判定规则**。
**风险**：与 `tests/prompt_contract_weak_model.rs:40-42` 的断言冲突（断言 `引用纪律：requirement_refs/done_when_refs 只能逐字复制 spec 已定义 id（REQ-*/AC-*/NFR-*）` + `且都必须逐字取自 [design_requirements] 清单` + `清单外 AC-* → unknown_done_when_ref 拒绝`）。**这三条断言正是病灶本身，必须改**。这是本方案里最需要显式承认的「改测试」——不是「改断言让测试过」，而是「断言钉住的正是错误规则」。

### 3.3 D2 治法：命名空间物理隔离（`AC-` → plan 域独立前缀）

#### 3.3.1 候选前缀与选型

| 候选 | 含 `AC-` 子串？ | 评价 |
|---|---|---|
| `WAC-001` | 是（`WAC-` 末尾含 `AC-`） | ❌ **不可用**——`fingerprint.rs:145-154` 的 `stable_ids()` 用 `field.find(prefix)` **无 token 边界**，`"WAC-001"` 会同时被 `AC-` 匹配并抽出 `AC-001` → 与 spec 域 `AC-001` **指纹碰撞**（DesignConvergence 已独立指出，实测 `find` 确无边界检查） |
| `PAC-001` | 是（同上） | ❌ 同上 |
| **`WIC-001`** | **否** | ✅ **推荐** |
| `CRIT-001` / `WC-001` | 否 | 可选；`WIC-` 与既有 `WI-`/`CT-`/`TASK-`/`CHECK-` 命名族风格最接近 |

**推荐 `WIC-`（Work Item Criterion）**。

#### 3.3.2 兼容性影响：逐处查证结果

**`grammar.rs` 本身不改**（重要结论）：
- `grammar::STRUCTURED_KEYS`（`grammar.rs:85`）含 `"criterion_id"` —— 这是 **key 名**，不是值格式 → **不受前缀变更影响**。
- `grammar.rs` **没有任何 criterion_id 值格式校验**。值格式校验在 `parse.rs:443`：
  ```rust
  "criterion_id" => ("AC-", "- criterion_id: AC-001", &mut criterion_ids),
  ```
  → **改前缀只需改 `parse.rs:443` 这一行 + 其 repair_example 字符串**。
- `grammar.rs` 的 `STRUCTURED_SECTIONS`（14 个 section 名）/ `FREE_TEXT_SECTIONS` / `ALLOWED_*` 值域：与 criterion_id 前缀**无关**，不动。

**`parse.rs:443` 改动的连带影响**：
- `validate_identifier()` → `is_valid_identifier(value, prefix)`（`parse.rs:488-492`）：要求「前缀 + 全 ASCII 数字」。`WIC-001` 通过（`strip_prefix("WIC-")` + `"001".bytes().all(is_ascii_digit)`）。✅
- `criterion_ids` 是**全局** HashSet（`parse.rs:427`，在 `for item in items` 循环外声明）→ **`criterion_id` 全文档唯一**这一既有语义不变。✅ 这正是 GRM 第 8 行「全局唯一」规则的机器对应，改前缀不破坏它。
- **新前缀不得被 `stable_ids` 误抽**：`WIC-` 不含 `AC-`/`WI-`/`CT-` 子串 → 但 `stable_ids` 也不会抽出 `WIC-001`（因为 `STABLE_ID_PREFIXES` 里没有 `WIC-`）→ `contract_field` 里若含 `WIC-001`，`stable_ids` 返回空 → 该 finding 落入 **unstable 域**（`fingerprint.rs:99-107`）→ `identity_unstable=true` → **平台禁止自动裁决**（`evaluate.rs` 的 unstable 分支走人工）。
  → **必须同时把 `"WIC-"` 加进 `fingerprint.rs:8` 的 `STABLE_ID_PREFIXES`**，并**修 `stable_ids`/`remove_stable_ids` 的前缀边界**（最长前缀优先 + 前一字符不得为字母数字），否则：
  - `WIC-001` 若在 `STABLE_ID_PREFIXES` 里但 `find` 无边界 → 不会被 `AC-` 匹配（`WIC-001` 不含 `AC-`）✅ 安全；
  - 但若未来有人加回含 `AC-` 的前缀就会碰撞。**修边界是防御性的、必须的**。
  → **改动清单**：`fingerprint.rs:8` 加 `"WIC-"`；`fingerprint.rs:143-160` 与 `:174-200` 改为最长前缀优先 + 边界检查；新增回归测试 `WIC-001 ≠ AC-001`（两个域的 fingerprint 必须不同）。

**其它硬编码 `AC-` 处的逐项裁决**：

| 位置 | 语义域 | 是否需改 | 说明 |
|---|---|---|---|
| `parse.rs:443` | plan 域 | **改** | `("AC-", …)` → `("WIC-", …)`，repair_example → `- criterion_id: WIC-001` |
| `lower.rs:610` | plan 域 | **改** | repair_example 字符串 |
| `fingerprint.rs:8` | plan 域 | **改（+边界修复）** | 见上 |
| `work_item_contract/validation.rs:245-312` | plan 域 | **不改** | AC 集合比对是**纯字符串集合运算**，不看前缀 → 隔离后自动生效 ✅ |
| `work_item_contract/validation.rs:294`（`reviewer_check_refs` ⊆ 本 WI AC 集） | plan 域 | **不改** | 同上 ✅ |
| `work_item_contract/validation.rs:508`（`is_process_evidence_acceptance_criterion`） | plan 域 | **不改** | 只看 statement 文本 ✅ |
| `work_item_projection/reviewer.rs:19`（`task.done_when_refs.contains(&criterion.criterion_id)`） | plan 域 | **不改** | 纯字符串相等 ✅ |
| `work_item_split_engine/context.rs:131`（`extract_registered_requirement_ids` 从 spec 正文抽 `AC-`/`REQ-`/`NFR-`） | **spec 域** | **不改** ✅ | 它抽的是 story/design spec 里的编号，spec 域的 `AC-` 前缀保持不变 |
| `single_candidate.rs:422`（`reference.starts_with("AC-")` → `forged_done_when`） | **spec 域检查** | **改（删）** | 见 §3.3.4 |
| `single_candidate.rs:406-411`（`forged`：Traceability.requirement_id 必须在 `registered` 内） | spec 域 | **不改** ✅ | 隔离后它只查 spec 域，语义终于正确 |
| `artifact_constraints.rs:137`（`[AC-*]` id_rule） | **Story/Design artifact** | **不改** ✅ | 那是 Story/Design 工作区的 artifact 约束，与 plan 的 criterion_id 无关 |
| `work_item_plan_compiler/check_target_forbidden.rs:222` | 测试文案 | **改** | repair_example 里的 `AC-001 → src/lib.rs` |
| `prompts.rs:1134/1147` | **legacy JSON draft 路径** | **不改** ✅ | 那是 `WORK_ITEM_SPLIT_OUTPUT_SCHEMA`，非 SC markdown 路径 |
| `prompts.rs:306`（`WORK_ITEM_PLAN_MULTI_SOURCE_TRACEABILITY_EXAMPLE` 的 `- requirement_id: AC-000`） | spec 域 | **不改** ✅ | 这是 Traceability 的 requirement_id，spec 域 |
| `sc_author_teaching.rs:44`（GRM 第 8 行「验收从 AC-004 继续」） | plan 域 | **改** | → `WIC-004` |
| `sc_author_teaching.rs:125-127`（WMP AC 纪律的 `AC-xxx` 例子） | plan 域 | **改** | → `WIC-xxx` |
| `sc_author_teaching.rs:161/166/178`（`minimum_legal_source` 的 `done_when_refs: AC-001` / `criterion_id: AC-001` / `reviewer_check_refs: AC-001`） | plan 域 | **改** | → `WIC-001` |
| `golden_findings.json`（11 判例含 1 处 `AC-002`） | **spec 域**（rep1-f2 的证据引用 story 的 AC-002） | **不改** ✅ | 判例描述的是 spec 侧事实 |
| `tests/prompt_contract.rs:820/896` | plan 域 | **改** | 断言逐字含旧串 |
| `tests/prompt_contract_weak_model.rs:37/38/138/139/140` | plan 域 | **改** | 断言逐字含 `AC-xxx` |
| `tests/prompt_contract_weak_model.rs:163-172`（反例构造 `reviewer_check_refs: AC-001` → `AC-002`） | plan 域 | **改** | 构造 WIC 版 |
| `tests/prompt_contract_weak_model.rs:245`（断言 message 含 `AC-001 has no handoff reviewer check`） | plan 域 | **改** | → `WIC-001 has no handoff reviewer check` |
| `tests/prompt_contract.rs:1002-1012`（用 `parse_work_item_plan(minimum_source)` 验证模板可解析） | — | **自动通过** ✅ | 前提是 minimum_legal_source 已同步改 `WIC-`，且 `parse.rs:443` 已改 |

#### 3.3.3 reviewer 侧一致性

- reviewer 的 `Canonical Contract Candidates` 段（`single_candidate_input.inc.rs:118-140`）直接序列化 `item.contract.acceptance_criteria`（含 `criterion_id` 字符串）→ 前缀变 `WIC-` 后**自动跟随，无代码改动** ✅。
- `reviewer_check_refs` 配对逻辑（`validation.rs:293-312`）是**纯集合运算**，不按前缀区分 → **自动正确** ✅。
- `parsers.rs` / `classify.rs`：只读 `category`/`class_hint`/`severity`/`message`/`contract_field`，**不读 criterion_id** → 不受影响 ✅。
- **一处需注意**：`fingerprint.rs` 的 `stable_ids` 从 `contract_field` 抽 `WI-\d+|CT-\d+|AC-\d+` 作为**稳定身份**。reviewer 若把 `contract_field` 写成 `WI-001.acceptance_criteria[WIC-001]`，加 `WIC-` 到 `STABLE_ID_PREFIXES` 后可稳定抽取 → 指纹稳定 ✅。若不加，则 `stable_ids` 只拿到 `WI-001`（仍非空）→ 仍属稳定域，但尾段 `normalize_tail` 会保留 `WIC-001` 文本 → 不同 WI 的同名 criterion 尾段可能撞。**必须加。**

#### 3.3.4 D2 隔离后，`single_candidate.rs:416-434` 的 `forged_done_when` 必须删

现状：`forged_done_when` 要求 `done_when_refs` 里以 `AC-` 开头的引用**在 spec 清单内**（spec 域）。隔离后 plan 域用 `WIC-`，`reference.starts_with("AC-")` 恒 false → 该检查**静默失效**（不是报错，是永不触发）。但它本来就是**错**的（基线 §10.3 指出它与 `validation.rs:256-270` 作用域相反）。

**处置**：**删除 `forged_done_when` 分支**，只保留 `forged`（查 Traceability.requirement_id ⊆ spec 清单）。
- 删掉后 D1 仍由 `validation.rs` 逐 WI 校验（权威）✅；
- D2 由 `forged` 校验（spec 域）✅；
- **两个检查的作用域终于一致**，基线 §10.3 的潜在缺陷（spec 只有 3 个 AC 而 plan 需 5 个时把合法 plan 判为伪造）**消失**。

⚠️ **但这带来一个必须承认的新风险**：删除 `forged_done_when` 后，plan 域 criterion_id 完全脱离 spec 清单（这是隔离的目的），因此「plan 域 AC 数量可以超过 spec 域 AC 数量」——这是**正确的**（plan 域本就独立）。而 §10.2 的「溯源造假」靠什么被拦住？靠 `forged`（Traceability.requirement_id 必须逐字在 spec 清单内）+ **prompt 要求 Traceability 登记真实 spec AC**。但 `forged` 只查「编号在清单内」，**不查语义**——所以 §10.2 的 2 条「编号对得上但语义不相符」**仍能通过**。

→ **所以 §3.3 隔离**不能**单独**堵住 §10.2。必须配合 §3.4。

### 3.4 溯源造假的三层防御（补 §3.3 的缺口）

| 层 | 手段 | 效果 | 代价 |
|---|---|---|---|
| L1 隔离 | plan 域 `WIC-` / spec 域 `AC-` | 让「编号巧合」**不再可能**：plan 里写 `criterion_id: WIC-003` 与 `requirement_id: AC-003` 是两个不同域，模型必须真的读过 story 才能填对 `AC-003` | 前缀变更的全量改动（§3.3.2 表） |
| L2 prompt 约束 | Traceability 三元组要求**贴 spec 原文** | 让造假**在文本上暴露**：如果 `requirement_id: AC-003` 对应的 plan criterion 说的是 busi 物理删除，而 spec 的 AC-003 说的是前端成功流，**两者并列在文档里，reviewer 一眼可见** | Traceability 字段扩容（需 `source_excerpt` 之类新 key → 触碰 `grammar::STRUCTURED_KEYS`）**或**不改 schema，只在 prompt 要求 `statement` 里带上 spec 原文措辞 |
| L3 平台语义校验 | 比对 plan criterion statement 与 spec AC statement 的关键词覆盖 | 机器判定 | **误判率高**（中文自由文本、缩写、同义改写）→ **不推荐做硬 gate**；可做 **advisory warning** |

**推荐组合**：**L1 + L2（prompt 版）**，L3 不做。

**L2 的 prompt 版草案**（不改 grammar，只改 `[multi_source_traceability_example]` —— 它本来就要删/改）：
```
[traceability_rule]
Traceability 每组三元组的 requirement_id 必须逐字取自 [design_requirements] 的 spec 域编号；该编号对应的 spec 原文在 [confirmed_context] 中可查——登记前必须回到 spec 原文核对该编号讲的是不是同一件事；编号相同但语义不同的，按「该 WI 的验收标准未真正溯源到 spec」处理：要么改写本 WI 的 criterion statement 使其覆盖该 spec 条目，要么不登记该编号。禁止按编号相近/顺序对应来登记。
```
**字节**：~330 字符 / ~600 字节。
**兼容性**：不改 `grammar::STRUCTURED_KEYS`（Traceability 仍是 `source_type`/`source_id`/`requirement_id` 三元组）→ **零解析器影响** ✅。
**效果**：直接针对基线 §10.2 的 4 条登记（2 不符 + 2 部分不符）。**「禁止按编号相近/顺序对应来登记」这一句是 D2 的 prompt 侧根治点**——它明确切断「编号巧合」这条捷径，而这正是 §10.2 的成因。
**风险**：增加了模型要「回查 spec 原文」的工作量 → 增加探索/思考。**但按基线 §11.1，spec 原文已在 `[confirmed_context]` 里**（实测 `story:` 段含完整 story spec 正文，含 `- **[AC-001]** …` 全文）。**不需要额外探索** → 成本仅是注意力，不是墙钟。

### 3.5 「把合法编号清单改成更易引用的形态」——独立评估（Main 要求的第 3 题原案）

| 形态 | 能否消灭 §9 的 15 条错误 | 能否消灭 §10 的造假 | 判定 |
|---|---|---|---|
| 按 WI 分组的映射表 | ❌ **不能**。映射表反而**强化**「AC 编号是全局的」这一错误心智模型（把 spec 的 AC-001..006 按 WI 分组展示，等于告诉模型「WI-001 可以用 AC-001」——正是 run1 的行为） | ❌ 不能 | **否决** |
| 模板槽位留空让模型填合法值 | ❌ 不能。留空只解决「形状」，不解决「作用域」 | ❌ 不能 | **否决** |
| 模板形态上让 AC 与 reviewer_check 成对出现 | ✅ **能**——见下 | ❌ 不能 | **采纳（低成本）** |
| 命名空间物理隔离（L1） | ❌ 不能（跨 WI 引 `WIC-002` 照样越界） | ⚠️ 部分（切断编号巧合，但语义不符仍可过） | **采纳（作为 L1 层）** |

**「成对出现」的模板草案（低成本、零解析器影响）**：
现状问题：模型要跨 3 个 section（Acceptance Criteria / Tasks / Handoff Schema）维护同一个 AC 集合，任一处漏改即触发 `acceptance_criterion_without_reviewer_check`（run1 WI-002 的 `reviewer_check_refs` 少一条就是这种形态）。

**改法**（只改 prompt 里的 `minimum_legal_source` + CRD 的 AC 纪律行，**不改 grammar、不改 parser**）：
```
### Acceptance Criteria
- criterion_id: WIC-001
- statement: ...
- required_evidence: source_diff
### Handoff Schema
- required_fields: ...
- provided_contract_refs: ...
- reviewer_check_refs: WIC-001     ← 与上面 criterion_id 逐字相同
```
即：**在 `minimum_legal_source` 里把 `criterion_id` 与其配对的 `reviewer_check_refs` 写成同一个占位符**（现在是 `AC-001` / `AC-001` 两个独立字面量，模型需要自己「发现」它们该相等）。改为同一个占位符 `<本 WI 的每个 criterion_id 各出现一次>` 的提示形式：
```
（本模板中每个 criterion_id 必须在 Handoff Schema 段再出现一次、逐字相同，作为 reviewer_check_refs；一一对应，不多不少。）
```
**收益**：把「模型自行推断配对关系」变成「模板内可见的一一对应」。
**兼容性**：`minimum_legal_source` 必须仍能通过 `parse_work_item_plan()`（`tests/prompt_contract.rs:1002-1012` 断言）。→ **模板的 key 名/顺序/值形态都不变**，只改 `criterion_id`/`reviewer_check_refs` 的**值**（`AC-001`→`WIC-001`）与**段前说明文字**。段落说明文字在 `### ` 之前，属于文档正文 → **会被 parser 判「非空内容必须位于合法 section 中」**？→ 需放在 `[minimum_legal_source]` 段头（不在 `### ` section 内），或放在 `### Handoff Schema` 段内的 key 行之间。**实测：`parse.rs:266-280` 的 `_ =>` 分支对「不在任何 section 内的非空内容」报 `unknown_structured_key`。`[minimum_legal_source]` 段头在 prompt 里位于 `### ` 之前 → 安全**（模板本身从 `# Work Item Plan` 开始）。

**结论**：模板改造 = 改值 + 段头加一句说明，**零解析器改动** ✅。

### 3.6 provider 端 schema 约束为什么不通（按调研结论写明）

调研结论 1：**constrained decoding / strict schema 只能消灭「形状类」错误，不能消灭「跨引用完整性」错误**；JSON Schema 无跨实例引用完整性表达能力。
- SC 路径产出 Markdown（基线 §13 明确「provider 端没有 JSON schema 强约束」）。
- 即使改成 JSON，`enum` 只能枚举**全局域**（`requirement_refs` 可枚举 `[design_requirements]` 的 13 项 ✅），**覆盖不了 `done_when_refs`**（WI 本地域，合法值依赖同一 JSON 里的其它字段 → 跨字段引用完整性，JSON Schema 表达不了）。
- **§9 的根因恰恰是 WI 本地域 → 这条路在原理上不通**，不是「做了收益小」，是「做不到」。

---

## 4. 第 4 题：review prompt 优化（42,449 字符 / 52,297 字节，9 section）

### 4.1 逐 section 判定

实测各 section 字符/字节：

| # | Section | 字符 | 字节 | 给模型看？ | 判定 | 理由 |
|---|---|---|---|---|---|---|
| 1 | Canonical Contract Candidates | 17,581 | 22,315 | **是（核心）** | **保留** | 这是 reviewer 的审查对象本体。**但它不含 handoff_contract / design_traceability**（实测 prompt 里 `reviewer_check_refs`/`provided_contract_refs`/`required_fields`/`design_traceability`/`source_id` 出现次数**均为 0**）→ reviewer **无法审 AC↔reviewer_check 配对、无法审溯源**！这是 §10.2 造假能通过 review 的直接原因 |
| 2 | Dependency Contract Graph | 256 | 256 | 是（但冗余） | **删除** | 256 字符，= `topological_order` + `edges`；与 #3 的 `dependency_graph.declared_edges` + `contract_edges` **完全重复**（#3 内含 `depends_on`/`declared_edges`/`contract_edges`/`cycles`/`duplicate_edges`/`unknown_providers`） |
| 3 | Reviewer Capability Coverage Projection | 12,578 | 13,778 | 是 | **压缩** | 内部 4 块：`capability_coverage` 1,875（**与 #1 的 capabilities 逐字重复**：实测 16 条 cap 串 100% 已在 #1 出现）、`dependency_graph` 2,080（**与 #2 重复**）、`handoff_consumption` 383（**新增信息**：consumed=true/false，有价值）、`write_scope_conflicts` 7,249（**27/29 条是跨仓假阳性**——实测 27 条 cross-repo、仅 2 条 same-repo；跨仓 forbidden/exclusive 冲突在本 multi-repo 场景无意义） |
| 4 | Cross-Item Contract Supply / Demand | 3,506 | 4,584 | 是（但冗余） | **删除** | `supplies` = #1 的 `output_contracts` 逐字重复；`demands` = #1 的 `input_contracts` + `depends_on` 逐字重复。实测 19 条 cap 串 100% 已在 #1 出现 |
| 5 | Projection Validation Report | 584 | 584 | 是 | **保留** | 机械校验摘要（error_count/warning_count/findings），reviewer 判 pass 的依据。584 字符性价比极高 |
| 6 | Immutable Candidate Artifact Refs | 394 | 394 | **审计** | **删除** | 只有两个 durable ref 字符串。平台自己用 `plan_candidate_ir_ref`/`mechanical_report_ref` 就能定位；**模型无法据此做任何判断**（它不能读 durable store）。纯审计信息 |
| 7 | Deterministic Structure Report | 1,948 | 1,948 | **一半是审计** | **压缩** | `effective_edges` 1,300+ 字符 = 与 #3 `dependency_graph` 的边信息重复；`cycle_witness`/`findings.failures`/`protected_predicates`/`structurally_valid` 是**平台确定性结论**——`structurally_valid:true` 有判断价值，`protected_predicates`（4 个谓词名）是纯审计 |
| 8 | Structure Context Identity | 868 | 868 | **审计** | **删除** | 7 个 hash/ref（`context_digest`/`source_hash`/`report_digest`/`rule_version`/3 个 ref）。模型无法验证任何 hash。纯审计 |
| 9 | Structural Repair Audit | 3,736 | 6,106 | **审计** | **压缩** | `instruction_refs`/`repair_audit_refs` 两空数组（初轮）+ 后面 3,607 字符的「审核边界/能力覆盖机械规则/repetition 纪律/cadence 规则/read gate/输出契约」**（这一大块不是本 section，是被我的切分吞进去的尾巴，见 §4.2）** |

### 4.2 ⚠️ 关键发现（对 Main §13 表格的一处纠正）：「Structural Repair Audit = 4,651 字符（11%）」是**切分假象**

Main 的 §13 表格按 `^#{2,3} ` 切分，把 `### Structural Repair Audit`（绝对偏移 37,799）到 **prompt 末尾**（42,449）的全部内容都算进这一段 = **4,650 字符**。

**但这 4,650 字符里，真正的审计 payload 只有 129 字符（2.8%）**，其余 4,521 字符是**后面 9 段活着的 reviewer 纪律**，被这一段的尾部吞掉了。逐段实测（绝对偏移）：

| 起止 | 长度 | 内容 | 性质 |
|---|---|---|---|
| 37,799–37,883 | **84** | `{"instruction_refs":[],"repair_audit_refs":[]}` | **真审计 payload（空）** |
| 37,883–37,928 | **45** | 「结构修复审计说明：初轮合法候选无修复指令与审计引用…」 | 空值语义说明句 |
| 37,928–38,315 | 387 | 审核边界 + 能力覆盖机械规则 | **活的 reviewer 纪律** |
| 38,315–38,512 | 197 | `[review_repetition_discipline]` | 活的纪律 |
| 38,512–38,917 | 197 | `[cadence_project_rules]` | 活的纪律 |
| 38,917–39,089 | 172 | `[cadence_rule_read_gate]` | 活的纪律 |
| 39,089–39,309 | 220 | 输出契约段头 | 活的纪律 |
| 39,309–39,731 | 422 | `OWNER_GUIDANCE_RULES` | 活的纪律 |
| 39,731–40,434 | 703 | `FINDING_SCOPE_RULES` | 活的纪律 |
| 40,434–40,991 | 557 | schema 示例（EXAMPLE_NONCE） | 活的契约 |
| 40,991–41,535 | 544 | schema 模板（实际 nonce） | 活的契约 |
| 41,535–42,449 | 914 | `## 服务端审核 invocation scope` | 活的契约 |

**结论**：Main 表中「Structural Repair Audit 4,651 字符 = 11.0%，本次内容全空」的**前半句成立、后半句不成立**。
- ✅ 成立：该段的**真 payload 确实全空**（`instruction_refs:[]` / `repair_audit_refs:[]` / `cycle_witness:[]` / `structurally_valid:true`）。
- ❌ 不成立：11% 的 prompt 里只有 **129 字符（0.3% 全文）** 是空洞的；**4,521 字符（10.7%）是活着的 reviewer 纪律与输出契约**，删掉会直接摧毁 reviewer 的行为约束（尤其 `FINDING_SCOPE_RULES` 422 字符 = disposition 语义 + 范围外事实降级规则 + owner 处置去重，正是用户抱怨③「反复失败耗尽预算」的约束面）。

→ **因此「条件化 Structural Repair Audit」的正确形态是：条件化那 129 字符的 payload，不是条件化整个 4,650 字符的尾部。** 见 §4.3 的 L2 层。

同理，Main 表中「头+审核边界+契约纪律+cadence_project_rules ~1,030 字符」实际对应上表 387+197+197+172+220+422+557+544+914 = **3,610 字符**（Main 把它们分到了「Structural Repair Audit」桶里）。

### 4.3 分层结论（Main 要求的四层）

#### L1 —— 可折叠/移除（纯审计面）：**1,262 字符（3.0%）**

| Section | 字符 | 字节 | 判定 |
|---|---|---|---|
| `Structure Context Identity` | 868 | 868 | **删**。内容 = `source_revision_ref` / `plan_candidate_ir_ref` / `mechanical_report_ref` / `source_hash` / `context_digest` / `rule_version` / `report_digest` —— 全是哈希与引用字符串 |
| `Immutable Candidate Artifact Refs` | 394 | 394 | **删**。内容 = 两个 durable ref 字符串 |
| 合计 | **1,262** | **1,262** | |

**平台侧比对逻辑是否依赖模型读它们？——不依赖，我独立核实了 Main 的结论**：

`single_candidate_input.inc.rs:96-140`（渲染**之前**）已经做了全部 join 校验，且都是**平台 Rust 代码**：
```rust
if report_record.ir_id != ir_record.id
    || report_record.report.source_revision_hash != ir_record.ir.source_revision_hash
    || report_record.report.compiler_version != ir_record.ir.compiler_version { return Err(...) }
...
if structure.context_digest.trim().is_empty() { return Err(...) }
if structure.rule_version != live_context.rule_version { return Err("... rule stale ...") }
if structure.context_digest != live_context.context_digest { return Err("... context stale ...") }
```
→ **digest/rule_version 的同代 join 校验 100% 在平台侧完成**，模型只是「被告知结果」的旁证。删掉这两段**零判断损失、零平台影响**。

（唯一的例外：`Structure Context Identity` 里的 `source_revision_ref` 与 `plan_candidate_ir_ref` 出现在 `## 服务端审核 invocation scope` 段的 `immutable initial revision` 字段里 —— 那一处**保留**，因为 scope 契约要求 reviewer 知道「本轮只能审这一版」。）

#### L2 —— 可条件化（空洞段）：**真 payload 129 字符**

**渲染代码位置**（Main 问的）：`single_candidate_input.inc.rs:227-241`，在 `if let Some(structure) = &report_record.structure_report {` 块内：
```rust
append_review_context_section(&mut prompt, "Structural Repair Audit",
    &json!({ "instruction_refs": structure.instruction_refs,
             "repair_audit_refs": structure.repair_audit_refs }))?;
prompt.push_str("结构修复审计说明：初轮合法候选无修复指令与审计引用（初轮无修复明确空，不伪模型许可）。\n");
```
**条件渲染是纯渲染层改动**，字段来源已在手（`structure.instruction_refs` / `structure.repair_audit_refs`，类型 `Vec<String>`，实测本轮均为空）。

**条件渲染方案**：
```rust
// 只在有修复指令/审计引用时才渲染该段；空则用一行说明句替代整个 JSON 段。
if !structure.instruction_refs.is_empty() || !structure.repair_audit_refs.is_empty() {
    append_review_context_section(&mut prompt, "Structural Repair Audit",
        &json!({ "instruction_refs": structure.instruction_refs,
                 "repair_audit_refs": structure.repair_audit_refs }))?;
} else {
    prompt.push_str("结构修复审计：本轮无修复指令与审计引用（初轮无修复明确空，不伪模型许可）。\n");
}
```
- **收益**：省 **84 字符 / ~130 字节**（把 84 字符的空 JSON + 45 字符说明句，换成 33 字符说明句）。
- **收益极小，但零风险**：这是「11% 空洞」这个说法落地后的真实数字——**0.16%**。
- ⚠️ **不能条件化的是**：§4.2 表里那 4,521 字符的活纪律（`FINDING_SCOPE_RULES` / `OWNER_GUIDANCE_RULES` / repetition 纪律 / 输出契约 / scope 指令）。它们与审计 payload 无关，**必须逐字保留**。

#### L3 —— 判断必需、不可动：**30,159 字符（71%）**，但**可裁剪的冗余实测如下**

Main 问「coverage projection 是否与 canonical contract 里的 capabilities 重复表达了同一信息」——**实测：是，且是精确重复**。我用「集合差」逐条验证（不是抽样）：

| 检查 | 结果 |
|---|---|
| `capability_coverage` 的 3 条边 | **100% 可从 CCC 的 `input_contracts`/`output_contracts` 推出**（CCC 隐含 6 个 (from,to,contract) 组合，coverage 的 3 条全在其中，差集 = 0） |
| `capability_coverage.provided_capabilities` 的 16 条 capability 串 | **16/16 已逐字出现在 CCC** |
| `capability_coverage.required_capabilities` 的 10 条 capability 串 | **10/10 已逐字出现在 CCC** |
| **`missing_capabilities` 的差集** | **空集**（本次 3 条边全无缺口）→ **coverage 段里没有任何一条 CCC 之外的信息** |
| `Cross-Item Supply/Demand` 的 29 条 distinct 串 | **29/29 已逐字出现在 CCC**（`supplies` = CCC 的 `output_contracts`；`demands` = CCC 的 `input_contracts` + `depends_on`） |
| 字符串级字节重叠 | coverage 的字符串部分 **1,757 字节 = coverage 的 12.8%** 逐字重复；cross 的 **1,485 字节 = cross 的 32.4%** 逐字重复（结构冗余更高，因为整块是同一数据的另一种序列化） |

**结论**：**「判断必需」的定性对，但「不可动」不成立**——这两段（12,578 + 3,506 = 16,084 字符 / 18,362 字节）里，**除了 `missing_capabilities`（本次为空）与 `handoff_consumption.consumed` 标记，没有任何 CCC 之外的信息**。

**裁剪后的形态**（保留全部判断力）：
```json
### Cross-Item Contract Edges (gaps only)
{"cross_item_contract_edges":[{"from":"WI-003","to":"WI-001","contract_id":"CT-003","policy":"require_all","missing":[]}]}
### Handoff Consumption
{"handoff_consumption":[{"provider":"WI-001","contract_ref":"CT-001","consumers":["WI-004"],"consumed":true}]}
```
实测字节：**462 + 383 = 845 字节**（替代 18,362 字节）。

**唯一被删掉的信息是 `write_scope_conflicts` 的 27 条跨仓假阳性**（实测 29 条中 27 条 cross-repo、仅 2 条 same-repo）—— 跨仓 forbidden/exclusive 冲突在 multi-repo 场景无意义（WI-001 的 `frontend/src/App.vue` 与 WI-002 的 `frontend/**` 分属不同成员仓，不构成写域冲突）。**同仓的 2 条必须保留**（WI-001 exclusive `frontend/src/App.vue` vs WI-004 forbidden `frontend/src/**` —— 同仓 `repository_6c8`，是**真实**冲突信号，reviewer 应据此判断）。**保留同仓 2 条 = 441 字节。**

→ **L3 裁剪**：13,778（COV）+ 4,584（CROSS）+ 256（DG）= 18,618 字节 → **1,747 字节**（462 gap + 383 handoff + 441 same-repo conflicts + 461 段头）。

#### L4 —— 判断必需、保留：CCC（17,581 / 22,315）+ PVR（584）+ DSR 压缩（1,948 → 219）

**CCC 必须补两个字段块**（这是 L3/L4 之外**唯一的新增**）：
```json
"handoff_contract": { "required_fields": [...], "provided_contract_refs": [...], "reviewer_check_refs": [...] },
"design_traceability": [ {"source_type": "...", "source_id": "...", "requirement_id": "..."} ]
```
实测增加 **~571 字节/item × 4 items = 2,284 字节**（用 run2 的真实 handoff/traceability 内容估算）。
→ CCC: 22,315 → **~24,600 字节**。

**DSR 压缩**：`effective_edges`（6 条边，含 `order_indices`/`spans`/`sources`）与 coverage 的 `dependency_graph` 重复 → 删；保留 `structurally_valid` / `cycle_witness` / `protected_predicates`：
实测 **219 字节**（从 1,948）。

### 4.4 review prompt 重构后的完整字节账

| 项 | 旧字节 | 新字节 | Δ |
|---|---|---|---|
| CCC（+handoff +traceability） | 22,315 | 24,600 | **+2,285** |
| COV（→ gap edges + handoff + same-repo conflicts） | 13,778 | 1,747 | **−12,031** |
| CROSS | 4,584 | 0 | **−4,584** |
| DG（Dependency Contract Graph） | 256 | 0 | **−256** |
| DSR（→ 结构摘要） | 1,948 | 219 | **−1,729** |
| PVR | 584 | 584 | 0 |
| ICAR（审计面，删） | 394 | 0 | **−394** |
| SCI（审计面，删） | 868 | 0 | **−868** |
| SRA payload（条件化） | 215 | 85 | **−130** |
| **结构段小计** | **44,942** | **27,235** | **−17,707（−39.4%）** |
| 头 + 活纪律 + 输出契约 + scope 指令（**不可动**） | 7,355 | 7,355 | 0 |
| **review prompt 总计** | **52,297** | **34,590** | **−17,707（−33.9%）** |

**review prompt：52,297 → ~34,590 字节（−33.9%），且判断力净增**（reviewer 新增看到 `reviewer_check_refs` 与 `design_traceability`）。

### 4.5 四个「审计 vs 模型」section 的最终判定

| Section | 给模型还是给审计？ | 依据 | 判定 |
|---|---|---|---|
| `Deterministic Structure Report` | **混合**：`structurally_valid`/`cycle_witness` 是**给模型**的（reviewer 据此判拓扑合法）；`effective_edges`（与 coverage 的 dependency_graph 重复）+ `protected_predicates`（4 个谓词名，平台自用）是**给审计**的 | 混合 | **压缩为 219 字节摘要** |
| `Structure Context Identity` | **纯审计**。7 个 hash/ref，模型无法验证任何一个（无 durable store 访问权）。**平台侧 join 校验已在 `single_candidate_input.inc.rs:96-140` 完成**（我已核实：ir_id / source_revision_hash / compiler_version / context_digest / rule_version 五项比对全在 Rust 侧） | 纯审计 | **删（零判断损失）** |
| `Structural Repair Audit` | **纯审计**。实测 payload 恒空（`instruction_refs:[]`/`repair_audit_refs:[]`）。**但 §4.2 已纠正：该桶的 4,650 字符里 4,521 是活纪律** | 纯审计（仅 129 字符） | **条件化 payload（84→0）+ 留一句说明（−130 字节）**。⚠️ **不可条件化尾部 4,521 字符** |
| `Immutable Candidate Artifact Refs` | **纯审计**。两个 ref 字符串，模型无法据此读任何东西 | 纯审计 | **删（零判断损失）** |

### 4.6 对 reviewer 判断质量的影响（分层）

| 动作 | 判断影响 | 理由 |
|---|---|---|
| 删 ICAR + SCI（1,262 字符） | **零影响** | 纯 hash/ref，模型无法消费；平台侧比对不依赖 prompt |
| 条件化 SRA payload（129 字符） | **零影响** | 恒空；换成一句等义说明 |
| COV/CROSS/DG 裁剪（−16,871 字节） | **正面** | 三段的信息 100% 可从 CCC 推出（实测集合差 = 0）；裁掉 27 条跨仓假阳性冲突（reviewer 本轮要逐条判断它们是否真冲突 → 消耗注意力且诱导误报）；保留 2 条**同仓真实**冲突 + 全部 gap 信息 + handoff consumed 标记 |
| DSR 压缩（−1,729 字节） | **零影响** | 边信息在 coverage/新 gap 段里都有 |
| **CCC 补 handoff_contract + design_traceability（+2,285 字节）** | **正面，且是本方案最大的一项** | 现状 reviewer prompt 里 `reviewer_check_refs` / `provided_contract_refs` / `required_fields` / `design_traceability` / `source_id` 出现次数**均为 0** → reviewer 对「AC 是否都有 reviewer check」「溯源是否真实」**结构性失明**。这正是 §10.2 的 4 条假登记能拿到 pass 的直接原因 |

**风险**：补齐后 reviewer **更可能发现**问题 → 现在的 pass 可能变成 revise → 返修轮次增加。**这是把「漏检」纠正为「检到」，不是「制造问题」**。净效果需实测（基线 §12 记账缺陷 2.4×~5.3× → 见 §7.4）。

**兼容性**：改动只在 `single_candidate_input.inc.rs` 的 prompt 构造层（`:118-140` contract_candidates / `:160-175` coverage+cross / `:196-241` 三可信 section / `:227-241` SRA 条件化）。**不动 IR、不动 validator、不动 `grammar.rs`、不动 `fingerprint.rs`**。

### 4.7 目标形态（与 §4.4 字节账一一对应）

```
## Plan Review Context
### Canonical Contract Candidates        ← 22,315 → 24,600 字节（补 handoff_contract + design_traceability）
### Cross-Item Contract Edges (gaps only) ← 新增 462 字节
### Handoff Consumption                   ← 新增 383 字节
### Write Scope Conflicts (same repository)← 新增 441 字节（只留同仓 2 条）
### Projection Validation Report          ← 584 字节（不动）
### Structure Verdict                     ← 1,948 → 219 字节（structurally_valid + cycle_witness + protected_predicates）
（删除：Dependency Contract Graph / Cross-Item Supply-Demand / Immutable Candidate Artifact Refs / Structure Context Identity）
（Structural Repair Audit：条件化 → 初轮只留一句说明）
[活纪律全部逐字保留：审核边界 / 能力覆盖机械规则 / repetition / cadence 规则 / OWNER_GUIDANCE_RULES / FINDING_SCOPE_RULES / 输出契约 / invocation scope]
```

---

## 5. 第 5 题：reviewer 与 author 的探索重复

### 5.1 实测重叠

- author：13 次 shell（`timeline_node_001.json`），工具输出 42,587 字节（其中 32,609 是 prompt 回显 → **真实探索输出 ≈ 9,978 字节**）
- reviewer：14 次 shell（`timeline_node_002.json`），工具输出 107,044 字节（其中 42,449 是 prompt 回显 → **真实探索输出 ≈ 64,595 字节**）
- **reviewer 的探索输出是 author 的 6.5 倍**（64,595 vs 9,978 字节）。这与基线 §6 的「reviewer 61,051 input token 主要来源」吻合。

**具体重复点**（实测两条命令集）：
- 双方都读：`busi/src/...` 目录树、`frontend/src`、`NoticeControllerTest.java`、`OpenApiDocTest.java`、`frontend/package.json`、`eslint.config` / `vite.config`、policy 政策正文。
- author 独读：`busi/checkstyle.xml`、`podman --version/images/ps`、`.prettierrc.json`、`busi/pom.xml`、`IntegrationTestSupport.java`、`cadence/legacy` 遗留 md。
- reviewer 独读：`GlobalExceptionHandler.java`、`ErrorCode.java`、`frontend/src/api.js`、`frontend/src/App.vue`、`frontend-ui-standards.md`、`ElMessageBox` grep。

**reviewer 独读的部分不是浪费**——它的 5 条 finding 中有 3 条引用了这些文件的事实：
- finding 2：eslint 配置只忽略 dist/node_modules/target/.worktrees → 来自 `eslint.config.js`
- finding 4：gateway 既有统一异常处理已具备透传能力 → 来自 `GlobalExceptionHandler.java`
- finding 3：CHECK-002 缺固定命令/导入路径 → 来自 `frontend/src/api.js`

→ **reviewer 的探索有实质产出**。问题不是「reviewer 探索了」，而是「reviewer 探索了 author 已探索过的同一批文件，而 author 的结论没有以任何形式传过去」。

### 5.2 Main 提的第三条路（比 A/B 都更省）：把「命令可执行性」从 reviewer 的职责里移出去

Main 的观察是对的，我把它形式化：**reviewer prompt 里已有 canonical contract 全文（含 `verification_checks[].command` 与 `manual_instruction`），reviewer 却自己去确认「这个命令能不能跑」。**

这句话点出了 A/B 之外的第三种处置：**这类事实根本不该由 reviewer 在审核时现查，而应该在 author 阶段就被「证明」并落进 IR。**

对照实测：
| 事实 | 现在谁负责 | 现状 |
|---|---|---|
| 命令是否真实存在于仓库、能否跑 | reviewer 现查（读 `package.json`/`eslint.config`/`vite.config`） | reviewer 读了 14 次，其中 ~4 次是纯工具链确认 |
| 契约供需是否闭合 | 机械投影（`capability_coverage.missing_capabilities`） | 已有 ✅ |
| AC↔reviewer_check 是否配对 | 机械校验（`validation.rs:293-312`） | 已有 ✅（但 reviewer prompt 看不到，见 §4.3 L3/L4） |
| 溯源登记是否真实 | **无人负责**（§10.2） | reviewer 结构失明 |

→ **可机械化的都已经机械化了**；reviewer 现查仓库填的正是「命令可执行性」这一类**本可以在 author 阶段确定**的事实。

### 5.3 三个选项的完整评估

| 选项 | 做法 | 收益 | 代价 / 风险（实测） |
|---|---|---|---|
| **A. durable artifact 扩展** | author 探索结论结构化落盘（工具链版本、测试文件位置、命令可执行性），注入 reviewer prompt | reviewer 不重复探索；「命令是否可跑」有据 | 需新增 durable artifact 类型 + 注入点（**平台侧**）；**author 漏了 reviewer 也漏**；仍有 ~64.6KB 残留（reviewer 仍需读 author 未读的文件） |
| **B. reviewer 只审 IR** | reviewer prompt 禁读仓库；命令真实性交给作者自证 + 机械校验 | reviewer 探索归零 → **省 64,595 字节**（reviewer input token 大头） | **会失去实测 5 条 finding 中 3 条的依据**（eslint 配置 / gateway 异常处理 / api.js）→ 漏检增加；「命令臆造」只有读仓库能发现 → **把漏检从 reviewer 挪到无人的地方** |
| **C. 把命令可执行性上移到 author（推荐）** | author 在生成时把「命令 → 存在性证据」写进 `manual_instruction`/plan；reviewer 只审 IR 与已落盘的证据 | reviewer 探索可收窄到**只剩「plan 声明的能力是否与仓库既有实现冲突」一类**；同时**提升 plan 质量**（作者被迫核实自己的命令） | 需要 author prompt 强化（§1 已在做）；**不能完全消除 reviewer 探索**（finding 4 仍需读 `GlobalExceptionHandler.java`）；作者侧增加一次核实成本（但 §11.1：探索只占 ~200 秒，不构成墙钟主因） |

### 5.4 推荐：**C 为主 + R1 收窄边界，A 留作后续；明确否决 B**

**推荐结论**：
1. **否决 B（reviewer 只审 IR）** —— 实测会失去 5 条 finding 中 3 条的依据，而这些 finding 本轮恰恰是**有价值**的（finding 2 指出 eslint 扫描范围与 lint 门禁口径冲突、finding 4 指出网关既有能力已覆盖、finding 3 指出 CHECK-002 不可复现）。把「命令臆造」「能力与既有实现冲突」这两类检查从审核面删掉，是**把漏检从 reviewer 挪到无人的地方**。
2. **采纳 C 的精神 + R1 的收窄**：reviewer 保留仓库访问权，但**明确划出「不必读」与「必须读」的分界**，把探索预算从「无约束」压到「只为 Plan Review Context 答不了的事实」。
3. **A（durable artifact）作为后续项**，不在本轮——它需要平台侧新增 artifact 类型与绑定校验，且收益（−~30KB 探索输出）与 R1 重叠。

**具体做法（按优先级）**：

**R1（prompt 侧、零平台改动、立即可做）——把 reviewer 的探索边界收窄到「不可从 IR 推出的事实」**。

现状：审核边界段（`single_candidate_input.inc.rs:239-243`）已经说了「只审核 Plan Review Context 中的权威 canonical contract、依赖拓扑、跨 WorkItem 契约供需与机械校验摘要」，但实测 reviewer 仍读了 14 次仓库。**边界说了，没约束**（且 §4.3 L3/L4 补齐 `handoff_contract`/`design_traceability` 后，「Plan Review Context」本身的信息量会增加 → 边界的说服力增强）。

**改法（prompt 段草案，插在 `single_candidate_input.inc.rs:239` 的审核边界段之后）**：
```
[review_exploration_scope]
下列判断只依据 Plan Review Context 即可完成，不得为它们读仓库：
- 契约供需是否闭合、能力是否逐字覆盖 → Cross-Item Contract Edges 的 missing 列表
- 每个 acceptance criterion 是否都有配对的 reviewer_check_refs → Canonical Contract Candidates 的 handoff_contract
- 溯源登记的 requirement_id 是否在 spec 清单内、语义是否相符 → design_traceability + [confirmed_context] 的 spec 原文
- 拓扑是否有环、写入范围是否冲突 → Structure Verdict
只有两类事实允许读仓库：
(a) Plan 的 Verification.command / manual_instruction 所依赖的脚本、容器、端口、路径是否真实存在且前提成立；
(b) Plan 声明的能力是否与仓库既有实现冲突（例如声称要新增的能力其实已存在、或声称要改的目录其实已被统一机制覆盖）。
读仓库时只读与该结论直接相关的文件，且每条结论必须给出文件路径证据。
```
**逐条对应实测的 14 次探索**：

| reviewer 的探索 | 新边界下的归属 | 保留？ |
|---|---|---|
| `ls` 成员仓树 / `find gateway/src frontend/src` | 目录结构核对 | **可省**（`(b)` 类只需定位单个文件，不必全树扫描） |
| `cat NoticeClient.java` | `(b)` 既有实现是否已覆盖 | **保留**（finding 4 同类） |
| `cat GlobalExceptionHandler.java` / `ErrorCode.java` | `(b)` 既有实现 | **保留**（finding 4 的依据） |
| `cat frontend/src/api.js` / `App.vue` | `(a)` 命令依赖的脚本/路径 | **保留**（finding 3 的依据） |
| `cat package.json` / `eslint.config.js` / `vite.config.js` | `(a)` 命令前提 | **保留**（finding 2 的依据） |
| `cat NoticeControllerTest.java` / `OpenApiDocTest.java` | `(a)` 命令依赖的测试文件 | **保留** |
| `cat frontend-ui-standards.md` / `grep ElMessageBox` | 项目规范/组件是否可用 | **归 (b)**，但只在校验相关组件时读一次 |
| policy 政策正文读取（3 次） | `[cadence_rule_read_gate]` 强制 | **不可省**（平台强制） |

**净收益（推导，非实测）**：reviewer 探索 14 次 → **~8-9 次**（省掉全树扫描 + 重复的目录定位 + 与 finding 无关的文件），探索输出 64,595 → **~38,000~42,000 字节（−35%~−41%）**。
⚠️ **注意量级**：按基线 §11.1，reviewer 探索时间的绝对值不大；这里的收益主要是 **token/上下文占用**，不是墙钟。**基线 §12 的记账缺陷（2.4×~5.3×）使精确 token 数不可测 → 净账需实测**。

**R2 —— `confirmed_context` 已在 prompt 里含 spec 原文，溯源核对零探索成本**。
实测：`[confirmed_context]` 的 `story:` 段含 story spec 全文，含 `- **[AC-001]** 公告列表中每一行末尾都显示「删除」按钮…` 的**逐条 AC 原文**（我逐条 grep 过 AC-001..AC-006 全部命中）。
→ **§3.4 L2 的「登记前必须回到 spec 原文核对」不需要任何额外探索**，spec 原文已在 prompt 里。这是本方案「零探索成本」的依据。

**R3 —— prompt caching 友好布局：静态在前、动态在后**。
基线 §12 + 调研结论 5：author 侧 `cache_read_tokens` 仅 128（≈0）→ 32,609 字符的静态 prompt 在重驱时**被完整重发**。
- 现状 author prompt 布局：**动态段 `[confirmed_context]`（14,835 字符）在中段**，静态规范面（CRD/WMP/GRM/few_shot）在其后 → 前缀不稳定，缓存无从命中。
- **改法**：把 `[confirmed_context]` 移到**最后**（紧邻 `[output]`），静态规范面提到最前 → 前缀 = 纯静态规范面（~13,084 字符 ≈ 3.5k token，**高于 provider 最小缓存阈值 1024/4096**）→ 可命中缓存。
- **收益量级**：调研结论 5 实测区间 = prompt caching 成本 **−41%~−80%**、TTFT **−6%~−31%**。本项目 prompt 32,609 字符 ≈ 8k token，**高于阈值，可受益**。**本项目净账需实测**（基线 §12 记账缺陷）。
- **代价**：§1 的所有 prompt 改动都会改变静态前缀 → 首次跑必然冷缓存；**改动上线后每次重驱都命中**。且 §3 的 `[design_requirements]` 是**动态的**（随 spec 变）→ 它也必须移到动态区（否则静态前缀含动态内容 → 缓存永不稳定）。
- ⚠️ **与 Main/性能角色的交互点**：缓存布局属性能角色的地盘。我只提出约束：**「静态规范面必须连续前置、动态段（`[design_requirements]`/`[confirmed_context]`）必须后置且不得夹在静态段中间」**；具体切分点由性能角色裁决。

**R4 —— author 侧：把「命令可执行性」上移（C 的落地）**。
在 author prompt 的 `[markdown_grammar]` 第 6 行（现文「Verification.command 直接声明，将按声明执行；命令证据不足时改用 manual_instruction 或 blocker，禁止臆造命令。」）后补一句可机械执行的约束：
```
Verification.command 只能写你在本仓库实际读到过、且确认可执行的确切命令；命令依赖的脚本/配置/容器入口必须在 plan 中以文件路径或命令参数逐字给出，使复核者不必推测即可复现；无法确认时写 - command: null 并在 manual_instruction 给出确定的人工步骤。
```
**收益**：让 reviewer 在 `(a)` 类问题上**多数情况不必读仓库**（命令已自证）；同时直接提升 plan 质量（作者被迫核实命令）。
**代价**：作者侧增加一次核实成本 → 但按 §11.1，探索只占 ~200 秒；且**这正是用户抱怨①的正解方向**（模型本来就在读这些文件，只是没有把结论沉淀进 plan）。

---

## 6. 第 6 题：reviewer 输出契约的准确性（`class_hint` 该不该去掉）

### 6.1 平台侧实际信任链（实测代码路径）

```
classify.rs:117-131  classify_finding():
    if !finding.is_actionable() { class = Advisory }          ← disposition 优先，先于 class_hint
    else { class = class_hint.map(class_for_hint).unwrap_or_else(|| fallback_class(raw_verdict, finding)) }
```
```
review.rs:176-178  is_actionable() = disposition_or_fail_safe() == Actionable
review.rs:170-173  disposition 缺失 → None → Actionable（fail-safe）
```
```
parsers.rs:427-455  effective_work_item_plan_review_verdict():
    pass 且存在 finding.is_actionable() && severity ∈ {blocking, must_fix} → Revise
```
```
evaluate.rs:381-390  任何 finding.class == HumanRequired → HumanRequired（转人工）
evaluate.rs:404-415  is_automatic_repair_candidate = MechanicalError | Repairable
```

### 6.2 `class_hint` 的残余作用（去掉后会怎样）

去掉 `class_hint` 后 `classify_finding` 走 `fallback_class(raw_verdict, finding)`（`classify.rs:161-172`）：
- `raw_verdict = Pass` → **Advisory**（全部）
- `raw_verdict = Revise` + category ∈ {contract_gap, self_contradiction} + contract_field 非空 → **Repairable**
- `raw_verdict = Revise` 其它 → **HumanRequired**
- `raw_verdict = NeedsHuman` → **HumanRequired**

**关键**：`class_hint` **只在 `disposition == actionable` 时才被读**（`classify.rs:117` 先判 actionable）。而实测 reviewer 输出（`timeline_node_002.json` 的 verdict）：**5 条 finding 的 `disposition` 是 4×advisory + 1×actionable**，而 5 条的 `class_hint` **全是 advisory**。

即：**本轮 `class_hint` 对最终分类的贡献 = 0**（4 条走 Advisory 分支不看 class_hint；1 条 actionable 的 category=contract_gap 但 class_hint=advisory → class=Advisory，与 `fallback_class(Pass,…)` 结果**相同**）。

**收益量化（去 `class_hint` 后）**：
- 本轮：分类结果**逐条不变**（实测推导）。
- 一般情形：唯一会变的是「reviewer 判 `revise` + `disposition=actionable` + category ∉ {contract_gap, self_contradiction} + 无 contract_field」→ 原 `class_hint=repairable` → Repairable（自动返修）；去掉后 → fallback → **HumanRequired**（转人工）。
  → 这**正是我们想要的**：把「自动返修」的判定权从模型手里收回到平台侧。基线 §8③ 的抱怨正是「reviewer 反复失败耗尽自动化预算」——`class_hint=repairable` 让模型能自己宣称「这条我能自动修」，是预算耗尽的一条路径。

### 6.3 推荐

**推荐：让 reviewer 只输出 `disposition`，不输出 `class_hint`；`category` 保留但降为「可空标注」**。

理由：
1. **平台对 `class_hint` 的信任度本就有限**（`classify.rs:117` 让 `disposition` 先于它）→ 去掉不损失分类信息（§6.2 已推导本轮逐条不变）。
2. **`class_hint` 是模型对「我这条能不能自动修」的自评** → 去掉后自动返修的资格由 `fallback_class` + `contract_field` 决定（更保守：需要具体的 contract_field 才能自动修）。
3. **`class_hint` 的 3 值白名单本身是失败源**：`parsers.rs:772-785` 的 `parse_finding_class_hint` 对未知值返回 `UnknownFindingClassHint` → `routing.rs:415-428` 判 **fatal**（`FatalReason::UnknownClassHint`）→ **整轮 abort**。同理 `category` 的 6 值白名单 → `FatalReason::UnknownCategory` → 整轮 abort。
   → 实测本轮 5 条 finding 的 `category` 全合法，但**模型输出白名单外的 category 是整轮 fatal 的单点**。这正是「reviewer 反复失败耗尽预算」的机制之一。
4. **`class_hint` 与 `disposition` 语义重叠但不一致**：本轮实测出现 `disposition=actionable` + `class_hint=advisory` 的组合（finding 1）→ 模型自己都没对齐两个字段。去掉一个减少不一致面。

**但 `category` 不能一起去掉**：
- `category` 是 **fingerprint 身份的第一成分**（`fingerprint.rs:91-118`：`category=None` → legacy 域，哈希 class+message+field；`category=Some` → 稳定/不稳定域，哈希 category+ID集+尾段）。
- 去掉 `category` → 所有 finding 落 legacy 域 → 指纹依赖 `message` 措辞 → **同题异措辞不再 collapse** → `RepeatedFingerprint` 闸门（`evaluate.rs:311-330`，用户抱怨③的直接闸门）**失效**。
→ **保留 `category`，但改为「缺失时按 `other` 兜底而非 fatal」**（把 `UnknownCategory` fatal 降级为 warning）。

**具体改动清单**：
| 位置 | 改动 |
|---|---|
| `review.rs:39-113` `review_scope_instructions` | 删「每个 finding 必须提供 category 与 class_hint 建议」；改为「category 建议给出，取不到用 other」；删 class_hint 白名单行；**删「每个 finding 对象只能包含以下字段：severity、message、evidence、required_action、category、class_hint、contract_field」这句**（它与 SC 路径的 schema 冲突，见 §6.4） |
| `single_candidate_input.inc.rs:250-262` schema 串 | findings 对象删 `category`/`class_hint`（现 schema 串里**本来就没有** `category`/`class_hint`！实测：`{"verdict":…,"findings":[{"severity","disposition","target_work_item_id","message","evidence","required_action","owner_question","owner_options"}]}`）→ **schema 侧已经不含 class_hint，只需删 `review_scope_instructions` 里那句矛盾的话** |
| `parsers.rs:753-770` `parse_finding_category` | 未知 category → 不再返回 Err，改 `Ok(None)` 或 `Ok(Other)`；同步 `routing.rs:411-424` 的 `UnknownCategory` fatal 分支移除 |
| `parsers.rs:772-785` `parse_finding_class_hint` | 若 schema 不再输出 class_hint → 该字段恒 None → `classify.rs` 走 fallback ✅。**保留解析函数**（历史记录兼容） |
| `review.rs:39-113` 增补 | 明确「平台按 disposition 决定是否阻断；category 只用于指纹稳定性，尽量给出」 |

### 6.4 顺带发现的一处**真实矛盾**（必须一并修）

`review_scope_instructions`（`review.rs:63`）说：
> 每个 finding 对象只能包含以下字段：severity、message、evidence（可选）、required_action（可选）、category、class_hint、contract_field（可选）——不得添加 finding_id、code、work_item_ids 或其他字段。

但 SC 路径的 schema 串（`single_candidate_input.inc.rs:255`）要求 findings 含 **`disposition`、`target_work_item_id`、`owner_question`、`owner_options`** —— **这四个字段在 scope 指令的「只能包含」白名单里没有**，而 `disposition` 恰恰是平台阻断链的第一环（`review.rs:176`）。

即：**scope 指令按字面读会禁止模型输出 `disposition`** → `disposition` 缺失 → `disposition_or_fail_safe()` → Actionable（fail-safe）→ **advisory finding 被当成 actionable** → 阻断率虚高、返修预算被误消耗。

实测本轮 reviewer 输出了 `disposition`（5/5）——说明模型**没有**照 scope 指令的字面白名单执行（schema 串在 prompt 里更靠后、更具体）。**但这是靠模型「猜对」维持的，不是有约束保证的**。

**处置**：把 scope 指令里的字段白名单改为与 SC schema 一致，或直接删掉那句（字段白名单由 schema 串 + strict 解析器 `parsers.rs:679-702` 双重保证，无需 prompt 重复）。
**推荐**：**删掉那句白名单**（strict 解析器已经在 `parsers.rs:679-702` 拒白名单外字段，prompt 再说一遍只会产生矛盾）。
**兼容性**：`parsers.rs:679-702` 的 strict 白名单**已经包含** `disposition`/`target_work_item_id`/`owner_question`/`owner_options` ✅ → 删 prompt 那句不损失任何机器约束。

---

## 7. 第 7 题：重构后目标形态 + 确定性测试

### 7.1 生成 prompt 目标形态（分段 + 职责 + 字数预算）

**布局原则（R3 呼应）**：**静态规范面前置、动态内容后置**，让缓存前缀连续稳定。

| # | 段 | 职责 | 目标字符 | 目标字节 | vs 现在 |
|---|---|---|---|---|---|
| 1 | `[issue]` + `[routing_reference]` + `[cadence_project_rules]` + `[project_rule_priority]` + `[code_usage_summary]` + `[code_reading_summary]` + `[cadence_rule_read_gate]` | 上下文路由与语言纪律 | 1,779 | 3,225 | 不变 |
| 2 | `[markdown_grammar]` | **机械边界单源**：section/key/值域/EARS/ID 规则 | 3,600 | 5,270 | −79 字符（删 Verification.command 重复行） |
| 3 | `[cross_reference_discipline]` | **跨引用纪律**：两域定义 + 判定规则 + 正反例 | 2,183 | 3,347 | +381 字符（§3.2.2 草案） |
| 4 | `[weak_model_precision]` | **弱模型精度**：AC 配对 + 两域引用 + 输出契约 + 成对书写 | 1,266 | 1,896 | −55 字符 |
| 5 | `[real_finding_few_shot]` | **真实判例**：错误模式 → 修正原则（证据改写为 SC 合法 key） | 3,800 | 6,900 | −901 字符 |
| 6 | `[minimum_legal_source]` | **最小合法形状**（WIC 版 + 配对说明） | 1,240 | 1,290 | +28 字符 |
| 7 | `[traceability_rule]`（新，替 `[multi_source_traceability_example]`） | **溯源真实性**：spec 原文核对 + 禁按编号登记 | 330 | 600 | −39 字符 + 新段 |
| 8 | `[source_boundary]` | 输出边界 | 340 | 700 | −183 字符（删 2 行重复） |
| 9 | `[plan_options_mirror]` + `[confirmed_member_order]` | 条件渲染 + LC 专属 | 2,001 | 3,013 | 不变 |
| 10 | `[format_clamp]` + `[output]` | 末尾钳制 | 206 | 392 | 不变 |
| — | **静态小计** | | **16,745** | **26,633** | 现在 13,084 静态 + 其它 ≈ 15,700 → **+~1,000** |
| 11 | `[issue]`… 已在 1 | | | | |
| 12 | `[design_requirements]`（**动态**，spec 清单） | 合法编号清单（两域标注） | 240 | 380 | +119 字符 |
| 13 | `[confirmed_context]`（**动态**，story+design+structure） | 事实上下文（含 spec 原文，§5.4 R2） | 14,835 | 24,211 | 不变 |
| — | **总计** | | **32,030** | **51,224** | 现在 32,609 / 52,355 → **−579 字符 / −1,131 字节（−2.2%）** |

**关键结论**：
- **字节只降 2.2%**，因为 §3 的两域规则与 §3.4 的溯源规则是**净增**（+~1,000 字符），而 §1/§2 的去重是净减（−~1,600 字符）。
- **这个净增是必须的**：它买的是「消除 15 条 IR 失败 + 堵住溯源造假」。省字节不是本方案的目标，**修规则才是**。
- 若同时采纳 §5.3 R3（`[confirmed_context]` 移到末尾），静态前缀 = 段 1~11 = 26,633 字节 ≈ 6.6k token **连续稳定** → 可命中缓存。

**预算影响**：
- `WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES = 23_500` 是**字节**口径，测的是 **fixture prompt**（`tests/prompt_contract.rs:1037` 用 `split_prompt_fixture()`，fixture 的 story/design context 极小）。
- 我**没有实测 fixture prompt 的当前字节数**（需要跑 `cargo test`，本片段是设计角色不执行验证）。基线批注称「最大 fixture 实测 23,456B / 余 44B」→ **余量仅 44 字节**。
- → **§3 的净增（+~1,000 字符 ≈ +~1,700 字节）会击穿 23,500 红线** → 必须**先按仓库既有惯例提额**（`prompts.rs:136` 上方批注记录了 14 次提额历史，规则是「先上调至整百级并注明 margin 惯例，再添加教学内容」）。
- **建议提额至 25,500**（+2,000），留 ~800 字节余量。这与仓库惯例一致（历史提额幅度 200~2,000）。

### 7.2 review prompt 目标形态（与 §4.4 字节账一致）

| # | 段 | 职责 | 目标字节 | vs 现在 |
|---|---|---|---|---|
| 1 | `## Plan Review Context` 头 | 范围声明 | 24 | 不变 |
| 2 | `### Canonical Contract Candidates`（**+ handoff_contract + design_traceability**） | 审查对象本体 + 现在缺的配对/溯源 | 24,600 | **+2,285** |
| 3 | `### Cross-Item Contract Edges (gaps only)` | 跨 WI 契约供需**缺口**投影 | 462 | 新增 |
| 4 | `### Handoff Consumption` | 契约是否真被下游消费 | 383 | 新增 |
| 5 | `### Write Scope Conflicts (same repository)` | 只留同仓真实冲突 | 441 | 新增 |
| 6 | `### Projection Validation Report` | 机械校验摘要 | 584 | 不变 |
| 7 | `### Structure Verdict` | 拓扑合法性摘要 | 219 | 新增（从 DSR 1,948 压缩） |
| 8 | `### Structural Repair Audit` | 条件化：初轮只留一句说明 | 85 | −130 |
| 9 | 审核边界 + 能力覆盖机械规则 + repetition + cadence 规则 + read gate + 输出契约 + OWNER/FINDING scope rules + schema 示例/模板 | **活的 reviewer 纪律（逐字保留）** | 6,900 | **不变（不可动）** |
| 10 | `## 服务端审核 invocation scope`（删字段白名单句 + 删 class_hint 必填行） | 协议边界 | 1,358 | −~150 |
| — | **结构段小计** | | **27,235** | 44,942 → **−17,707（−39.4%）** |
| — | **总计** | | **≈ 34,590** | **52,297 → −17,707 字节（−33.9%）** |

第 9 项 6,900 字节 = §4.2 表中「活的纪律」逐段相加（387+197+197+172+220+422+703+557+544+914 = 4,313 字符 → 字节需实施时精确测，此处按比例估）。**该段是 §4.2 已纠正的「被 Structural Repair Audit 吞掉的 4,521 字符」，必须逐字保留。**

### 7.3 确定性测试：现有测试怎么办

| 现有测试 | 处置 | 具体 |
|---|---|---|
| `prompts.rs:136` `WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES = 23_500` | **提额至 25,500** | 按 `prompts.rs:136-166` 的批注惯例写明「第 N 次提额 + margin + 触发原因」 |
| `prompt_contract.rs:1033-1035` 断言 `== 23_500` | **改** | → `25_500`，断言消息同步 |
| `prompt_contract.rs:1189-1193` 断言 `== 23_500` | **改** | 同上 |
| `prompt_contract_sc_revision.rs:584-586` 断言 `== 23_500` | **改** | 同上 |
| `prompt_contract.rs:820` / `:896` 断言 GRM 第 8 行逐字旧串 | **改** | → §1.5 新串 |
| `prompt_contract.rs:852-1016` 逐条断言（含 `done_when_refs 仅引用先定义 criterion_id`、`requirement_refs 仅引用清单；清单外 REQ-* 拒绝。`、CRD 全 19 行、few_shot 全字段） | **大改** | 逐条映射到新串；**删除**被合并/删除的行（CRD 第 6-8 行三合一、CRD 第 12 行、GRM 第 6 行、source_boundary 2 行、few_shot 标题正反例）的断言 |
| `prompt_contract.rs:972-1000` 断言 11 条判例逐字在 prompt 中 | **保留 + 新增** | 保留（fixture 不改）；新增「prompt 中不出现 `oc_`/`criterion_backend`/`draft_00` 等 SC 不存在的字段路径」 |
| `prompt_contract.rs:1002-1012` 断言 `parse_work_item_plan(minimum_source)` 解析出 1 item | **保留（自动通过）** | 前提：minimum_legal_source 已改 WIC 版 + `parse.rs:443` 已改 |
| `prompt_contract_weak_model.rs:37/38/138/139` 断言 `AC-xxx` / `AC-001` | **改** | → `WIC-xxx` / `WIC-001` |
| `prompt_contract_weak_model.rs:40-42` 断言「引用纪律：requirement_refs/done_when_refs 只能逐字复制 spec 已定义 id…」 | **改** | → §3.2.2 两域版（**这三条断言钉的正是病灶**） |
| `prompt_contract_weak_model.rs:52-58` 断言 CRD 四句保留 | **改** | 「requirement_refs 仅引用清单；清单外 REQ-* 拒绝。」被改写 |
| `prompt_contract_weak_model.rs:163-172` 反例构造 | **改** | `reviewer_check_refs: WIC-001` → `WIC-002` |
| `prompt_contract_weak_model.rs:245` 断言 message 含 `AC-001 has no handoff reviewer check` | **改** | → `WIC-001 has no handoff reviewer check`（`validation.rs:311` 的 message 是 `format!("acceptance criterion {criterion_id} has no handoff reviewer check")` → 自动跟随 criterion_id 值 ✅） |
| `prompt_contract.rs:1040-1047` 断言 prompt 长度 < 预算 | **保留** | 提额后自动通过 |

**新增确定性测试**（本方案应带的测试）：

| 新测试 | 断言 | 对应缺陷 |
|---|---|---|
| `sc_author_prompt_separates_spec_and_plan_id_domains` | prompt 同时含「spec 域」与「plan 域」两句；且**不含**旧串「done_when_refs 仅引用先定义 criterion_id」 | §9.3 |
| `sc_author_prompt_forbids_cross_wi_done_when_refs` | **行为级**：用 `minimum_legal_source` 构造一个「task 的 done_when_refs 指向另一 WI 的 criterion_id」的 plan → 断言被 `validation.rs` 以 `unknown_done_when_ref` 拒绝 | §9.1/§9.2（**这是本方案最重要的一条测试**） |
| `sc_author_prompt_traceability_rule_forbids_number_coincidence` | prompt 含「禁止按编号相近/顺序对应来登记」；且**不含** `[multi_source_traceability_example]` 旧段头 | §10.2 |
| `compiler_accepts_wic_criterion_prefix_and_isolates_fingerprints` | `parse.rs` 接受 `WIC-001`；`stable_ids("WIC-001") != stable_ids("AC-001")`；`WIC-001` 不被 `AC-` 前缀抽出 | §3.3 |
| `review_prompt_omits_audit_only_sections` | review prompt **不含** `Structure Context Identity` / `Immutable Candidate Artifact Refs` / `Structural Repair Audit` 的 JSON 段；**含** `handoff_contract` 与 `design_traceability` | §4 |
| `review_prompt_does_not_forbid_disposition_field` | review prompt **不含**「每个 finding 对象只能包含以下字段」那句（避免禁掉 `disposition`） | §6.4 |
| `review_prompt_declares_class_hint_optional` | review prompt 不含「class_hint 只能取三值之一」的**必填**表述 | §6.3 |

**测试形态建议**：新增断言全部走**「prompt 逐字 contains / not-contains」**（与仓库既有 `prompt_contract*.rs` 一致）+ **行为级编译/校验闭环**（复用 `prompt_contract_weak_model.rs:167-250` 已有的 compile→validate→断言错误码范式）。

### 7.4 验证方式与「先修记账」的要求

**基线 §12 的记账缺陷使任何 token 收益都测不准**：
- 平台上报 author `in 90,486 / out 43,929`，转录逐条求和 `in 213,837 / out 232,106` → **差 2.4×~5.3×**。
- 采集点：`cross_cutting/claude_code_provider/stream.rs:334/474-486` 的 `parse_claude_result_usage`（只取 `result` 事件 usage 的 4 字段，不跨请求累加）。
→ **所有「省 X token」的估算必须标注「当前测不准；先修 `parse_claude_result_usage` 的跨请求累加，再量化」**。

本片段的**可确定性验证**部分（不依赖 token 记账）：
| 指标 | 怎么测 | 期望 |
|---|---|---|
| prompt 字节 | `prompt.len()`（Rust，字节口径） | 生成 prompt 52,355 → **~51,200 字节**（−2.2%）；review prompt 52,297 → **~34,590 字节（−33.9%）** |
| IR 失败率 | 跑 `validate_plan_candidate_ir` 于新 prompt 的产出 | run1 的 **15 条 `unknown_done_when_ref` → 0**（§3.2 规则修正的直接 KPI） |
| 溯源准确率 | 人工/LLM 判定新产出 Traceability 的 requirement_id 与 spec 原文语义是否相符 | 基线 §10.2 的 **6 条中 4 条不符 → 0**（§3.3 隔离 + §3.4 L2 的 KPI） |
| reviewer 覆盖面 | review prompt 中 `reviewer_check_refs` / `design_traceability` 出现次数 | **0 → ≥1**（§4.3 L3/L4） |
| reviewer 探索次数 | `execution_events` 中 shell 调用数 | 14 → **~8~9**（§5.4 R1，推导非实测） |
| 预算红线 | fixture prompt 字节 | < 25,500（提额后） |
| reviewer 阻断正确性 | 构造「advisory 但模型误标 blocking」用例，断言平台仍按 disposition 降级 | 不变（`classify.rs:117` + `review.rs:187` 已有） |
| 缓存命中 | 重驱轮的 `usage.cache_read_tokens` | 现在 128（≈0）→ **显著 > 0**（§5.4 R3，需先修 §12 记账才能测） |

---

## 8. 与其他角色的交互点（不越界，只标注）

| 角色 | 交互点 | 方向 |
|---|---|---|
| **DesignPerfToken** | ① R3 缓存布局：静态规范面必须连续前置、`[confirmed_context]`/`[design_requirements]` 必须后置且不得夹在静态段中间（§5.4 R3）。② §3.2/§3.4 的规则增字会让 author 多「想」——按基线 §11.1，探索只占 ~200 秒，长思考才是 44 分钟主因 → **规则增字不增加墙钟主因**。③ reviewer 探索收窄（§5.4 R1/R4）与性能角色的探索预算控制是同一件事的两面。④ **基线 §12 的记账缺陷**：任何 prompt 改动的 token 收益都测不准 → 建议性能角色先修 `parse_claude_result_usage` 的跨请求累加，否则双方都无法量化 |
| **DesignConvergence** | ① 「返修已有 ID 禁止重编号，新增 criterion 用最大已分配序号+1」→ §1.5 的 GRM 第 8 行改写已对齐（去掉「文档顺序严格递增」）。② 命名隔离：§3.3 选 **`WIC-`**（不选 WAC-/PAC-，因 `fingerprint.rs` 的 `find` 无 token 边界会碰撞 —— 你的 scout 结论我独立复核属实）。③ **`forged_done_when` 删除**：§3.3.4 建议删（它与 `validation.rs` 作用域相反，基线 §10.3）；删后由 `validation.rs` 逐 WI 校验 D1、`forged` 校验 D2。**需要你确认**：你的收敛方案若依赖 `forged_done_when` 覆盖 D1，删除会留缺口（我的方案是 D1 由 prompt 规则 + `validation.rs` 权威覆盖，不依赖它） |
| **Main** | ① §1.1 的测量口径纠正：基线 §4 的「cross_reference_discipline 6,803」与「15,000 静态面」两个数字都偏大，真实值 **1,802 / 13,084** → 建议同步更新证据基线。② §4.2 的切分纠正：基线 §13 表格的「Structural Repair Audit 4,651 字符（11%）全空」**前半句成立、后半句不成立** —— 真 payload 只有 **129 字符（0.3% 全文）**，另 4,521 字符（10.7%）是**活着的 reviewer 纪律**（含 `FINDING_SCOPE_RULES` 422 字符 = disposition 语义 + 范围外降级 + owner 去重，正是抱怨③的约束面）→ **不可条件化尾部**。③ §6.4 的「scope 指令禁 `disposition`」是**独立于本次三个抱怨的第四个缺陷**，它直接放大抱怨③（advisory 被当 actionable → 返修预算被误耗）→ 建议纳入本轮 |

---

## 9. 状态

**DONE_WITH_CONCERNS**

**已核实的（实测，非引用基线）**：
- author prompt 逐段字符/字节（自己用 `^\s*\[[a-z_]+\]` 切分）→ 基线 §4 的 `cross_reference_discipline` 数字偏大 **3.8 倍**（真实 1,802 字符 / 2,822 字节）
- run1 的 15 条 `unknown_done_when_ref` 全部为跨 WI 引用（逐条交叉验证：cross-WI=15, same-WI=0）
- `validation.rs:245-270` 按 WI 本地作用域；`single_candidate.rs:416-434` 按 spec 全局作用域（两处相反）
- `criterion_id` 值格式**只在 `parse.rs:443`**，不在 `grammar.rs`（`grammar.rs:85` 只是 key 名；`grammar.rs` 无任何值格式校验）
- `fingerprint.rs:145-160` 的 `find(prefix)` 无 token 边界 → `WAC-`/`PAC-` 会与 `AC-` 碰撞（DesignConvergence 的 scout 结论独立复核属实）
- review prompt 9 section 的逐段字符**与字节**；`reviewer_check_refs`/`provided_contract_refs`/`required_fields`/`design_traceability`/`source_id` 在 review prompt 中出现 **0** 次 → reviewer 对 AC 配对与溯源**结构性失明**
- review prompt 的 `write_scope_conflicts` 29 条中 **27 条跨仓**（假阳性），仅 2 条同仓（且那 2 条是**真实**冲突信号，必须保留）
- **coverage/cross 与 CCC 的重叠是精确重复**（集合差实测）：coverage 的 3 条边 100% 可从 CCC 推出（差集 0）、16 条 provided + 10 条 required capability **全部**已逐字在 CCC、`missing_capabilities` 差集为空；cross 的 29 条 distinct 串 **29/29** 已在 CCC
- **平台侧 join 校验不依赖模型读审计面**：我核实了 `single_candidate_input.inc.rs:96-140` 的五项比对（ir_id / source_revision_hash / compiler_version / context_digest / rule_version）全在 Rust 侧 → 删 ICAR + SCI 零平台影响
- **「Structural Repair Audit 4,651 全空」是切分假象**：真 payload 129 字符，另 4,521 字符是活纪律（逐段偏移表见 §4.2）
- reviewer 5 条 finding 中 **3 条依赖仓库探索**（eslint 配置 / gateway 异常处理 / api.js）→ 「reviewer 只审 IR」会失去这些依据
- reviewer 探索输出 64,595 字节 = author 的 6.5 倍
- spec 原文（AC-001..AC-006 全文）**已在** `[confirmed_context]` 的 `story:` 段内 → §3.4 L2 零探索成本
- `classify.rs:117` 的 disposition 优先链；本轮 `class_hint` 对分类贡献 = 0（5 条 finding 逐条推导）
- `review_scope_instructions` 的字段白名单与 SC schema 冲突（禁掉了 `disposition`）

**未核实（需实施时验证）**：
- fixture prompt 的当前字节数与提额后的余量（需跑 `cargo test`；我只从代码批注读到「23,456B/余 44B」）
- review prompt 重构后的精确字节（§4.4/§7.2 给的是实测基线 + 构造值，实施时需复测）
- 一切 token 收益（基线 §12 记账缺陷 2.4×~5.3×）→ **先修 `parse_claude_result_usage` 再量化**
- §5.4 R1 的 reviewer 探索次数下降（8~9 次是逐条推导，非实测）
- §3.3 隔离后 §10.2 语义造假**不能完全堵住**（需 §3.4 L2 补齐；L2 的效果需实测）
- L3（平台语义校验）的误判率 → 我建议**不做**，故未评估
- 缓存命中率提升（§5.4 R3）→ 依赖记账修复

**需 Main / DesignConvergence 裁决的**：
1. `forged_done_when` 是否删除（§3.3.4）——与 DesignConvergence 的收敛方案耦合
2. 预算提额至 25,500 是否照批注惯例批（§7.1）
3. §6.4 的 scope 指令字段白名单矛盾是否纳入本轮（§8）
4. 证据基线的两处切分口径纠正是否同步更新（§8 → Main）
