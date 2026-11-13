# Tasks: repair-wig-fact-projections

> **8 个 Task × 5 个阶段 = 35 个编号工作包**（每个 Task 的末条合并承载 W3/W4/W5）。每个 Task 的五阶段固定为：
> **W1** 红测/不变量测试（先红，失败原因须为断言缺失而非编译错误）· **W2** 实现 · **W3** controller 定向红/绿证据 · **W4** 独立只读评审与修正 · **W5** 固定提交与回执。
> **不是**把文档、准备或评审伪装成代码 Task。**每 Task ≤3 个文件且 <4 h。**
>
> **实施基准**：干净 `a953327d`（序4 的 8 个半成品文件已 park 于 `wip/wig-seq4-t1`，**不进入任何 lane 的 diff**）。
> **门禁**：任何 cargo/fmt/rustfmt 验证**只在所有 lane 静止时**由 controller 执行；worker 只写红测与实现。

## 0. 契约与归属（controller，非代码 Task）

- [x] 0.1 **契约双审**：proposal / design / specs delta / tasks 四件齐备；一致性裁决 + 接地审查各一轮。验证：双评审结论与修正记录
- [x] 0.2 **requirement 归属唯一**：本 change 声明 REQ-WSC-06（**最新态经 `deliver-d5-d6-fact-injection` 同步进主规格**，本 change 对其为 MODIFIED）、REQ-WSC-32（承接自 `teach-sc-compile-reredrive-feedback`）、REQ-WSC-12（MODIFIED）三者互不相同；无其它 active change 声明同一 requirement。验证：`openspec list` + 全目录 grep
- [x] 0.3 **描述段门禁前置**：三个 requirement 的描述段按 **openspec 自身解析口径**实测（`openspec show … --deltas-only` → `requirement.text` 长度）REQ-WSC-06 = **479**、REQ-WSC-12 = **488**、REQ-WSC-32 = **349** 字符（均 ≤500；裸 markdown 计法为 480/489/349，差异 ≤1 字符源于段落间空白归并）。验证：python 实测 + openspec 解析复核
- [x] 0.4 **既有场景逐字保全**：主规格 REQ-WSC-06 的 **34 个**场景、REQ-WSC-12 的 **5 个**场景标题与正文**逐字**进入 delta。验证：脚本比对（主规格侧零改动）
- [x] 0.5 **已发现的规格缺陷与修正（评审前自查发现）**：S0 拆分 `deliver-d5-d6-fact-injection` 时，主规格 REQ-WSC-06 出现**两个同名场景**——「注入前轮结构化清单」「重复判定不靠记忆」各有**弱版**（原有 28 场景内）与**强化版**（D5/D6 交付内容，多出「SHALL 附本轮机械复算状态」「且按内容身份去重」「SHALL 以平台给出的复算分区事实为准」等 SHALL）。**同名两版并存 ⇒ 规范歧义（哪一版 binding 不定）。** **修正**：以「交付行为为准」保留**强化版**、删除弱版 ⇒ 主规格 REQ-WSC-06 由 36 场景收敛为 **34 场景**（28 − 2 + 8）。**义务零丢失**（强化版是弱版的**严格超集**，只增不减 SHALL）；其余 **26 个原有场景逐字未变**。**同步修正**：归档拆分件 delta（36 → 34）与本 change 的 delta（REQ-WSC-06 = 34 + 4 新 = 38）。验证：脚本比对「主规格 REQ-WSC-06 标题集合 == 归档拆分件标题集合」、重复标题零命中、26 个原有场景正文逐字比对通过
- [x] 0.6 **delta 形态归一**：`## MODIFIED Requirements` 与 `## ADDED Requirements` 各**一个**头，其下挂多个 `### Requirement:` 块（此前 REQ-WSC-06 与 REQ-WSC-12 各带一个重复头，虽 openspec 可容忍，但非规范形态）。验证：`grep '^## '` 仅两行


## 1. A1 · 预算内事实不丢字段（3 文件）

> 独占文件：`web/workspace_ws_handler/run/single_candidate_compile_preflight.rs`（1141/1200，≤+40）· `product/workspace_engine/prompts/sc_compile_teaching.rs`（357/1200，**+180～300**；原 +150～250 偏乐观）· `web/workspace_ws_handler/run/single_candidate_structure_reredrive.rs`（679/1200，+100～180）。详测**内联**进后两文件。

- [x] 1.1 **W1 红测**：多条**同 message、不同 field** 的真实形状 fixture ⇒ 最终教学 prompt 逐条含原 `code:line:message` 三段 + 精确 `field` + `repair_example`；原三段逐字在场。验证：红测存在且失败原因为断言缺失
- [x] 1.2 **W2 实现**：保留 `TeachingPayload.parse_reasons: Vec<String>` 与既有 builder 签名（**不引入新持久 schema、不引入 patch 协议**）；每条附 field/example；**仅** `unknown_structured_key` 分支按需附 40 个合法 `STRUCTURED_KEYS`。不解析 message 猜字段、不填业务值
- [x] 1.3 **W2 实现（有界投影）**：parse 事实块 ≤32,000 B，按**完整 UTF-8 行**计量；只保留完整诊断块；单条过大**整条省略**；显式呈现总数 / 呈现数 / **省略数**；顺序稳定。**不新增失败、不新增模型调用、不新增重试**。旧 lowering/IR/registered IDs **不扩修、不截断**
- [x] 1.4 **W2 实现（审计与身份中性）**：完整五字段走**独立审计文本**（可放既有 `source_missing_reason` 的明确标记分区，保留原缺失说明）；**不得拼入被哈希的 `detail`**；`failure_id` / attempt 身份 / 终态 code·message / fingerprint **逐字不变**；审计写失败沿既有 `candidate_attempt_log_write_failed` 可见降级
- [x] 1.5 **W3–W5**：定向红/绿证据 → 独立只读评审与修正 → 固定提交与回执。验证：clippy exit 0、fmt、include 目标单独 rustfmt、行数守卫绿

## 2. B1 · SC 会话有效视图（谓词 + live/历史同源，3 文件）

> 独占文件：`product/workspace_engine/review/disposition_override.rs`（364/1200，+50～90）· `product/workspace_engine/session_state_parts/historical_review.inc.rs`（148/1200，净 0–5）· `product/workspace_engine/tests/review_disposition_platform.rs`（432/1200，+160～240）。

- [x] 2.1 **W1 红测**：真值表矩阵 = `severity`（Suggestion / must_fix / blocking）× `class_hint`（Repairable / HumanRequired / Advisory）× `disposition`（None / Actionable / Advisory）× category（Completeness / VerificationUnattributable / Other / ContractGap / SelfContradiction / ScopeConflict / **缺失** / **未知**）× verdict（Pass / Revise / NeedsHuman）× 有无 `structured_output_diagnostic` / 有无 `machine_repair` / 有无 `owner_question`
- [x] 2.2 **W2 实现**：私有谓词 `is_sc_non_contract_suggestion(verdict, finding)` + 会话方法 `apply_sc_effective_review_verdict(verdict)`；**不得**命名为 `requires_closure`（序6 已占用该词且为跨作用域全口径）；命中时**仅在内存有效视图**投影为 `Advisory`；severity / message / evidence / action / finding_key / **raw durable 均不改**
- [x] 2.3 **W2 实现（live 与历史同源）**：live 在存原始后用会话视图；历史刷新在原反序列化/历史兼容写回**之后**投影，**不得**把新 `Advisory` 写进 durable；两入口口径一致
- [x] 2.4 **W2 实现（边界）**：契约类 / 缺或未知 category / must_fix·blocking / 机械 Error / 原生 NeedsHuman / 产品问句 / 异常 scope / 状态损坏 / 预算终止 **一律保持原语义**；缺 disposition 时**仅**在命中窄例外时降级，否则维持 fail-closed
- [x] 2.5 **W3–W5**：定向红/绿证据 → 独立只读评审与修正 → 固定提交与回执

## 3. B2 · 前轮 reviewer 投影同源（2 文件）

> 独占文件：`product/workspace_engine/prompts/review_context.rs`（827/1200，−5～+10）· `product/workspace_engine/tests/single_candidate_prompt/review_prompt_d6.rs`（640/1200，+100～180）。

- [x] 3.1 **W1 红测**：历史 raw 先归一再分类；前轮清单的内容身份求交、去重、分区（待办 / 已关闭 / 重复计数）与 D6 既有口径**逐条一致**
- [x] 3.2 **W2 实现**：**先用同一会话视图再 classify**；**保留 D6 内容身份求交**（不得因投影来源改变而改变求交键或分区结果）
- [x] 3.3 **W2 实现（不越界）**：不改 D6 的求交键、去重口径、分区语义；不改 close 的闭合义务判定
- [x] 3.4 **W3–W5**：定向红/绿证据 → 独立只读评审与修正 → 固定提交与回执

## 4. B3 · 路由收口（3 文件，含顶格净零）

> 独占文件：`product/workspace_engine/review/routing.rs`（1367，登记上限 **1391**，≤+10）· `product/workspace_engine/review/routing_scope.rs`（**1200/1200 ⇒ ≤0**）· `product/workspace_engine/review/tests_policy_routing.rs`（987/1200，≤+180）。

- [x] 4.1 **W1 红测**：前后两轮重复集合**同源**；「只建议 revise / needs_human」在 Initial 与 Verification 均**不驱动作者**、不因重复或耗尽而升级人工；真 `must_fix` / 契约类 finding **仍阻断**
- [x] 4.2 **W2 实现**：两侧重复集合使用同一会话视图；**只在有效 review 与 mechanical 均无非 Advisory 义务时**才绕过人工耗尽捷径，仍**继续执行** `evaluate.rs:235-273` 的状态/scope/transition 检查，**不提前 return Pass**、不增加 provider 调用
- [x] 4.3 **W2 实现（耗尽分支不动）**：`routing.rs:503-511` 的 `RepairBudgetExhausted` 分支**一字不改**（`history_delta: Default::default()`）；`:533-549` 的原生 NeedsHuman 覆盖保留；`routing_scope.rs` 的改动**净零**（替换调用并删失效局部变量，**不靠压空行/合语句**）
- [x] 4.4 **W2 实现（第 5 条验收的作用域）**：耗尽 / 诊断 / 真人工上下文中，即使有效 findings 全为 Advisory **也仍 HumanRequired、非静默 pass**；**fresh-cycle 不承诺**非通过态（现有「Revise 全 Advisory ⇒ Pass」「evaluate 空集 ⇒ Valid」保持不变）
- [x] 4.5 **W3–W5**：定向红/绿证据 → 独立只读评审与修正 → 固定提交与回执

## 5. B4 · 作者返修执行清单（2 文件）

> 独占文件：`product/work_item_split_engine/prompts/sc_revision.rs`（185/1200，生产净减）· `product/work_item_split_engine/tests/prompt_contract_sc_revision.rs`（703/1200，+100～180）。

- [x] 5.1 **W1 红测**：作者返修输入中**不含**被窄例外排除的 finding；真必修的 `required_action` / `evidence` **不丢**；散文不再把建议写成「全部要改」
- [x] 5.2 **W2 实现**：作者只消费**有效 structured finding** 作为执行清单；raw 散文保留在原审计面，作者侧仅见非执行计数说明；**不改**返修上下文的构造方式（返修仍以首轮完整 prompt 为底，属未闭合项）
- [x] 5.3 **W3–W5**：定向红/绿证据 → 独立只读评审与修正 → 固定提交与回执

## 6. C1 · 成员事实 renderer（2 文件）

> 独占文件：`product/work_item_split_engine/context.rs`（531/1200，+180～300）· `product/work_item_split_engine/prompts.rs`（1162/1200，**≤0**）。`prompts.rs` 的四个 legacy 调用**不改**。

- [x] 6.1 **W1 红测**：>12 点文件且成员仓名**字典序靠后**、12 成员 × 6 子目录、40 行耗尽、多字节长前缀、根成员为 `.`、嵌套成员、成员不可读 ⇒ 3,200 B 内**所有成员身份头都在**、无半行、无非成员冒充
- [x] 6.2 **W2 实现**：新增 **SC 专用**成员感知装配/renderer（新接口，非现有符号）；**保留**旧 `summarize_repository_structure(path, logical)` 供四个 legacy 调用（**单仓路径逐字不变**）；只消费本轮 `member_targets` 与**同源**前缀交集（`draft_batch/member_checkout_prefixes::plan_member_checkout_prefixes`；**alias 不得当作已验证 checkout**）
- [x] 6.3 **W2 实现（保底与显式失败）**：**所有成员身份头先保留**，再轮转每仓子项；12 项 / 40 行**只约束可轮转细节**；按**完整 UTF-8 行**装配（上层 budget 函数**不再切成半行**）；全部身份头自身超限时显式 `member_inventory_budget_exceeded` + 总数/所需字节 ⇒ **不假称全员在场**、**不新增模型调用**、**不让作者轮失败**；缺权威映射**显式失败、绝不猜目录**
- [x] 6.4 **W2 实现（事实边界）**：可选事实只含权威成员相对根、`member_reality::detect_entrypoints` 探到的**根下清单文件存在性**、一级目录；**不采纳** package.json ⇒ npm test 等启发式命令；**不新增**源码/README/环境探测；**不把「构建入口文件」吹成业务源入口或可执行命令**；首轮固定纪律句**等长或净减**替换为「已给事实不重复全仓探测；缺事实才针对性查证」（**不禁止必要查证、不禁止产出真实验证命令**）
- [x] 6.5 **W3–W5**：定向红/绿证据 → 独立只读评审与修正 → 固定提交与回执

## 7. C2 · 首轮实际接线 + A1 审计接入（2 文件，顶格净零）

> 独占文件：`web/workspace_ws_handler/run/single_candidate.rs`（**1200/1200 ⇒ ≤0**）· `web/workspace_ws_handler/tests/single_candidate_provider_run/aggregate_ir_target.rs`（238，**已存在**，+120～200；**不新建测试模块、不触父测试入口**）。

- [x] 7.1 **W1 红测**：捕获**真实 provider 输入** ⇒ 首轮 prompt 的 `structure:` 内含全部成员身份头、预算不越界、**无额外 provider 启动**；单仓路径逐字不变
- [x] 7.2 **W2 实现**：聚合根取 **`lc_plan_launch.working_dir`**（**不是** `repository.path` —— focus 可能是成员 checkout）；前缀复用 `plan_member_checkout_prefixes`；旧摘要分支换专用 helper；**runner 改动净零**（不靠压空行/合语句）
- [x] 7.3 **W2 实现（A1 审计接入）**：在 `run/single_candidate.rs:850-874` 附近接入 A1 的审计写入（该处同已有完整 `diagnostics` + `payload` + `reredrive_prompt` + 记录调用）；**净零**；**审计组装只落 A1 的 `single_candidate_structure_reredrive.rs`，runner 只透传既有 `&diagnostics`（不得把审计拼装写进 runner，否则破线）**；审计**不进 prompt**、不改窗口/启动/schema；**A1 单独交付时不可声称「可完整取回」，C2 接入后才成立**
- [x] 7.4 **W3–W5**：定向红/绿证据 → 独立只读评审与修正 → 固定提交与回执

## 8. D1 · reviewer 无损紧凑序列化（2 文件）

> 独占文件：`product/workspace_engine/prompts/review_parts/single_candidate_input.inc.rs`（319/1200，+10～20；**include 目标，单独 rustfmt**）· `product/workspace_engine/tests/single_candidate_prompt/review_prompt.rs`（648/1200，+100～180）。

- [x] 8.1 **W1 红测**：同一真实 IR/report/structure fixture，逐段 JSON 解析后 compact 与原 pretty 对象 **deep-equal**；标题、refs、nonce/scope 事实完整；字节数**严格减少**且不超既有上限
- [x] 8.2 **W2 实现**：在 SC include 目标内增加**局部** compact 序列化 helper 并替换 SC 本体 JSON 段输出；**保留** section 顺序、所有键值、数组顺序、绑定 refs、结构审计与 D6；**不改**通用 pretty helper（不与 B2 共写 `review_context.rs`）；**Cross-Item 段及其值完整保留**（今天不删）
- [x] 8.3 **W2 实现（边界）**：**不产生第二个全局序列化策略**；绑定不匹配仍拒发；**只保证字节下降，不承诺 token 同比或质量必升**
- [x] 8.4 **W3–W5**：定向红/绿证据 → 独立只读评审与修正 → 固定提交与回执

## 9. 汇合与验收（controller）

- [x] 9.1 **文件集与行数机械核对**：每 Task 提交文件集合 ⊆ 其声明闭集；顶格文件净零；`large_file_guard` 登记表 **diff 为空**；8 个 effort WIP 文件 **diff 为空**；逐文件实测行数表
- [x] 9.2 **完整门禁链（静止树上）**：`cargo fmt --all --check` · 本次触及 include 目标**逐个** `rustfmt --check` · `cargo clippy --all-targets --all-features --locked -- -D warnings`（**exit 0 / 0 error**）· `cargo check --locked` · `cargo test --locked --lib` · `cargo test --locked --test it_core` · `cargo test --locked --test it_web` · `cargo test --locked`
- [x] 9.3 **整分支终审**：逐条核契约场景覆盖 / 核心不变量 / 断言强度 / 公开面 / 验收记录可信度 ⇒ 出「Ready for the next link?」；**修正后必须重取最终绿色证据**
- [x] 9.4 **验收记录**：真实命令输出摘要 + 逐文件行数实测表 + 基线对照 + **未闭合项逐条登记（事实 → 现状 → 归谁 → 建议落点）**（含 design §9 的 11 条 + 8.12 脚手架拆除的 owner 例外裁决）
- [x] 9.5 **归档**：本 change 以 `openspec archive repair-wig-fact-projections -y`（**带 spec 同步**）归档；归档后 `openspec validate --specs --strict` 全绿；主规格三个 requirement 的描述段实测 ≤500
- [ ] 9.6 **真机（明日）**：8 次串行、同装置、固定二进制与配置、不边改边测；记录 launch→首个 provider 事件 / 首个可见 token / 首个正文 token、探索工具数与探索墙钟、author/reviewer 的 input/output/cache-read、教学 attempt、review 轮次、confirmed/failed/human 与失败码。**8 次不足以证一般性改善 ⇒ 只报方向性观察与样本量**

---

## 本轮实施与门禁证据回填（2026-11-13）

- **逐 Task 独立只读评审**：A1 / B1 / B2 / B3 / B4 / C1 / C2 / D1 共 8 轮全部完成，各轮结论与行号级证据见
  `cadence/notes/2026-11-13_验收记录_WIG事实投影修复_v1.0.md` §⑤；其中 D1 首轮为「不可合并」（Refs 段重复发射 + 断言首现盲区），
  修正后复核通过。
- **整分支终审**（read-only，pinned diff `a953327d..a64feb03`）：跨 Task 语义、契约场景覆盖（主规格 REQ-WSC-06 34 场景 +
  REQ-WSC-12 5 场景零漂移）、核心不变量、公开面**全部核清且干净**；唯 Critical 1（记账行改写后 C2 e2e 搜索串过时）已修，
  Important 1（勾选对账）即本节。
- **静止树门禁**（实测）：`cargo fmt --all --check` exit 0；3 个 include 目标逐个 `rustfmt --check` exit 0；
  `cargo clippy --all-targets --all-features --locked -- -D warnings` exit 0 / 0 error；
  `cargo check --locked --all-targets` 0 error；`cargo test --locked --test it_core` 221 passed / 0 failed。
  `--lib` / `--test it_web` / 全量三轮结果见验收记录 §③。
- **门禁暴露并修复的 6 处失败**（门禁有效性证据，根因与处置见验收记录 §⑥）：A1 事实块缺逐条条数界（真契约违反，已改双重界）、
  B3 断言过粗（已收窄到真实升级面，不删断言）、C1 两处夹具错（渲染器零改动）、C2 e2e 搜索串过时（测试侧一行）、
  clippy 6 处机械错。
- **未闭合项**：20 条，逐条按「事实 → 现状 → 归谁 → 建议落点」登记于验收记录 §⑦；**真机（9.6）与归档（9.5）尚未执行**，
  归 owner / 集成 owner。
